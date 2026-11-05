//! Export data (rows of a source, optionally only the plotted channels or the
//! selected range) and export a reproducible Python script.
use dv_chart::ChartState;
use dv_core::{common::clean_col, DataFrame};
use dv_plot::ScriptOptions;
use eframe::egui::{self, RichText};

use super::{hint, modal, primary_button};
use crate::{
    ui::{chart_view::ChartOptions, theme, widgets},
    workspace::Workspace,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Rows {
    #[default]
    All,
    View,
    Selection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Columns {
    #[default]
    Plotted,
    All,
}

pub(crate) struct DataExportDialog {
    pub open: bool,
    source: usize,
    rows: Rows,
    columns: Columns,
    format: &'static str,
}

impl Default for DataExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            source: 0,
            rows: Rows::All,
            columns: Columns::Plotted,
            format: "csv",
        }
    }
}

pub(crate) enum ExportOutcome {
    Done(String),
    Failed(String),
}

impl DataExportDialog {
    pub(crate) fn open(&mut self, workspace: &Workspace, chart: &ChartState) {
        self.open = true;
        self.source = workspace
            .sources
            .iter()
            .position(|source| source.active)
            .unwrap_or(0);
        self.rows = if chart.selection.is_some() {
            Rows::Selection
        } else {
            Rows::All
        };
    }

    /// Open preselected on one source (from the source's context menu).
    pub(crate) fn open_source(&mut self, workspace: &Workspace, chart: &ChartState, index: usize) {
        self.open(workspace, chart);
        if index < workspace.sources.len() {
            self.source = index;
        }
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: &Workspace,
        chart: &ChartState,
    ) -> Option<ExportOutcome> {
        if !self.open || workspace.tables.tabs.is_empty() {
            self.open = false;
            return None;
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        self.source = self.source.min(workspace.tables.tabs.len() - 1);
        let mut outcome = None;
        let writers = dv_io::writers();
        let (_, closed) = modal(ctx, "data-export", "Export data", 460.0, |ui, close| {
            if workspace.sources.len() > 1 {
                ui.label(RichText::new("Source").color(tokens.text_secondary));
                egui::ComboBox::from_id_salt("export-source")
                    .width(ui.available_width())
                    .selected_text(workspace.sources[self.source].name.as_str())
                    .show_ui(ui, |ui| {
                        for (index, source) in workspace.sources.iter().enumerate() {
                            ui.selectable_value(&mut self.source, index, &source.name);
                        }
                    });
                ui.add_space(6.0);
            }
            ui.label(RichText::new("Rows").color(tokens.text_secondary));
            let mut rows: Vec<(Rows, &str, &str)> =
                vec![(Rows::All, "All rows", "Every row of the source")];
            if chart.x_view.is_some() {
                rows.push((
                    Rows::View,
                    "Visible range",
                    "Rows whose X is in the visible range",
                ));
            }
            if chart.selection.is_some() {
                rows.push((
                    Rows::Selection,
                    "Selection",
                    "Rows whose X is in the selected range",
                ));
            }
            if !rows.iter().any(|(choice, _, _)| *choice == self.rows) {
                self.rows = Rows::All;
            }
            if let Some(choice) = widgets::segmented(ui, self.rows, &rows) {
                self.rows = choice;
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Columns").color(tokens.text_secondary));
            if let Some(choice) = widgets::segmented(
                ui,
                self.columns,
                &[
                    (
                        Columns::Plotted,
                        "Plotted channels",
                        "X plus every plotted channel",
                    ),
                    (Columns::All, "All columns", "Every column of the source"),
                ],
            ) {
                self.columns = choice;
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Format").color(tokens.text_secondary));
            egui::ComboBox::from_id_salt("export-format")
                .selected_text(format_label(self.format))
                .show_ui(ui, |ui| {
                    for writer in &writers {
                        ui.selectable_value(
                            &mut self.format,
                            writer.name,
                            format_label(writer.name),
                        );
                    }
                });
            ui.add_space(8.0);
            let frame = self.frame(workspace, chart);
            match &frame {
                Ok(df) => hint(
                    ui,
                    &format!("{} rows × {} columns", count(df.nrows()), df.ncols()),
                ),
                Err(error) => {
                    ui.label(RichText::new(error).color(tokens.error));
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "Save…", frame.is_ok()).clicked() {
                        if let Ok(df) = &frame {
                            let writer = writers.iter().find(|writer| writer.name == self.format);
                            let extension = writer
                                .and_then(|writer| writer.extensions.first())
                                .map_or("csv", |ext| ext.trim_start_matches('.'));
                            let stem = workspace.sources[self.source]
                                .name
                                .rsplit_once('.')
                                .map_or(
                                    workspace.sources[self.source].name.as_str(),
                                    |(stem, _)| stem,
                                )
                                .to_owned();
                            if let Some(mut path) = rfd::FileDialog::new()
                                .add_filter(format_label(self.format), &[extension])
                                .set_file_name(format!("{stem}-export.{extension}"))
                                .save_file()
                            {
                                if path.extension().is_none() {
                                    path.set_extension(extension);
                                }
                                outcome = Some(
                                    match dv_io::write_dataframe(df, &path, Some(self.format)) {
                                        Ok(()) => {
                                            *close = true;
                                            ExportOutcome::Done(format!(
                                                "Exported {} rows to {}",
                                                count(df.nrows()),
                                                path.display()
                                            ))
                                        }
                                        Err(error) => ExportOutcome::Failed(format!(
                                            "Could not export to {}: {error}",
                                            path.display()
                                        )),
                                    },
                                );
                            }
                        }
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
        if closed {
            self.open = false;
        }
        outcome
    }

    fn frame(&self, workspace: &Workspace, chart: &ChartState) -> Result<DataFrame, String> {
        let table = &workspace.tables.tabs[self.source];
        let source = &workspace.sources[self.source];
        let columns = table.data.columns();
        let x_index = workspace.x_key.as_ref().and_then(|key| {
            columns
                .iter()
                .position(|column| clean_col(&column.name) == *key)
        });
        let index_column = columns
            .first()
            .is_some_and(|column| crate::selection::is_index(&column.name));
        let selected: Vec<usize> = match self.columns {
            Columns::All => (usize::from(index_column)..columns.len()).collect(),
            Columns::Plotted => {
                // Exactly what the chart shows for this source, in plot order;
                // series removed from the chart are not exported.
                let mut indices: Vec<usize> = x_index.into_iter().collect();
                for series in workspace
                    .series
                    .iter()
                    .filter(|series| series.source == self.source)
                {
                    if let Some(index) = columns
                        .iter()
                        .position(|column| clean_col(&column.name) == series.key)
                    {
                        if !indices.contains(&index) {
                            indices.push(index);
                        }
                    }
                }
                if indices.len() <= usize::from(x_index.is_some()) {
                    return Err("This source has none of the plotted channels.".into());
                }
                indices
            }
        };
        let range = match self.rows {
            Rows::All => None,
            Rows::View => chart.x_view,
            Rows::Selection => chart.selection,
        }
        .map(|[a, b]| [a.min(b), a.max(b)]);
        let mut keep: Vec<bool> = table
            .mask
            .clone()
            .unwrap_or_else(|| vec![true; table.data.nrows()]);
        keep.resize(table.data.nrows(), false);
        if let Some([a, b]) = range {
            let Some(values) = x_index.and_then(|index| match &table.data.column(index).data {
                dv_core::ColumnData::Float(values) | dv_core::ColumnData::DateTime(values) => {
                    Some(values.as_slice())
                }
                dv_core::ColumnData::Str(_) => None,
            }) else {
                return Err("The X axis is not numeric, so a range cannot be applied.".into());
            };
            // Plotted X includes the source's alignment and offset.
            let start = if source.align_start {
                values
                    .iter()
                    .copied()
                    .find(|v| v.is_finite())
                    .unwrap_or(0.0)
            } else {
                0.0
            };
            let shift = source.offset - start;
            for (keep, value) in keep.iter_mut().zip(values) {
                *keep &= (value + shift) >= a && (value + shift) <= b;
            }
        }
        let df = table.data.select_columns(&selected).filter_rows(&keep);
        if df.nrows() == 0 {
            return Err("No rows fall in this range.".into());
        }
        Ok(df)
    }
}

fn format_label(name: &str) -> &str {
    match name {
        "csv" => "CSV (comma-separated)",
        "parquet" => "Parquet",
        "outb" => "OpenFAST binary (.outb)",
        "wnd" => "Wind file (.wnd)",
        other => other,
    }
}

fn count(value: usize) -> String {
    dv_chart::format::format_value(value as f64, "", 15)
}

const LIBRARIES: [(&str, &str, &str); 3] = [
    (
        "pydatview",
        "pyDatView",
        "Uses pydatview's readers and tools",
    ),
    ("welib", "welib", "Uses welib's readers and tools"),
    (
        "openfast_toolbox",
        "OpenFAST toolbox",
        "Uses openfast_toolbox's readers and tools",
    ),
];
const FRAMES: [(&str, &str, &str); 3] = [
    ("dict", "Dictionary", "Data frames in a dict keyed by name"),
    ("list", "List", "Data frames in a list"),
    ("enumeration", "Variables", "One variable per data frame"),
];

pub(crate) struct ScriptDialog {
    pub open: bool,
    options: ScriptOptions,
    /// Generated when opened and whenever an option changes.
    script: Result<String, String>,
    dirty: bool,
}

impl Default for ScriptDialog {
    fn default() -> Self {
        Self {
            open: false,
            options: ScriptOptions::default(),
            script: Ok(String::new()),
            dirty: true,
        }
    }
}

impl ScriptDialog {
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.dirty = true;
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: &Workspace,
        chart: &ChartOptions,
    ) -> Option<ExportOutcome> {
        if !self.open {
            return None;
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        if self.dirty {
            self.script = crate::export::python_script(workspace, chart, self.options.clone());
            self.dirty = false;
        }
        let mut outcome = None;
        let height = (ctx.content_rect().height() - 260.0).clamp(200.0, 520.0);
        let (_, closed) = modal(
            ctx,
            "script-export",
            "Export Python script",
            760.0,
            |ui, close| {
                hint(ui, "A script that reproduces this chart from the source files, including processing steps and formulas. Running it needs Python and the chosen library.");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Library");
                    if let Some(value) =
                        widgets::segmented(ui, self.options.lib_flavor.as_str(), &LIBRARIES)
                            .map(str::to_owned)
                    {
                        self.options.lib_flavor = value;
                        self.dirty = true;
                    }
                    ui.add_space(12.0);
                    ui.label("Data frames");
                    if let Some(value) =
                        widgets::segmented(ui, self.options.dfs_flavor.as_str(), &FRAMES)
                            .map(str::to_owned)
                    {
                        self.options.dfs_flavor = value;
                        self.dirty = true;
                    }
                });
                ui.horizontal(|ui| {
                    self.dirty |= ui
                        .checkbox(&mut self.options.one_tab_per_file, "One table per file")
                        .changed();
                    ui.label("Comments");
                    self.dirty |= ui
                        .add(egui::Slider::new(
                            &mut self.options.verbose_comment_level,
                            0..=3,
                        ))
                        .changed();
                });
                ui.add_space(6.0);
                match &self.script {
                    Ok(script) => {
                        egui::Frame::new()
                            .fill(tokens.surface)
                            .stroke(egui::Stroke::new(1.0, tokens.border))
                            .corner_radius(8)
                            .inner_margin(egui::Margin::same(8))
                            .show(ui, |ui| {
                                egui::ScrollArea::both()
                                    .max_height(height)
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(script.as_str())
                                                    .monospace()
                                                    .size(12.0),
                                            )
                                            .selectable(true)
                                            .extend(),
                                        );
                                    });
                            });
                    }
                    Err(error) => {
                        ui.label(RichText::new(error).color(tokens.error));
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ok = self.script.is_ok();
                        if primary_button(ui, "Save…", ok).clicked() {
                            if let (Ok(script), Some(path)) = (
                                &self.script,
                                crate::export::save_dialog(workspace, "", "py")
                                    .add_filter("Python", &["py"])
                                    .save_file(),
                            ) {
                                outcome = Some(match std::fs::write(&path, script) {
                                    Ok(()) => {
                                        *close = true;
                                        ExportOutcome::Done(format!(
                                            "Saved script to {}",
                                            path.display()
                                        ))
                                    }
                                    Err(error) => ExportOutcome::Failed(format!(
                                        "Could not save {}: {error}",
                                        path.display()
                                    )),
                                });
                            }
                        }
                        if ui
                            .add_enabled(
                                ok,
                                egui::Button::new("Copy").min_size(egui::vec2(0.0, 30.0)),
                            )
                            .clicked()
                        {
                            if let Ok(script) = &self.script {
                                ui.ctx().copy_text(script.clone());
                                outcome = Some(ExportOutcome::Done(
                                    "Script copied to the clipboard".into(),
                                ));
                            }
                        }
                    });
                });
            },
        );
        if closed {
            self.open = false;
        }
        outcome
    }
}
