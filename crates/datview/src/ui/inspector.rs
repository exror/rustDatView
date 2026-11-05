//! Editable presentation controls for the current chart.
use dv_chart::{format, ChartState};
use eframe::egui::{self, RichText, Stroke};

use super::{chart_view, icons, theme, widgets};
use crate::{
    panes::{Annotation, FilterMode, LineDraw, PaneGrouping, PaneSettings, RangeRule, SeriesStyle},
    settings::Settings,
    workspace::{Mode, Workspace},
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Series,
    Marks,
    Filters,
}

#[derive(Default)]
pub(crate) struct Inspector {
    tab: Tab,
    new_pane: String,
}

/// Marks use a neutral slate by default: visible on dark and light figures
/// and never mistaken for a series colour.
pub(crate) const MARK_COLOR: [u8; 3] = [140, 150, 168];

impl Inspector {
    /// Returns true when the panel's close button is pressed.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &mut Workspace,
        chart: &mut ChartState,
        settings: &Settings,
        options: &chart_view::ChartOptions,
        cursor: Option<f64>,
    ) -> bool {
        let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
        let mut close = false;
        ui.horizontal(|ui| {
            widgets::section_label(ui, "Inspector");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = widgets::icon_button(ui, icons::CLOSE, "Close inspector · Ctrl+I").clicked();
            });
        });
        ui.add_space(6.0);
        if let Some(tab) = widgets::segmented(
            ui,
            self.tab,
            &[
                (Tab::Series, "Series", "Style and arrange plotted series"),
                (Tab::Marks, "Marks", "Annotations and reference lines"),
                (Tab::Filters, "Filters", "Hide samples outside chosen ranges without changing the data"),
            ],
        ) {
            self.tab = tab;
        }
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt("inspector-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let rebuild = match self.tab {
                    Tab::Series => self.series(ui, workspace, chart, settings, tokens),
                    Tab::Marks => {
                        marks(ui, workspace, chart, options, cursor, tokens);
                        false
                    }
                    Tab::Filters => rules(ui, workspace, tokens),
                };
                if rebuild {
                    workspace.rebuild();
                }
            });
        close
    }

    fn series(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &mut Workspace,
        chart: &mut ChartState,
        settings: &Settings,
        tokens: theme::Tokens,
    ) -> bool {
        if workspace.series.is_empty() && workspace.panes.removed.is_empty() {
            ui.label(RichText::new("Choose channels to add them to the chart.").color(tokens.text_muted));
            return false;
        }
        let palette = theme::palette_for(theme::is_dark(ui.ctx()), settings.palette);
        let names = pane_names(&workspace.series);
        let mut rebuild = false;
        let mut remove = None;
        let mut move_to = None;
        for (index, series) in workspace.series.iter().enumerate() {
            let current_color = chart_view::series_color(workspace, &palette, series);
            let [r, g, b, _] = current_color.to_srgba_unmultiplied();
            let mut color = [r, g, b];
            let mut style = workspace
                .panes
                .style_for(series.id, &series.key)
                .cloned()
                .unwrap_or_default();
            let mut changed = false;
            let mut reset = false;
            let visible = !chart.hidden.contains(&series.id);
            let source = workspace.sources.get(series.source).map_or("", |source| source.name.as_str());
            ui.push_id(("inspector-series", series.id), |ui| {
                card(ui, tokens, |ui| {
                    ui.horizontal(|ui| {
                        if widgets::icon_button(
                            ui,
                            if visible { icons::VISIBLE } else { icons::HIDDEN },
                            if visible { "Hide series" } else { "Show series" },
                        ).clicked() {
                            if visible { chart.hidden.insert(series.id); } else { chart.hidden.remove(&series.id); }
                        }
                        let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 24.0), egui::Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, current_color);
                        // Trash pinned right; the label takes what is left and truncates.
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::icon_button(ui, icons::TRASH, "Remove this series from the chart").clicked() {
                                remove = Some((series.id, series.label.clone()));
                            }
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                ui.add(egui::Label::new(RichText::new(&series.label).strong().color(tokens.text)).truncate())
                                    .on_hover_text(&series.label);
                            });
                        });
                    });
                    let detail = match (series.unit.is_empty(), source.is_empty()) {
                        (false, false) => format!("{}  ·  {}", series.unit, source),
                        (false, true) => series.unit.clone(),
                        (true, false) => source.to_owned(),
                        (true, true) => String::new(),
                    };
                    if !detail.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&detail).size(11.5).color(tokens.text_secondary)).truncate())
                            .on_hover_text(&detail);
                    }
                    egui::CollapsingHeader::new("Style & pane")
                        .default_open(index == 0)
                        .show(ui, |ui| {
                            // Label column plus grid spacing; controls take the rest.
                            let control = (ui.available_width() - 72.0).max(80.0);
                            let mut dash = style.dash.unwrap_or(series.dash);
                            let default_draw = if workspace.mode == Mode::Signals && series.family == "Flags" {
                                LineDraw::Steps
                            } else {
                                LineDraw::Lines
                            };
                            let mut draw = style.draw.unwrap_or(default_draw);
                            widgets::property_grid(ui, "style", |ui| {
                                ui.label("Color");
                                if ui.color_edit_button_srgb(&mut color).changed() {
                                    style.color = Some(color);
                                    changed = true;
                                }
                                ui.end_row();
                                ui.label("Width");
                                let mut width = style.width.unwrap_or(settings.line_width);
                                if ui.add(egui::DragValue::new(&mut width).range(0.5..=6.0).speed(0.05).suffix(" px")).changed() {
                                    style.width = Some(width);
                                    changed = true;
                                }
                                ui.end_row();
                                ui.label("Dash");
                                egui::ComboBox::from_id_salt("dash")
                                    .width(control)
                                    .selected_text(dash_label(dash))
                                    .show_ui(ui, |ui| {
                                        for (value, label) in [(0, "Solid"), (1, "Dashed"), (2, "Dotted"), (3, "Dash-dot")] {
                                            changed |= ui.selectable_value(&mut dash, value, label).changed();
                                        }
                                    });
                                ui.end_row();
                                ui.label("Draw");
                                egui::ComboBox::from_id_salt("draw")
                                    .width(control)
                                    .selected_text(draw_label(draw))
                                    .show_ui(ui, |ui| {
                                        for (value, label) in [
                                            (LineDraw::Lines, "Lines"),
                                            (LineDraw::Points, "Points"),
                                            (LineDraw::LinesPoints, "Lines + points"),
                                            (LineDraw::Steps, "Steps"),
                                        ] {
                                            changed |= ui.selectable_value(&mut draw, value, label).changed();
                                        }
                                    });
                                ui.end_row();
                                ui.label("Pane");
                                egui::ComboBox::from_id_salt("pane")
                                    .width(control)
                                    .selected_text(&series.pane)
                                    .show_ui(ui, |ui| {
                                        for name in &names {
                                            if ui.selectable_label(series.pane == *name, *name).clicked() && series.pane != *name {
                                                move_to = Some((series.id, (*name).to_owned()));
                                            }
                                        }
                                    });
                                ui.end_row();
                            });
                            if dash != style.dash.unwrap_or(series.dash) { style.dash = Some(dash); }
                            if draw != style.draw.unwrap_or(default_draw) { style.draw = Some(draw); }
                            ui.horizontal(|ui| {
                                let width = (ui.available_width() - 70.0).max(60.0);
                                let edit = ui.add(egui::TextEdit::singleline(&mut self.new_pane).hint_text("New pane name").desired_width(width));
                                let submit = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if ui.add_enabled(!self.new_pane.trim().is_empty(), egui::Button::new("Move"))
                                    .on_hover_text("Move this series to a new pane")
                                    .clicked()
                                    || (submit && !self.new_pane.trim().is_empty())
                                {
                                    move_to = Some((series.id, self.new_pane.trim().to_owned()));
                                    self.new_pane.clear();
                                }
                            });
                            if ui.button("Reset style").clicked() { reset = true; }
                        });
                });
            });
            if reset {
                // An explicit default also overrides legacy channel-wide styles.
                workspace.panes.styles.insert(PaneSettings::series_style_key(series.id), SeriesStyle::default());
                rebuild = true;
            } else if changed {
                workspace.panes.styles.insert(PaneSettings::series_style_key(series.id), style);
                rebuild = true;
            }
            ui.add_space(6.0);
        }
        if let Some((id, label)) = remove {
            workspace.panes.removed.insert(id, label);
            chart.hidden.remove(&id);
            rebuild = true;
        }
        if let Some((id, name)) = move_to {
            move_series(workspace, id, name);
            rebuild = true;
        }
        if !workspace.panes.removed.is_empty() {
            ui.add_space(4.0);
            widgets::section_label(ui, "Removed series");
            let mut restore = None;
            for (&id, label) in &workspace.panes.removed {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 70.0).max(40.0);
                    ui.add_sized([width, 24.0], egui::Label::new(label).halign(egui::Align::Min).truncate()).on_hover_text(label);
                    if ui.small_button("Restore").clicked() { restore = Some(id); }
                });
            }
            if let Some(id) = restore {
                workspace.panes.removed.remove(&id);
                rebuild = true;
            }
        }
        rebuild
    }
}

/// Preserve the existing visible grouping when making the first manual move.
pub(crate) fn move_series(workspace: &mut Workspace, id: u64, name: String) {
    if workspace.panes.grouping != PaneGrouping::Custom {
        workspace.panes.custom.clear();
        workspace.panes.custom_series.clear();
        for series in &workspace.series {
            workspace.panes.custom_series.insert(series.id, series.pane.clone());
        }
        workspace.panes.grouping = PaneGrouping::Custom;
    }
    workspace.panes.custom_series.insert(id, name);
}

fn card(ui: &mut egui::Ui, tokens: theme::Tokens, contents: impl FnOnce(&mut egui::Ui)) {
    // Inner margin (2 × 8), stroke (2 × 1) and a little slack for pixel
    // rounding: the card must never be wider than the panel, or the panel grows.
    let width = (ui.available_width() - 20.0).max(0.0);
    egui::Frame::new()
        .fill(tokens.surface)
        .stroke(Stroke::new(1.0, tokens.border))
        .corner_radius(7)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            contents(ui);
        });
}

fn pane_names(series: &[crate::series::SeriesData]) -> Vec<&str> {
    let mut names = Vec::new();
    for item in series {
        if !names.contains(&item.pane.as_str()) { names.push(item.pane.as_str()); }
    }
    names
}

fn dash_label(dash: usize) -> &'static str {
    match dash % 4 { 0 => "Solid", 1 => "Dashed", 2 => "Dotted", _ => "Dash-dot" }
}
fn draw_label(draw: LineDraw) -> &'static str {
    match draw {
        LineDraw::Lines => "Lines",
        LineDraw::Points => "Points",
        LineDraw::LinesPoints => "Lines + points",
        LineDraw::Steps => "Steps",
    }
}

fn marks(
    ui: &mut egui::Ui,
    workspace: &mut Workspace,
    chart: &mut ChartState,
    options: &chart_view::ChartOptions,
    cursor: Option<f64>,
    tokens: theme::Tokens,
) {
    let color = MARK_COLOR;
    let selection = chart.selection.filter(|[a, b]| a.is_finite() && b.is_finite() && a != b);
    let event_x = selection.map(|[a, b]| a.min(b)).or(cursor).filter(|x| x.is_finite());
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(selection.is_some(), egui::Button::new("Region"))
            .on_hover_text("Mark the selected range")
            .on_disabled_hover_text("Select a range on the chart first")
            .clicked()
        {
            if let Some([a, b]) = selection {
                if let Some(region) = Annotation::region(workspace.mode, workspace.x_key.clone().unwrap_or_default(), a, b, "Selection".into(), color) {
                    workspace.annotations.push(region);
                }
            }
        }
        if ui.add_enabled(event_x.is_some(), egui::Button::new("Event"))
            .on_hover_text("Vertical marker at the selection start or the last cursor position")
            .on_disabled_hover_text("Point at the chart or select a range first")
            .clicked()
        {
            if let Some(x) = event_x {
                workspace.annotations.push(Annotation::VLine {
                    mode: workspace.mode, x_axis: workspace.x_key.clone().unwrap_or_default(), x,
                    label: "Event".into(), color,
                });
            }
        }
        if ui.add_enabled(!workspace.series.is_empty(), egui::Button::new("Reference line"))
            .on_hover_text("Horizontal line at the mean of the first series; edit the value below")
            .clicked()
        {
            if let Some(first) = workspace.series.first() {
                let (sum, count) = first.y.iter().filter(|v| v.is_finite()).fold((0.0, 0_usize), |(s, n), v| (s + v, n + 1));
                let y = if count > 0 { sum / count as f64 } else { 0.0 };
                workspace.annotations.push(Annotation::HLine {
                    mode: workspace.mode, pane: first.pane.clone(), y,
                    label: "Reference".into(), color,
                });
            }
        }
    });
    ui.add_space(8.0);
    let names = pane_names(&workspace.series);
    let x_input = XInput::of(workspace);
    let mut delete = None;
    let mut zoom = None;
    let mut any = false;
    for (index, mark) in workspace.annotations.iter_mut().enumerate() {
        if mark.mode() != workspace.mode { continue; }
        any = true;
        ui.push_id(("inspector-mark", index), |ui| {
            card(ui, tokens, |ui| {
                let (kind, label, color) = match &mut *mark {
                    Annotation::Point { label, color, .. } => ("Point", label, color),
                    Annotation::Region { label, color, .. } => ("Region", label, color),
                    Annotation::VLine { label, color, .. } => ("Event", label, color),
                    Annotation::HLine { label, color, .. } => ("Reference line", label, color),
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(kind).strong().color(tokens.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, icons::TRASH, "Delete mark").clicked() { delete = Some(index); }
                        if widgets::icon_button(ui, icons::ZOOM_IN, "Zoom to mark").clicked() { zoom = Some(index); }
                        ui.color_edit_button_srgb(color).on_hover_text("Mark color");
                    });
                });
                ui.add(egui::TextEdit::singleline(label).hint_text("Label").desired_width(ui.available_width() - 12.0));
                widgets::property_grid(ui, "mark", |ui| match &mut *mark {
                    Annotation::Point { curve, x, y, .. } => {
                        let series = point_series(&workspace.series, curve);
                        ui.label("Series");
                        let channel = series.map_or(curve.as_str(), |s| s.label.as_str());
                        ui.add(egui::Label::new(RichText::new(channel).color(tokens.text_secondary)).truncate()).on_hover_text(channel);
                        ui.end_row();
                        x_row(ui, "X", x, x_input);
                        value_row(ui, "Y", y, series.map_or("", |s| s.unit.as_str()));
                    }
                    Annotation::Region { min, max, .. } => {
                        x_row(ui, "Start", min, x_input);
                        x_row(ui, "End", max, x_input);
                    }
                    Annotation::VLine { x, .. } => x_row(ui, "X", x, x_input),
                    Annotation::HLine { pane, y, .. } => {
                        ui.label("Pane");
                        egui::ComboBox::from_id_salt("reference-pane")
                            .width((ui.available_width() - 24.0).max(70.0))
                            .selected_text(pane.as_str())
                            .show_ui(ui, |ui| {
                                for name in &names { ui.selectable_value(pane, (*name).to_owned(), *name); }
                            });
                        ui.end_row();
                        let unit = workspace.series.iter().find(|s| s.pane == *pane).map_or("", |s| s.unit.as_str());
                        value_row(ui, "Value", y, unit);
                    }
                });
            });
        });
        ui.add_space(6.0);
    }
    if !any {
        ui.label(RichText::new("No marks in this mode. Add one above or right-click the chart.").color(tokens.text_muted));
    }
    if let Some(index) = zoom {
        if let Some(mark) = workspace.annotations.get(index) {
            zoom_to_mark(workspace, chart, options, mark);
        }
    }
    if let Some(index) = delete { workspace.annotations.remove(index); }
}

/// How mark positions along X are written and typed: the way the X axis shows them.
#[derive(Clone, Copy)]
enum XInput {
    Number,
    Clock,
    Duration,
}

impl XInput {
    fn of(workspace: &Workspace) -> Self {
        match chart_view::x_axis_format(workspace).0 {
            dv_chart::AxisFormat::DateTime => Self::Clock,
            dv_chart::AxisFormat::Duration => Self::Duration,
            _ => Self::Number,
        }
    }
}

fn x_row(ui: &mut egui::Ui, name: &str, value: &mut f64, input: XInput) {
    ui.label(name);
    // Typing a time of day keeps the date of the current position.
    let day = (*value / 86_400.0).floor() * 86_400.0;
    let drag = egui::DragValue::new(value).range(-f64::MAX..=f64::MAX).speed(0.1);
    let drag = match input {
        XInput::Number => drag,
        XInput::Clock => drag
            .custom_formatter(|v, _| format::format_time(v, 0.1))
            .custom_parser(move |text| format::parse_seconds(text).map(|seconds| day + seconds)),
        XInput::Duration => drag
            .custom_formatter(|v, _| if v.abs() < 60.0 { format::format_value(v, "s", 6) } else { format::format_duration(v, 0.001) })
            .custom_parser(format::parse_seconds),
    };
    ui.add(drag);
    ui.end_row();
}

fn value_row(ui: &mut egui::Ui, name: &str, value: &mut f64, unit: &str) {
    ui.label(name);
    let speed = (value.abs() * 0.002).max(0.01);
    let mut drag = egui::DragValue::new(value).range(-f64::MAX..=f64::MAX).speed(speed);
    if !unit.is_empty() {
        drag = drag.suffix(format!(" {unit}"));
    }
    ui.add(drag);
    ui.end_row();
}

fn point_series<'a>(series: &'a [crate::series::SeriesData], curve: &str) -> Option<&'a crate::series::SeriesData> {
    let id = RangeRule::target_id(curve);
    series.iter().find(|s| id.map_or_else(|| s.key == curve || s.label == curve || s.channel_label == curve, |id| s.id == id))
}

fn zoom_to_mark(workspace: &Workspace, chart: &mut ChartState, options: &chart_view::ChartOptions, mark: &Annotation) {
    let (x, y) = match mark {
        Annotation::Region { min, max, .. } if min.is_finite() && max.is_finite() && min != max => (Some([min.min(*max), min.max(*max)]), None),
        Annotation::Point { curve, x, y, .. } => {
            let pane = point_series(&workspace.series, curve).map(|s| s.pane.as_str());
            (x.is_finite().then(|| centered_x(workspace, chart, *x)), pane.map(|pane| (pane, *y)))
        }
        Annotation::VLine { x, .. } => (x.is_finite().then(|| centered_x(workspace, chart, *x)), None),
        Annotation::HLine { pane, y, .. } => (None, Some((pane.as_str(), *y))),
        _ => (None, None),
    };
    if let Some(range) = x {
        if options.link_x { chart.x_view = Some(range); } else {
            for series in &workspace.series { chart.pane_x_views.insert(chart_view::pane_id(&series.pane), range); }
        }
    }
    if let Some((pane, value)) = y.filter(|(_, y)| y.is_finite()) {
        let id = chart_view::pane_id(pane);
        let log = options.pane_log_y.get(&id).copied().unwrap_or(options.log_y);
        if log {
            if value > 0.0 { chart.y_views.insert(id, [value / 2.0, value * 2.0]); }
        } else {
            let span = chart.y_views.get(&id).map(|r| (r[1] - r[0]).abs()).filter(|v| v.is_finite() && *v > 0.0).or_else(|| {
                let mut min = f64::INFINITY;
                let mut max = f64::NEG_INFINITY;
                for series in workspace.series.iter().filter(|s| s.pane == pane) {
                    for &y in series.y.iter().filter(|y| y.is_finite()) { min = min.min(y); max = max.max(y); }
                }
                (max > min).then_some(max - min)
            }).unwrap_or_else(|| value.abs().max(1.0));
            let half = (span * 0.2).max(value.abs() * f64::EPSILON * 4.0).max(1e-9);
            chart.y_views.insert(id, [value - half, value + half]);
        }
    }
}

fn centered_x(workspace: &Workspace, chart: &ChartState, x: f64) -> [f64; 2] {
    let span = chart.x_view.map(|r| (r[1] - r[0]).abs()).filter(|v| v.is_finite() && *v > 0.0).or_else(|| {
        let series = workspace.series.first()?;
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &value in series.x.iter().filter(|v| v.is_finite()) { min = min.min(value); max = max.max(value); }
        (max > min).then_some(max - min)
    }).unwrap_or(1.0);
    let half = (span * 0.1).max(x.abs() * f64::EPSILON * 4.0).max(1e-9);
    [x - half, x + half]
}

fn rules(ui: &mut egui::Ui, workspace: &mut Workspace, tokens: theme::Tokens) -> bool {
    ui.label(RichText::new("Filters hide samples on the chart without changing source data. Statistics and new analyses use the filtered series.").size(11.5).color(tokens.text_secondary));
    ui.add_space(8.0);
    let mut rebuild = false;
    if ui.button("Add filter").clicked() {
        workspace.rules.push(RangeRule { enabled: true, mode: workspace.mode, target: String::new(), bounds: [None; 4], action: FilterMode::Keep });
        rebuild = true;
    }
    ui.add_space(8.0);
    let mut delete = None;
    let mut any = false;
    for (index, rule) in workspace.rules.iter_mut().enumerate() {
        if rule.mode != workspace.mode { continue; }
        any = true;
        ui.push_id(("inspector-rule", index), |ui| {
            card(ui, tokens, |ui| {
                ui.horizontal(|ui| {
                    rebuild |= ui.checkbox(&mut rule.enabled, "Enabled").changed();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, icons::TRASH, "Delete filter").clicked() { delete = Some(index); }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Action");
                    egui::ComboBox::from_id_salt("filter-action")
                        .selected_text(match rule.action { FilterMode::Keep => "Keep inside", FilterMode::Exclude => "Exclude inside" })
                        .show_ui(ui, |ui| {
                            rebuild |= ui.selectable_value(&mut rule.action, FilterMode::Keep, "Keep inside").changed();
                            rebuild |= ui.selectable_value(&mut rule.action, FilterMode::Exclude, "Exclude inside").changed();
                        });
                });
                let target_id = RangeRule::target_id(&rule.target);
                let target_label = if rule.target.is_empty() { "All plotted series" } else {
                    workspace.series.iter().find(|s| Some(s.id) == target_id || s.key == rule.target || s.label == rule.target)
                        .map_or("Unavailable series", |s| s.label.as_str())
                };
                ui.label(RichText::new("Apply to").color(tokens.text_secondary));
                egui::ComboBox::from_id_salt("filter-target")
                    .width((ui.available_width() - 24.0).max(70.0))
                    .selected_text(target_label)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(rule.target.is_empty(), "All plotted series").clicked() && !rule.target.is_empty() {
                            rule.target.clear(); rebuild = true;
                        }
                        for series in &workspace.series {
                            if ui.selectable_label(target_id == Some(series.id), &series.label).clicked() && target_id != Some(series.id) {
                                rule.target = RangeRule::series_target(series.id); rebuild = true;
                            }
                        }
                    });
                ui.add_space(4.0);
                ui.label(RichText::new("Inclusive bounds").size(11.5).color(tokens.text_secondary));
                for (label, bound) in ["X minimum", "X maximum", "Y minimum", "Y maximum"].into_iter().zip(&mut rule.bounds) {
                    rebuild |= bound_row(ui, label, bound);
                }
            });
        });
        ui.add_space(6.0);
    }
    if !any { ui.label(RichText::new("No filters in this mode.").color(tokens.text_muted)); }
    if let Some(index) = delete { workspace.rules.remove(index); rebuild = true; }
    rebuild
}

fn bound_row(ui: &mut egui::Ui, label: &str, bound: &mut Option<f64>) -> bool {
    let mut enabled = bound.is_some();
    let mut value = bound.unwrap_or(0.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        changed |= ui.checkbox(&mut enabled, label).changed();
        if enabled {
            changed |= ui.add(egui::DragValue::new(&mut value).range(-f64::MAX..=f64::MAX).speed(0.1)).changed();
        } else {
            ui.label(RichText::new("Any").weak());
        }
    });
    if changed { *bound = enabled.then_some(value); }
    changed
}
