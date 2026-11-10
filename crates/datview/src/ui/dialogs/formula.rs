//! "New channel": derived channels from formulas, with templates for the
//! everyday cases (sums of rails, efficiency, rate of change, energy).
use dv_core::{common::clean_col, formula::eval_formula, units::split_name_unit};
use eframe::egui::{self, Color32, RichText};

use super::{hint, modal, primary_button};
use crate::{
    channels::Channel,
    ui::{theme, widgets},
    workspace::Workspace,
};

#[derive(Default)]
pub(crate) struct FormulaDialog {
    pub open: bool,
    name: String,
    unit: String,
    expression: String,
    all_sources: bool,
    /// Full column name of the formula channel being edited.
    editing: Option<String>,
    preview: Option<(String, Result<Preview, String>)>,
    focus_expression: bool,
}

struct Preview {
    points: Vec<f32>,
    min: f64,
    mean: f64,
    max: f64,
    finite: usize,
    total: usize,
}

/// What the dialog did, for the app's notifications.
pub(crate) enum FormulaOutcome {
    Added(String),
    Failed(String),
}

impl FormulaDialog {
    pub(crate) fn open_new(&mut self) {
        *self = Self {
            open: true,
            all_sources: true,
            name: "New channel".into(),
            focus_expression: true,
            ..Self::default()
        };
    }

    /// Prefill from a built-in template. `primary` (a channel key) becomes A;
    /// otherwise the selection order decides A and B.
    pub(crate) fn open_template(
        &mut self,
        workspace: &Workspace,
        template: Template,
        primary: Option<&str>,
    ) {
        self.open_new();
        template.apply(self, workspace, primary);
    }

    /// Edit the formula channel with this key, if one exists in any source.
    pub(crate) fn open_edit(&mut self, workspace: &Workspace, key: &str) -> bool {
        let Some(formula) = workspace
            .tables
            .tabs
            .iter()
            .flat_map(|table| &table.formulas)
            .find(|formula| clean_col(&formula.name) == key)
        else {
            return false;
        };
        let (name, unit) = split_name_unit(&formula.name);
        *self = Self {
            open: true,
            name,
            unit,
            expression: formula.formula.clone(),
            all_sources: true,
            editing: Some(formula.name.clone()),
            focus_expression: true,
            ..Self::default()
        };
        true
    }

    fn full_name(&self) -> String {
        let name = self.name.trim();
        let unit = self.unit.trim();
        if unit.is_empty() {
            name.to_owned()
        } else {
            format!("{name} [{unit}]")
        }
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: &mut Workspace,
        accent: Color32,
    ) -> Option<FormulaOutcome> {
        if !self.open {
            return None;
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let title = if self.editing.is_some() {
            "Edit channel"
        } else {
            "New channel"
        };
        let mut outcome = None;
        let mut apply = false;
        let mut delete = false;
        let (_, closed) = modal(ctx, "formula-dialog", title, 620.0, |ui, close| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Name").color(tokens.text_secondary));
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(420.0));
                });
                ui.vertical(|ui| {
                    ui.label(RichText::new("Unit").color(tokens.text_secondary));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.unit)
                            .hint_text("e.g. W")
                            .desired_width(150.0),
                    );
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Formula").color(tokens.text_secondary));
            let edit = ui.add(
                egui::TextEdit::multiline(&mut self.expression)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("{CPU Package Power} + {GPU Power}")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .lock_focus(true),
            );
            if self.focus_expression {
                edit.request_focus();
                self.focus_expression = false;
            }
            self.suggestions(ui, workspace, &edit);
            hint(ui, "Reference channels as {name}. Functions: + − × ÷ **, abs, sqrt, exp, log, sin, gradient(y, x), cumtrapz(y, x), movmean(y, n), where(c, a, b), clip(x, lo, hi), min/max/mean.");
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Templates").color(tokens.text_secondary));
                for template in Template::ALL {
                    if widgets::chip(ui, template.label())
                        .on_hover_text(template.description())
                        .clicked()
                    {
                        template.apply(self, workspace, None);
                    }
                }
            });
            ui.add_space(10.0);
            self.preview_ui(ui, workspace, accent);
            ui.add_space(8.0);
            if workspace.sources.len() > 1 {
                ui.checkbox(
                    &mut self.all_sources,
                    "Add to every source that has these channels",
                );
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if self.editing.is_some() && ui.button("Delete channel").clicked() {
                    delete = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let valid = !self.name.trim().is_empty()
                        && !self.expression.trim().is_empty()
                        && self
                            .preview
                            .as_ref()
                            .is_some_and(|(_, preview)| preview.is_ok());
                    let label = if self.editing.is_some() {
                        "Save changes"
                    } else {
                        "Add channel"
                    };
                    if primary_button(ui, label, valid).clicked() {
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
            outcome = Some(self.apply(workspace));
            if matches!(outcome, Some(FormulaOutcome::Added(_))) {
                self.open = false;
            }
        } else if delete {
            outcome = Some(self.delete(workspace));
            self.open = false;
        }
        if closed {
            self.open = false;
        }
        outcome
    }

    /// While a `{` is open, offer channel names; clicking one completes it.
    fn suggestions(&mut self, ui: &mut egui::Ui, workspace: &mut Workspace, edit: &egui::Response) {
        let Some(open) = self.expression.rfind('{') else {
            return;
        };
        if self.expression[open..].contains('}') {
            return;
        }
        let query = self.expression[open + 1..].to_owned();
        let matches: Vec<_> = workspace.search(&query).into_iter().take(8).collect();
        if matches.is_empty() {
            return;
        }
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            for index in matches {
                let name = formula_name(&workspace.channels[index], &workspace.channels);
                if widgets::chip(ui, &name).clicked() {
                    chosen = Some(name);
                }
            }
        });
        let tab = ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
        if chosen.is_none() && tab && edit.has_focus() {
            chosen = workspace
                .search(&query)
                .first()
                .map(|&index| formula_name(&workspace.channels[index], &workspace.channels));
        }
        if let Some(label) = chosen {
            self.expression.truncate(open);
            self.expression.push('{');
            self.expression.push_str(&label);
            self.expression.push('}');
            self.focus_expression = true;
        }
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui, workspace: &Workspace, accent: Color32) {
        let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
        let Some((source_index, table)) = workspace
            .sources
            .iter()
            .zip(&workspace.tables.tabs)
            .enumerate()
            .find(|(_, (source, _))| source.active)
            .map(|(index, (_, table))| (index, table))
        else {
            return;
        };
        let key = format!(
            "{}\u{0}{}\u{0}{}",
            self.expression,
            source_index,
            table.identity()
        );
        if self
            .preview
            .as_ref()
            .is_none_or(|(cached, _)| cached != &key)
        {
            let result = if self.expression.trim().is_empty() {
                Err("Type a formula to preview the new channel.".to_owned())
            } else {
                eval_formula(&table.data, &self.expression)
                    .map(|values| Preview::new(&values))
                    .map_err(|error| error.to_string())
            };
            self.preview = Some((key, result));
        }
        let Some((_, result)) = &self.preview else {
            return;
        };
        egui::Frame::new()
            .fill(tokens.surface)
            .stroke(egui::Stroke::new(1.0, tokens.border))
            .corner_radius(8)
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match result {
                    Ok(preview) => {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 54.0),
                            egui::Sense::hover(),
                        );
                        let points: Vec<_> = preview
                            .points
                            .iter()
                            .enumerate()
                            .filter(|(_, y)| y.is_finite())
                            .map(|(i, &y)| {
                                egui::pos2(
                                    rect.left()
                                        + rect.width() * i as f32
                                            / (preview.points.len().max(2) - 1) as f32,
                                    rect.bottom() - rect.height() * y,
                                )
                            })
                            .collect();
                        if points.len() > 1 {
                            ui.painter()
                                .add(egui::Shape::line(points, egui::Stroke::new(1.5, accent)));
                        }
                        let unit = dv_core::units::parse_unit(&self.unit).symbol;
                        let value = |v: f64| dv_chart::format::format_value(v, &unit, 4);
                        let mut text = format!(
                            "min {}   ·   mean {}   ·   max {}",
                            value(preview.min),
                            value(preview.mean),
                            value(preview.max)
                        );
                        if preview.finite < preview.total {
                            text.push_str(&format!(
                                "   ·   {} of {} samples missing",
                                preview.total - preview.finite,
                                preview.total
                            ));
                        }
                        ui.label(RichText::new(text).color(tokens.text_secondary));
                    }
                    Err(error) => {
                        // An empty formula is a prompt, not a mistake.
                        let color = if self.expression.trim().is_empty() {
                            tokens.text_muted
                        } else {
                            tokens.error
                        };
                        ui.label(RichText::new(error).color(color));
                    }
                }
            });
    }

    fn apply(&mut self, workspace: &mut Workspace) -> FormulaOutcome {
        let full_name = self.full_name();
        let key = clean_col(&full_name);
        let mut added = 0;
        let mut errors = Vec::new();
        let active: Vec<_> = workspace
            .sources
            .iter()
            .map(|source| source.active)
            .collect();
        let first_active = active.iter().position(|&active| active);
        for (index, table) in workspace.tables.tabs.iter_mut().enumerate() {
            if !active[index] || (!self.all_sources && Some(index) != first_active) {
                continue;
            }
            let name = &workspace.sources[index].name;
            let result = match &self.editing {
                Some(old) => match table
                    .formulas
                    .iter()
                    .find(|formula| &formula.name == old)
                    .map(|formula| formula.pos)
                {
                    Some(pos) => table.set_column_by_formula(&full_name, &self.expression, pos),
                    None => continue,
                },
                None if table
                    .data
                    .columns()
                    .iter()
                    .any(|column| clean_col(&column.name) == key) =>
                {
                    errors.push(format!(
                        "{name} already has a channel named “{}”",
                        self.name.trim()
                    ));
                    continue;
                }
                None => table.add_column_by_formula(&full_name, &self.expression, None),
            };
            match result {
                Ok(()) => added += 1,
                Err(error) => errors.push(format!("{name}: {error}")),
            }
        }
        if added == 0 {
            return FormulaOutcome::Failed(if errors.is_empty() {
                "No source could compute this formula".into()
            } else {
                errors.join("\n")
            });
        }
        if let Some(old) = &self.editing {
            let old_key = clean_col(old);
            for selected in &mut workspace.selected {
                if *selected == old_key {
                    selected.clone_from(&key);
                }
            }
        }
        workspace.data_changed();
        if !workspace.selected.contains(&key) {
            workspace.toggle(&key);
        }
        workspace.rebuild();
        let sources = if added == 1 {
            "1 source".to_owned()
        } else {
            format!("{added} sources")
        };
        let verb = if self.editing.is_some() {
            "Updated"
        } else {
            "Added"
        };
        let mut message = format!("{verb} “{}” in {sources}", self.name.trim());
        if !errors.is_empty() {
            message.push_str(&format!(" · skipped {}", errors.len()));
        }
        FormulaOutcome::Added(message)
    }

    fn delete(&mut self, workspace: &mut Workspace) -> FormulaOutcome {
        let Some(old) = self.editing.clone() else {
            return FormulaOutcome::Failed("Nothing to delete".into());
        };
        for table in &mut workspace.tables.tabs {
            if let Some(pos) = table
                .formulas
                .iter()
                .find(|formula| formula.name == old)
                .map(|formula| formula.pos)
            {
                table.delete_columns(&[pos]);
            }
        }
        let key = clean_col(&old);
        workspace.selected.retain(|selected| *selected != key);
        workspace.data_changed();
        workspace.rebuild();
        FormulaOutcome::Added(format!("Deleted “{}”", split_name_unit(&old).0))
    }
}

impl Preview {
    fn new(values: &[f64]) -> Self {
        const BINS: usize = 96;
        let (mut min, mut max, mut sum, mut finite) =
            (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0usize);
        let mut bins = vec![(f64::INFINITY, f64::NEG_INFINITY); BINS];
        for (index, &value) in values.iter().enumerate() {
            if !value.is_finite() {
                continue;
            }
            min = min.min(value);
            max = max.max(value);
            sum += value;
            finite += 1;
            let bin = &mut bins[index * BINS / values.len().max(1)];
            bin.0 = bin.0.min(value);
            bin.1 = bin.1.max(value);
        }
        let span = (max - min).max(f64::EPSILON);
        let points = bins
            .iter()
            .map(|&(lo, hi)| {
                if lo.is_finite() {
                    (((lo + hi) * 0.5 - min) / span) as f32
                } else {
                    f32::NAN
                }
            })
            .collect();
        let mean = if finite > 0 {
            sum / finite as f64
        } else {
            f64::NAN
        };
        Self {
            points,
            min,
            mean,
            max,
            finite,
            total: values.len(),
        }
    }
}

/// Built-in starting points, filled from the selected channels and X axis.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Template {
    Sum,
    Difference,
    Ratio,
    Efficiency,
    RateOfChange,
    Energy,
    MovingAverage,
    TimesThousand,
    DivideThousand,
}

impl Template {
    pub(crate) const ALL: [Self; 9] = [
        Self::Sum,
        Self::Difference,
        Self::Ratio,
        Self::Efficiency,
        Self::RateOfChange,
        Self::Energy,
        Self::MovingAverage,
        Self::TimesThousand,
        Self::DivideThousand,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sum => "Sum",
            Self::Difference => "A − B",
            Self::Ratio => "A ÷ B",
            Self::Efficiency => "Efficiency",
            Self::RateOfChange => "Rate of change",
            Self::Energy => "Energy ∫ dt",
            Self::MovingAverage => "Moving average",
            Self::TimesThousand => "× 1000",
            Self::DivideThousand => "÷ 1000",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Sum => "Add the selected channels, e.g. total rail power",
            Self::Difference => "First selected channel minus the second",
            Self::Ratio => "First selected channel divided by the second",
            Self::Efficiency => "100 × output ÷ input, in percent (first ÷ second)",
            Self::RateOfChange => "Derivative of the first selected channel over X",
            Self::Energy => "Running integral over X; power in W over seconds gives J",
            Self::MovingAverage => "Centered moving mean of the first selected channel",
            Self::TimesThousand => "Scale the first selected channel up by 1000",
            Self::DivideThousand => "Scale the first selected channel down by 1000",
        }
    }

    fn apply(self, dialog: &mut FormulaDialog, workspace: &Workspace, primary: Option<&str>) {
        let channel = |key: &str| workspace.channels.iter().find(|channel| channel.key == key);
        let selected: Vec<_> = primary
            .into_iter()
            .chain(
                workspace
                    .selected
                    .iter()
                    .map(String::as_str)
                    .filter(|key| Some(*key) != primary),
            )
            .filter_map(channel)
            .collect();
        let x = workspace.x_key.as_deref().and_then(channel);
        let reference = |index: usize| {
            selected.get(index).map_or_else(
                || "A".to_owned(),
                |channel| formula_name(channel, &workspace.channels),
            )
        };
        let a = reference(0);
        let b = reference(1);
        let a_unit = selected
            .first()
            .map_or("", |channel| channel.unit.as_str())
            .to_owned();
        let x_name = x.map_or_else(
            || "Time".to_owned(),
            |channel| formula_name(channel, &workspace.channels),
        );
        let x_unit = x
            .map(|channel| {
                if channel.family == "Time" && channel.unit.is_empty() {
                    "s".to_owned()
                } else {
                    channel.unit.clone()
                }
            })
            .unwrap_or_default();
        let (name, unit, expression) = match self {
            Self::Sum => {
                let terms: Vec<_> = if selected.len() >= 2 {
                    selected
                        .iter()
                        .map(|c| format!("{{{}}}", formula_name(c, &workspace.channels)))
                        .collect()
                } else {
                    vec!["{A}".into(), "{B}".into()]
                };
                let same_unit = selected.iter().all(|c| c.unit == a_unit);
                (
                    "Total".to_owned(),
                    if same_unit {
                        a_unit.clone()
                    } else {
                        String::new()
                    },
                    terms.join(" + "),
                )
            }
            Self::Difference => (
                format!("{a} − {b}"),
                a_unit.clone(),
                format!("{{{a}}} - {{{b}}}"),
            ),
            Self::Ratio => (
                format!("{a} ÷ {b}"),
                String::new(),
                format!("{{{a}}} / {{{b}}}"),
            ),
            Self::Efficiency => (
                format!("{a} efficiency"),
                "%".to_owned(),
                format!("100 * {{{a}}} / {{{b}}}"),
            ),
            Self::RateOfChange => {
                let unit = match (a_unit.is_empty(), x_unit.is_empty()) {
                    (false, false) => format!("{a_unit}/{x_unit}"),
                    (false, true) => a_unit.clone(),
                    _ => String::new(),
                };
                (
                    format!("d({a})/dt"),
                    unit,
                    format!("gradient({{{a}}}, {{{x_name}}})"),
                )
            }
            Self::Energy => {
                let unit = if a_unit == "W" && x_unit == "s" {
                    "J".to_owned()
                } else if a_unit.is_empty() {
                    x_unit.clone()
                } else {
                    format!("{a_unit}·{x_unit}")
                };
                (
                    format!("{a} energy"),
                    unit,
                    format!("cumtrapz({{{a}}}, {{{x_name}}})"),
                )
            }
            Self::MovingAverage => (
                format!("{a} (average)"),
                a_unit.clone(),
                format!("movmean({{{a}}}, 10)"),
            ),
            Self::TimesThousand => (
                format!("{a} × 1000"),
                String::new(),
                format!("{{{a}}} * 1000"),
            ),
            Self::DivideThousand => (
                format!("{a} ÷ 1000"),
                String::new(),
                format!("{{{a}}} / 1000"),
            ),
        };
        if dialog.editing.is_none() {
            dialog.name = name;
            dialog.unit = unit;
        }
        dialog.expression = expression;
        dialog.focus_expression = true;
    }
}

/// How a formula refers to a channel: by the name shown in the channel list,
/// unless another channel shows the same one (`VDD` for both `VDD_V` and
/// `VDD_mA`); then by its full column name.
fn formula_name(channel: &Channel, channels: &[Channel]) -> String {
    if channels.iter().filter(|c| c.label == channel.label).count() > 1 {
        channel.original.clone()
    } else {
        channel.label.clone()
    }
}
