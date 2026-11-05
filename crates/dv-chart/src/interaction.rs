use std::collections::{BTreeMap, BTreeSet, HashMap};

use egui::{CursorIcon, Key, Modifiers, PointerButton, Pos2, Rect, Response, Ui, Vec2};

use crate::{
    render::{valid_range, Axis, Layout, PaneLayout, SeriesCache},
    ChartConfig, ChartState, Click, Context, Pane, PaneAction, PaneId, Scale, SeriesId, Tool,
};

#[derive(Clone, Debug, Default, PartialEq)]
struct View {
    x: Option<[f64; 2]>,
    pane_x: BTreeMap<PaneId, [f64; 2]>,
    y: BTreeMap<PaneId, [f64; 2]>,
}
impl View {
    fn capture(state: &ChartState) -> Self {
        Self {
            x: state.x_view,
            pane_x: state.pane_x_views.clone(),
            y: state.y_views.clone(),
        }
    }
    fn restore(&self, state: &mut ChartState) {
        state.x_view = self.x;
        state.pane_x_views.clone_from(&self.pane_x);
        state.y_views.clone_from(&self.y);
    }
}
#[derive(Clone, Default)]
struct History {
    views: Vec<View>,
    current: usize,
}
impl History {
    fn push(&mut self, before: View, after: View) {
        if before == after {
            return;
        }
        if self.views.is_empty() {
            self.views.push(before.clone());
            self.current = 0;
        }
        if self.views[self.current] != before {
            self.views.truncate(self.current + 1);
            self.views.push(before);
            self.current += 1;
        }
        self.views.truncate(self.current + 1);
        self.views.push(after);
        if self.views.len() > 100 {
            self.views.remove(0);
        }
        self.current = self.views.len() - 1;
    }
    fn back(&mut self) -> Option<View> {
        if self.current > 0 {
            self.current -= 1;
            self.views.get(self.current).cloned()
        } else {
            None
        }
    }
    fn forward(&mut self) -> Option<View> {
        if self.current + 1 < self.views.len() {
            self.current += 1;
            self.views.get(self.current).cloned()
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionHit {
    Left,
    Right,
    Inside,
    Outside,
}
fn selection_hit(selection: Option<[f64; 2]>, x: Axis, rect: Rect, pos: Pos2) -> SelectionHit {
    let Some([a, b]) = selection else {
        return SelectionHit::Outside;
    };
    let Some((a, b)) = x.x(rect, a).zip(x.x(rect, b)) else {
        return SelectionHit::Outside;
    };
    let (left, right) = (a.min(b), a.max(b));
    if (pos.x - left).abs() <= 6.0 {
        SelectionHit::Left
    } else if (pos.x - right).abs() <= 6.0 {
        SelectionHit::Right
    } else if pos.x > left && pos.x < right {
        SelectionHit::Inside
    } else {
        SelectionHit::Outside
    }
}
#[derive(Clone, Copy)]
enum OverviewDrag {
    Move,
    Left,
    Right,
}
#[derive(Clone)]
enum Kind {
    Select(SelectionHit),
    Pan,
    BoxZoom,
    Divider {
        above: PaneId,
        below: PaneId,
        a: f32,
        b: f32,
    },
    Overview(OverviewDrag),
    Legend(SeriesId),
    Action(PaneAction),
}
#[derive(Clone)]
struct Drag {
    kind: Kind,
    button: PointerButton,
    origin: Pos2,
    current: Pos2,
    pane: PaneId,
    plot: Rect,
    x: Axis,
    y: Axis,
    selection: Option<[f64; 2]>,
    before: View,
    moved: bool,
}
#[derive(Clone, Default)]
pub(crate) struct Runtime {
    pub caches: HashMap<SeriesId, SeriesCache>,
    pub external_cursor: Option<f64>,
    history: History,
    drag: Option<Drag>,
    solo: Option<(SeriesId, BTreeSet<SeriesId>)>,
}
impl Runtime {
    pub fn box_rect(&self) -> Option<Rect> {
        self.drag.as_ref().and_then(|d| {
            matches!(d.kind, Kind::BoxZoom)
                .then(|| Rect::from_two_pos(d.origin, d.current).intersect(d.plot))
        })
    }
}
#[derive(Default)]
pub(crate) struct Events {
    pub clicked: Option<Click>,
    pub context: Option<Context>,
    pub selection_changed: bool,
    pub view_changed: bool,
    pub legend_clicked: Option<SeriesId>,
    pub pane_action: Option<PaneAction>,
}
fn normalized(a: f64, b: f64) -> [f64; 2] {
    [a.min(b), a.max(b)]
}
fn set_x(state: &mut ChartState, config: &ChartConfig<'_>, pane: PaneId, range: [f64; 2]) {
    if !valid_range(range, config.x_scale) {
        return;
    }
    if config.link_x {
        state.x_view = Some(range);
    } else {
        state.pane_x_views.insert(pane, range);
    }
}
fn set_y(state: &mut ChartState, pane: PaneId, range: [f64; 2], scale: Scale) {
    if valid_range(range, scale) {
        state.y_views.insert(pane, range);
    }
}
fn shift(axis: Axis, delta: f64) -> [f64; 2] {
    normalized(axis.value(-delta), axis.value(1.0 - delta))
}
fn zoom(axis: Axis, center: f64, factor: f64) -> [f64; 2] {
    let factor = factor.clamp(0.001, 1000.0);
    normalized(
        axis.value(center * (1.0 - factor)),
        axis.value(center + (1.0 - center) * factor),
    )
}
fn plot_at(layout: &Layout, pos: Pos2) -> Option<&PaneLayout> {
    layout.panes.iter().find(|p| {
        pos.y >= p.plot.top()
            && pos.y <= p.plot.bottom()
            && pos.x >= layout.rect.left()
            && pos.x <= p.plot.right()
    })
}
fn legend_at(layout: &Layout, pos: Pos2) -> Option<SeriesId> {
    layout
        .panes
        .iter()
        .flat_map(|p| &p.legend)
        .find(|entry| entry.rect.contains(pos))
        .map(|entry| entry.id)
}
fn action_at(layout: &Layout, pos: Pos2) -> Option<PaneAction> {
    layout
        .panes
        .iter()
        .flat_map(|p| &p.actions)
        .find(|entry| entry.rect.contains(pos))
        .map(|entry| entry.action)
}
fn click_at(layout: &Layout, pos: Pos2) -> Option<Click> {
    let pane = plot_at(layout, pos)?;
    Some(Click {
        pane: Some(pane.id),
        series: legend_at(layout, pos),
        x: pane.x.at_x(pane.plot, pos.x),
        y: pane.y.at_y(pane.plot, pos.y),
    })
}
fn push_history(state: &mut ChartState, before: View) -> bool {
    let after = View::capture(state);
    if before == after {
        return false;
    }
    state.runtime.history.push(before, after);
    true
}
fn with_history(state: &mut ChartState, action: impl FnOnce(&mut ChartState)) -> bool {
    let before = View::capture(state);
    action(state);
    push_history(state, before)
}
fn drag_start(
    kind: Kind,
    button: PointerButton,
    pos: Pos2,
    pane: &PaneLayout,
    state: &ChartState,
) -> Drag {
    Drag {
        kind,
        button,
        origin: pos,
        current: pos,
        pane: pane.id,
        plot: pane.plot,
        x: pane.x,
        y: pane.y,
        selection: state.selection,
        before: View::capture(state),
        moved: false,
    }
}
fn move_selection(selection: [f64; 2], axis: Axis, from: f64, to: f64) -> [f64; 2] {
    if axis.scale == Scale::Log10 && from > 0.0 && to > 0.0 {
        let ratio = to / from;
        normalized(selection[0] * ratio, selection[1] * ratio)
    } else {
        normalized(selection[0] + to - from, selection[1] + to - from)
    }
}
fn update_drag(
    state: &mut ChartState,
    config: &ChartConfig<'_>,
    drag: &mut Drag,
    pos: Pos2,
    shift_held: bool,
    events: &mut Events,
) {
    drag.current = pos;
    if pos.distance(drag.origin) >= 3.0 {
        drag.moved = true;
    }
    if !drag.moved {
        return;
    }
    match drag.kind {
        Kind::Select(hit) => {
            let x0 = drag.x.at_x(
                drag.plot,
                drag.origin.x.clamp(drag.plot.left(), drag.plot.right()),
            );
            let x1 = drag
                .x
                .at_x(drag.plot, pos.x.clamp(drag.plot.left(), drag.plot.right()));
            let selection = match (hit, drag.selection) {
                (SelectionHit::Inside, Some(s)) => move_selection(s, drag.x, x0, x1),
                (SelectionHit::Left, Some(s)) => {
                    let last = if drag.x.flip { s[0] } else { s[1] };
                    normalized(x1, last)
                }
                (SelectionHit::Right, Some(s)) => {
                    let first = if drag.x.flip { s[1] } else { s[0] };
                    normalized(first, x1)
                }
                _ => normalized(x0, x1),
            };
            if state.selection != Some(selection) {
                state.selection = Some(selection);
                events.selection_changed = true;
            }
        }
        Kind::Pan => {
            let dx = f64::from((pos.x - drag.origin.x) / drag.plot.width().max(1.0));
            let dy = f64::from((pos.y - drag.origin.y) / drag.plot.height().max(1.0));
            set_x(state, config, drag.pane, shift(drag.x, dx));
            if (shift_held || drag.origin.x < drag.plot.left()) && dy != 0.0 {
                set_y(state, drag.pane, shift(drag.y, -dy), drag.y.scale);
            }
            events.view_changed = true;
        }
        Kind::BoxZoom => {}
        Kind::Divider { above, below, a, b } => {
            let change = pos.y - drag.origin.y;
            let minimum = 35.0_f32.min((a + b) * 0.5);
            let new_a = (a + change).clamp(minimum, a + b - minimum);
            state.pane_weights.insert(above, new_a);
            state.pane_weights.insert(below, a + b - new_a);
        }
        Kind::Overview(part) => {
            let from = drag.x.at_x(drag.plot, drag.origin.x);
            let to = drag.x.at_x(drag.plot, pos.x);
            let range = match part {
                OverviewDrag::Move => {
                    move_selection(drag.selection.unwrap_or(drag.x.range), drag.x, from, to)
                }
                OverviewDrag::Left => {
                    let s = drag.selection.unwrap_or(drag.x.range);
                    normalized(to, if drag.x.flip { s[0] } else { s[1] })
                }
                OverviewDrag::Right => {
                    let s = drag.selection.unwrap_or(drag.x.range);
                    normalized(if drag.x.flip { s[1] } else { s[0] }, to)
                }
            };
            set_x(state, config, drag.pane, range);
            events.view_changed = true;
        }
        Kind::Legend(_) | Kind::Action(_) => {}
    }
}

pub(crate) fn handle(
    ui: &mut Ui,
    state: &mut ChartState,
    panes: &[Pane<'_>],
    config: &ChartConfig<'_>,
    layout: &Layout,
    response: &Response,
) -> Events {
    let mut events = Events::default();
    let ctx = ui.ctx();
    let (pos, pressed, released, mods, scroll, pinch) = ctx.input(|i| {
        (
            i.pointer.interact_pos(),
            [
                PointerButton::Primary,
                PointerButton::Secondary,
                PointerButton::Middle,
            ]
            .into_iter()
            .find(|b| i.pointer.button_pressed(*b)),
            [
                PointerButton::Primary,
                PointerButton::Secondary,
                PointerButton::Middle,
            ]
            .into_iter()
            .find(|b| i.pointer.button_released(*b)),
            i.modifiers,
            i.smooth_scroll_delta(),
            i.zoom_delta(),
        )
    });
    let pointer = ctx.input(|i| i.pointer.hover_pos()).or(pos);
    state.highlighted = pointer
        .filter(|p| action_at(layout, *p).is_none())
        .and_then(|p| legend_at(layout, p))
        .filter(|id| {
            !state.hidden.contains(id)
                && panes
                    .iter()
                    .flat_map(|pane| &pane.series)
                    .any(|series| series.id == *id && series.visible)
        });
    if let Some(pos) = pointer.filter(|_| response.hovered()) {
        let cursor = if state
            .runtime
            .drag
            .as_ref()
            .is_some_and(|d| matches!(d.kind, Kind::Pan | Kind::Overview(OverviewDrag::Move)))
        {
            CursorIcon::Grabbing
        } else if layout.dividers.iter().any(|r| r.expand(3.0).contains(pos)) {
            CursorIcon::ResizeVertical
        } else if action_at(layout, pos).is_some() || legend_at(layout, pos).is_some() {
            CursorIcon::PointingHand
        } else if let Some(pl) = plot_at(layout, pos) {
            if state.tool == Tool::Select
                && matches!(
                    selection_hit(state.selection, pl.x, pl.plot, pos),
                    SelectionHit::Left | SelectionHit::Right
                )
            {
                CursorIcon::ResizeHorizontal
            } else {
                match state.tool {
                    Tool::Select => CursorIcon::Crosshair,
                    Tool::Pan => CursorIcon::Grab,
                    Tool::Zoom => CursorIcon::ZoomIn,
                }
            }
        } else {
            CursorIcon::Default
        };
        ctx.set_cursor_icon(cursor);
    }
    if response.double_clicked() {
        if let Some(pos) = pos {
            if let Some(index) = layout
                .dividers
                .iter()
                .position(|r| r.expand(3.0).contains(pos))
            {
                if let (Some(a), Some(b)) = (layout.panes.get(index), layout.panes.get(index + 1)) {
                    for p in &layout.panes {
                        state.pane_weights.insert(p.id, p.plot.height());
                    }
                    let mean = (a.plot.height() + b.plot.height()) * 0.5;
                    state.pane_weights.insert(a.id, mean);
                    state.pane_weights.insert(b.id, mean);
                }
            } else if let Some(pl) = plot_at(layout, pos) {
                if pos.x < pl.plot.left() {
                    events.view_changed |= with_history(state, |s| {
                        s.y_views.remove(&pl.id);
                    });
                } else if selection_hit(state.selection, pl.x, pl.plot, pos)
                    != SelectionHit::Outside
                {
                    if let Some(selection) = state.selection {
                        events.view_changed |= with_history(state, |s| {
                            set_x(s, config, pl.id, normalized(selection[0], selection[1]))
                        });
                    }
                } else {
                    events.view_changed |= with_history(state, ChartState::fit_all);
                }
            } else {
                events.view_changed |= with_history(state, ChartState::fit_all);
            }
            state.runtime.drag = None;
            response.request_focus();
        }
    } else if let (Some(button), Some(pos)) = (pressed, pos) {
        if response.hovered() && layout.rect.contains(pos) {
            response.request_focus();
            let pane = plot_at(layout, pos).or_else(|| layout.panes.last());
            if let Some(pl) = pane {
                let mut drag = if button == PointerButton::Primary {
                    if let Some(index) = layout
                        .dividers
                        .iter()
                        .position(|r| r.expand(3.0).contains(pos))
                    {
                        if let (Some(a), Some(b)) =
                            (layout.panes.get(index), layout.panes.get(index + 1))
                        {
                            for p in &layout.panes {
                                state.pane_weights.insert(p.id, p.plot.height());
                            }
                            drag_start(
                                Kind::Divider {
                                    above: a.id,
                                    below: b.id,
                                    a: a.plot.height(),
                                    b: b.plot.height(),
                                },
                                button,
                                pos,
                                pl,
                                state,
                            )
                        } else {
                            drag_start(Kind::Pan, button, pos, pl, state)
                        }
                    } else if let Some(action) = action_at(layout, pos) {
                        drag_start(Kind::Action(action), button, pos, pl, state)
                    } else if let Some(id) = legend_at(layout, pos) {
                        drag_start(Kind::Legend(id), button, pos, pl, state)
                    } else if let Some(overview) = layout.overview.filter(|r| r.contains(pos)) {
                        let full = Axis {
                            range: if config.link_x {
                                layout.full_x
                            } else {
                                layout.panes.first().map_or(layout.full_x, |p| p.full_x)
                            },
                            scale: config.x_scale,
                            flip: config.x_flip,
                        };
                        let view = layout.panes.first().map_or(full.range, |p| p.x.range);
                        let hit = selection_hit(Some(view), full, overview, pos);
                        let kind = match hit {
                            SelectionHit::Left => OverviewDrag::Left,
                            SelectionHit::Right => OverviewDrag::Right,
                            _ => OverviewDrag::Move,
                        };
                        let mut drag = drag_start(Kind::Overview(kind), button, pos, pl, state);
                        drag.plot = overview;
                        drag.x = full;
                        drag.selection = Some(view);
                        drag
                    } else if mods.ctrl || state.tool == Tool::Zoom {
                        drag_start(Kind::BoxZoom, button, pos, pl, state)
                    } else if state.tool == Tool::Pan {
                        drag_start(Kind::Pan, button, pos, pl, state)
                    } else {
                        drag_start(
                            Kind::Select(selection_hit(state.selection, pl.x, pl.plot, pos)),
                            button,
                            pos,
                            pl,
                            state,
                        )
                    }
                } else {
                    drag_start(Kind::Pan, button, pos, pl, state)
                };
                drag.current = pos;
                state.runtime.drag = Some(drag);
            }
        }
    }
    if let Some(mut drag) = state.runtime.drag.take() {
        if let Some(pos) = pos {
            update_drag(state, config, &mut drag, pos, mods.shift, &mut events);
        }
        if released == Some(drag.button) {
            if drag.moved {
                if matches!(drag.kind, Kind::BoxZoom) {
                    let a = drag.origin.clamp(drag.plot.min, drag.plot.max);
                    let b = drag.current.clamp(drag.plot.min, drag.plot.max);
                    if (a.x - b.x).abs() >= 3.0 && (a.y - b.y).abs() >= 3.0 {
                        set_x(
                            state,
                            config,
                            drag.pane,
                            normalized(drag.x.at_x(drag.plot, a.x), drag.x.at_x(drag.plot, b.x)),
                        );
                        set_y(
                            state,
                            drag.pane,
                            normalized(drag.y.at_y(drag.plot, a.y), drag.y.at_y(drag.plot, b.y)),
                            drag.y.scale,
                        );
                    }
                }
                events.view_changed |= push_history(state, drag.before);
            } else if drag.button == PointerButton::Primary {
                match drag.kind {
                    Kind::Legend(id) => {
                        events.legend_clicked = Some(id);
                        if mods.alt {
                            if state
                                .runtime
                                .solo
                                .as_ref()
                                .is_some_and(|(solo, _)| *solo == id)
                            {
                                if let Some((_, previous)) = state.runtime.solo.take() {
                                    state.hidden = previous;
                                }
                            } else {
                                let previous = state
                                    .runtime
                                    .solo
                                    .take()
                                    .map_or_else(|| state.hidden.clone(), |(_, previous)| previous);
                                state.hidden = previous.clone();
                                state.hidden.remove(&id);
                                for series in panes.iter().flat_map(|p| &p.series) {
                                    if series.id != id && series.visible {
                                        state.hidden.insert(series.id);
                                    }
                                }
                                state.runtime.solo = Some((id, previous));
                            }
                        } else {
                            state.runtime.solo = None;
                            if !state.hidden.remove(&id) {
                                state.hidden.insert(id);
                            }
                        }
                    }
                    Kind::Action(action) => {
                        events.pane_action = Some(action);
                        match action {
                            PaneAction::AutoY(id) => {
                                events.view_changed |= with_history(state, |s| {
                                    s.y_views.remove(&id);
                                });
                            }
                            PaneAction::ToggleLockY(id) => {
                                events.view_changed |= with_history(state, |s| {
                                    if s.y_views.remove(&id).is_none() {
                                        s.y_views.insert(id, drag.y.range);
                                    }
                                });
                            }
                            PaneAction::ToggleMaximize(id) => {
                                state.maximized = if state.maximized == Some(id) {
                                    None
                                } else {
                                    Some(id)
                                };
                                events.view_changed = true;
                            }
                            _ => {}
                        }
                    }
                    Kind::Select(SelectionHit::Outside) => {
                        if state.selection.take().is_some() {
                            events.selection_changed = true;
                        }
                    }
                    _ => {}
                }
                if !matches!(drag.kind, Kind::Legend(_) | Kind::Action(_)) {
                    events.clicked = click_at(layout, drag.current);
                }
            } else if drag.button == PointerButton::Secondary {
                events.context = click_at(layout, drag.current).map(|c| Context {
                    pane: c.pane,
                    series: c.series,
                    x: c.x,
                    y: c.y,
                });
            }
        } else {
            state.runtime.drag = Some(drag);
        }
    }
    if let Some(pos) = pointer.filter(|_| response.hovered()) {
        if scroll != Vec2::ZERO || (pinch - 1.0).abs() > 0.0001 {
            let pl = plot_at(layout, pos).or_else(|| layout.panes.last());
            if let Some(pl) = pl {
                let before = View::capture(state);
                let horizontal =
                    mods.shift || (scroll.x.abs() > scroll.y.abs() && scroll.x.abs() > 0.0);
                let over_y =
                    pos.x < pl.plot.left() && pos.y >= pl.plot.top() && pos.y <= pl.plot.bottom();
                let over_x = layout.x_axis.contains(pos);
                if horizontal && !over_y && !mods.ctrl {
                    let d = if scroll.x.abs() > 0.0 {
                        scroll.x
                    } else {
                        scroll.y
                    };
                    set_x(
                        state,
                        config,
                        pl.id,
                        shift(pl.x, f64::from(d / pl.plot.width().max(1.0))),
                    );
                } else {
                    let factor = if (pinch - 1.0).abs() > 0.0001 {
                        1.0 / f64::from(pinch)
                    } else {
                        (-f64::from(scroll.y) * 0.0025).exp()
                    };
                    if (mods.ctrl || over_y) && !over_x {
                        let center =
                            f64::from((pl.plot.bottom() - pos.y) / pl.plot.height().max(1.0));
                        set_y(state, pl.id, zoom(pl.y, center, factor), pl.y.scale);
                    } else {
                        let center = f64::from((pos.x - pl.plot.left()) / pl.plot.width().max(1.0));
                        set_x(state, config, pl.id, zoom(pl.x, center, factor));
                    }
                }
                events.view_changed |= push_history(state, before);
                ctx.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            }
        }
    }
    if (response.hovered() || response.has_focus()) && !ctx.text_edit_focused() {
        let mut handled = false;
        let pl = pointer
            .and_then(|p| plot_at(layout, p))
            .or_else(|| layout.panes.first());
        let key = |key| ctx.input(|i| i.key_pressed(key));
        if mods.alt && (key(Key::ArrowLeft) || key(Key::ArrowRight)) {
            let view = if key(Key::ArrowLeft) {
                state.runtime.history.back()
            } else {
                state.runtime.history.forward()
            };
            if let Some(view) = view {
                view.restore(state);
                events.view_changed = true;
            }
            handled = true;
        } else if key(Key::Escape) {
            if state.selection.take().is_some() {
                events.selection_changed = true;
            }
            state.runtime.drag = None;
            state.tool = Tool::Select;
            handled = true;
        } else if key(Key::Home) || key(Key::Num0) {
            events.view_changed |= with_history(state, ChartState::fit_all);
            handled = true;
        } else if let Some(pl) = pl {
            let before = View::capture(state);
            if key(Key::ArrowLeft) {
                set_x(state, config, pl.id, shift(pl.x, 0.1));
                handled = true;
            } else if key(Key::ArrowRight) {
                set_x(state, config, pl.id, shift(pl.x, -0.1));
                handled = true;
            } else if key(Key::Plus) || key(Key::Equals) {
                set_x(state, config, pl.id, zoom(pl.x, 0.5, 0.8));
                handled = true;
            } else if key(Key::Minus) {
                set_x(state, config, pl.id, zoom(pl.x, 0.5, 1.25));
                handled = true;
            }
            if handled {
                events.view_changed |= push_history(state, before);
            }
        }
        if handled {
            ctx.input_mut(|i| {
                for key in [
                    Key::ArrowLeft,
                    Key::ArrowRight,
                    Key::Home,
                    Key::Num0,
                    Key::Escape,
                    Key::Plus,
                    Key::Equals,
                    Key::Minus,
                ] {
                    i.consume_key(Modifiers::NONE, key);
                    i.consume_key(Modifiers::ALT, key);
                }
            });
        }
    }
    if events.view_changed || events.selection_changed || state.runtime.drag.is_some() {
        ctx.request_repaint();
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_changes_y_only_with_shift_or_from_the_gutter() {
        let plot = Rect::from_min_max(Pos2::new(20.0, 0.0), Pos2::new(120.0, 100.0));
        let axis = Axis {
            range: [0.0, 100.0],
            scale: Scale::Linear,
            flip: false,
        };
        for (origin, shift_held, expect_y) in [
            (Pos2::new(50.0, 50.0), false, false),
            (Pos2::new(50.0, 50.0), true, true),
            (Pos2::new(10.0, 50.0), false, true),
        ] {
            let mut state = ChartState::default();
            let mut drag = Drag {
                kind: Kind::Pan,
                button: PointerButton::Secondary,
                origin,
                current: origin,
                pane: 1,
                plot,
                x: axis,
                y: axis,
                selection: None,
                before: View::default(),
                moved: false,
            };
            let mut events = Events::default();
            update_drag(
                &mut state,
                &ChartConfig::default(),
                &mut drag,
                origin + Vec2::new(10.0, 20.0),
                shift_held,
                &mut events,
            );
            assert!(state.x_view.is_some());
            assert_eq!(state.y_views.contains_key(&1), expect_y);
        }
    }
    #[test]
    fn selection_hit_edges_and_reversed_axis() {
        let rect = Rect::from_min_max(Pos2::new(20.0, 0.0), Pos2::new(120.0, 40.0));
        let axis = Axis {
            range: [0.0, 100.0],
            scale: Scale::Linear,
            flip: false,
        };
        assert_eq!(
            selection_hit(Some([20.0, 80.0]), axis, rect, Pos2::new(41.0, 10.0)),
            SelectionHit::Left
        );
        assert_eq!(
            selection_hit(Some([20.0, 80.0]), axis, rect, Pos2::new(99.0, 10.0)),
            SelectionHit::Right
        );
        assert_eq!(
            selection_hit(Some([20.0, 80.0]), axis, rect, Pos2::new(60.0, 10.0)),
            SelectionHit::Inside
        );
        assert_eq!(
            selection_hit(Some([20.0, 80.0]), axis, rect, Pos2::new(25.0, 10.0)),
            SelectionHit::Outside
        );
        let reversed = Axis { flip: true, ..axis };
        assert_eq!(
            selection_hit(Some([20.0, 80.0]), reversed, rect, Pos2::new(41.0, 10.0)),
            SelectionHit::Left
        );
    }
    #[test]
    fn history_branches_and_restores() {
        let a = View::default();
        let b = View {
            x: Some([0.0, 10.0]),
            ..View::default()
        };
        let c = View {
            x: Some([1.0, 5.0]),
            ..View::default()
        };
        let d = View {
            x: Some([3.0, 4.0]),
            ..View::default()
        };
        let mut h = History::default();
        h.push(a.clone(), b.clone());
        h.push(b.clone(), c.clone());
        assert_eq!(h.back(), Some(b.clone()));
        assert_eq!(h.back(), Some(a.clone()));
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(b.clone()));
        h.push(b, d.clone());
        assert_eq!(h.forward(), None);
        assert_eq!(
            h.back(),
            Some(View {
                x: Some([0.0, 10.0]),
                ..View::default()
            })
        );
        assert_eq!(h.forward(), Some(d));
    }
    #[test]
    fn zoom_anchors_and_log_pan_preserve_ratios() {
        let linear = Axis {
            range: [0.0, 100.0],
            scale: Scale::Linear,
            flip: false,
        };
        assert_eq!(zoom(linear, 0.25, 0.5), [12.5, 62.5]);
        let log = Axis {
            range: [1.0, 100.0],
            scale: Scale::Log10,
            flip: false,
        };
        let shifted = shift(log, 0.5);
        assert!((shifted[0] - 0.1).abs() < 1e-12 && (shifted[1] - 10.0).abs() < 1e-12);
    }
}
