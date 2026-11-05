use std::collections::{BTreeMap, HashMap, HashSet};

use dv_core::{
    common::clean_col,
    units::{parse_unit, split_name_unit, UnitFamily},
    Column, ColumnData, Table,
};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::{
    settings::Settings,
    ui::{icons, theme, widgets},
    workspace::{ColorBy, Workspace},
};

#[derive(Default)]
pub(crate) struct Sidebar {
    pub query: String,
    pub focus_search: bool,
    group_by_unit: bool,
    show_index: bool,
    initialized: bool,
    last_clicked: Option<String>,
    previews: HashMap<(u64, String), Option<Preview>>,
    summaries: HashMap<(u64, String), String>,
    preview_sources: HashMap<String, u64>,
    cardinality: HashMap<u64, CardinalityScan>,
    suggestions: HashMap<(u64, String), Option<(usize, usize)>>,
    catalog_revision: u64,
    presence_signature: Vec<(u64, bool)>,
    presence: HashMap<String, usize>,
    renaming: Option<(String, String)>,
    rename_focus: bool,
    stack_common_only: bool,
    keyboard_key: Option<String>,
    keyboard_id: Option<egui::Id>,
}

/// A request for the New/Edit channel dialog.
pub(crate) enum FormulaRequest {
    New,
    Template(crate::ui::dialogs::formula::Template, String),
    Edit(String),
}

#[derive(Default)]
pub(crate) struct SidebarResponse {
    pub add: bool,
    pub reload: Option<usize>,
    pub table: Option<usize>,
    pub info: bool,
    pub export: Option<usize>,
    pub changed: bool,
    pub formula: Option<FormulaRequest>,
    pub processing: bool,
}

struct Preview {
    points: Vec<f32>,
    min: f64,
    mean: f64,
    max: f64,
    sampled: bool,
}

enum RowAction {
    Toggle(String, bool, bool),
    SetX(String),
    Solo(String),
    SelectAll,
    ClearAll,
}

enum SourceAction {
    Rename(usize),
    Move(usize, usize),
    Sort,
    AlignAll,
    Merge,
    Stack,
    Split(usize, usize),
    Transpose(usize),
    TransposeActive,
    Remove(usize),
}

#[derive(Default)]
struct DistinctValues {
    numbers: HashSet<u64>,
    strings: HashSet<String>,
    missing: bool,
}

impl DistinctValues {
    fn add(&mut self, column: &Column, row: usize) {
        match &column.data {
            ColumnData::Float(values) | ColumnData::DateTime(values) => {
                if let Some(&value) = values.get(row) {
                    if value.is_nan() {
                        self.missing = true;
                    } else {
                        self.numbers.insert(if value == 0.0 {
                            0.0f64.to_bits()
                        } else {
                            value.to_bits()
                        });
                    }
                }
            }
            ColumnData::Str(values) => {
                if let Some(value) = values.get(row) {
                    let value = value.trim();
                    if value.is_empty() || value == "-" {
                        self.missing = true;
                    } else if !self.strings.contains(value) {
                        self.strings.insert(value.to_owned());
                    }
                }
            }
        }
    }
    fn len(&self) -> usize {
        self.numbers.len() + self.strings.len() + usize::from(self.missing)
    }
}

/// Columns that identify a device, best first. Names make readable source
/// labels (`RTX 4090`); identical names fall back to indices automatically
/// because the split needs one value per device.
fn split_priority(name: &str) -> u8 {
    match clean_col(name).as_str() {
        "name" | "gpuname" | "devicename" | "device" | "gpu" => 0,
        "index" | "gpuindex" | "deviceindex" => 1,
        "core" | "cor" | "cpu" | "cpuid" | "coreid" => 2,
        "processid" | "pid" | "application" | "thread" | "thr" => 3,
        _ => 4,
    }
}

struct CardinalityScan {
    order: Vec<usize>,
    position: usize,
    row: usize,
    values: DistinctValues,
    candidates: Vec<(usize, usize)>,
}
impl CardinalityScan {
    fn new(table: &Table) -> Self {
        let mut order: Vec<_> = (0..table.ncols())
            .filter(|&index| !(index == 0 && table.data.column(index).name == "Index"))
            .collect();
        order.sort_by_key(|&index| split_priority(&table.data.column(index).name));
        Self {
            order,
            position: 0,
            row: 0,
            values: DistinctValues::default(),
            candidates: Vec::new(),
        }
    }
    fn advance(&mut self, table: &Table, mut budget: usize) -> bool {
        while self.position < self.order.len() && budget > 0 {
            let index = self.order[self.position];
            let column = table.data.column(index);
            while self.row < column.len() && budget > 0 && self.values.len() <= 64 {
                self.values.add(column, self.row);
                self.row += 1;
                budget -= 1;
            }
            if self.row == column.len() || self.values.len() > 64 {
                let n = self.values.len();
                if (2..=64).contains(&n) {
                    self.candidates.push((index, n));
                }
                self.position += 1;
                self.row = 0;
                self.values = DistinctValues::default();
            }
        }
        self.position == self.order.len()
    }
}

/// The column that tells interleaved devices apart and how many there are.
/// Inspects a bounded prefix; the split itself uses every row.
pub(crate) fn interleaved_suggestion(table: &Table, x_key: Option<&str>) -> Option<(usize, usize)> {
    let time_index =
        x_key
            .and_then(|key| {
                table
                    .data
                    .columns()
                    .iter()
                    .position(|column| clean_col(&column.name) == key)
            })
            .filter(|&index| {
                let column = table.data.column(index);
                let (_, unit) = split_name_unit(&column.name);
                column.is_datetime()
                    || parse_unit(&unit).family == UnitFamily::Time
                    || clean_col(&column.name).contains("time")
            })
            .or_else(|| {
                table.data.columns().iter().position(|column| {
                    column.is_datetime() || clean_col(&column.name).contains("time")
                })
            })?;
    let xs = match &table.data.column(time_index).data {
        ColumnData::Float(values) | ColumnData::DateTime(values) => values,
        ColumnData::Str(_) => return None,
    };
    let mut runs = HashMap::<usize, usize>::new();
    let mut previous = None;
    let mut length = 0usize;
    for &value in xs.iter().take(4096).filter(|value| value.is_finite()) {
        if previous == Some(value) {
            length += 1;
        } else {
            if length > 0 {
                *runs.entry(length).or_default() += 1;
            }
            previous = Some(value);
            length = 1;
        }
    }
    if length > 0 {
        *runs.entry(length).or_default() += 1;
    }
    let (&repeat, &frequency) = runs.iter().max_by_key(|(_, frequency)| *frequency)?;
    if !(2..=64).contains(&repeat) || frequency < 2 {
        return None;
    }
    let mut indices: Vec<_> = (0..table.ncols())
        .filter(|&index| {
            index != time_index && !(index == 0 && table.data.column(index).name == "Index")
        })
        .collect();
    indices.sort_by_key(|&index| split_priority(&table.data.column(index).name));
    for index in indices {
        let column = table.data.column(index);
        let mut distinct = DistinctValues::default();
        for row in 0..column.len().min(4096) {
            distinct.add(column, row);
            if distinct.len() > repeat {
                break;
            }
        }
        if distinct.len() == repeat {
            return Some((index, repeat));
        }
    }
    None
}

fn source_summary(table: &Table, x_key: Option<&str>, precision: usize) -> String {
    let mut summary = format!(
        "{} rows · {} ch",
        count(table.nrows()),
        count(table.ncols().saturating_sub(1))
    );
    let Some(column) = x_key.and_then(|key| {
        table
            .data
            .columns()
            .iter()
            .find(|column| clean_col(&column.name) == key)
    }) else {
        return summary;
    };
    let (values, factor) = match &column.data {
        ColumnData::DateTime(values) => (values, 1.0),
        ColumnData::Float(values) => {
            let (_, unit) = split_name_unit(&column.name);
            let unit = parse_unit(&unit);
            if unit.family != UnitFamily::Time {
                return summary;
            }
            (values, unit.to_base)
        }
        ColumnData::Str(_) => return summary,
    };
    let Some((first_index, &first)) = values.iter().enumerate().find(|(_, x)| x.is_finite()) else {
        return summary;
    };
    let Some((last_index, &last)) = values.iter().enumerate().rev().find(|(_, x)| x.is_finite())
    else {
        return summary;
    };
    let duration = (last - first) * factor;
    if duration > 0.0 && last_index > first_index {
        let rate = (last_index - first_index) as f64 / duration;
        summary.push_str(&format!(
            " · {} · {}",
            if duration < 60.0 {
                dv_chart::format::format_value(duration, "s", 3)
            } else {
                dv_chart::format::format_duration(duration, 0.1)
            },
            dv_chart::format::format_value(rate, "Hz", precision)
        ));
    }
    summary
}

fn count(value: usize) -> String {
    let text = value.to_string();
    let mut result = String::with_capacity(text.len() + text.len() / 3);
    for (i, c) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(c);
    }
    result
}

fn preview(workspace: &Workspace, key: &str) -> Option<((u64, String), Preview)> {
    for (table, source) in workspace.tables.tabs.iter().zip(&workspace.sources) {
        if !source.active {
            continue;
        }
        let Some(column) = table
            .data
            .columns()
            .iter()
            .find(|column| clean_col(&column.name) == key)
        else {
            continue;
        };
        let values = match &column.data {
            ColumnData::Float(values) | ColumnData::DateTime(values) => values,
            ColumnData::Str(_) => continue,
        };
        if values.is_empty() {
            continue;
        }
        // Bound first-paint work for large logs; the miniature is explicitly a preview.
        let stride = values.len().div_ceil(4096).max(1);
        let mut bins = vec![(f64::INFINITY, f64::NEG_INFINITY); 32];
        let (mut min, mut max, mut sum, mut n) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0usize);
        for (index, &value) in values.iter().enumerate().step_by(stride) {
            if !value.is_finite() {
                continue;
            }
            min = min.min(value);
            max = max.max(value);
            sum += value;
            n += 1;
            let bin = (index * bins.len() / values.len()).min(bins.len() - 1);
            bins[bin].0 = bins[bin].0.min(value);
            bins[bin].1 = bins[bin].1.max(value);
        }
        if n == 0 {
            return None;
        }
        let span = (max - min).max(f64::EPSILON);
        let points = bins
            .into_iter()
            .map(|(a, b)| {
                if a.is_finite() {
                    (((a + b) * 0.5 - min) / span) as f32
                } else {
                    f32::NAN
                }
            })
            .collect();
        return Some((
            (table.identity(), key.to_owned()),
            Preview {
                points,
                min,
                mean: sum / n as f64,
                max,
                sampled: stride > 1,
            },
        ));
    }
    None
}

impl Sidebar {
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &mut Workspace,
        settings: &Settings,
    ) -> SidebarResponse {
        if !self.initialized {
            self.group_by_unit = true;
            self.initialized = true;
        }
        let mut output = SidebarResponse::default();
        let colors = theme::palette_for(theme::is_dark(ui.ctx()), settings.palette);
        let tokens = theme::Tokens::for_dark(theme::is_dark(ui.ctx()));
        let signature: Vec<_> = workspace
            .tables
            .tabs
            .iter()
            .zip(&workspace.sources)
            .map(|(table, source)| (table.identity(), source.active))
            .collect();
        if signature != self.presence_signature || self.catalog_revision != workspace.data_revision
        {
            self.presence_signature = signature;
            self.catalog_revision = workspace.data_revision;
            self.presence.clear();
            self.preview_sources.clear();
            self.previews.clear();
            self.summaries.clear();
            self.cardinality.clear();
            self.suggestions.clear();
            for (table, source) in workspace.tables.tabs.iter().zip(&workspace.sources) {
                if source.active {
                    let keys: HashSet<_> = table
                        .data
                        .columns()
                        .iter()
                        .map(|column| clean_col(&column.name))
                        .collect();
                    for key in keys {
                        self.preview_sources
                            .entry(key.clone())
                            .or_insert(table.identity());
                        *self.presence.entry(key).or_default() += 1;
                    }
                }
            }
        }
        let active_sources = workspace
            .sources
            .iter()
            .filter(|source| source.active)
            .count();
        let source_count = workspace.sources.len();
        let by_source = match workspace.panes.color_by {
            ColorBy::Source => true,
            ColorBy::Channel => false,
            ColorBy::Auto => {
                active_sources > 1
                    && workspace.series.iter().any(|series| {
                        workspace
                            .series
                            .iter()
                            .all(|other| other.pane != series.pane || other.key == series.key)
                    })
            }
        };
        let mut source_action = None;
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            widgets::section_label(ui, "Sources");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::PLUS, "Add files · Ctrl+Shift+O").clicked() {
                    output.add = true;
                }
                ui.menu_button(
                    egui::RichText::new(icons::MORE).family(theme::icon_family()),
                    |ui| {
                        if ui
                            .add_enabled(source_count > 1, egui::Button::new("Sort by name"))
                            .clicked()
                        {
                            source_action = Some(SourceAction::Sort);
                            ui.close();
                        }
                        if ui
                            .add_enabled(active_sources > 1, egui::Button::new("Align all starts"))
                            .on_hover_text("Start every selected source at zero and clear offsets")
                            .clicked()
                        {
                            source_action = Some(SourceAction::AlignAll);
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .add_enabled(
                                active_sources > 1,
                                egui::Button::new("Merge selected on X"),
                            )
                            .clicked()
                        {
                            source_action = Some(SourceAction::Merge);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                active_sources > 1,
                                egui::Button::new("Stack selected rows"),
                            )
                            .clicked()
                        {
                            source_action = Some(SourceAction::Stack);
                            ui.close();
                        }
                        ui.checkbox(&mut self.stack_common_only, "Stack common columns only");
                        if ui
                            .add_enabled(
                                active_sources > 0,
                                egui::Button::new("Transpose selected"),
                            )
                            .clicked()
                        {
                            source_action = Some(SourceAction::TransposeActive);
                            ui.close();
                        }
                        ui.label(
                            egui::RichText::new("Selected sources are checked above.")
                                .small()
                                .color(tokens.text_muted),
                        );
                    },
                )
                .response
                .on_hover_text("Source actions");
            });
        });
        // Offsets are in X axis units; timestamps shift in seconds.
        let offset_unit = match crate::ui::chart_view::x_axis_format(workspace).0 {
            dv_chart::AxisFormat::DateTime | dv_chart::AxisFormat::Duration => "s".to_owned(),
            _ => workspace
                .x_key
                .as_ref()
                .and_then(|key| {
                    workspace
                        .channels
                        .iter()
                        .find(|channel| &channel.key == key)
                })
                .map(|channel| channel.unit.clone())
                .unwrap_or_default(),
        };
        let offset_suffix = if offset_unit.is_empty() {
            String::new()
        } else {
            format!(" {offset_unit}")
        };
        let mut rename_commit = None;
        let mut rename_cancel = false;
        let list_height = (ui.available_height() * 0.4).max(150.0);
        // Sources already split need no split suggestion.
        let split_parents: HashSet<String> = workspace
            .sources
            .iter()
            .filter_map(|source| match &source.recipe {
                Some(crate::workspace::GeneratedRecipe::Split { parent, .. }) => {
                    Some(parent.clone())
                }
                _ => None,
            })
            .collect();
        egui::ScrollArea::vertical().id_salt("sources-list").max_height(list_height).show(ui, |ui| {
            for (index, source) in workspace.sources.iter_mut().enumerate() {
                let Some(table) = workspace.tables.tabs.get(index) else { continue };
                let key = source.stable_key();
                let summary_key = (table.identity(), format!("{}:{}", workspace.x_key.as_deref().unwrap_or(""), settings.precision));
                let summary = self.summaries.entry(summary_key)
                    .or_insert_with(|| source_summary(table, workspace.x_key.as_deref(), settings.precision));
                let source_color = by_source.then(|| workspace.panes.source_slots.get(&key).copied())
                    .flatten().map(|slot| colors[slot % colors.len()]);
                let suggestion_key = (table.identity(), workspace.x_key.clone().unwrap_or_default());
                let suggestion = *self.suggestions.entry(suggestion_key)
                    .or_insert_with(|| interleaved_suggestion(table, workspace.x_key.as_deref()));
                let suggestion = suggestion.filter(|_| !split_parents.contains(&key));
                egui::Frame::new()
                    .fill(tokens.surface)
                    .stroke(Stroke::new(1.0, tokens.border))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut source.active, "")
                                .on_hover_text("Include this source in the chart and source actions").changed()
                            {
                                output.changed = true;
                            }
                            if let Some(color) = source_color {
                                let (dot, _) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                                ui.painter().circle_filled(dot.center(), 4.0, color);
                            }
                            if let Some((_, draft)) = self.renaming.as_mut().filter(|(editing, _)| *editing == key) {
                                let response = ui.add(egui::TextEdit::singleline(draft).desired_width(ui.available_width()));
                                if self.rename_focus {
                                    response.request_focus();
                                    self.rename_focus = false;
                                }
                                if response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                                    rename_cancel = true;
                                } else if response.lost_focus() || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))) {
                                    rename_commit = Some((index, draft.trim().to_owned()));
                                }
                            } else {
                                let text = egui::RichText::new(&source.name).strong().color(if source.active { tokens.text } else { tokens.text_muted });
                                let response = ui.add(egui::Label::new(text).truncate());
                                let tip = if source.path.as_os_str().is_empty() { source.name.clone() } else { source.path.display().to_string() };
                                response.clone().on_hover_text(tip);
                                response.context_menu(|ui| {
                                    if ui.button("Rename").clicked() {
                                        source_action = Some(SourceAction::Rename(index));
                                        ui.close();
                                    }
                                    if ui.add_enabled(!source.path.as_os_str().is_empty(), egui::Button::new("Reload source")).clicked() {
                                        output.reload = Some(index);
                                        ui.close();
                                    }
                                    if ui.button("Show in table").clicked() {
                                        output.table = Some(index);
                                        ui.close();
                                    }
                                    if ui.button("File information").clicked() {
                                        output.info = true;
                                        ui.close();
                                    }
                                    if ui.button("Export data…").clicked() {
                                        output.export = Some(index);
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.checkbox(&mut source.align_start, "Align start to 0").on_hover_text("Subtract the first X value before applying the offset").changed() {
                                        output.changed = true;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label("X offset");
                                        output.changed |= ui.add(egui::DragValue::new(&mut source.offset).speed(0.1)).changed();
                                    });
                                    ui.separator();
                                    ui.menu_button("Split by…", |ui| {
                                        let scan = self.cardinality.entry(table.identity())
                                            .or_insert_with(|| CardinalityScan::new(table));
                                        let complete = scan.advance(table, 200_000);
                                        ui.label(egui::RichText::new("Split rows by column").strong());
                                        egui::ScrollArea::vertical().max_height(270.0).show(ui, |ui| {
                                            for &(column, values) in &scan.candidates {
                                                let name = &table.data.column(column).name;
                                                if ui.button(format!("{name} · {} values", count(values))).clicked() {
                                                    source_action = Some(SourceAction::Split(index, column));
                                                    ui.close();
                                                }
                                            }
                                        });
                                        if !complete {
                                            ui.label(egui::RichText::new("Finding columns…").small().color(tokens.text_muted));
                                            ui.ctx().request_repaint();
                                        } else if scan.candidates.is_empty() {
                                            ui.label(egui::RichText::new("No columns with 2–64 distinct values.").small().color(tokens.text_muted));
                                        }
                                    });
                                    if ui.button("Transpose").clicked() {
                                        source_action = Some(SourceAction::Transpose(index));
                                        ui.close();
                                    }
                                    if ui.add_enabled(index > 0, egui::Button::new("Move up")).clicked() {
                                        source_action = Some(SourceAction::Move(index, index.saturating_sub(1)));
                                        ui.close();
                                    }
                                    if ui.add_enabled(index + 1 < source_count, egui::Button::new("Move down")).clicked() {
                                        source_action = Some(SourceAction::Move(index, index + 1));
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Remove from workspace").clicked() {
                                        source_action = Some(SourceAction::Remove(index));
                                        ui.close();
                                    }
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(24.0);
                            ui.add(egui::Label::new(egui::RichText::new(summary.as_str()).size(11.0).color(tokens.text_muted)).truncate())
                                .on_hover_text(summary.as_str());
                        });
                        if let Some((column, devices)) = suggestion {
                            let label = format!("Rows interleave {} devices — Split by {}", count(devices), table.data.column(column).name);
                            if ui.link(egui::RichText::new(&label).size(11.0)).on_hover_text("Create a source for each value and hide this source").clicked() {
                                source_action = Some(SourceAction::Split(index, column));
                            }
                        }
                        if active_sources > 1 && source.active {
                            ui.horizontal(|ui| {
                                ui.add_space(22.0);
                                output.changed |= ui.checkbox(&mut source.align_start, "Start at 0")
                                    .on_hover_text("Align the first X value to zero").changed();
                                ui.label(egui::RichText::new("Offset").size(11.0).color(tokens.text_secondary));
                                output.changed |= ui.add(egui::DragValue::new(&mut source.offset).speed(0.1).max_decimals(6).suffix(offset_suffix.as_str()))
                                    .on_hover_text("Shift this source along X").changed();
                            });
                        }
                    });
                ui.add_space(3.0);
            }
        });
        if rename_cancel {
            self.renaming = None;
        } else if let Some((index, name)) = rename_commit {
            if !name.is_empty() {
                if let Some(source) = workspace.sources.get_mut(index) {
                    if source.name != name {
                        source.name = name;
                        output.changed = true;
                    }
                }
            }
            self.renaming = None;
        }
        match source_action {
            Some(SourceAction::Rename(index)) => {
                if let Some(source) = workspace.sources.get(index) {
                    self.renaming = Some((source.stable_key(), source.name.clone()));
                    self.rename_focus = true;
                }
            }
            Some(SourceAction::Move(from, to)) => {
                workspace.move_source(from, to);
                output.changed = true;
            }
            Some(SourceAction::Sort) => {
                workspace.sort_sources();
                output.changed = true;
            }
            Some(SourceAction::AlignAll) => output.changed |= workspace.align_all_starts(),
            Some(SourceAction::Merge) => match workspace.merge_sources() {
                Ok(()) => output.changed = true,
                Err(error) => workspace.problems.push(error),
            },
            Some(SourceAction::Stack) => match workspace.stack_sources(self.stack_common_only) {
                Ok(()) => output.changed = true,
                Err(error) => workspace.problems.push(error),
            },
            Some(SourceAction::Split(index, column)) => match workspace.split_source(index, column)
            {
                Ok(()) => output.changed = true,
                Err(error) => workspace.problems.push(error),
            },
            Some(SourceAction::Transpose(index)) => match workspace.transpose_source(index) {
                Ok(()) => output.changed = true,
                Err(error) => workspace.problems.push(error),
            },
            Some(SourceAction::TransposeActive) => {
                let indices: Vec<_> = workspace
                    .sources
                    .iter()
                    .enumerate()
                    .filter_map(|(index, source)| source.active.then_some(index))
                    .collect();
                for index in indices {
                    match workspace.transpose_source(index) {
                        Ok(()) => output.changed = true,
                        Err(error) => workspace.problems.push(error),
                    }
                }
            }
            Some(SourceAction::Remove(index)) => {
                workspace.remove_source(index);
                output.changed = true;
            }
            None => {}
        }
        let steps: Vec<String> = workspace
            .pipeline
            .actions
            .iter()
            .filter(|action| action.enabled)
            .map(|action| action.plugin.summary())
            .collect();
        if !steps.is_empty() {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                widgets::section_label(ui, "Processing");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, icons::SLIDERS, "Edit processing steps").clicked() {
                        output.processing = true;
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                for step in &steps {
                    if widgets::chip(ui, step)
                        .on_hover_text("Edit processing steps")
                        .clicked()
                    {
                        output.processing = true;
                    }
                }
            });
        }
        ui.add_space(9.0);
        ui.separator();
        ui.add_space(5.0);
        let formula_keys: HashSet<String> = workspace
            .tables
            .tabs
            .iter()
            .flat_map(|table| &table.formulas)
            .map(|formula| clean_col(&formula.name))
            .collect();
        ui.horizontal(|ui| {
            widgets::section_label(ui, "Channels");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(
                    egui::RichText::new(icons::MORE).family(theme::icon_family()),
                    |ui| {
                        if ui
                            .selectable_label(self.group_by_unit, "Group by unit")
                            .clicked()
                        {
                            self.group_by_unit = true;
                            ui.close();
                        }
                        if ui
                            .selectable_label(!self.group_by_unit, "Source order")
                            .clicked()
                        {
                            self.group_by_unit = false;
                            ui.close();
                        }
                        ui.separator();
                        ui.checkbox(&mut self.show_index, "Show index channel");
                    },
                )
                .response
                .on_hover_text("Channel display");
                if widgets::icon_button(ui, icons::FORMULA, "New channel from a formula · Ctrl+N")
                    .clicked()
                {
                    output.formula = Some(FormulaRequest::New);
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("X")
                    .strong()
                    .color(tokens.text_secondary),
            );
            let x_label = workspace
                .x_key
                .as_ref()
                .and_then(|key| workspace.channels.iter().find(|c| &c.key == key))
                .map_or("Choose an axis", |c| c.label.as_str());
            let mut x = None;
            egui::ComboBox::from_id_salt("sidebar-x")
                .width((ui.available_width() - 5.0).max(60.0))
                .selected_text(x_label)
                .show_ui(ui, |ui| {
                    for channel in &workspace.channels {
                        if ui
                            .selectable_label(
                                workspace.x_key.as_deref() == Some(&channel.key),
                                format!(
                                    "{}{}",
                                    channel.label,
                                    if channel.unit.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" [{}]", channel.unit)
                                    }
                                ),
                            )
                            .clicked()
                        {
                            x = Some(channel.key.clone());
                        }
                    }
                });
            if let Some(key) = x {
                workspace.set_x(&key);
                output.changed = true;
            }
        });
        ui.add_space(4.0);
        let search = widgets::search_field(ui, &mut self.query, "Find a channel…   Ctrl+F");
        if self.focus_search {
            search.request_focus();
            self.focus_search = false;
        }
        if search.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.query.clear();
        }
        let search_response = Some(search);
        let mut matches = workspace.search(&self.query);
        if !self.show_index {
            matches
                .retain(|&index| !crate::selection::is_index(&workspace.channels[index].original));
        }
        let mut action = None;
        if search_response
            .as_ref()
            .is_some_and(|r| r.has_focus() || r.lost_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            if ui.input(|i| i.modifiers.command) {
                action = Some(RowAction::SelectAll);
            } else if let Some(&index) = matches.first() {
                action = Some(RowAction::Toggle(
                    workspace.channels[index].key.clone(),
                    false,
                    false,
                ));
            }
        }
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} of {} selected",
                    workspace.selected.len(),
                    workspace.channels.len()
                ))
                .size(11.0)
                .color(tokens.text_muted),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(
                    egui::RichText::new(icons::MORE).family(theme::icon_family()),
                    |ui| {
                        if ui.button("Select matching channels").clicked() {
                            action = Some(RowAction::SelectAll);
                            ui.close();
                        }
                        if ui.button("Clear selection").clicked() {
                            action = Some(RowAction::ClearAll);
                            ui.close();
                        }
                    },
                )
                .response
                .on_hover_text("Channel selection");
            });
        });
        ui.add_space(2.0);
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        if self.group_by_unit {
            let mut lookup = BTreeMap::new();
            for &index in &matches {
                let family = &workspace.channels[index].family;
                let group = *lookup.entry(family.clone()).or_insert_with(|| {
                    groups.push((family.clone(), Vec::new()));
                    groups.len() - 1
                });
                groups[group].1.push(index);
            }
        } else {
            groups.push(("All channels".into(), matches.clone()));
        }
        let ordered: Vec<_> = groups
            .iter()
            .flat_map(|(_, indices)| indices.iter().copied())
            .collect();
        let list_focused = self
            .keyboard_id
            .is_some_and(|id| ui.memory(|memory| memory.has_focus(id)));
        let search_focused = search_response
            .as_ref()
            .is_some_and(|response| response.has_focus());
        let direction = if list_focused || search_focused {
            ui.input_mut(|input| {
                if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    1isize
                } else if list_focused
                    && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                {
                    -1
                } else {
                    0
                }
            })
        } else {
            0
        };
        let mut focus_target = None;
        if direction != 0 && !ordered.is_empty() {
            let current = if list_focused {
                self.keyboard_key.as_ref().and_then(|key| {
                    ordered
                        .iter()
                        .position(|&index| workspace.channels[index].key == *key)
                })
            } else {
                None
            };
            let next = current.map_or(0, |index| {
                (index as isize + direction).clamp(0, ordered.len() as isize - 1) as usize
            });
            focus_target = Some(workspace.channels[ordered[next]].key.clone());
        }
        let mut previews_left = 3usize;
        egui::ScrollArea::vertical()
            .id_salt("channel-list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if groups.is_empty() {
                    ui.add_space(14.0);
                    ui.weak("No channels match your search.");
                }
                for (group, indices) in &groups {
                    let heading = format!("{}   {}", group.to_uppercase(), indices.len());
                    egui::CollapsingHeader::new(
                        egui::RichText::new(heading)
                            .size(11.0)
                            .color(tokens.text_secondary)
                            .strong(),
                    )
                    .id_salt(("channel-group", group))
                    .default_open(true)
                    .show(ui, |ui| {
                        for &index in indices {
                            let channel = &workspace.channels[index];
                            let selected = workspace.selected.contains(&channel.key);
                            let slot = workspace
                                .series
                                .iter()
                                .find(|series| series.key == channel.key)
                                .map(|series| series.color_slot)
                                .or_else(|| workspace.panes.slots.get(&channel.key).copied())
                                .unwrap_or(0)
                                % colors.len();
                            let color = colors[slot];
                            let (rect, response) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 27.0),
                                Sense::click(),
                            );
                            if focus_target.as_deref() == Some(&channel.key) {
                                response.request_focus();
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            if response.has_focus() {
                                self.keyboard_key = Some(channel.key.clone());
                                self.keyboard_id = Some(response.id);
                            }
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let painter = ui.painter().with_clip_rect(rect);
                            if response.hovered() || response.has_focus() {
                                painter.rect_filled(rect, 5.0, tokens.accent_soft);
                            }
                            let center = Pos2::new(rect.left() + 8.0, rect.center().y);
                            if selected {
                                painter.circle_filled(center, 4.2, color);
                            } else {
                                painter.circle_stroke(
                                    center,
                                    4.0,
                                    Stroke::new(1.3, tokens.text_muted),
                                );
                            }
                            let unit_width = if channel.unit.is_empty() { 0.0 } else { 42.0 };
                            let spark_width = if settings.show_sparklines { 48.0 } else { 0.0 };
                            let availability =
                                self.presence.get(&channel.key).copied().unwrap_or(0);
                            let badge_width = if active_sources > 1 && availability < active_sources
                            {
                                32.0
                            } else {
                                0.0
                            };
                            let name_rect = Rect::from_min_max(
                                Pos2::new(rect.left() + 20.0, rect.top()),
                                Pos2::new(
                                    (rect.right() - unit_width - spark_width - badge_width - 3.0)
                                        .max(rect.left() + 21.0),
                                    rect.bottom(),
                                ),
                            );
                            let name_color = if selected {
                                tokens.text
                            } else {
                                tokens.text_secondary
                            };
                            let mut job = egui::text::LayoutJob::single_section(
                                channel.label.clone(),
                                egui::TextFormat::simple(FontId::proportional(12.5), name_color),
                            );
                            job.wrap = egui::text::TextWrapping {
                                max_width: name_rect.width(),
                                max_rows: 1,
                                break_anywhere: true,
                                overflow_character: Some('…'),
                            };
                            let galley = painter.layout_job(job);
                            painter.galley(
                                Pos2::new(
                                    name_rect.left(),
                                    rect.center().y - galley.size().y * 0.5,
                                ),
                                galley,
                                name_color,
                            );
                            if badge_width > 0.0 {
                                let right = rect.right() - unit_width - spark_width - 3.0;
                                let badge = Rect::from_center_size(
                                    Pos2::new(right - 15.0, rect.center().y),
                                    Vec2::new(29.0, 18.0),
                                );
                                painter.rect_filled(badge, 4.0, tokens.accent_soft);
                                painter.text(
                                    badge.center(),
                                    Align2::CENTER_CENTER,
                                    format!("{availability}/{active_sources}"),
                                    FontId::proportional(10.5),
                                    tokens.text_secondary,
                                );
                            }
                            if !channel.unit.is_empty() {
                                painter.text(
                                    Pos2::new(rect.right() - spark_width - 3.0, rect.center().y),
                                    Align2::RIGHT_CENTER,
                                    &channel.unit,
                                    FontId::proportional(11.0),
                                    tokens.text_muted,
                                );
                            }
                            let mut tooltip = format!(
                                "{}{}",
                                channel.label,
                                if channel.unit.is_empty() {
                                    String::new()
                                } else {
                                    format!(" [{}]", channel.unit)
                                }
                            );
                            if settings.show_sparklines {
                                let cache_key = self
                                    .preview_sources
                                    .get(&channel.key)
                                    .map(|&identity| (identity, channel.key.clone()));
                                if let Some(cache_key) = cache_key {
                                    if !self.previews.contains_key(&cache_key) {
                                        if previews_left > 0 {
                                            previews_left -= 1;
                                            let value = preview(workspace, &channel.key)
                                                .map(|(_, value)| value);
                                            self.previews.insert(cache_key.clone(), value);
                                        } else {
                                            ui.ctx().request_repaint();
                                        }
                                    }
                                    if let Some(p) =
                                        self.previews.get(&cache_key).and_then(Option::as_ref)
                                    {
                                        let spark = Rect::from_min_size(
                                            Pos2::new(rect.right() - 43.0, rect.top() + 5.0),
                                            Vec2::new(39.0, 17.0),
                                        );
                                        let points: Vec<_> = p
                                            .points
                                            .iter()
                                            .enumerate()
                                            .filter(|(_, y)| y.is_finite())
                                            .map(|(i, &y)| {
                                                Pos2::new(
                                                    spark.left()
                                                        + spark.width() * i as f32
                                                            / (p.points.len() - 1).max(1) as f32,
                                                    spark.bottom() - spark.height() * y,
                                                )
                                            })
                                            .collect();
                                        if points.len() >= 2 {
                                            painter.add(egui::Shape::line(
                                                points,
                                                Stroke::new(
                                                    1.0,
                                                    color.gamma_multiply(if selected {
                                                        0.8
                                                    } else {
                                                        0.45
                                                    }),
                                                ),
                                            ));
                                        }
                                        tooltip.push_str(&format!(
                                            "\n{}min {} · mean {} · max {}",
                                            if p.sampled { "Preview: " } else { "" },
                                            dv_chart::format::format_value(
                                                p.min,
                                                &channel.unit,
                                                settings.precision
                                            ),
                                            dv_chart::format::format_value(
                                                p.mean,
                                                &channel.unit,
                                                settings.precision
                                            ),
                                            dv_chart::format::format_value(
                                                p.max,
                                                &channel.unit,
                                                settings.precision
                                            )
                                        ));
                                    }
                                }
                            }
                            response.clone().on_hover_text(tooltip);
                            if response.clicked()
                                || (response.has_focus()
                                    && ui.input(|input| {
                                        input.key_pressed(egui::Key::Space)
                                            || input.key_pressed(egui::Key::Enter)
                                    }))
                            {
                                response.request_focus();
                                self.keyboard_key = Some(channel.key.clone());
                                self.keyboard_id = Some(response.id);
                                let (shift, alt) =
                                    ui.input(|i| (i.modifiers.shift, i.modifiers.alt));
                                action = Some(RowAction::Toggle(channel.key.clone(), shift, alt));
                            }
                            response.context_menu(|ui| {
                                if ui.button("Set as X axis").clicked() {
                                    action = Some(RowAction::SetX(channel.key.clone()));
                                    ui.close();
                                }
                                if ui.button("Plot only this channel").clicked() {
                                    action = Some(RowAction::Solo(channel.key.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                ui.menu_button("New channel from this", |ui| {
                                    for template in crate::ui::dialogs::formula::Template::ALL {
                                        if ui.button(template.label()).clicked() {
                                            output.formula = Some(FormulaRequest::Template(
                                                template,
                                                channel.key.clone(),
                                            ));
                                            ui.close();
                                        }
                                    }
                                });
                                if formula_keys.contains(&channel.key)
                                    && ui.button("Edit formula…").clicked()
                                {
                                    output.formula =
                                        Some(FormulaRequest::Edit(channel.key.clone()));
                                    ui.close();
                                }
                                ui.separator();
                                if ui.button("Copy channel name").clicked() {
                                    ui.ctx().copy_text(channel.label.clone());
                                    ui.close();
                                }
                            });
                        }
                    });
                }
            });
        match action {
            Some(RowAction::Toggle(key, shift, alt)) => {
                if alt {
                    workspace.selected.clear();
                    workspace.toggle(&key);
                } else if shift {
                    let start = self.last_clicked.as_ref().and_then(|k| {
                        matches
                            .iter()
                            .position(|&i| &workspace.channels[i].key == k)
                    });
                    let end = matches
                        .iter()
                        .position(|&i| workspace.channels[i].key == key);
                    if let (Some(a), Some(b)) = (start, end) {
                        let keys: Vec<_> = matches[a.min(b)..=a.max(b)]
                            .iter()
                            .map(|&i| workspace.channels[i].key.clone())
                            .collect();
                        for item in keys {
                            if !workspace.selected.contains(&item) {
                                workspace.toggle(&item);
                            }
                        }
                    } else {
                        workspace.toggle(&key);
                    }
                } else {
                    workspace.toggle(&key);
                }
                self.last_clicked = Some(key);
                output.changed = true;
            }
            Some(RowAction::SetX(key)) => {
                workspace.set_x(&key);
                output.changed = true;
            }
            Some(RowAction::Solo(key)) => {
                workspace.selected.clear();
                workspace.toggle(&key);
                output.changed = true;
            }
            Some(RowAction::SelectAll) => {
                let keys: Vec<_> = matches
                    .iter()
                    .map(|&i| workspace.channels[i].key.clone())
                    .collect();
                for key in keys {
                    if !workspace.selected.contains(&key) && workspace.x_key.as_ref() != Some(&key)
                    {
                        workspace.toggle(&key);
                    }
                }
                output.changed = true;
            }
            Some(RowAction::ClearAll) => {
                workspace.selected.clear();
                output.changed = true;
            }
            None => {}
        }
        if output.changed {
            workspace.rebuild();
        }
        output
    }
}
