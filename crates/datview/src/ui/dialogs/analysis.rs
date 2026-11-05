//! Analyze: curve fitting, damping (logarithmic decrement), fatigue
//! equivalent loads and a full statistics report for one series. Tools work
//! on the selected range when there is one, so "fit the loaded section" is
//! drag, then click.
use dv_chart::ChartState;
use dv_core::{dataframe::Column, DataFrame};
use dv_tools::{
    curve_fitting::{model_fit, models, FitOptions, FitResult, ModelInfo},
    damping::{freq_damp_estimator, DampingResult},
    fatigue::{equivalent_load_with_options, Bins, EquivalentLoad, FatigueOptions, RainflowMethod},
    stats::{summarize, SignalSummary},
};
use eframe::egui::{self, RichText};

use crate::{
    series::SeriesData,
    ui::{theme, widgets},
    workspace::{Overlay, Workspace},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tool {
    #[default]
    Fit,
    Damping,
    Fatigue,
    Report,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scope {
    #[default]
    Selection,
    View,
    All,
}

pub(crate) struct AnalysisWindow {
    pub open: bool,
    pub tool: Tool,
    scope: Scope,
    target: Option<u64>,
    catalog: Vec<ModelInfo>,
    model: usize,
    custom: String,
    order: usize,
    exponents: String,
    guesses: Vec<(String, f64)>,
    fit: Option<Result<FitOutput, String>>,
    damping: Option<Result<DampingResult, String>>,
    /// The envelope overlay must use exactly the samples of the estimate.
    damping_range: Option<[f64; 2]>,
    wohler: f64,
    teq: f64,
    bins: usize,
    method: RainflowMethod,
    fatigue: Option<Result<EquivalentLoad, String>>,
    overlay: bool,
    /// Bumped whenever a result changes; overlays are rebuilt only then.
    revision: u64,
    applied: Option<(Tool, bool, bool, Option<u64>, u64)>,
    /// The report for (series id, data generation, range bits); summaries of
    /// long series are too costly to recompute every frame.
    report: Option<((u64, u64, Option<[u64; 2]>), Vec<(&'static str, String)>)>,
}

struct FitOutput {
    result: FitResult,
    origin: f64,
    series: String,
    unit: String,
}

/// Requests the window cannot fulfil itself.
pub(crate) enum AnalysisOutcome {
    Notify(String),
    AddTable(DataFrame, String),
}

impl Default for AnalysisWindow {
    fn default() -> Self {
        let catalog = models();
        let model = catalog
            .iter()
            .position(|model| model.id == "fitter: polynomial_continuous")
            .unwrap_or(0);
        let mut window = Self {
            open: false,
            tool: Tool::Fit,
            scope: Scope::Selection,
            target: None,
            catalog,
            model,
            custom: String::new(),
            order: 1,
            exponents: "0, 1, 2".into(),
            guesses: Vec::new(),
            fit: None,
            damping: None,
            damping_range: None,
            wohler: 3.0,
            teq: 1.0,
            bins: 100,
            method: RainflowMethod::Windap,
            fatigue: None,
            overlay: true,
            revision: 0,
            applied: None,
            report: None,
        };
        window.reset_guesses();
        window
    }
}

impl AnalysisWindow {
    pub(crate) fn open_tool(&mut self, tool: Tool) {
        self.open = true;
        self.tool = tool;
    }

    fn reset_guesses(&mut self) {
        self.guesses = self
            .catalog
            .get(self.model)
            .map(|model| model.coefficients.clone())
            .unwrap_or_default();
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: &mut Workspace,
        chart: &ChartState,
    ) -> Option<AnalysisOutcome> {
        if !self.open {
            self.update_overlays(workspace);
            return None;
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let mut outcome = None;
        let mut open = self.open;
        egui::Window::new(
            RichText::new("Analyze")
                .family(theme::semibold())
                .size(15.0),
        )
        .id(egui::Id::unique("analysis-window"))
        .open(&mut open)
        .default_width(400.0)
        .default_pos(ctx.content_rect().right_top() + egui::vec2(-430.0, 90.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            if let Some(tool) = widgets::segmented(
                ui,
                self.tool,
                &[
                    (Tool::Fit, "Curve fit", "Fit a model to one series"),
                    (
                        Tool::Damping,
                        "Damping",
                        "Natural frequency and damping from a decaying oscillation",
                    ),
                    (
                        Tool::Fatigue,
                        "Fatigue",
                        "Rainflow counting and damage-equivalent load",
                    ),
                    (Tool::Report, "Report", "Every statistic for one series"),
                ],
            ) {
                self.tool = tool;
            }
            ui.add_space(8.0);
            if workspace.series.is_empty() {
                ui.label(RichText::new("Plot a channel to analyze it.").color(tokens.text_muted));
                return;
            }
            self.series_picker(ui, workspace);
            self.scope_picker(ui, chart);
            let Some(series) = self
                .target
                .and_then(|id| workspace.series.iter().find(|series| series.id == id))
            else {
                return;
            };
            let range = self.range(chart);
            ui.separator();
            outcome = match self.tool {
                Tool::Fit => self.fit_ui(ui, series, range),
                Tool::Damping => {
                    self.damping_ui(ui, series, range);
                    None
                }
                Tool::Fatigue => {
                    self.fatigue_ui(ui, series, range);
                    None
                }
                Tool::Report => {
                    let key = (series.id, series.generation, range.map(|[a, b]| [a.to_bits(), b.to_bits()]));
                    if self.report.as_ref().is_none_or(|(cached, _)| *cached != key) {
                        let summary = summarize(&series.x, &series.y, range);
                        self.report = Some((key, report_rows(&summary, &series.unit, series.datetime)));
                    }
                    let rows = self.report.as_ref().map_or(&[][..], |(_, rows)| rows.as_slice());
                    report_ui(ui, &series.label, rows)
                }
            };
        });
        self.open = open;
        self.update_overlays(workspace);
        outcome
    }

    fn series_picker(&mut self, ui: &mut egui::Ui, workspace: &Workspace) {
        if self
            .target
            .is_none_or(|id| !workspace.series.iter().any(|series| series.id == id))
        {
            self.target = workspace.series.first().map(|series| series.id);
            self.clear_results();
        }
        let current = self
            .target
            .and_then(|id| workspace.series.iter().find(|series| series.id == id))
            .map_or("", |series| series.label.as_str());
        ui.horizontal(|ui| {
            ui.label("Series");
            egui::ComboBox::from_id_salt("analysis-series")
                .width(ui.available_width())
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for series in &workspace.series {
                        if ui
                            .selectable_label(self.target == Some(series.id), &series.label)
                            .clicked()
                        {
                            self.target = Some(series.id);
                            self.clear_results();
                        }
                    }
                });
        });
    }

    fn scope_picker(&mut self, ui: &mut egui::Ui, chart: &ChartState) {
        let has_selection = chart.selection.is_some();
        if !has_selection && self.scope == Scope::Selection {
            self.scope = Scope::View;
        }
        let choices: &[(Scope, &str, &str)] = if has_selection {
            &[
                (
                    Scope::Selection,
                    "Selection",
                    "Samples in the selected range",
                ),
                (Scope::View, "View", "Samples in the visible range"),
                (Scope::All, "All", "Every sample"),
            ]
        } else {
            &[
                (
                    Scope::View,
                    "View",
                    "Samples in the visible range · drag across the chart to select a range",
                ),
                (Scope::All, "All", "Every sample"),
            ]
        };
        ui.horizontal(|ui| {
            ui.label("Samples");
            if let Some(scope) = widgets::segmented(ui, self.scope, choices) {
                self.scope = scope;
                self.clear_results();
            }
        });
    }

    fn range(&self, chart: &ChartState) -> Option<[f64; 2]> {
        let range = match self.scope {
            Scope::Selection => chart.selection,
            Scope::View => chart.x_view,
            Scope::All => None,
        };
        range.map(|[a, b]| [a.min(b), a.max(b)])
    }

    fn clear_results(&mut self) {
        self.fit = None;
        self.damping = None;
        self.fatigue = None;
        self.revision += 1;
    }

    fn fit_ui(
        &mut self,
        ui: &mut egui::Ui,
        series: &SeriesData,
        range: Option<[f64; 2]>,
    ) -> Option<AnalysisOutcome> {
        let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
        let mut outcome = None;
        let before = self.model;
        ui.horizontal(|ui| {
            ui.label("Model");
            let label = if self.model == usize::MAX {
                "Custom formula"
            } else {
                self.catalog
                    .get(self.model)
                    .map_or("", |model| model.label.as_str())
            };
            egui::ComboBox::from_id_salt("fit-model")
                .width(ui.available_width())
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (index, model) in self.catalog.iter().enumerate() {
                        ui.selectable_value(&mut self.model, index, &model.label)
                            .on_hover_text(&model.formula);
                    }
                    ui.selectable_value(&mut self.model, usize::MAX, "Custom formula");
                });
        });
        if self.model != before {
            self.reset_guesses();
            self.fit = None;
        }
        let id = if self.model == usize::MAX {
            String::new()
        } else {
            self.catalog[self.model].id.clone()
        };
        if self.model == usize::MAX {
            ui.add(
                egui::TextEdit::singleline(&mut self.custom)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("{a} * exp(-x / {tau}) + {c}")
                    .desired_width(f32::INFINITY),
            );
            super::hint(
                ui,
                "Coefficients are written in braces; x is the series' X value.",
            );
        } else {
            ui.label(
                RichText::new(&self.catalog[self.model].formula)
                    .monospace()
                    .size(11.5)
                    .color(tokens.text_secondary),
            );
        }
        if id == "fitter: polynomial_continuous" {
            ui.horizontal(|ui| {
                ui.label("Order");
                ui.add(egui::DragValue::new(&mut self.order).range(0..=12));
            });
        } else if id == "fitter: polynomial_discrete" {
            ui.horizontal(|ui| {
                ui.label("Exponents");
                ui.add(
                    egui::TextEdit::singleline(&mut self.exponents)
                        .hint_text("0, 2, 3")
                        .desired_width(140.0),
                );
            });
        }
        if !self.guesses.is_empty() {
            egui::CollapsingHeader::new("Initial guesses")
                .default_open(false)
                .show(ui, |ui| {
                    egui::Grid::new("fit-guesses")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            for (name, value) in &mut self.guesses {
                                ui.label(name.as_str());
                                ui.add(egui::DragValue::new(value).speed(0.01));
                                ui.end_row();
                            }
                        });
                });
        }
        ui.horizontal(|ui| {
            if super::primary_button(ui, "Fit", true).clicked() {
                self.fit = Some(self.run_fit(&id, series, range));
                self.revision += 1;
            }
            ui.checkbox(&mut self.overlay, "Show on chart");
        });
        match &self.fit {
            Some(Ok(fit)) => {
                ui.add_space(6.0);
                let value = |v: f64| dv_chart::format::format_value(v, "", 6);
                widgets::property_grid(ui, "fit-results", |ui| {
                    ui.label("R²");
                    ui.label(RichText::new(value(fit.result.r2)).strong());
                    ui.end_row();
                    ui.label("RMSE");
                    ui.label(dv_chart::format::format_value(
                        fit.result.rmse,
                        &fit.unit,
                        4,
                    ));
                    ui.end_row();
                    for (name, coefficient) in &fit.result.coefficients {
                        ui.label(name.as_str());
                        ui.label(value(*coefficient));
                        ui.end_row();
                    }
                    ui.label("Iterations");
                    ui.label(fit.result.iterations.to_string());
                    ui.end_row();
                });
                if fit.origin != 0.0 {
                    super::hint(
                        ui,
                        "X is measured from the first sample of the fitted range.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Copy results").clicked() {
                        let mut text = format!(
                            "{}\nformula\t{}\nR²\t{}\nRMSE\t{}\n",
                            fit.series, fit.result.formula, fit.result.r2, fit.result.rmse
                        );
                        for (name, coefficient) in &fit.result.coefficients {
                            text.push_str(&format!("{name}\t{coefficient}\n"));
                        }
                        ui.ctx().copy_text(text);
                        outcome = Some(AnalysisOutcome::Notify("Fit results copied".into()));
                    }
                    if ui
                        .button("Add fit as table")
                        .on_hover_text("Adds the fitted curve as a new source")
                        .clicked()
                    {
                        let x: Vec<f64> = fit.result.x.iter().map(|x| x + fit.origin).collect();
                        if let Ok(df) = DataFrame::from_columns(vec![
                            Column::float("x", x),
                            Column::float(
                                format!("Fit of {}", fit.series),
                                fit.result.y_fit.clone(),
                            ),
                        ]) {
                            outcome = Some(AnalysisOutcome::AddTable(
                                df,
                                format!("Fit · {}", fit.series),
                            ));
                        }
                    }
                });
            }
            Some(Err(error)) => {
                ui.label(RichText::new(error).color(tokens.error));
            }
            None => {}
        }
        outcome
    }

    fn run_fit(
        &self,
        id: &str,
        series: &SeriesData,
        range: Option<[f64; 2]>,
    ) -> Result<FitOutput, String> {
        let (x, y) = samples(series, range);
        if x.len() < 2 {
            return Err("Not enough samples in this range.".into());
        }
        // Epoch seconds are ~1.7e9: fitting polynomials there is ill-conditioned.
        let origin = if series.datetime { x[0] } else { 0.0 };
        let x: Vec<f64> = x.iter().map(|x| x - origin).collect();
        let mut options = FitOptions {
            order: self.order,
            max_iterations: 2000,
            ..FitOptions::default()
        };
        options.guesses = self.guesses.iter().cloned().collect();
        if id == "fitter: polynomial_discrete" {
            options.exponents = self
                .exponents
                .split(',')
                .map(|part| {
                    part.trim()
                        .parse::<i32>()
                        .map_err(|_| format!("“{}” is not an integer exponent", part.trim()))
                })
                .collect::<Result<_, _>>()?;
        }
        let model = if self.model == usize::MAX {
            format!("eval: {}", self.custom)
        } else {
            id.to_owned()
        };
        let result = model_fit(&model, &x, &y, &options).map_err(|error| error.to_string())?;
        Ok(FitOutput {
            result,
            origin,
            series: series.label.clone(),
            unit: series.unit.clone(),
        })
    }

    fn damping_ui(&mut self, ui: &mut egui::Ui, series: &SeriesData, range: Option<[f64; 2]>) {
        let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
        super::hint(ui, "Select a decaying oscillation. Peaks are detected automatically (logarithmic decrement).");
        ui.horizontal(|ui| {
            if super::primary_button(ui, "Estimate", true).clicked() {
                let (x, y) = samples(series, range);
                self.damping_range = range;
                self.revision += 1;
                self.damping = Some(if x.len() < 4 {
                    Err("Not enough samples in this range.".into())
                } else {
                    freq_damp_estimator(&y, &x, "fromPeaks").map_err(|error| error.to_string())
                });
            }
            ui.checkbox(&mut self.overlay, "Show envelope");
        });
        match &self.damping {
            Some(Ok(result)) => {
                let value = |v: f64, unit: &str| dv_chart::format::format_value(v, unit, 5);
                widgets::property_grid(ui, "damping-results", |ui| {
                    for (label, text) in [
                        ("Natural frequency", value(result.fn_, "Hz")),
                        ("Damped frequency", value(result.fd, "Hz")),
                        ("Damped period", value(result.td, "s")),
                        (
                            "Damping ratio ζ",
                            format!(
                                "{} ({}%)",
                                value(result.zeta, ""),
                                value(result.zeta * 100.0, "")
                            ),
                        ),
                        (
                            "ζ range",
                            format!(
                                "{} – {}",
                                value(result.zeta_min, ""),
                                value(result.zeta_max, "")
                            ),
                        ),
                        (
                            "Logarithmic decrement",
                            format!(
                                "{} ± {}",
                                value(result.logdec, ""),
                                value(result.logdec_std, "")
                            ),
                        ),
                        (
                            "Peaks used",
                            format!(
                                "{} positive, {} negative",
                                result.positive_peaks.len(),
                                result.negative_peaks.len()
                            ),
                        ),
                    ] {
                        ui.label(label);
                        ui.label(text);
                        ui.end_row();
                    }
                });
                if !result.decay {
                    ui.label(
                        RichText::new(
                            "The peaks are not decaying; the estimate may be meaningless.",
                        )
                        .color(tokens.warning),
                    );
                }
            }
            Some(Err(error)) => {
                ui.label(RichText::new(error).color(tokens.error));
            }
            None => {}
        }
    }

    fn fatigue_ui(&mut self, ui: &mut egui::Ui, series: &SeriesData, range: Option<[f64; 2]>) {
        let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
        egui::Grid::new("fatigue-options")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Wöhler exponent m");
                ui.add(
                    egui::DragValue::new(&mut self.wohler)
                        .range(0.5..=30.0)
                        .speed(0.1),
                );
                ui.end_row();
                ui.label("Equivalent period");
                ui.add(
                    egui::DragValue::new(&mut self.teq)
                        .range(1e-6..=1e9)
                        .speed(0.1)
                        .suffix(" s"),
                );
                ui.end_row();
                ui.label("Range bins");
                ui.add(egui::DragValue::new(&mut self.bins).range(1..=10_000));
                ui.end_row();
                ui.label("Rainflow method");
                egui::ComboBox::from_id_salt("rainflow-method")
                    .selected_text(method_label(self.method))
                    .show_ui(ui, |ui| {
                        for method in [
                            RainflowMethod::Windap,
                            RainflowMethod::Astm,
                            RainflowMethod::Fatpack,
                        ] {
                            ui.selectable_value(&mut self.method, method, method_label(method));
                        }
                    });
                ui.end_row();
            });
        if super::primary_button(ui, "Compute", true).clicked() {
            let (x, y) = samples(series, range);
            let options = FatigueOptions {
                m: self.wohler,
                teq: self.teq,
                bins: Bins::Count(self.bins),
                method: self.method,
                ..FatigueOptions::default()
            };
            self.fatigue = Some(if x.len() < 3 {
                Err("Not enough samples in this range.".into())
            } else {
                equivalent_load_with_options(&x, &y, &options).map_err(|error| error.to_string())
            });
        }
        match &self.fatigue {
            Some(Ok(result)) => {
                let cycles: f64 = result.cycles.iter().sum();
                widgets::property_grid(ui, "fatigue-results", |ui| {
                    ui.label("Damage-equivalent load");
                    ui.label(
                        RichText::new(dv_chart::format::format_value(result.load, &series.unit, 5))
                            .strong(),
                    );
                    ui.end_row();
                    ui.label("Counted cycles");
                    ui.label(dv_chart::format::format_value(cycles, "", 5));
                    ui.end_row();
                    ui.label("Largest range");
                    ui.label(dv_chart::format::format_value(
                        result.ranges.iter().copied().fold(0.0, f64::max),
                        &series.unit,
                        5,
                    ));
                    ui.end_row();
                });
                super::hint(ui, "Ranges are peak-to-peak, binned; damage is normalized to the equivalent period.");
            }
            Some(Err(error)) => {
                ui.label(RichText::new(error).color(tokens.error));
            }
            None => {}
        }
    }

    /// Fits and damping envelopes are drawn as dashed overlays on their series.
    /// They are cloned into the workspace only when something changed.
    fn update_overlays(&mut self, workspace: &mut Workspace) {
        let signature = (
            self.tool,
            self.open,
            self.overlay,
            self.target,
            self.revision,
        );
        if self.applied == Some(signature) {
            return;
        }
        self.applied = Some(signature);
        workspace.overlays.clear();
        if !self.open || !self.overlay {
            return;
        }
        let Some(target) = self.target else { return };
        match self.tool {
            Tool::Fit => {
                if let Some(Ok(fit)) = &self.fit {
                    workspace.overlays.push(Overlay {
                        target,
                        label: "Fit".into(),
                        x: fit.result.x.iter().map(|x| x + fit.origin).collect(),
                        y: fit.result.y_fit.clone(),
                        generation: self.revision,
                    });
                }
            }
            Tool::Damping => {
                if let (Some(Ok(result)), Some(series)) = (
                    &self.damping,
                    workspace.series.iter().find(|series| series.id == target),
                ) {
                    let (x, _) = samples(series, self.damping_range);
                    if x.len() == result.epos.len() {
                        workspace.overlays.push(Overlay {
                            target,
                            label: "Upper envelope".into(),
                            x: x.clone(),
                            y: result.epos.clone(),
                            generation: self.revision,
                        });
                        workspace.overlays.push(Overlay {
                            target,
                            label: "Lower envelope".into(),
                            x,
                            y: result.eneg.clone(),
                            generation: self.revision,
                        });
                    }
                }
            }
            Tool::Fatigue | Tool::Report => {}
        }
    }
}

fn method_label(method: RainflowMethod) -> &'static str {
    match method {
        RainflowMethod::Windap => "WindAP (default)",
        RainflowMethod::Astm => "ASTM E1049",
        RainflowMethod::Fatpack => "fatpack",
    }
}

/// Finite (x, y) pairs whose x lies inside `range`.
fn samples(series: &SeriesData, range: Option<[f64; 2]>) -> (Vec<f64>, Vec<f64>) {
    series
        .x
        .iter()
        .zip(series.y.iter())
        .filter(|(x, y)| {
            x.is_finite() && y.is_finite() && range.is_none_or(|[a, b]| **x >= a && **x <= b)
        })
        .map(|(x, y)| (*x, *y))
        .unzip()
}

fn report_ui(
    ui: &mut egui::Ui,
    label: &str,
    rows: &[(&'static str, String)],
) -> Option<AnalysisOutcome> {
    let mut outcome = None;
    egui::ScrollArea::vertical()
        .max_height(420.0)
        .show(ui, |ui| {
            widgets::property_grid(ui, "report", |ui| {
                for (name, value) in rows {
                    ui.label(*name);
                    ui.label(value);
                    ui.end_row();
                }
            });
        });
    if ui.button("Copy report").clicked() {
        let text: String = rows
            .iter()
            .map(|(name, value)| format!("{name}\t{value}\n"))
            .collect();
        ui.ctx().copy_text(format!("{label}\n{text}"));
        outcome = Some(AnalysisOutcome::Notify("Report copied".into()));
    }
    outcome
}

fn report_rows(s: &SignalSummary, unit: &str, datetime: bool) -> Vec<(&'static str, String)> {
    use dv_chart::format::{format_duration, format_time, format_value};
    let v = |value: f64| format_value(value, unit, 6);
    let n = |value: f64| format_value(value, "", 6);
    // Datetime X is epoch seconds: positions read as clock times, spans as durations.
    let at = |value: f64| if datetime { format_time(value, 0.001) } else { n(value) };
    let span = |value: f64| if datetime { format_duration(value, 0.001) } else { n(value) };
    let mut rows = vec![
        ("Samples", format_value(s.n as f64, "", 15)),
        ("Minimum", v(s.min)),
        ("Maximum", v(s.max)),
        ("Mean", v(s.mean)),
        ("Median", v(s.median)),
        ("Standard deviation", v(s.std)),
        ("Variance", n(s.variance)),
        ("RMS", v(s.rms)),
        ("P1", v(s.p1)),
        ("P5", v(s.p5)),
        ("P95", v(s.p95)),
        ("P99", v(s.p99)),
        ("Peak-to-peak", v(s.peak_to_peak)),
        ("Absolute maximum", v(s.abs_max)),
        ("Start value", v(s.start)),
        ("End value", v(s.end)),
        ("Change (end − start)", v(s.delta)),
        ("Slope (least squares)", n(s.slope)),
        ("Integral", n(s.integral)),
        ("Integral mean", v(s.integral_mean)),
    ];
    // Moments about x are meaningless for absolute timestamps.
    if !datetime {
        rows.extend([
            ("Integral of x·y", n(s.integral_x)),
            ("Normalized integral of x·y", n(s.integral_x_normalized)),
            ("Integral of x²·y", n(s.integral_x2)),
        ]);
    }
    rows.extend([
        ("X at minimum", at(s.x_at_min)),
        ("X at maximum", at(s.x_at_max)),
        ("X span", span(s.duration)),
        ("Mean sample spacing", span(s.mean_dt)),
        ("Turbulence intensity (std/mean)", n(s.ti)),
    ]);
    rows
}
