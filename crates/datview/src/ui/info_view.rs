use std::time::UNIX_EPOCH;

use eframe::egui::{self, RichText};

use crate::workspace::Workspace;

/// File-system and reader metadata are refreshed only when the workspace changes.
#[derive(Default)]
pub(crate) struct InfoView {
    generation: Option<u64>,
    details: Vec<SourceDetails>,
}

#[derive(Default)]
struct SourceDetails {
    size: Option<String>,
    modified: Option<String>,
    reader_info: Option<String>,
    sampling: Option<(f64, f64)>,
    visible_rows: usize,
}

impl InfoView {
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, workspace: &Workspace) {
        if self.generation != Some(workspace.data_revision) {
            self.details = workspace
                .tables
                .tabs
                .iter()
                .enumerate()
                .map(|(index, table)| {
                    let metadata = workspace
                        .sources
                        .get(index)
                        .and_then(|source| std::fs::metadata(&source.path).ok());
                    let size = metadata
                        .as_ref()
                        .map(|metadata| format_bytes(metadata.len()));
                    let modified = metadata
                        .and_then(|metadata| metadata.modified().ok())
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map(|time| {
                            let value = dv_core::dataframe::format_datetime(time.as_secs_f64());
                            value.split('.').next().unwrap_or(&value).to_owned()
                        });
                    let reader_info = table
                        .fileobject
                        .as_ref()
                        .map(|file| file.info().replace("\n: ", "\n"))
                        .filter(|s| !s.trim().is_empty());
                    let visible_rows = table.mask.as_ref().map_or(table.data.nrows(), |mask| {
                        mask.iter().filter(|&&visible| visible).count()
                    });
                    let sampling = sampling_summary(table);
                    SourceDetails {
                        size,
                        modified,
                        reader_info,
                        sampling,
                        visible_rows,
                    }
                })
                .collect();
            self.generation = Some(workspace.data_revision);
        }

        ui.heading("File information");
        ui.add_space(6.0);
        if workspace.tables.tabs.is_empty() {
            ui.label("Open a file to see its information.");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("source-information")
            .show(ui, |ui| {
                for (index, table) in workspace.tables.tabs.iter().enumerate() {
                    let Some(source) = workspace.sources.get(index) else {
                        continue;
                    };
                    let Some(details) = self.details.get(index) else {
                        continue;
                    };
                    ui.add_space(10.0);
                    ui.heading(&source.name);
                    egui::Grid::new(("source-properties", table.identity()))
                        .num_columns(2)
                        .spacing([24.0, 7.0])
                        .striped(false)
                        .show(ui, |ui| {
                            property(ui, "Path", &source.path.display().to_string());
                            property(
                                ui,
                                "Format",
                                if table.fileformat_name.is_empty() {
                                    "Unknown"
                                } else {
                                    &table.fileformat_name
                                },
                            );
                            if !table.source_frame_name().is_empty() {
                                property(ui, "Table", table.source_frame_name());
                            }
                            property(ui, "Rows", &format_count(table.data.nrows()));
                            if table.mask.is_some() {
                                property(
                                    ui,
                                    "Rows after mask",
                                    &format_count(details.visible_rows),
                                );
                            }
                            property(ui, "Columns", &format_count(table.data.ncols()));
                            if let Some(size) = &details.size {
                                property(ui, "File size", size);
                            }
                            if let Some(modified) = &details.modified {
                                property(ui, "Modified", modified);
                            }
                            if let Some((duration, rate)) = details.sampling {
                                property(
                                    ui,
                                    "Sample rate",
                                    &dv_chart::format::format_value(rate, "Hz", 4),
                                );
                                property(
                                    ui,
                                    "Duration",
                                    &dv_chart::format::format_duration(duration, 0.001),
                                );
                            }
                            property(
                                ui,
                                "Included in chart",
                                if source.active { "Yes" } else { "No" },
                            );
                            if source.align_start {
                                property(ui, "X alignment", "Start at zero");
                            }
                            if source.offset != 0.0 {
                                property(
                                    ui,
                                    "X offset",
                                    &dv_chart::format::format_value(source.offset, "", 6),
                                );
                            }
                            if !table.mask_string.is_empty() {
                                property(ui, "Mask", &table.mask_string);
                            }
                        });
                    if let Some(info) = &details.reader_info {
                        ui.add_space(6.0);
                        ui.strong("Reader notes");
                        ui.label(info);
                    }
                    if !table.formulas.is_empty() {
                        ui.add_space(10.0);
                        ui.strong("Formula columns");
                        egui::Grid::new(("source-formulas", table.identity()))
                            .num_columns(2)
                            .spacing([24.0, 5.0])
                            .striped(true)
                            .show(ui, |ui| {
                                for formula in &table.formulas {
                                    ui.label(&formula.name);
                                    ui.monospace(&formula.formula);
                                    ui.end_row();
                                }
                            });
                    }
                    if index + 1 < workspace.tables.tabs.len() {
                        ui.add_space(10.0);
                        ui.separator();
                    }
                }
                if !workspace.pipeline.actions.is_empty() {
                    ui.add_space(18.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.heading("Processing steps");
                    ui.add_space(4.0);
                    for action in &workspace.pipeline.actions {
                        ui.horizontal(|ui| {
                            ui.label(if action.enabled { "●" } else { "○" });
                            ui.label(action.plugin.summary());
                            if !action.enabled {
                                ui.weak("Off");
                            }
                        });
                    }
                }
            });
    }
}

/// Summarize each source's own time column, independently of the chart's X selection.
fn sampling_summary(table: &dv_core::Table) -> Option<(f64, f64)> {
    use dv_core::{
        units::{parse_unit, split_name_unit, UnitFamily},
        ColumnData,
    };
    let column = table
        .data
        .columns()
        .get(crate::selection::default_x(table))?;
    let (values, factor) = match &column.data {
        ColumnData::DateTime(values) => (values.as_slice(), 1.0),
        ColumnData::Float(values) => {
            let (_, unit) = split_name_unit(&column.name);
            let unit = parse_unit(&unit);
            if unit.family != UnitFamily::Time {
                return None;
            }
            (values.as_slice(), unit.to_base)
        }
        ColumnData::Str(_) => return None,
    };
    let first = values.iter().position(|value| value.is_finite())?;
    let last = values.iter().rposition(|value| value.is_finite())?;
    let duration = (values[last] - values[first]) * factor;
    if !duration.is_finite() || duration <= 0.0 || first == last {
        return None;
    }
    Some((duration, (last - first) as f64 / duration))
}

fn property(ui: &mut egui::Ui, name: &str, value: &str) {
    ui.label(RichText::new(name).weak());
    ui.label(value);
    ui.end_row();
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn format_count(count: usize) -> String {
    let raw = count.to_string();
    let mut formatted = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index) % 3 == 0 {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted
}
