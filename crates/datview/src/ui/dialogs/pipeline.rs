//! Processing: an ordered list of steps (mask, filter, resample, bin, …) with
//! typed forms. Steps are drafted here and applied together.
use dv_core::formula::eval_mask;
use dv_plot::{Action, Pipeline, Plugin};
use dv_tools::signal_analysis::{ParameterInfo, SamplerParameterShape, FILTERS, SAMPLERS};
use eframe::egui::{self, RichText};

use super::{hint, modal, primary_button};
use crate::{
    ui::{icons, theme, widgets},
    workspace::Workspace,
};

#[derive(Default)]
pub(crate) struct PipelineDialog {
    pub open: bool,
    steps: Vec<Step>,
    error: Option<String>,
}

struct Step {
    enabled: bool,
    plugin: Plugin,
    /// Text form of vector parameters (sampler coordinates), kept while typing.
    text: String,
}

pub(crate) enum PipelineOutcome {
    Applied(String),
    Failed(String),
}

const RENAME_FLAVORS: [(&str, &str); 4] = [
    ("OF2To3", "OpenFAST 2 → 3 (B1N… → AB1N…)"),
    ("OF3To2", "OpenFAST 3 → 2 (AB1N… → B1N…)"),
    ("FldToAero", "Fld → Aero"),
    ("AeroToFld", "Aero → Fld"),
];
const RADIAL_METHODS: [(&str, &str); 3] = [
    ("constantwindow", "Last part of the signal (fraction)"),
    ("periods", "Last N rotor revolutions"),
    ("periods_omega", "Last N revolutions, from rotor speed"),
];

impl PipelineDialog {
    pub(crate) fn open(&mut self, workspace: &Workspace) {
        self.open = true;
        self.error = None;
        self.steps = workspace
            .pipeline
            .actions
            .iter()
            .map(|action| Step {
                enabled: action.enabled,
                text: vector_text(&action.plugin),
                plugin: action.plugin.clone(),
            })
            .collect();
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: &mut Workspace,
    ) -> Option<PipelineOutcome> {
        if !self.open {
            return None;
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let mut apply = false;
        let mut outcome = None;
        let (_, closed) = modal(ctx, "pipeline-dialog", "Processing", 640.0, |ui, close| {
            hint(ui, "Steps run in order on every source. Filter, resample, bin and outlier steps change only what is plotted; mask, units and table steps change the tables (reloading restores the originals).");
            ui.add_space(8.0);
            let mut remove = None;
            let mut swap = None;
            let count = self.steps.len();
            egui::ScrollArea::vertical()
                .max_height(ctx.content_rect().height() * 0.55)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if self.steps.is_empty() {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("No processing steps. Add one below.")
                                .color(tokens.text_muted),
                        );
                        ui.add_space(12.0);
                    }
                    for (index, step) in self.steps.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            egui::Frame::new()
                                .fill(tokens.surface)
                                .stroke(egui::Stroke::new(1.0, tokens.border))
                                .corner_radius(8)
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.checkbox(&mut step.enabled, "");
                                        ui.label(
                                            RichText::new(step.plugin.summary())
                                                .family(theme::semibold())
                                                .color(if step.enabled {
                                                    tokens.text
                                                } else {
                                                    tokens.text_muted
                                                }),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if widgets::icon_button(
                                                    ui,
                                                    icons::TRASH,
                                                    "Remove step",
                                                )
                                                .clicked()
                                                {
                                                    remove = Some(index);
                                                }
                                                if index + 1 < count
                                                    && widgets::icon_button(
                                                        ui,
                                                        icons::MOVE_DOWN,
                                                        "Move down",
                                                    )
                                                    .clicked()
                                                {
                                                    swap = Some((index, index + 1));
                                                }
                                                if index > 0
                                                    && widgets::icon_button(
                                                        ui,
                                                        icons::MOVE_UP,
                                                        "Move up",
                                                    )
                                                    .clicked()
                                                {
                                                    swap = Some((index - 1, index));
                                                }
                                            },
                                        );
                                    });
                                    step_form(ui, step, workspace);
                                });
                            ui.add_space(6.0);
                        });
                    }
                });
            if let Some(index) = remove {
                self.steps.remove(index);
            }
            if let Some((a, b)) = swap {
                self.steps.swap(a, b);
            }
            ui.menu_button(RichText::new("+  Add step").color(tokens.accent), |ui| {
                ui.set_min_width(300.0);
                for info in Plugin::catalog() {
                    if ui
                        .button(info.label)
                        .on_hover_text(info.description)
                        .clicked()
                    {
                        self.steps.push(Step {
                            enabled: true,
                            text: vector_text(&info.default),
                            plugin: info.default,
                        });
                        ui.close();
                    }
                }
            });
            if let Some(error) = &self.error {
                ui.label(RichText::new(error).color(tokens.error));
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui
                    .button("Load…")
                    .on_hover_text("Replace these steps with a saved pipeline (JSON)")
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Pipeline", &["json"])
                        .pick_file()
                    {
                        match Pipeline::load(&path) {
                            Ok(pipeline) => {
                                self.steps = pipeline
                                    .actions
                                    .iter()
                                    .map(|action| Step {
                                        enabled: action.enabled,
                                        text: vector_text(&action.plugin),
                                        plugin: action.plugin.clone(),
                                    })
                                    .collect();
                                self.error = None;
                            }
                            Err(error) => {
                                self.error =
                                    Some(format!("Could not load {}: {error}", path.display()))
                            }
                        }
                    }
                }
                if ui
                    .button("Save…")
                    .on_hover_text(
                        "Save these steps to reuse them (JSON, also accepted by --pipeline)",
                    )
                    .clicked()
                {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Pipeline", &["json"])
                        .set_file_name("pipeline.json")
                        .save_file()
                    {
                        match self.pipeline().save(&path) {
                            Ok(()) => {
                                outcome = Some(PipelineOutcome::Applied(format!(
                                    "Saved pipeline to {}",
                                    path.display()
                                )))
                            }
                            Err(error) => {
                                self.error =
                                    Some(format!("Could not save {}: {error}", path.display()))
                            }
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Apply", true).clicked() {
                        apply = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(0.0, 30.0)))
                        .clicked()
                    {
                        *close = true;
                    }
                });
            });
        });
        if apply {
            for step in &mut self.steps {
                if let Err(error) = parse_vector(step) {
                    return Some(PipelineOutcome::Failed(error));
                }
            }
            workspace.pipeline = self.pipeline();
            workspace.rebuild();
            self.open = false;
            let active = self.steps.iter().filter(|step| step.enabled).count();
            outcome = Some(PipelineOutcome::Applied(match active {
                0 => "Processing cleared".to_owned(),
                1 => "Applied 1 processing step".to_owned(),
                n => format!("Applied {n} processing steps"),
            }));
        }
        if closed {
            self.open = false;
        }
        outcome
    }

    fn pipeline(&self) -> Pipeline {
        let mut pipeline = Pipeline::default();
        for (index, step) in self.steps.iter().enumerate() {
            let mut action = Action::new(
                format!("{} {}", step.plugin.summary(), index + 1),
                step.plugin.clone(),
            );
            action.enabled = step.enabled;
            pipeline.actions.push(action);
        }
        pipeline
    }
}

fn step_form(ui: &mut egui::Ui, step: &mut Step, workspace: &Workspace) {
    let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
    match &mut step.plugin {
        Plugin::Mask { expression } => {
            ui.add(
                egui::TextEdit::singleline(expression)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("{Time} > 5 and {CPU Package Power} < 150")
                    .desired_width(f32::INFINITY),
            );
            if let Some(table) = workspace
                .sources
                .iter()
                .zip(&workspace.tables.tabs)
                .find(|(source, _)| source.active)
                .map(|(_, table)| table)
            {
                if !expression.trim().is_empty() {
                    match eval_mask(&table.data, expression) {
                        Ok(mask) => {
                            let kept = mask.iter().filter(|&&keep| keep).count();
                            hint(
                                ui,
                                &format!(
                                    "Keeps {} of {} rows in the first source",
                                    count(kept),
                                    count(mask.len())
                                ),
                            );
                        }
                        Err(error) => {
                            ui.label(
                                RichText::new(error.to_string())
                                    .size(11.5)
                                    .color(tokens.error),
                            );
                        }
                    }
                }
            }
        }
        Plugin::Filter { name, param } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("filter-kind")
                    .selected_text(name.as_str())
                    .show_ui(ui, |ui| {
                        for filter in FILTERS {
                            if ui
                                .selectable_label(name == filter.name, filter.name)
                                .clicked()
                            {
                                *name = filter.name.to_owned();
                                *param = filter.parameter;
                            }
                        }
                    });
                if let Some(info) = FILTERS
                    .iter()
                    .find(|filter| filter.name == name)
                    .map(|filter| filter.parameter_info())
                {
                    parameter(ui, &info, param);
                }
            });
        }
        Plugin::Sampler { name, param } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("sampler-kind")
                    .selected_text(name.as_str())
                    .show_ui(ui, |ui| {
                        for sampler in SAMPLERS {
                            if ui
                                .selectable_label(name == sampler.name, sampler.name)
                                .clicked()
                            {
                                *name = sampler.name.to_owned();
                                *param = sampler.default_parameters().to_vec();
                                step.text = join(param);
                            }
                        }
                    });
            });
            if let Some(sampler) = SAMPLERS.iter().find(|sampler| sampler.name == name) {
                let fields = sampler.parameter_info();
                match sampler.parameter_shape() {
                    SamplerParameterShape::Vector => {
                        ui.horizontal(|ui| {
                            ui.label(fields.first().map_or("Values", |field| field.label));
                            ui.add(
                                egui::TextEdit::singleline(&mut step.text)
                                    .hint_text("1, 2, 5")
                                    .desired_width(260.0),
                            );
                        });
                    }
                    SamplerParameterShape::Fixed => {
                        param.resize(fields.len(), 0.0);
                        ui.horizontal_wrapped(|ui| {
                            for (info, value) in fields.iter().zip(param.iter_mut()) {
                                parameter(ui, info, value);
                            }
                        });
                    }
                    SamplerParameterShape::OneOrThree => {
                        let mut bounds = param.len() == 3;
                        if param.is_empty() {
                            param.push(0.1);
                        }
                        ui.horizontal_wrapped(|ui| {
                            if let Some(info) = fields.first() {
                                parameter(ui, info, &mut param[0]);
                            }
                            if ui.checkbox(&mut bounds, "Custom range").changed() {
                                if bounds {
                                    param.extend([f64::NAN, f64::NAN]);
                                } else {
                                    param.truncate(1);
                                }
                            }
                            if bounds {
                                param.resize(3, f64::NAN);
                                for (info, value) in
                                    fields.iter().skip(1).zip(param.iter_mut().skip(1))
                                {
                                    if value.is_nan() {
                                        *value = 0.0;
                                    }
                                    parameter(ui, info, value);
                                }
                            }
                        });
                    }
                }
            }
        }
        Plugin::Binning { bins, min, max } => {
            ui.horizontal(|ui| {
                ui.label("Bins");
                ui.add(egui::DragValue::new(bins).range(1..=1_000_000));
                ui.label("from");
                ui.add(egui::DragValue::new(min).speed(0.1));
                ui.label("to");
                ui.add(egui::DragValue::new(max).speed(0.1));
            });
        }
        Plugin::Outliers { median_deviation } => {
            ui.horizontal(|ui| {
                ui.label("Remove samples farther than");
                ui.add(
                    egui::DragValue::new(median_deviation)
                        .range(0.0..=1000.0)
                        .speed(0.1),
                );
                ui.label("median absolute deviations from the median");
            });
        }
        Plugin::Units { flavor } => {
            ui.horizontal(|ui| {
                ui.label("Convention");
                let current = flavor.clone();
                if let Some(choice) = widgets::segmented(
                    ui,
                    current.as_str(),
                    &[
                        ("SI", "SI", "Base SI units"),
                        ("WE", "Wind energy", "kN, kW, rpm, deg …"),
                    ],
                ) {
                    *flavor = choice.to_owned();
                }
            });
        }
        Plugin::RenameChannels { flavor } => {
            egui::ComboBox::from_id_salt("rename-flavor")
                .selected_text(
                    RENAME_FLAVORS
                        .iter()
                        .find(|(id, _)| id == flavor)
                        .map_or(flavor.as_str(), |(_, label)| label),
                )
                .show_ui(ui, |ui| {
                    for (id, label) in RENAME_FLAVORS {
                        if ui.selectable_label(flavor == id, label).clicked() {
                            *flavor = id.to_owned();
                        }
                    }
                });
        }
        Plugin::RadialAverage { method, param } => {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("radial-method")
                    .selected_text(
                        RADIAL_METHODS
                            .iter()
                            .find(|(id, _)| id == method)
                            .map_or(method.as_str(), |(_, label)| label),
                    )
                    .show_ui(ui, |ui| {
                        for (id, label) in RADIAL_METHODS {
                            if ui.selectable_label(method == id, label).clicked() {
                                *method = id.to_owned();
                            }
                        }
                    });
                ui.add(egui::DragValue::new(param).speed(0.1).range(0.0..=1e6));
            });
        }
        Plugin::RadialConcat => hint(
            ui,
            "Creates one table with spanwise channels concatenated per node.",
        ),
        Plugin::Concat { common_only } => {
            ui.checkbox(common_only, "Only channels present in every table");
        }
    }
}

fn parameter(ui: &mut egui::Ui, info: &ParameterInfo, value: &mut f64) {
    ui.label(info.label).on_hover_text(info.description);
    let mut drag = egui::DragValue::new(value)
        .speed(info.increment.max(1e-9))
        .max_decimals(info.digits.max(if info.integer { 0 } else { 3 }));
    if let (Some(min), Some(max)) = (info.min, info.max) {
        drag = drag.range(min..=max);
    } else if let Some(min) = info.min {
        drag = drag.range(min..=f64::MAX);
    }
    if !info.unit.is_empty() && info.unit != "x units" {
        drag = drag.suffix(format!(" {}", info.unit));
    }
    ui.add(drag);
    if info.integer {
        *value = value.round();
    }
}

fn vector_text(plugin: &Plugin) -> String {
    match plugin {
        Plugin::Sampler { param, .. } => join(param),
        _ => String::new(),
    }
}

fn join(values: &[f64]) -> String {
    values
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Commit the text field of vector samplers (Replace/Insert/Remove).
fn parse_vector(step: &mut Step) -> Result<(), String> {
    let Plugin::Sampler { name, param } = &mut step.plugin else {
        return Ok(());
    };
    let Some(sampler) = SAMPLERS.iter().find(|sampler| sampler.name == name) else {
        return Ok(());
    };
    if sampler.parameter_shape() != SamplerParameterShape::Vector {
        return Ok(());
    }
    let values: Result<Vec<f64>, _> = step
        .text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::parse::<f64>)
        .collect();
    match values {
        Ok(values) if !values.is_empty() => {
            *param = values;
            Ok(())
        }
        _ => Err(format!(
            "{name}: enter comma-separated numbers, e.g. 0, 0.5, 1"
        )),
    }
}

fn count(value: usize) -> String {
    dv_chart::format::format_value(value as f64, "", 15)
}
