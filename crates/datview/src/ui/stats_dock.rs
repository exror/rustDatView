use std::cmp::Ordering;
use std::collections::HashMap;

use dv_chart::ChartState;
use dv_core::common::clean_col;
use dv_core::units::{
    format_quantity, integral_unit, parse_unit, scale_energy_joules, split_name_unit, UnitFamily,
    UnitInfo,
};
use dv_tools::stats::{summarize, time_above, SignalSummary};
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use super::chart_view::pane_id;
use crate::series::SeriesData;
use crate::workspace::{Mode, Workspace};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StatsScope {
    #[default]
    Selection,
    View,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum StatsColumn {
    N,
    Min,
    Mean,
    Max,
    Std,
    P99,
    Energy,
    Integral,
    Median,
    Rms,
    P1,
    P5,
    P95,
    PeakToPeak,
    Start,
    End,
    Delta,
    Slope,
    TimeAbove,
    IntegralMean,
    XAtMin,
    XAtMax,
    Duration,
    MeanDx,
    Ti,
    Variance,
    AbsMax,
    IntegralX,
    IntegralXNormalized,
    IntegralX2,
}

impl StatsColumn {
    const ALL: [Self; 30] = [
        Self::N,
        Self::Min,
        Self::Mean,
        Self::Max,
        Self::Std,
        Self::P99,
        Self::Energy,
        Self::Integral,
        Self::Median,
        Self::Rms,
        Self::P1,
        Self::P5,
        Self::P95,
        Self::PeakToPeak,
        Self::Start,
        Self::End,
        Self::Delta,
        Self::Slope,
        Self::TimeAbove,
        Self::IntegralMean,
        Self::XAtMin,
        Self::XAtMax,
        Self::Duration,
        Self::MeanDx,
        Self::Ti,
        Self::Variance,
        Self::AbsMax,
        Self::IntegralX,
        Self::IntegralXNormalized,
        Self::IntegralX2,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::N => "N",
            Self::Min => "Min",
            Self::Mean => "Mean",
            Self::Max => "Max",
            Self::Std => "Std",
            Self::P99 => "P99",
            Self::Energy => "Energy",
            Self::Integral => "∫ dx",
            Self::Median => "Median",
            Self::Rms => "RMS",
            Self::P1 => "P1",
            Self::P5 => "P5",
            Self::P95 => "P95",
            Self::PeakToPeak => "Peak-to-peak",
            Self::Start => "Start",
            Self::End => "End",
            Self::Delta => "Δ",
            Self::Slope => "Slope",
            Self::TimeAbove => "Time above threshold",
            Self::IntegralMean => "Integral mean",
            Self::XAtMin => "X at min",
            Self::XAtMax => "X at max",
            Self::Duration => "Duration",
            Self::MeanDx => "Mean Δx",
            Self::Ti => "TI",
            Self::Variance => "Variance",
            Self::AbsMax => "Abs max",
            Self::IntegralX => "Integral x",
            Self::IntegralXNormalized => "Integral x normalized",
            Self::IntegralX2 => "Integral x²",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct StatsSettings {
    pub(crate) scope: StatsScope,
    pub(crate) columns: Vec<StatsColumn>,
    /// Threshold is interpreted in each displayed series' own unit.
    pub(crate) threshold: f64,
}

impl Default for StatsSettings {
    fn default() -> Self {
        Self {
            scope: StatsScope::Selection,
            columns: vec![
                StatsColumn::Min,
                StatsColumn::Mean,
                StatsColumn::Max,
                StatsColumn::Std,
                StatsColumn::P99,
                StatsColumn::Energy,
                StatsColumn::N,
            ],
            threshold: 0.0,
        }
    }
}

#[derive(Default)]
pub(crate) struct StatsDock {
    pub(crate) settings: StatsSettings,
    cache: HashMap<u64, [Option<CachedSummary>; 3]>,
    sort: Option<Sort>,
    had_selection: Option<bool>,
    focused: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CacheKey {
    generation: u64,
    range: Option<[u64; 2]>,
}

struct CachedSummary {
    key: CacheKey,
    summary: SignalSummary,
    unit: UnitInfo,
    x_unit: UnitInfo,
    above: Option<(u64, f64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Series,
    Unit,
    Column(StatsColumn),
}
#[derive(Clone, Copy)]
struct Sort {
    key: SortKey,
    descending: bool,
}

struct Row<'a> {
    series: &'a SeriesData,
    source: &'a str,
    cached: &'a CachedSummary,
    energy_factor: Option<f64>,
    datetime: bool,
}

#[derive(Clone, Copy)]
enum CopyFormat {
    Tsv,
    Csv,
    Markdown,
}

impl StatsDock {
    /// Restore session preferences without mistaking the restored selection
    /// for a newly drawn selection and replacing the saved scope.
    pub(crate) fn restore_settings(&mut self, settings: StatsSettings) {
        self.settings = settings;
        self.had_selection = None;
    }

    /// Draw the dock. An export failure is returned for the application's
    /// Problems list; copying uses egui's clipboard. `link_x` is the chart's
    /// current link setting, so View can use each unlinked pane's own range.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &Workspace,
        chart: &ChartState,
        link_x: bool,
        precision: usize,
        palette: &[egui::Color32],
    ) -> Option<String> {
        let selected = chart.selection.filter(valid_range);
        if self.had_selection == Some(false) && selected.is_some() {
            self.settings.scope = StatsScope::Selection;
        }
        self.had_selection = Some(selected.is_some());
        let mut action = None;
        let mut export = false;
        let tokens = super::theme::Tokens::for_dark(super::theme::is_dark(ui.ctx()));
        let effective_scope = if self.settings.scope == StatsScope::Selection && selected.is_none()
        {
            StatsScope::View
        } else {
            self.settings.scope
        };
        ui.horizontal(|ui| {
            super::widgets::section_label(ui, "Statistics");
            ui.add_space(8.0);
            let scopes: &[(StatsScope, &str, &str)] = if selected.is_some() {
                &[
                    (StatsScope::Selection, "Selection", "Samples inside the selected X range"),
                    (StatsScope::View, "View", "Samples in the visible X range"),
                    (StatsScope::All, "All", "Every sample"),
                ]
            } else {
                &[
                    (StatsScope::View, "View", "Samples in the visible X range · drag across the chart to select a range"),
                    (StatsScope::All, "All", "Every sample"),
                ]
            };
            if let Some(scope) = super::widgets::segmented(ui, effective_scope, scopes) {
                self.settings.scope = scope;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(RichText::new(super::icons::SLIDERS).family(super::theme::icon_family()).size(15.0), |ui| {
                    ui.set_min_width(220.0);
                    ui.label(RichText::new("Columns").strong());
                    for column in StatsColumn::ALL {
                        let mut enabled = self.settings.columns.contains(&column);
                        if ui.checkbox(&mut enabled, column.label()).changed() {
                            if enabled {
                                self.settings.columns.push(column);
                            } else {
                                self.settings.columns.retain(|item| *item != column);
                            }
                        }
                    }
                }).response.on_hover_text("Choose statistics");
                ui.menu_button(RichText::new(super::icons::COPY).family(super::theme::icon_family()).size(15.0), |ui| {
                    if ui.add(egui::Button::new("Copy for Excel (TSV)").shortcut_text("Ctrl+Shift+C")).clicked() {
                        action = Some(CopyFormat::Tsv);
                        ui.close();
                    }
                    if ui.button("Copy as CSV").clicked() {
                        action = Some(CopyFormat::Csv);
                        ui.close();
                    }
                    if ui.button("Copy as Markdown").clicked() {
                        action = Some(CopyFormat::Markdown);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Export as CSV…").clicked() {
                        export = true;
                        ui.close();
                    }
                }).response.on_hover_text("Copy or export statistics");
                if self.settings.columns.contains(&StatsColumn::TimeAbove) {
                    ui.add(egui::DragValue::new(&mut self.settings.threshold).speed(0.1));
                    ui.label(RichText::new("Threshold").color(tokens.text_secondary))
                        .on_hover_text("“Time above” counts time where each series exceeds this value, in its own unit");
                }
                if let Some([a, b]) = selected.filter(|_| effective_scope == StatsScope::Selection) {
                    if let Some(description) = range_description(workspace, a, b) {
                        ui.label(RichText::new(description).color(tokens.text_secondary));
                    }
                }
            });
        });
        ui.add_space(2.0);
        if ui.ctx().input(|input| input.pointer.primary_clicked()) {
            self.focused = ui.rect_contains_pointer(ui.max_rect());
        }
        if self.focused
            && !ui.ctx().text_edit_focused()
            && ui.ctx().input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::C,
                ))
            })
        {
            action = Some(CopyFormat::Tsv);
        }

        let scope = effective_scope;
        self.update_cache(workspace, chart, link_x, scope);
        if workspace.series.is_empty() {
            ui.add_space(6.0);
            ui.label(
                RichText::new("Select channels to see their statistics.").color(tokens.text_muted),
            );
        }
        let slot = scope_index(scope);
        let mut rows: Vec<_> = workspace
            .series
            .iter()
            .filter(|series| !chart.hidden.contains(&series.id))
            .filter_map(|series| {
                let cached = self.cache.get(&series.id)?.get(slot)?.as_ref()?;
                let source = workspace
                    .sources
                    .get(series.source)
                    .map(|source| source.name.as_str())
                    .unwrap_or("");
                let datetime = series.datetime;
                let x_seconds = if workspace.mode == Mode::Signals
                    && cached.x_unit.family == UnitFamily::Time
                {
                    Some(cached.x_unit.to_base)
                } else {
                    None
                };
                let energy_factor = x_seconds.and_then(|x_factor| {
                    integral_unit(&cached.unit, true).map(|(_, y_factor)| x_factor * y_factor)
                });
                Some(Row {
                    series,
                    source,
                    cached,
                    energy_factor,
                    datetime,
                })
            })
            .collect();
        if let Some(sort) = self.sort {
            rows.sort_by(|a, b| compare_rows(a, b, sort));
        }
        let multiple_sources = workspace.sources.len() > 1;
        let mut requested_sort = None;
        let columns = &self.settings.columns;
        let height = ui.available_height().max(50.0);
        egui::ScrollArea::horizontal()
            .id_salt("stats-horizontal")
            .show(ui, |ui| {
                let mut builder = TableBuilder::new(ui)
                    .id_salt(("stats-table", columns))
                    .striped(true)
                    .resizable(true)
                    .min_scrolled_height(0.0)
                    .max_scroll_height(height)
                    .cell_layout(egui::Layout::right_to_left(egui::Align::Center))
                    .column(Column::initial(240.0).at_least(120.0).clip(true))
                    .column(Column::initial(56.0).at_least(40.0).clip(true));
                for _ in columns {
                    builder = builder.column(Column::initial(104.0).at_least(64.0).clip(true));
                }
                builder
                    .header(22.0, |mut header| {
                        header.col(|ui| {
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    if sort_header(ui, "Series", SortKey::Series, self.sort) {
                                        requested_sort = Some(SortKey::Series);
                                    }
                                },
                            );
                        });
                        header.col(|ui| {
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    if sort_header(ui, "Unit", SortKey::Unit, self.sort) {
                                        requested_sort = Some(SortKey::Unit);
                                    }
                                },
                            );
                        });
                        for &column in columns {
                            header.col(|ui| {
                                if sort_header(
                                    ui,
                                    column.label(),
                                    SortKey::Column(column),
                                    self.sort,
                                ) {
                                    requested_sort = Some(SortKey::Column(column));
                                }
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(24.0, rows.len(), |mut table_row| {
                            let row = &rows[table_row.index()];
                            table_row.col(|ui| {
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        let color = super::chart_view::series_color(
                                            workspace, palette, row.series,
                                        );
                                        let (dot, _) = ui.allocate_exact_size(
                                            egui::vec2(10.0, 10.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().circle_filled(dot.center(), 4.0, color);
                                        let label = series_label(row, multiple_sources);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&label).color(tokens.text),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(label);
                                    },
                                );
                            });
                            table_row.col(|ui| {
                                ui.with_layout(
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(&row.cached.unit.symbol)
                                                .color(tokens.text_muted),
                                        );
                                    },
                                );
                            });
                            for &column in columns {
                                table_row.col(|ui| {
                                    ui.label(
                                        RichText::new(format_cell(row, column, precision))
                                            .color(tokens.text),
                                    );
                                });
                            }
                        });
                    });
            });
        if rows.is_empty() && !workspace.series.is_empty() {
            ui.label(RichText::new("No visible series.").weak());
        }
        if let Some(key) = requested_sort {
            self.sort = Some(match self.sort {
                Some(old) if old.key == key => Sort {
                    key,
                    descending: !old.descending,
                },
                _ => Sort {
                    key,
                    descending: false,
                },
            });
        }
        if let Some(format) = action {
            ui.ctx()
                .copy_text(serialize_rows(&rows, columns, multiple_sources, format));
        }
        if export {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("statistics.csv")
                .add_filter("CSV", &["csv"])
                .save_file()
            {
                let text = serialize_rows(&rows, columns, multiple_sources, CopyFormat::Csv);
                if let Err(error) = std::fs::write(&path, text) {
                    return Some(format!(
                        "Could not export statistics to {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        None
    }

    fn update_cache(
        &mut self,
        workspace: &Workspace,
        chart: &ChartState,
        link_x: bool,
        scope: StatsScope,
    ) {
        if self.cache.len() > workspace.series.len().saturating_mul(2).saturating_add(16) {
            self.cache
                .retain(|id, _| workspace.series.iter().any(|series| series.id == *id));
        }
        let threshold_needed = self.settings.columns.contains(&StatsColumn::TimeAbove);
        for series in &workspace.series {
            if chart.hidden.contains(&series.id) {
                continue;
            }
            let range = match scope {
                StatsScope::Selection => chart.selection,
                StatsScope::All => None,
                StatsScope::View if !link_x => chart
                    .pane_x_views
                    .get(&pane_id(&series.pane))
                    .copied()
                    .or(chart.x_view),
                StatsScope::View => chart.x_view,
            }
            .filter(valid_range)
            .map(normalize_range);
            let key = CacheKey {
                generation: series.generation,
                range: range.map(|range| [range[0].to_bits(), range[1].to_bits()]),
            };
            let slots = self
                .cache
                .entry(series.id)
                .or_insert_with(|| std::array::from_fn(|_| None));
            let cached = &mut slots[scope_index(scope)];
            if cached.as_ref().is_none_or(|cached| cached.key != key) {
                *cached = Some(CachedSummary {
                    key,
                    summary: summarize(&series.x, &series.y, range),
                    unit: parse_unit(&series.unit),
                    x_unit: x_unit(workspace, series),
                    above: None,
                });
            }
            if let Some(cached) = cached {
                if threshold_needed
                    && cached
                        .above
                        .is_none_or(|(threshold, _)| threshold != self.settings.threshold.to_bits())
                {
                    cached.above = Some((
                        self.settings.threshold.to_bits(),
                        time_above(&series.x, &series.y, self.settings.threshold, range),
                    ));
                }
            }
        }
    }
}

fn valid_range(range: &[f64; 2]) -> bool {
    range[0].is_finite() && range[1].is_finite()
}

/// "12:36:01.2 – 12:38:15.0 · 2:14" for time axes, "0.5 s – 2.5 s · Δ 2 s" otherwise.
fn range_description(workspace: &Workspace, a: f64, b: f64) -> Option<String> {
    let (a, b) = (a.min(b), a.max(b));
    let series = workspace.series.first()?;
    // A power of ten ~1/100 of the span, so labels only show meaningful digits.
    let step = 10_f64.powf(((b - a) / 100.0).max(1e-6).log10().floor());
    if series.datetime {
        use dv_chart::format::{format_duration, format_time};
        return Some(format!(
            "{} – {}  ·  {}",
            format_time(a, step),
            format_time(b, step),
            format_duration(b - a, step)
        ));
    }
    let unit = x_unit(workspace, series).symbol;
    let value = |v| dv_chart::format::format_value(v, &unit, 5);
    Some(format!(
        "{} – {}  ·  Δ {}",
        value(a),
        value(b),
        value(b - a)
    ))
}
fn normalize_range(range: [f64; 2]) -> [f64; 2] {
    [range[0].min(range[1]), range[0].max(range[1])]
}
fn scope_index(scope: StatsScope) -> usize {
    match scope {
        StatsScope::Selection => 0,
        StatsScope::View => 1,
        StatsScope::All => 2,
    }
}

fn x_unit(workspace: &Workspace, series: &SeriesData) -> UnitInfo {
    match workspace.mode {
        Mode::Spectrum => parse_unit(match workspace.options.fft.x_type.as_str() {
            "x" => "s",
            "2pi/x" => "rad/s",
            _ => "Hz",
        }),
        Mode::Distribution => column_unit(workspace, series, Some(series.key.as_str())),
        Mode::Normalized if workspace.options.normalized.x_scale => parse_unit(""),
        Mode::Polar => parse_unit(if workspace.options.polar.degrees {
            "°"
        } else {
            "rad"
        }),
        _ if series.datetime || raw_x_is_datetime(workspace, series) => parse_unit("s"),
        _ => column_unit(workspace, series, workspace.x_key.as_deref()),
    }
}

fn source_column<'a>(
    workspace: &'a Workspace,
    series: &SeriesData,
    key: &str,
) -> Option<&'a dv_core::Column> {
    workspace
        .tables
        .tabs
        .get(series.source)?
        .data
        .columns()
        .iter()
        .find(|column| clean_col(&column.name) == key)
}

fn raw_x_is_datetime(workspace: &Workspace, series: &SeriesData) -> bool {
    workspace
        .x_key
        .as_deref()
        .and_then(|key| source_column(workspace, series, key))
        .is_some_and(|column| column.is_datetime())
}

fn column_unit(workspace: &Workspace, series: &SeriesData, key: Option<&str>) -> UnitInfo {
    let Some(key) = key else {
        return parse_unit("");
    };
    if let Some(column) = source_column(workspace, series, key) {
        let (_, unit) = split_name_unit(&column.name);
        parse_unit(&unit)
    } else {
        parse_unit(
            workspace
                .channels
                .iter()
                .find(|channel| channel.key == key)
                .map_or("", |channel| channel.unit.as_str()),
        )
    }
}

fn sort_header(ui: &mut egui::Ui, label: &str, key: SortKey, sort: Option<Sort>) -> bool {
    let tokens = super::theme::Tokens::for_dark(super::theme::is_dark(ui.ctx()));
    let active = sort.is_some_and(|sort| sort.key == key);
    let suffix = match sort {
        Some(sort) if sort.key == key => {
            if sort.descending {
                "  ↓"
            } else {
                "  ↑"
            }
        }
        _ => "",
    };
    let text = RichText::new(format!("{label}{suffix}"))
        .size(11.5)
        .family(super::theme::semibold())
        .color(if active {
            tokens.text
        } else {
            tokens.text_muted
        });
    ui.add(
        egui::Label::new(text)
            .sense(egui::Sense::click())
            .truncate(),
    )
    .on_hover_text("Sort by this column")
    .clicked()
}

fn compare_rows(a: &Row<'_>, b: &Row<'_>, sort: Sort) -> Ordering {
    let ordering = match sort.key {
        SortKey::Series => (a.source, a.series.channel_label.as_str())
            .cmp(&(b.source, b.series.channel_label.as_str())),
        SortKey::Unit => a.cached.unit.symbol.cmp(&b.cached.unit.symbol),
        SortKey::Column(column) => {
            let a = sort_value(a, column);
            let b = sort_value(b, column);
            if a.is_nan() != b.is_nan() {
                return if a.is_nan() {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
            }
            a.total_cmp(&b)
        }
    };
    if sort.descending {
        ordering.reverse()
    } else {
        ordering
    }
}

fn sort_value(row: &Row<'_>, column: StatsColumn) -> f64 {
    let raw = value(row, column);
    let y = &row.cached.unit;
    let x = &row.cached.x_unit;
    match column {
        StatsColumn::Min
        | StatsColumn::Mean
        | StatsColumn::Max
        | StatsColumn::P99
        | StatsColumn::Median
        | StatsColumn::P1
        | StatsColumn::P5
        | StatsColumn::P95
        | StatsColumn::Start
        | StatsColumn::End
        | StatsColumn::IntegralMean => y.to_base_value(raw),
        StatsColumn::Std | StatsColumn::PeakToPeak | StatsColumn::Delta | StatsColumn::AbsMax => {
            raw * y.to_base
        }
        StatsColumn::Variance => raw * y.to_base * y.to_base,
        StatsColumn::Slope => raw * y.to_base / x.to_base,
        StatsColumn::XAtMin
        | StatsColumn::XAtMax
        | StatsColumn::Duration
        | StatsColumn::MeanDx
        | StatsColumn::TimeAbove
        | StatsColumn::IntegralXNormalized => raw * x.to_base,
        _ => raw,
    }
}

fn value(row: &Row<'_>, column: StatsColumn) -> f64 {
    let s = &row.cached.summary;
    match column {
        StatsColumn::N => s.n as f64,
        StatsColumn::Min => s.min,
        StatsColumn::Mean => s.mean,
        StatsColumn::Max => s.max,
        StatsColumn::Std => s.std,
        StatsColumn::P99 => s.p99,
        StatsColumn::Energy => row
            .energy_factor
            .map_or(f64::NAN, |factor| s.integral * factor),
        StatsColumn::Integral => s.integral,
        StatsColumn::Median => s.median,
        StatsColumn::Rms => s.rms,
        StatsColumn::P1 => s.p1,
        StatsColumn::P5 => s.p5,
        StatsColumn::P95 => s.p95,
        StatsColumn::PeakToPeak => s.peak_to_peak,
        StatsColumn::Start => s.start,
        StatsColumn::End => s.end,
        StatsColumn::Delta => s.delta,
        StatsColumn::Slope => s.slope,
        StatsColumn::TimeAbove => row.cached.above.map_or(f64::NAN, |(_, value)| value),
        StatsColumn::IntegralMean => s.integral_mean,
        StatsColumn::XAtMin => s.x_at_min,
        StatsColumn::XAtMax => s.x_at_max,
        StatsColumn::Duration => s.duration,
        StatsColumn::MeanDx => s.mean_dt,
        StatsColumn::Ti => s.ti,
        StatsColumn::Variance => s.variance,
        StatsColumn::AbsMax => s.abs_max,
        StatsColumn::IntegralX => s.integral_x,
        StatsColumn::IntegralXNormalized => s.integral_x_normalized,
        StatsColumn::IntegralX2 => s.integral_x2,
    }
}

fn format_cell(row: &Row<'_>, column: StatsColumn, precision: usize) -> String {
    let number = value(row, column);
    if column == StatsColumn::N {
        return count(row.cached.summary.n);
    }
    if !number.is_finite() {
        return "—".to_owned();
    }
    match column {
        // The Unit column already names the unit; repeating it (and switching
        // to GHz/kW per cell) makes a column hard to scan and compare.
        StatsColumn::Min
        | StatsColumn::Mean
        | StatsColumn::Max
        | StatsColumn::Std
        | StatsColumn::P99
        | StatsColumn::Median
        | StatsColumn::Rms
        | StatsColumn::P1
        | StatsColumn::P5
        | StatsColumn::P95
        | StatsColumn::PeakToPeak
        | StatsColumn::Start
        | StatsColumn::End
        | StatsColumn::Delta
        | StatsColumn::IntegralMean
        | StatsColumn::AbsMax => dv_chart::format::format_value(number, "", precision),
        StatsColumn::XAtMin | StatsColumn::XAtMax if row.datetime => dv_chart::format::format_time(
            number,
            (row.cached.summary.mean_dt.abs() / 10.0).max(1e-6),
        ),
        StatsColumn::XAtMin
        | StatsColumn::XAtMax
        | StatsColumn::Duration
        | StatsColumn::MeanDx
        | StatsColumn::TimeAbove
        | StatsColumn::IntegralXNormalized => {
            format_quantity(number, &row.cached.x_unit, precision)
        }
        StatsColumn::Energy => {
            let (scaled, unit) = scale_energy_joules(number);
            compound(scaled, unit, precision)
        }
        StatsColumn::Integral => compound(
            number,
            &unit_product(&row.cached.unit.symbol, &row.cached.x_unit.symbol, 1),
            precision,
        ),
        StatsColumn::Slope => compound(
            number,
            &unit_ratio(&row.cached.unit.symbol, &row.cached.x_unit.symbol),
            precision,
        ),
        StatsColumn::Variance => {
            compound(number, &unit_power(&row.cached.unit.symbol, 2), precision)
        }
        StatsColumn::IntegralX => compound(
            number,
            &unit_product(&row.cached.unit.symbol, &row.cached.x_unit.symbol, 2),
            precision,
        ),
        StatsColumn::IntegralX2 => compound(
            number,
            &unit_product(&row.cached.unit.symbol, &row.cached.x_unit.symbol, 3),
            precision,
        ),
        StatsColumn::Ti => format_quantity(number, &parse_unit(""), precision),
        StatsColumn::N => unreachable!(),
    }
}

fn compound(value: f64, unit: &str, precision: usize) -> String {
    let number = format_quantity(value, &parse_unit(""), precision);
    if unit.is_empty() {
        number
    } else {
        format!("{number} {unit}")
    }
}
fn unit_power(unit: &str, exponent: usize) -> String {
    if unit.is_empty() {
        String::new()
    } else if exponent == 1 {
        unit.to_owned()
    } else if exponent == 2 {
        format!("{unit}²")
    } else {
        format!("{unit}³")
    }
}
fn unit_product(y: &str, x: &str, exponent: usize) -> String {
    let x = unit_power(x, exponent);
    if y.is_empty() {
        x
    } else if x.is_empty() {
        y.to_owned()
    } else {
        format!("{y}·{x}")
    }
}
fn unit_ratio(y: &str, x: &str) -> String {
    if x.is_empty() {
        y.to_owned()
    } else if y.is_empty() {
        format!("1/{x}")
    } else {
        format!("{y}/{x}")
    }
}
fn series_label(row: &Row<'_>, multiple_sources: bool) -> String {
    if multiple_sources && !row.source.is_empty() {
        format!("{} · {}", row.source, row.series.channel_label)
    } else {
        row.series.channel_label.clone()
    }
}
fn count(value: usize) -> String {
    let raw = value.to_string();
    let mut output = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(ch);
    }
    output
}

fn serialize_rows(
    rows: &[Row<'_>],
    columns: &[StatsColumn],
    multiple_sources: bool,
    format: CopyFormat,
) -> String {
    let mut output = String::new();
    let mut header = vec!["Series".to_owned(), "Unit".to_owned()];
    for &column in columns {
        let title = column.label();
        header.push(title.to_owned());
        if has_derived_unit(column) {
            header.push(format!("{title} unit"));
        }
    }
    append_record(&mut output, &header, format);
    if matches!(format, CopyFormat::Markdown) {
        append_record(&mut output, &vec!["---".to_owned(); header.len()], format);
    }
    for row in rows {
        let mut record = Vec::with_capacity(header.len());
        record.push(series_label(row, multiple_sources));
        record.push(row.cached.unit.symbol.clone());
        for &column in columns {
            let number = value(row, column);
            record.push(if column == StatsColumn::N {
                row.cached.summary.n.to_string()
            } else if number.is_finite() {
                number.to_string()
            } else {
                String::new()
            });
            if has_derived_unit(column) {
                record.push(export_unit(row, column));
            }
        }
        append_record(&mut output, &record, format);
    }
    output
}

fn has_derived_unit(column: StatsColumn) -> bool {
    matches!(
        column,
        StatsColumn::Energy
            | StatsColumn::Integral
            | StatsColumn::Slope
            | StatsColumn::TimeAbove
            | StatsColumn::XAtMin
            | StatsColumn::XAtMax
            | StatsColumn::Duration
            | StatsColumn::MeanDx
            | StatsColumn::Variance
            | StatsColumn::IntegralX
            | StatsColumn::IntegralXNormalized
            | StatsColumn::IntegralX2
    )
}

fn export_unit(row: &Row<'_>, column: StatsColumn) -> String {
    let y = &row.cached.unit.symbol;
    let x = &row.cached.x_unit.symbol;
    match column {
        StatsColumn::Energy if row.energy_factor.is_some() => "J".to_owned(),
        StatsColumn::Integral => unit_product(y, x, 1),
        StatsColumn::Slope => unit_ratio(y, x),
        StatsColumn::Variance => unit_power(y, 2),
        StatsColumn::IntegralX => unit_product(y, x, 2),
        StatsColumn::IntegralX2 => unit_product(y, x, 3),
        StatsColumn::XAtMin | StatsColumn::XAtMax if row.datetime => {
            "s since Unix epoch (UTC)".to_owned()
        }
        StatsColumn::TimeAbove
        | StatsColumn::XAtMin
        | StatsColumn::XAtMax
        | StatsColumn::Duration
        | StatsColumn::MeanDx
        | StatsColumn::IntegralXNormalized => x.clone(),
        _ => String::new(),
    }
}

fn append_record(output: &mut String, fields: &[String], format: CopyFormat) {
    let separator = match format {
        CopyFormat::Tsv => "\t",
        CopyFormat::Csv => ",",
        CopyFormat::Markdown => " | ",
    };
    if matches!(format, CopyFormat::Markdown) {
        output.push_str("| ");
    }
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            output.push_str(separator);
        }
        match format {
            CopyFormat::Markdown => output.push_str(
                &field
                    .replace('\\', "\\\\")
                    .replace('|', "\\|")
                    .replace('\n', "<br>"),
            ),
            CopyFormat::Csv | CopyFormat::Tsv
                if field.contains('"')
                    || field.contains('\n')
                    || field.contains('\r')
                    || field.contains(separator) =>
            {
                output.push('\"');
                output.push_str(&field.replace('\"', "\"\""));
                output.push('\"');
            }
            CopyFormat::Csv | CopyFormat::Tsv => output.push_str(field),
        }
    }
    if matches!(format, CopyFormat::Markdown) {
        output.push_str(" |");
    }
    output.push('\n');
}
