use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::{
    canvas::{Anchor, Canvas, EguiCanvas, SvgCanvas},
    decimate::M4Index,
    format, interaction,
    ticks::{self, Tick},
    AxisFormat, ChartConfig, ChartResponse, ChartState, ChartTheme, Dash, Draw, FontSet, Hover,
    Mark, Pane, PaneAction, PaneId, Scale, Series, SeriesId,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Axis {
    pub range: [f64; 2],
    pub scale: Scale,
    pub flip: bool,
}
impl Axis {
    pub fn fraction(self, value: f64) -> Option<f64> {
        let transform = |v: f64| {
            if self.scale == Scale::Log10 {
                if v > 0.0 {
                    v.log10()
                } else {
                    f64::NAN
                }
            } else {
                v
            }
        };
        let a = transform(self.range[0]);
        let b = transform(self.range[1]);
        let v = transform(value);
        let t = (v - a) / (b - a);
        if t.is_finite() {
            Some(if self.flip { 1.0 - t } else { t })
        } else {
            None
        }
    }
    pub fn value(self, fraction: f64) -> f64 {
        let fraction = if self.flip { 1.0 - fraction } else { fraction };
        if self.scale == Scale::Log10 {
            10f64.powf(
                self.range[0].log10() + (self.range[1].log10() - self.range[0].log10()) * fraction,
            )
        } else {
            self.range[0] + (self.range[1] - self.range[0]) * fraction
        }
    }
    pub fn x(self, rect: Rect, value: f64) -> Option<f32> {
        self.fraction(value)
            .map(|t| rect.left() + rect.width() * t as f32)
    }
    pub fn y(self, rect: Rect, value: f64) -> Option<f32> {
        self.fraction(value)
            .map(|t| rect.bottom() - rect.height() * t as f32)
    }
    pub fn at_x(self, rect: Rect, x: f32) -> f64 {
        self.value(f64::from((x - rect.left()) / rect.width().max(1.0)))
    }
    pub fn at_y(self, rect: Rect, y: f32) -> f64 {
        self.value(f64::from((rect.bottom() - y) / rect.height().max(1.0)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DataKey {
    lo: u64,
    hi: u64,
    px: usize,
    log: bool,
    log_y: bool,
    raw: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ScreenKey {
    data: DataKey,
    y0: u64,
    y1: u64,
    xscale: Scale,
    yscale: Scale,
    xflip: bool,
    yflip: bool,
    rect: [u32; 4],
    draw: Draw,
}
#[derive(Clone)]
pub(crate) struct SeriesCache {
    generation: u64,
    x_ptr: usize,
    y_ptr: usize,
    len: usize,
    gap: Option<u64>,
    pub sorted: bool,
    index: Arc<M4Index>,
    data: Vec<(DataKey, Arc<Vec<[f64; 2]>>)>,
    screen: Option<(ScreenKey, Arc<Vec<Pos2>>)>,
}
impl SeriesCache {
    fn new(series: &Series<'_>, gap: Option<f64>) -> Self {
        Self {
            generation: series.generation,
            x_ptr: series.x.as_ptr() as usize,
            y_ptr: series.y.as_ptr() as usize,
            len: series.x.len().min(series.y.len()),
            gap: gap.map(f64::to_bits),
            sorted: crate::decimate::is_sorted(&series.x[..series.x.len().min(series.y.len())]),
            index: Arc::new(M4Index::new(series.x, series.y, gap)),
            data: Vec::new(),
            screen: None,
        }
    }
    fn matches(&self, series: &Series<'_>, gap: Option<f64>) -> bool {
        self.generation == series.generation
            && self.x_ptr == series.x.as_ptr() as usize
            && self.y_ptr == series.y.as_ptr() as usize
            && self.len == series.x.len().min(series.y.len())
            && self.gap == gap.map(f64::to_bits)
    }
    fn samples(
        &mut self,
        series: &Series<'_>,
        axis: Axis,
        px: usize,
        raw: bool,
        log_y: bool,
    ) -> (DataKey, Arc<Vec<[f64; 2]>>) {
        let key = DataKey {
            lo: axis.range[0].to_bits(),
            hi: axis.range[1].to_bits(),
            px: px.max(1),
            log: axis.scale == Scale::Log10,
            log_y,
            raw: raw || !self.sorted,
        };
        if let Some((_, data)) = self.data.iter().find(|(k, _)| *k == key) {
            return (key, Arc::clone(data));
        }
        let samples = if key.log_y && key.raw {
            self.index
                .query_raw_log_y(series.x, series.y, axis.range[0], axis.range[1], key.log)
        } else if key.log_y {
            self.index.query_log_y(
                series.x,
                series.y,
                axis.range[0],
                axis.range[1],
                key.px,
                key.log,
            )
        } else if key.raw && key.log {
            self.index
                .query_raw_log(series.x, series.y, axis.range[0], axis.range[1])
        } else if key.raw {
            self.index
                .query_raw(series.x, series.y, axis.range[0], axis.range[1])
        } else if key.log {
            self.index
                .query_log(series.x, series.y, axis.range[0], axis.range[1], key.px)
        } else {
            self.index
                .query(series.x, series.y, axis.range[0], axis.range[1], key.px)
        };
        let data = Arc::new(samples);
        if self.data.len() >= 4 {
            self.data.remove(0);
        }
        self.data.push((key, Arc::clone(&data)));
        (key, data)
    }
}
fn cache<'a>(
    caches: &'a mut HashMap<SeriesId, SeriesCache>,
    series: &Series<'_>,
    gap: Option<f64>,
) -> &'a mut SeriesCache {
    let entry = caches
        .entry(series.id)
        .or_insert_with(|| SeriesCache::new(series, gap));
    if !entry.matches(series, gap) {
        *entry = SeriesCache::new(series, gap);
    }
    entry
}

pub(crate) struct LegendBox {
    pub rect: Rect,
    pub id: SeriesId,
}
pub(crate) struct ActionBox {
    pub rect: Rect,
    pub action: PaneAction,
    label: &'static str,
    hint: &'static str,
}
pub(crate) struct PaneLayout {
    pub id: PaneId,
    pub plot: Rect,
    pub x: Axis,
    pub y: Axis,
    pub full_x: [f64; 2],
    y_ticks: Vec<Tick>,
    x_ticks: Vec<Tick>,
    pub legend: Vec<LegendBox>,
    pub actions: Vec<ActionBox>,
    legend_rect: Option<Rect>,
}
pub(crate) struct Layout {
    pub rect: Rect,
    pub panes: Vec<PaneLayout>,
    pub x_axis: Rect,
    pub overview: Option<Rect>,
    pub dividers: Vec<Rect>,
    pub full_x: [f64; 2],
}
fn expand_range(mut lo: f64, mut hi: f64, scale: Scale, pad: f64, include_zero: bool) -> [f64; 2] {
    if !lo.is_finite() || !hi.is_finite() || (scale == Scale::Log10 && (lo <= 0.0 || hi <= 0.0)) {
        return if scale == Scale::Log10 {
            [1.0, 10.0]
        } else {
            [0.0, 1.0]
        };
    }
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
    }
    if include_zero && scale == Scale::Linear {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    if scale == Scale::Log10 {
        let a = lo.log10();
        let b = hi.log10();
        let d = if (b - a).abs() < 1e-12 {
            0.5
        } else {
            (b - a) * pad
        };
        [10f64.powf(a - d), 10f64.powf(b + d)]
    } else {
        let d = if (hi - lo).abs() < f64::EPSILON * hi.abs().max(1.0) {
            hi.abs().max(1.0) * 0.05
        } else {
            (hi - lo) * pad
        };
        [lo - d, hi + d]
    }
}
pub(crate) fn valid_range(range: [f64; 2], scale: Scale) -> bool {
    range[0].is_finite()
        && range[1].is_finite()
        && range[1] > range[0]
        && (scale != Scale::Log10 || range[0] > 0.0)
}
/// Tick count requested for a pane of `height` px. Ticks round up to nice
/// steps, so asking for fewer than four often leaves a short pane unlabeled.
fn y_tick_target(height: f32) -> usize {
    ((height / 34.0) as usize).clamp(4, 12)
}
fn extent_union(target: &mut [f64; 2], add: [f64; 2]) {
    target[0] = target[0].min(add[0]);
    target[1] = target[1].max(add[1]);
}
fn domain(
    caches: &HashMap<SeriesId, SeriesCache>,
    pane: &Pane<'_>,
    state: &ChartState,
    scale: Scale,
) -> [f64; 2] {
    let mut bounds = [f64::INFINITY, f64::NEG_INFINITY];
    for series in &pane.series {
        if !series.visible || state.hidden.contains(&series.id) {
            continue;
        }
        if let Some(cached) = caches.get(&series.id) {
            if let Some(mut b) = cached.index.x_bounds() {
                if scale == Scale::Log10 {
                    if let Some(min) = cached.index.smallest_positive_x() {
                        b[0] = min;
                    } else {
                        continue;
                    }
                }
                extent_union(&mut bounds, b);
            }
        }
    }
    bounds
}
fn axis_for(range: [f64; 2], scale: Scale, flip: bool) -> Axis {
    Axis {
        range: if valid_range(range, scale) {
            range
        } else {
            expand_range(range[0], range[1], scale, 0.0, false)
        },
        scale,
        flip,
    }
}
/// "POWER · W": the quantity is set in small caps style, the unit keeps its
/// case (MHz, W²/Hz) because case is meaningful in unit symbols.
fn pane_heading(pane: &Pane<'_>) -> String {
    let title = match pane.title.split_once(" · ") {
        Some((quantity, unit)) => format!("{} · {unit}", quantity.to_uppercase()),
        None => pane.title.to_uppercase(),
    };
    match (title.is_empty(), pane.y_label.is_empty()) {
        (false, false) => format!("{title} · {}", pane.y_label),
        (false, true) => title,
        (true, false) => pane.y_label.to_owned(),
        (true, true) => String::new(),
    }
}
/// Lay out the pane heading and legend as a header strip above the plot area,
/// so legend entries never sit on top of data. `rect` is the whole pane.
fn legend_layout(
    canvas: &mut dyn Canvas,
    pane: &Pane<'_>,
    rect: Rect,
    config: &ChartConfig<'_>,
    theme: &ChartTheme,
) -> (Vec<LegendBox>, Option<Rect>) {
    let heading = pane_heading(pane);
    if !config.legend && heading.is_empty() {
        return (Vec::new(), None);
    }
    let size = theme.font_size;
    let left = rect.left() + 2.0;
    let top = rect.top() + 3.0;
    let title = if heading.is_empty() {
        0.0
    } else {
        canvas.measure_text(&heading, size - 1.0, true).x + 16.0
    };
    let mut x = left + title;
    let mut y = top;
    let row = size + 9.0;
    let mut right: f32 = left + title;
    let mut items = Vec::with_capacity(pane.series.len());
    for series in pane.series.iter().filter(|_| config.legend) {
        let width = 17.0
            + canvas.measure_text(series.label, size, false).x
            + if config.legend_values && !series.overlay {
                92.0
            } else {
                10.0
            };
        let width = width.min((rect.width() - 20.0).max(24.0));
        if x > left + title && x + width > rect.right() - 4.0 {
            x = left + title;
            y += row;
        }
        let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, row));
        items.push(LegendBox {
            rect: r,
            id: series.id,
        });
        right = right.max(r.right());
        x += width + 4.0;
    }
    let header = Rect::from_min_max(
        Pos2::new(left, top),
        Pos2::new(right.min(rect.right()), y + row),
    );
    (items, Some(header))
}
fn pane_actions(rect: Rect, id: PaneId) -> Vec<ActionBox> {
    if rect.width() < 320.0 || rect.height() < 60.0 {
        return Vec::new();
    }
    let actions = [
        ("Log", "Logarithmic Y axis", PaneAction::ToggleLog(id)),
        (
            "Fit",
            "Auto-fit Y to the visible data",
            PaneAction::AutoY(id),
        ),
        (
            "Zero",
            "Always include zero on Y",
            PaneAction::ToggleIncludeZero(id),
        ),
        (
            "Lock",
            "Keep this Y range while panning and zooming",
            PaneAction::ToggleLockY(id),
        ),
        (
            "Max",
            "Maximize or restore this pane",
            PaneAction::ToggleMaximize(id),
        ),
        (
            "×",
            "Remove this pane's channels from the chart",
            PaneAction::Remove(id),
        ),
    ];
    let width = 30.0;
    let left = rect.right() - width * actions.len() as f32;
    actions
        .into_iter()
        .enumerate()
        .map(|(i, (label, hint, action))| ActionBox {
            rect: Rect::from_min_size(
                Pos2::new(left + i as f32 * width, rect.top() + 2.0),
                Vec2::new(width - 3.0, 20.0),
            ),
            action,
            label,
            hint,
        })
        .collect()
}
fn layout(
    canvas: &mut dyn Canvas,
    state: &mut ChartState,
    panes: &[Pane<'_>],
    config: &ChartConfig<'_>,
    rect: Rect,
    theme: &ChartTheme,
) -> Layout {
    state.runtime.caches.retain(|id, _| {
        panes
            .iter()
            .any(|pane| pane.series.iter().any(|series| series.id == *id))
    });
    for pane in panes {
        for series in &pane.series {
            if series.visible {
                cache(&mut state.runtime.caches, series, config.gap_factor);
            }
        }
    }
    let domains: Vec<_> = panes
        .iter()
        .map(|p| domain(&state.runtime.caches, p, state, config.x_scale))
        .collect();
    let mut global = [f64::INFINITY, f64::NEG_INFINITY];
    for domain in &domains {
        if domain[0].is_finite() {
            extent_union(&mut global, *domain);
        }
    }
    global = axis_for(global, config.x_scale, config.x_flip).range;
    let title_height = if config.title.is_empty() { 0.0 } else { 26.0 }
        + if config.subtitle.is_empty() {
            0.0
        } else {
            21.0
        };
    let overview_on = config.overview
        && panes
            .iter()
            .flat_map(|p| &p.series)
            .any(|s| s.visible && s.x.len().min(s.y.len()) >= 2000);
    let overview_height = if overview_on { 48.0 } else { 0.0 };
    let axis_height = 31.0 + if config.x_label.is_empty() { 0.0 } else { 16.0 };
    let top = rect.top() + 8.0 + title_height;
    let bottom = (rect.bottom() - overview_height - axis_height - 6.0).max(top + 1.0);
    let maximized = state
        .maximized
        .filter(|id| panes.iter().any(|p| p.id == *id));
    let visible_count = panes
        .iter()
        .filter(|p| maximized.is_none_or(|id| id == p.id))
        .count();
    let total_weight: f32 = panes
        .iter()
        .filter(|p| maximized.is_none_or(|id| id == p.id))
        .map(|p| {
            state
                .pane_weights
                .get(&p.id)
                .copied()
                .unwrap_or(p.weight)
                .max(0.05)
        })
        .sum::<f32>()
        .max(0.05);
    let mut prepared = Vec::with_capacity(panes.len());
    let mut gutter = 34.0_f32;
    let usable_height = (bottom - top - (visible_count.saturating_sub(1) as f32 * 7.0)).max(1.0);
    for (index, pane) in panes.iter().enumerate() {
        if maximized.is_some_and(|id| id != pane.id) {
            continue;
        }
        let local_full = if valid_range(domains[index], config.x_scale) {
            domains[index]
        } else {
            global
        };
        let requested = if config.link_x {
            state.x_view
        } else {
            state.pane_x_views.get(&pane.id).copied().or(state.x_view)
        };
        let x = axis_for(
            requested
                .filter(|r| valid_range(*r, config.x_scale))
                .unwrap_or(if config.link_x { global } else { local_full }),
            config.x_scale,
            config.x_flip,
        );
        let mut yrange = [f64::INFINITY, f64::NEG_INFINITY];
        for series in &pane.series {
            if !series.visible || state.hidden.contains(&series.id) {
                continue;
            }
            let cached = cache(&mut state.runtime.caches, series, config.gap_factor);
            let bounds = if pane.y_scale == Scale::Log10 {
                cached
                    .index
                    .positive_y_bounds(series.x, series.y, x.range[0], x.range[1])
            } else {
                cached
                    .index
                    .y_bounds(series.x, series.y, x.range[0], x.range[1])
            };
            if let Some(bounds) = bounds {
                extent_union(&mut yrange, bounds);
            }
        }
        let auto_y = expand_range(yrange[0], yrange[1], pane.y_scale, 0.05, pane.include_zero);
        let y = axis_for(
            state
                .y_views
                .get(&pane.id)
                .copied()
                .filter(|r| valid_range(*r, pane.y_scale))
                .unwrap_or(auto_y),
            pane.y_scale,
            pane.y_flip,
        );
        let h = usable_height
            * state
                .pane_weights
                .get(&pane.id)
                .copied()
                .unwrap_or(pane.weight)
                .max(0.05)
            / total_weight;
        let y_ticks = ticks::axis_ticks(
            y.range[0],
            y.range[1],
            y_tick_target(h - 24.0),
            &pane.y_format,
            pane.y_scale,
        );
        for tick in &y_ticks {
            if !tick.label.is_empty() {
                gutter =
                    gutter.max(canvas.measure_text(&tick.label, theme.font_size, false).x + 17.0);
            }
        }
        prepared.push((pane, local_full, x, y, h, y_ticks));
    }
    gutter = gutter.min((rect.width() * 0.35).max(34.0));
    let left = rect.left() + gutter;
    let right = (rect.right() - 12.0).max(left + 1.0);
    let mut y_top = top;
    let mut pane_layouts = Vec::with_capacity(panes.len());
    let mut dividers = Vec::new();
    let prepared_len = prepared.len();
    for (i, (pane, local_full, mut x, mut y, h, mut y_ticks)) in prepared.into_iter().enumerate() {
        let pane_rect = Rect::from_min_max(Pos2::new(left, y_top), Pos2::new(right, y_top + h));
        // Leave room for the hover actions at the right of the header strip.
        let legend_area = if pane_rect.width() > 520.0 {
            Rect::from_min_max(
                pane_rect.min,
                Pos2::new(pane_rect.right() - 190.0, pane_rect.bottom()),
            )
        } else {
            pane_rect
        };
        let (legend, legend_rect) = legend_layout(canvas, pane, legend_area, config, theme);
        let header = legend_rect.map_or(0.0, |r| (r.bottom() - pane_rect.top() + 3.0).min(h * 0.5));
        let plot = Rect::from_min_max(Pos2::new(left, y_top + header), Pos2::new(right, y_top + h));
        if config.equal_aspect && x.scale == Scale::Linear && y.scale == Scale::Linear {
            let desired =
                (x.range[1] - x.range[0]) * f64::from(plot.height() / plot.width().max(1.0));
            let actual = y.range[1] - y.range[0];
            if desired >= actual {
                let mid = (y.range[0] + y.range[1]) * 0.5;
                y.range = [mid - desired * 0.5, mid + desired * 0.5];
            } else {
                let mid = (x.range[0] + x.range[1]) * 0.5;
                let span = actual * f64::from(plot.width() / plot.height().max(1.0));
                x.range = [mid - span * 0.5, mid + span * 0.5];
            }
        }
        if header > 0.0 || config.equal_aspect {
            y_ticks = ticks::axis_ticks(
                y.range[0],
                y.range[1],
                y_tick_target(plot.height()),
                &pane.y_format,
                pane.y_scale,
            );
        }
        let x_ticks = ticks::axis_ticks(
            x.range[0],
            x.range[1],
            ((plot.width() / 110.0) as usize).clamp(2, 12),
            &config.x_format,
            config.x_scale,
        );
        let actions = pane_actions(pane_rect, pane.id);
        pane_layouts.push(PaneLayout {
            id: pane.id,
            plot,
            x,
            y,
            full_x: local_full,
            y_ticks,
            x_ticks,
            legend,
            actions,
            legend_rect,
        });
        y_top += h;
        if i + 1 < prepared_len {
            dividers.push(Rect::from_min_max(
                Pos2::new(left, y_top),
                Pos2::new(right, y_top + 7.0),
            ));
            y_top += 7.0;
        }
    }
    let x_axis = Rect::from_min_max(
        Pos2::new(left, bottom),
        Pos2::new(right, bottom + axis_height),
    );
    let overview = overview_on.then(|| {
        Rect::from_min_size(
            Pos2::new(left, rect.bottom() - 43.0),
            Vec2::new((right - left).max(1.0), 35.0),
        )
    });
    Layout {
        rect,
        panes: pane_layouts,
        x_axis,
        overview,
        dividers,
        full_x: global,
    }
}
fn line(canvas: &mut dyn Canvas, a: Pos2, b: Pos2, color: Color32, width: f32) {
    canvas.polyline(&[a, b], Stroke::new(width, color), Dash::Solid);
}
fn pill(
    canvas: &mut dyn Canvas,
    pos: Pos2,
    text: &str,
    fill: Color32,
    text_color: Color32,
    size: f32,
) {
    if text.is_empty() {
        return;
    }
    let w = canvas.measure_text(text, size, false).x + 12.0;
    let r = Rect::from_min_size(pos, Vec2::new(w, size + 8.0));
    canvas.fill_rect(r, 4.0, fill);
    canvas.text(
        r.center(),
        Anchor::CenterCenter,
        text,
        size,
        text_color,
        false,
    );
}
fn color_dim(color: Color32, dim: bool) -> Color32 {
    if dim {
        color.gamma_multiply(0.25)
    } else {
        color
    }
}
fn x_readout(x: f64, config: &ChartConfig<'_>) -> String {
    match &config.x_format {
        AxisFormat::DateTime => format::format_time_with_date(x, 0.001),
        AxisFormat::Duration => format::format_duration(x, 0.001),
        AxisFormat::Category(categories) if x >= 0.0 && x.is_finite() => categories
            .get(x.round() as usize)
            .cloned()
            .unwrap_or_else(|| format::format_value(x, "", config.precision)),
        _ => format::format_value(x, "", config.precision.max(6)),
    }
}
fn screen_vertices(
    cached: &mut SeriesCache,
    series: &Series<'_>,
    plot: Rect,
    x: Axis,
    y: Axis,
    decimate: bool,
) -> Arc<Vec<Pos2>> {
    let raw = !decimate || series.draw == Draw::Points;
    let (data_key, data) = cached.samples(
        series,
        x,
        plot.width().ceil().max(1.0) as usize,
        raw,
        y.scale == Scale::Log10,
    );
    let key = ScreenKey {
        data: data_key,
        y0: y.range[0].to_bits(),
        y1: y.range[1].to_bits(),
        xscale: x.scale,
        yscale: y.scale,
        xflip: x.flip,
        yflip: y.flip,
        rect: [
            plot.left().to_bits(),
            plot.top().to_bits(),
            plot.right().to_bits(),
            plot.bottom().to_bits(),
        ],
        draw: series.draw,
    };
    if let Some((existing, points)) = &cached.screen {
        if *existing == key {
            return Arc::clone(points);
        }
    }
    let mut points = Vec::with_capacity(data.len());
    let mut occupied = HashSet::new();
    let mut previous: Option<Pos2> = None;
    for [vx, vy] in data.iter().copied() {
        let pos = x
            .x(plot, vx)
            .zip(y.y(plot, vy))
            .map(|(x, y)| Pos2::new(x, y));
        if let Some(pos) = pos.filter(|p| p.x.is_finite() && p.y.is_finite()) {
            if series.draw == Draw::Points {
                if !plot.expand(series.marker_radius).contains(pos)
                    || !occupied.insert((pos.x.round() as i32, pos.y.round() as i32))
                {
                    continue;
                }
            }
            if series.draw == Draw::Steps {
                if let Some(prev) = previous {
                    points.push(Pos2::new(pos.x, prev.y));
                }
            }
            points.push(pos);
            previous = Some(pos);
        } else if previous.take().is_some() {
            points.push(Pos2::new(f32::NAN, f32::NAN));
        }
    }
    let points = Arc::new(points);
    cached.screen = Some((key, Arc::clone(&points)));
    points
}
fn draw_marks(canvas: &mut dyn Canvas, pane: &PaneLayout, marks: &[Mark], theme: &ChartTheme) {
    for mark in marks {
        match mark {
            Mark::Point {
                pane: id,
                x,
                y,
                label,
                color,
            } if id.is_none() || *id == Some(pane.id) => {
                if let Some((px, py)) = pane.x.x(pane.plot, *x).zip(pane.y.y(pane.plot, *y)) {
                    let p = Pos2::new(px, py);
                    canvas.circle(p, 4.0, *color, Stroke::new(1.5, theme.surface));
                    pill(
                        canvas,
                        p + Vec2::new(6.0, -22.0),
                        label,
                        *color,
                        theme.surface,
                        theme.font_size,
                    );
                }
            }
            Mark::Region {
                x0,
                x1,
                label,
                color,
            } => {
                if let Some((a, b)) = pane.x.x(pane.plot, *x0).zip(pane.x.x(pane.plot, *x1)) {
                    let r = Rect::from_min_max(
                        Pos2::new(a.min(b), pane.plot.top()),
                        Pos2::new(a.max(b), pane.plot.bottom()),
                    );
                    canvas.fill_rect(r, 0.0, color.gamma_multiply(0.14));
                    line(canvas, r.left_top(), r.left_bottom(), *color, 1.0);
                    line(canvas, r.right_top(), r.right_bottom(), *color, 1.0);
                    pill(
                        canvas,
                        r.left_top() + Vec2::new(3.0, 3.0),
                        label,
                        *color,
                        theme.surface,
                        theme.font_size,
                    );
                }
            }
            Mark::VLine { x, label, color } => {
                if let Some(px) = pane.x.x(pane.plot, *x) {
                    line(
                        canvas,
                        Pos2::new(px, pane.plot.top()),
                        Pos2::new(px, pane.plot.bottom()),
                        *color,
                        1.0,
                    );
                    pill(
                        canvas,
                        Pos2::new(px + 3.0, pane.plot.top() + 3.0),
                        label,
                        *color,
                        theme.surface,
                        theme.font_size,
                    );
                }
            }
            Mark::HLine {
                pane: id,
                y,
                label,
                color,
            } if *id == pane.id => {
                if let Some(py) = pane.y.y(pane.plot, *y) {
                    line(
                        canvas,
                        Pos2::new(pane.plot.left(), py),
                        Pos2::new(pane.plot.right(), py),
                        *color,
                        1.0,
                    );
                    pill(
                        canvas,
                        Pos2::new(pane.plot.left() + 3.0, py + 3.0),
                        label,
                        *color,
                        theme.surface,
                        theme.font_size,
                    );
                }
            }
            _ => {}
        }
    }
}
fn draw(
    canvas: &mut dyn Canvas,
    state: &mut ChartState,
    panes: &[Pane<'_>],
    config: &ChartConfig<'_>,
    marks: &[Mark],
    theme: &ChartTheme,
    layout: &Layout,
    hover: Option<&Hover>,
    pointer: Option<Pos2>,
) {
    canvas.fill_rect(layout.rect, 0.0, theme.surface);
    let mut title_y = layout.rect.top() + 7.0;
    if !config.title.is_empty() {
        canvas.text(
            Pos2::new(layout.rect.left() + 14.0, title_y),
            Anchor::LeftTop,
            config.title,
            16.0,
            theme.text,
            true,
        );
        title_y += 26.0;
    }
    if !config.subtitle.is_empty() {
        canvas.text(
            Pos2::new(layout.rect.left() + 14.0, title_y),
            Anchor::LeftTop,
            config.subtitle,
            12.0,
            theme.text_muted,
            false,
        );
    }
    for pl in &layout.panes {
        let Some(pane) = panes.iter().find(|p| p.id == pl.id) else {
            continue;
        };
        canvas.set_clip(Some(pl.plot));
        if config.grid {
            for tick in &pl.y_ticks {
                if tick.major {
                    if let Some(y) = pl.y.y(pl.plot, tick.value) {
                        line(
                            canvas,
                            Pos2::new(pl.plot.left(), y),
                            Pos2::new(pl.plot.right(), y),
                            theme.grid,
                            1.0,
                        );
                    }
                }
            }
            for tick in &pl.x_ticks {
                if tick.major {
                    if let Some(x) = pl.x.x(pl.plot, tick.value) {
                        line(
                            canvas,
                            Pos2::new(x, pl.plot.top()),
                            Pos2::new(x, pl.plot.bottom()),
                            theme.grid,
                            1.0,
                        );
                    }
                }
            }
        }
        if let Some([a, b]) = state.selection {
            if let Some((x0, x1)) = pl.x.x(pl.plot, a).zip(pl.x.x(pl.plot, b)) {
                let r = Rect::from_min_max(
                    Pos2::new(x0.min(x1), pl.plot.top()),
                    Pos2::new(x0.max(x1), pl.plot.bottom()),
                );
                canvas.fill_rect(r, 0.0, theme.selection_fill);
                line(
                    canvas,
                    r.left_top(),
                    r.left_bottom(),
                    theme.selection_edge,
                    1.0,
                );
                line(
                    canvas,
                    r.right_top(),
                    r.right_bottom(),
                    theme.selection_edge,
                    1.0,
                );
            }
        }
        draw_marks(canvas, pl, marks, theme);
        for series in &pane.series {
            if !series.visible || state.hidden.contains(&series.id) {
                continue;
            }
            let points = screen_vertices(
                cache(&mut state.runtime.caches, series, config.gap_factor),
                series,
                pl.plot,
                pl.x,
                pl.y,
                config.decimate,
            );
            let stroke = Stroke::new(
                series.width.max(0.1),
                color_dim(
                    series.color,
                    state.highlighted.is_some_and(|id| id != series.id),
                ),
            );
            if series.draw != Draw::Points {
                for part in points.split(|p| !p.x.is_finite() || !p.y.is_finite()) {
                    canvas.polyline(part, stroke, series.dash);
                }
            }
            if matches!(series.draw, Draw::Points | Draw::LinesPoints) {
                for &p in points
                    .iter()
                    .filter(|p| p.x.is_finite() && p.y.is_finite() && pl.plot.contains(**p))
                {
                    canvas.circle(p, series.marker_radius.max(0.5), stroke.color, Stroke::NONE);
                }
            }
        }
        if let Some(box_rect) = state.runtime.box_rect().filter(|r| r.intersects(pl.plot)) {
            let r = box_rect.intersect(pl.plot);
            canvas.fill_rect(r, 0.0, theme.selection_fill);
            line(
                canvas,
                r.left_top(),
                r.right_top(),
                theme.selection_edge,
                1.0,
            );
            line(
                canvas,
                r.right_top(),
                r.right_bottom(),
                theme.selection_edge,
                1.0,
            );
            line(
                canvas,
                r.right_bottom(),
                r.left_bottom(),
                theme.selection_edge,
                1.0,
            );
            line(
                canvas,
                r.left_bottom(),
                r.left_top(),
                theme.selection_edge,
                1.0,
            );
        }
        if config.crosshair {
            if let Some(hover) = hover {
                if let Some(x) = pl.x.x(pl.plot, hover.x) {
                    line(
                        canvas,
                        Pos2::new(x, pl.plot.top()),
                        Pos2::new(x, pl.plot.bottom()),
                        theme.crosshair,
                        1.0,
                    );
                    for (id, _, sx, sy) in &hover.values {
                        if let Some(series) = pane.series.iter().find(|s| s.id == *id) {
                            if let Some((dx, dy)) = pl.x.x(pl.plot, *sx).zip(pl.y.y(pl.plot, *sy)) {
                                canvas.circle(
                                    Pos2::new(dx, dy),
                                    3.4,
                                    series.color,
                                    Stroke::new(1.3, theme.surface),
                                );
                            }
                        }
                    }
                }
            }
        }
        canvas.set_clip(None);
        for tick in &pl.y_ticks {
            if !tick.label.is_empty() {
                if let Some(y) = pl.y.y(pl.plot, tick.value) {
                    if y >= pl.plot.top() - 1.0 && y <= pl.plot.bottom() + 1.0 {
                        canvas.text(
                            Pos2::new(pl.plot.left() - 9.0, y),
                            Anchor::RightCenter,
                            &tick.label,
                            theme.font_size,
                            theme.axis_text,
                            false,
                        );
                    }
                }
            }
        }
        line(
            canvas,
            pl.plot.left_bottom(),
            pl.plot.right_bottom(),
            theme.axis_line,
            1.0,
        );
        if let Some(header) = pl.legend_rect {
            let heading = pane_heading(pane);
            let row_center = header.top() + (theme.font_size + 9.0) * 0.5;
            if !heading.is_empty() {
                canvas.text(
                    Pos2::new(header.left(), row_center),
                    Anchor::LeftCenter,
                    &heading,
                    theme.font_size - 1.0,
                    theme.text_muted,
                    true,
                );
            }
            for item in &pl.legend {
                if let Some(series) = pane.series.iter().find(|s| s.id == item.id) {
                    let hidden = !series.visible || state.hidden.contains(&series.id);
                    if pointer.is_some_and(|p| item.rect.contains(p)) {
                        canvas.fill_rect(
                            item.rect.shrink2(Vec2::new(0.0, 1.0)),
                            5.0,
                            theme.legend_bg,
                        );
                    }
                    let c = if hidden {
                        theme.text_muted.gamma_multiply(0.6)
                    } else {
                        series.color
                    };
                    let dot = Pos2::new(item.rect.left() + 7.0, item.rect.center().y);
                    if hidden {
                        canvas.circle(dot, 3.3, Color32::TRANSPARENT, Stroke::new(1.2, c));
                    } else {
                        canvas.circle(dot, 3.6, c, Stroke::NONE);
                    }
                    let pos = Pos2::new(item.rect.left() + 15.0, item.rect.center().y);
                    let label_color = if hidden { theme.text_muted } else { theme.text };
                    canvas.text(
                        pos,
                        Anchor::LeftCenter,
                        series.label,
                        theme.font_size,
                        label_color,
                        false,
                    );
                    if config.legend_values && !hidden && !series.overlay {
                        if let Some((_, _, _, value)) = hover
                            .and_then(|h| h.values.iter().find(|(id, _, _, _)| *id == series.id))
                        {
                            let width = canvas.measure_text(series.label, theme.font_size, false).x;
                            canvas.text(
                                pos + Vec2::new(width + 7.0, 0.0),
                                Anchor::LeftCenter,
                                &format::format_value(*value, series.unit, config.precision),
                                theme.font_size,
                                series.color,
                                true,
                            );
                        }
                    }
                }
            }
        }
        let pane_area = pl
            .legend_rect
            .map_or(pl.plot, |header| header.union(pl.plot));
        if pointer.is_some_and(|p| pane_area.contains(p)) {
            for action in &pl.actions {
                let active = match action.action {
                    PaneAction::ToggleLog(_) => pane.y_scale == Scale::Log10,
                    PaneAction::ToggleIncludeZero(_) => pane.include_zero,
                    PaneAction::ToggleLockY(_) => state.y_views.contains_key(&pl.id),
                    PaneAction::ToggleMaximize(_) => state.maximized == Some(pl.id),
                    _ => false,
                };
                let hovered = pointer.is_some_and(|p| action.rect.contains(p));
                if hovered || active {
                    canvas.fill_rect(
                        action.rect,
                        4.0,
                        if active {
                            theme.selection_fill
                        } else {
                            theme.legend_bg
                        },
                    );
                }
                let color = if active {
                    theme.selection_edge
                } else if hovered {
                    theme.text
                } else {
                    theme.text_muted
                };
                canvas.text(
                    action.rect.center(),
                    Anchor::CenterCenter,
                    action.label,
                    theme.font_size - 1.5,
                    color,
                    false,
                );
            }
        }
    }
    if let Some(last) = layout.panes.last() {
        for tick in &last.x_ticks {
            if !tick.label.is_empty() {
                if let Some(x) = last.x.x(last.plot, tick.value) {
                    if x >= last.plot.left() - 1.0 && x <= last.plot.right() + 1.0 {
                        canvas.text(
                            Pos2::new(x, layout.x_axis.top() + 8.0),
                            Anchor::CenterTop,
                            &tick.label,
                            theme.font_size,
                            theme.axis_text,
                            false,
                        );
                    }
                }
            }
        }
        if !config.x_label.is_empty() {
            canvas.text(
                Pos2::new(last.plot.center().x, layout.x_axis.bottom() - 1.0),
                Anchor::CenterBottom,
                config.x_label,
                theme.font_size,
                theme.axis_text,
                false,
            );
        }
        if config.crosshair {
            if let Some(h) = hover {
                if let Some(x) = last.x.x(last.plot, h.x) {
                    let label = x_readout(h.x, config);
                    let w = canvas.measure_text(&label, theme.font_size, false).x + 12.0;
                    pill(
                        canvas,
                        Pos2::new(
                            (x - w * 0.5).clamp(
                                last.plot.left(),
                                (last.plot.right() - w).max(last.plot.left()),
                            ),
                            layout.x_axis.top() + 3.0,
                        ),
                        &label,
                        theme.axis_line,
                        theme.text,
                        theme.font_size,
                    );
                }
            }
        }
    }
    if let Some(rect) = layout.overview {
        canvas.fill_rect(rect, 3.0, theme.bg);
        canvas.set_clip(Some(rect));
        if let Some(series) = panes
            .iter()
            .flat_map(|p| &p.series)
            .find(|s| s.visible && !state.hidden.contains(&s.id))
        {
            let full = if config.link_x {
                layout.full_x
            } else {
                layout.panes.first().map_or(layout.full_x, |p| p.full_x)
            };
            let x = axis_for(full, config.x_scale, config.x_flip);
            let (_, data) = cache(&mut state.runtime.caches, series, config.gap_factor).samples(
                series,
                x,
                rect.width() as usize,
                false,
                false,
            );
            let mut extent = [f64::INFINITY, f64::NEG_INFINITY];
            for [_, y] in data.iter().copied() {
                if y.is_finite() {
                    extent[0] = extent[0].min(y);
                    extent[1] = extent[1].max(y);
                }
            }
            let y = axis_for(
                expand_range(extent[0], extent[1], Scale::Linear, 0.05, false),
                Scale::Linear,
                false,
            );
            let points: Vec<_> = data
                .iter()
                .map(|[a, b]| {
                    x.x(rect, *a)
                        .zip(y.y(rect, *b))
                        .map(|(x, y)| Pos2::new(x, y))
                        .unwrap_or(Pos2::new(f32::NAN, f32::NAN))
                })
                .collect();
            for part in points.split(|p| !p.x.is_finite() || !p.y.is_finite()) {
                canvas.polyline(
                    part,
                    Stroke::new(1.0, series.color.gamma_multiply(0.6)),
                    Dash::Solid,
                );
            }
            if let Some(first) = layout.panes.first() {
                if let Some((a, b)) = x.x(rect, first.x.range[0]).zip(x.x(rect, first.x.range[1])) {
                    let r = Rect::from_min_max(
                        Pos2::new(a.min(b), rect.top()),
                        Pos2::new(a.max(b), rect.bottom()),
                    );
                    canvas.fill_rect(r, 0.0, theme.selection_fill);
                    line(
                        canvas,
                        r.left_top(),
                        r.left_bottom(),
                        theme.selection_edge,
                        1.5,
                    );
                    line(
                        canvas,
                        r.right_top(),
                        r.right_bottom(),
                        theme.selection_edge,
                        1.5,
                    );
                }
            }
            if let Some([a, b]) = state.selection {
                if let Some((a, b)) = x.x(rect, a).zip(x.x(rect, b)) {
                    canvas.fill_rect(
                        Rect::from_min_max(
                            Pos2::new(a.min(b), rect.top()),
                            Pos2::new(a.max(b), rect.bottom()),
                        ),
                        0.0,
                        theme.selection_fill,
                    );
                }
            }
        }
        canvas.set_clip(None);
    }
    if config.tooltip {
        if let (Some(h), Some(pos)) = (hover, pointer) {
            let x_value = x_readout(h.x, config);
            let mut rows = vec![if config.x_label.is_empty() {
                x_value
            } else {
                format!("{}  {}", config.x_label, x_value)
            }];
            for pane in panes {
                for s in &pane.series {
                    if let Some((_, _, _, v)) = h.values.iter().find(|(id, _, _, _)| *id == s.id) {
                        rows.push(format!(
                            "{}  {}",
                            s.label,
                            format::format_value(*v, s.unit, config.precision)
                        ));
                    }
                }
            }
            let width = rows
                .iter()
                .map(|s| canvas.measure_text(s, theme.font_size, false).x)
                .fold(0.0_f32, f32::max)
                + 16.0;
            let height = rows.len() as f32 * (theme.font_size + 5.0) + 8.0;
            let r = Rect::from_min_size(
                Pos2::new(
                    (pos.x + 14.0).min(layout.rect.right() - width),
                    (pos.y + 14.0).min(layout.rect.bottom() - height),
                ),
                Vec2::new(width, height),
            );
            canvas.fill_rect(r, 5.0, theme.legend_bg);
            for (i, row) in rows.iter().enumerate() {
                canvas.text(
                    r.min + Vec2::new(8.0, 4.0 + i as f32 * (theme.font_size + 5.0)),
                    Anchor::LeftTop,
                    row,
                    theme.font_size,
                    theme.text,
                    false,
                );
            }
        }
    }
    if let Some(pos) = pointer {
        let action = layout
            .panes
            .iter()
            .flat_map(|p| &p.actions)
            .find(|a| a.rect.contains(pos));
        let text = if let Some(action) = action {
            Some(action.hint)
        } else {
            layout
                .panes
                .iter()
                .flat_map(|p| &p.legend)
                .find(|entry| entry.rect.contains(pos))
                .and_then(|entry| {
                    panes
                        .iter()
                        .flat_map(|p| &p.series)
                        .find(|s| s.id == entry.id)
                        .and_then(|series| {
                            (canvas.measure_text(series.label, theme.font_size, false).x + 16.0
                                > entry.rect.width())
                            .then_some(series.label)
                        })
                })
        };
        if let Some(text) = text {
            let w = canvas.measure_text(text, theme.font_size, false).x + 12.0;
            let x = (pos.x + 8.0).min((layout.rect.right() - w).max(layout.rect.left()));
            pill(
                canvas,
                Pos2::new(
                    x,
                    (pos.y + 20.0).min(layout.rect.bottom() - theme.font_size - 8.0),
                ),
                text,
                theme.axis_line,
                theme.text,
                theme.font_size,
            );
        }
    }
}
fn hover_at(
    layout: &Layout,
    state: &ChartState,
    panes: &[Pane<'_>],
    pointer: Option<Pos2>,
) -> Option<Hover> {
    let pos = pointer?;
    let pl = layout.panes.iter().find(|p| p.plot.contains(pos))?;
    let x = pl.x.at_x(pl.plot, pos.x);
    let mut values = Vec::new();
    for pane in panes
        .iter()
        .filter(|pane| layout.panes.iter().any(|p| p.id == pane.id))
    {
        for series in &pane.series {
            if !series.visible || state.hidden.contains(&series.id) {
                continue;
            }
            let n = series.x.len().min(series.y.len());
            if n == 0 {
                continue;
            }
            let xs = &series.x[..n];
            let sorted = state
                .runtime
                .caches
                .get(&series.id)
                .is_some_and(|c| c.sorted);
            let index = if sorted {
                let i = xs.partition_point(|v| *v < x);
                if i == 0 {
                    0
                } else if i == n || (xs[i - 1] - x).abs() <= (xs[i] - x).abs() {
                    i - 1
                } else {
                    i
                }
            } else {
                match xs
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_finite())
                    .min_by(|(_, a), (_, b)| (*a - x).abs().total_cmp(&(*b - x).abs()))
                {
                    Some((i, _)) => i,
                    None => continue,
                }
            };
            let y = series.y[index];
            if xs[index].is_finite() && y.is_finite() {
                values.push((series.id, index, xs[index], y));
            }
        }
    }
    Some(Hover {
        x,
        pane: Some(pl.id),
        values,
    })
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &mut ChartState,
    panes: &[Pane<'_>],
    config: &ChartConfig<'_>,
    marks: &[Mark],
    theme: &ChartTheme,
) -> ChartResponse {
    let available = ui.available_size_before_wrap();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(available.x.max(80.0), available.y.max(80.0)),
        Sense::click_and_drag(),
    );
    let mut canvas = EguiCanvas::new(ui.painter().clone());
    let layout = layout(&mut canvas, state, panes, config, rect, theme);
    let events = interaction::handle(ui, state, panes, config, &layout, &response);
    let pointer = ui
        .ctx()
        .input(|i| i.pointer.hover_pos())
        .filter(|p| response.hovered() && rect.contains(*p));
    let hover = hover_at(&layout, state, panes, pointer);
    draw(
        &mut canvas,
        state,
        panes,
        config,
        marks,
        theme,
        &layout,
        hover.as_ref(),
        pointer,
    );
    ChartResponse {
        hover,
        clicked: events.clicked,
        context: events.context,
        selection_changed: events.selection_changed,
        view_changed: events.view_changed,
        legend_clicked: events.legend_clicked,
        pane_action: events.pane_action,
        response,
    }
}

pub(crate) fn svg(
    panes: &[Pane<'_>],
    state: &ChartState,
    config: &ChartConfig<'_>,
    marks: &[Mark],
    theme: &ChartTheme,
    fonts: &FontSet,
    size: [f32; 2],
) -> String {
    let size = [size[0].max(1.0), size[1].max(1.0)];
    let mut canvas = SvgCanvas::new(size, fonts);
    let mut state = state.clone();
    let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0], size[1]));
    let layout = layout(&mut canvas, &mut state, panes, config, rect, theme);
    draw(
        &mut canvas,
        &mut state,
        panes,
        config,
        marks,
        theme,
        &layout,
        None,
        None,
    );
    canvas.finish()
}
