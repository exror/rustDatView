use dv_core::common::clean_col;
use dv_core::dataframe::{format_datetime, format_float};
use dv_core::ColumnData;
use eframe::egui::{self, RichText};
use egui_extras::{Column, TableBuilder};

use crate::workspace::{Mode, Workspace};

/// The table reads visible cells directly from the source dataframe. Only a
/// row-index map for masked data and, when necessary, an X search index are kept.
#[derive(Default)]
pub(crate) struct TableView {
    pub(crate) source: usize,
    pub(crate) search: String,
    cache_key: Option<(u64, usize, u64)>,
    cached_x_key: Option<String>,
    cached_search: Option<String>,
    columns: Vec<usize>,
    rows: Option<Vec<usize>>,
    x_column: Option<usize>,
    origin: f64,
    x_index: XIndex,
    selected: Option<usize>,
    last_cursor: Option<(u64, u64)>,
}

#[derive(Default)]
enum XIndex {
    #[default]
    Missing,
    Ascending,
    Descending,
    Indexed {
        rows: Vec<usize>,
        ascending: bool,
    },
}

impl TableView {
    /// `cursor` is a chart X coordinate. Returns the corresponding plotted X
    /// when a table row is clicked, so the application can move its cursor.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &Workspace,
        cursor: Option<f64>,
    ) -> Option<f64> {
        if workspace.tables.tabs.is_empty() {
            ui.label("Open a file to view its data.");
            return None;
        }
        self.source = self.source.min(workspace.tables.tabs.len() - 1);
        let tokens = super::theme::Tokens::for_dark(super::theme::is_dark(ui.ctx()));
        ui.horizontal(|ui| {
            if workspace.sources.len() > 1 {
                let label = workspace
                    .sources
                    .get(self.source)
                    .map(|source| source.name.as_str())
                    .unwrap_or("Table");
                egui::ComboBox::from_id_salt("table-source")
                    .selected_text(label)
                    .width(220.0)
                    .show_ui(ui, |ui| {
                        for (index, source) in workspace.sources.iter().enumerate() {
                            ui.selectable_value(&mut self.source, index, &source.name);
                        }
                    });
            }
            ui.allocate_ui(egui::vec2(ui.available_width().min(300.0), 30.0), |ui| {
                super::widgets::search_field(ui, &mut self.search, "Find a column…");
            });
        });
        self.source = self.source.min(workspace.tables.tabs.len() - 1);
        let table = &workspace.tables.tabs[self.source];
        let Some(source) = workspace.sources.get(self.source) else {
            return None;
        };
        let key = (workspace.data_revision, self.source, table.identity());
        if self.cache_key != Some(key) || self.cached_x_key != workspace.x_key {
            self.rebuild_index(workspace);
            self.cache_key = Some(key);
            self.cached_x_key.clone_from(&workspace.x_key);
            self.cached_search = None;
            self.selected = None;
            self.last_cursor = None;
        }
        if self.cached_search.as_deref() != Some(self.search.as_str()) {
            let query = self.search.trim().to_lowercase();
            self.columns = table
                .data
                .columns()
                .iter()
                .enumerate()
                .filter(|(_, column)| {
                    query.is_empty() || column.name.to_lowercase().contains(&query)
                })
                .map(|(index, _)| index)
                .collect();
            self.cached_search = Some(self.search.clone());
        }

        let row_count = self.rows.as_ref().map_or(table.data.nrows(), Vec::len);
        let has_index = self.columns.contains(&0)
            && table
                .data
                .columns()
                .first()
                .is_some_and(|column| crate::selection::is_index(&column.name));
        ui.label(
            RichText::new(format!(
                "{} {}  ·  {} of {} {}",
                count(row_count),
                if row_count == 1 { "row" } else { "rows" },
                count(self.columns.len()),
                count(table.data.ncols()),
                if table.data.ncols() == 1 {
                    "column"
                } else {
                    "columns"
                }
            ))
            .size(11.5)
            .color(tokens.text_muted),
        );
        ui.add_space(2.0);
        if self.columns.is_empty() {
            ui.label("No columns match the search.");
        }

        let x_values = self
            .x_column
            .and_then(|index| numeric_values(&table.data.column(index).data));
        let mut scroll_to = None;
        if workspace.mode == Mode::Signals {
            if let (Some(cursor), Some(x_values)) =
                (cursor.filter(|value| value.is_finite()), x_values)
            {
                let shift = source.offset - if source.align_start { self.origin } else { 0.0 };
                let cursor_key = (cursor.to_bits(), shift.to_bits());
                if self.last_cursor != Some(cursor_key) {
                    self.selected = self.nearest(x_values, cursor - shift);
                    scroll_to = self.selected;
                    self.last_cursor = Some(cursor_key);
                }
            } else {
                self.last_cursor = None;
            }
        } else {
            self.last_cursor = None;
        }

        let mut clicked_x = None;
        let height = ui.available_height().max(80.0);
        egui::ScrollArea::horizontal()
            .id_salt("table-horizontal")
            .show(ui, |ui| {
                let mut builder = TableBuilder::new(ui)
                    .id_salt(("data-table", table.identity(), &self.search))
                    .striped(true)
                    .resizable(true)
                    .sense(egui::Sense::click())
                    .min_scrolled_height(0.0)
                    .max_scroll_height(height)
                    .cell_layout(egui::Layout::right_to_left(egui::Align::Center));
                if !has_index {
                    builder = builder.column(Column::initial(64.0).at_least(50.0).clip(true));
                }
                for &index in &self.columns {
                    let width = match table.data.column(index).data {
                        ColumnData::DateTime(_) => 190.0,
                        ColumnData::Str(_) => 150.0,
                        ColumnData::Float(_)
                            if crate::selection::is_index(&table.data.column(index).name) =>
                        {
                            70.0
                        }
                        ColumnData::Float(_) => 130.0,
                    };
                    builder = builder.column(Column::initial(width).at_least(56.0).clip(true));
                }
                if let Some(row) = scroll_to {
                    builder = builder.scroll_to_row(row, Some(egui::Align::Center));
                }
                builder
                    .header(34.0, |mut header| {
                        if !has_index {
                            header.col(|ui| {
                                ui.label(RichText::new("Row").size(11.5).color(tokens.text_muted));
                            });
                        }
                        for &index in &self.columns {
                            let name = &table.data.column(index).name;
                            let (label, unit) = dv_core::units::split_name_unit(name);
                            header.col(|ui| {
                                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&label)
                                                .size(12.0)
                                                .family(super::theme::semibold())
                                                .color(tokens.text),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(name);
                                    ui.label(
                                        RichText::new(if unit.is_empty() {
                                            " ".to_owned()
                                        } else {
                                            unit
                                        })
                                        .size(11.0)
                                        .color(tokens.text_muted),
                                    );
                                });
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(21.0, row_count, |mut row| {
                            let visible_row = row.index();
                            let raw_row = self
                                .rows
                                .as_ref()
                                .map_or(visible_row, |rows| rows[visible_row]);
                            row.set_selected(self.selected == Some(visible_row));
                            if !has_index {
                                row.col(|ui| {
                                    ui.label(
                                        RichText::new(count(raw_row + 1)).color(tokens.text_muted),
                                    );
                                });
                            }
                            for &index in &self.columns {
                                let column = table.data.column(index);
                                row.col(|ui| match &column.data {
                                    ColumnData::Float(values) => {
                                        let value = values[raw_row];
                                        if value.is_nan() {
                                            ui.label(RichText::new("—").color(tokens.text_muted))
                                                .on_hover_text("Missing value");
                                        } else {
                                            ui.label(format_float(value));
                                        }
                                    }
                                    ColumnData::DateTime(values) => {
                                        ui.label(format_datetime(values[raw_row]));
                                    }
                                    ColumnData::Str(values) => {
                                        ui.with_layout(
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                let value = values[raw_row].as_str();
                                                ui.add(egui::Label::new(value).truncate())
                                                    .on_hover_text(value);
                                            },
                                        );
                                    }
                                });
                            }
                            if row.response().clicked() {
                                self.selected = Some(visible_row);
                                if workspace.mode == Mode::Signals {
                                    if let Some(value) = x_values
                                        .and_then(|values| values.get(raw_row))
                                        .copied()
                                        .filter(|value| value.is_finite())
                                    {
                                        clicked_x = Some(
                                            value
                                                - if source.align_start {
                                                    self.origin
                                                } else {
                                                    0.0
                                                }
                                                + source.offset,
                                        );
                                    }
                                }
                            }
                        });
                    });
            });
        clicked_x
    }

    fn rebuild_index(&mut self, workspace: &Workspace) {
        let table = &workspace.tables.tabs[self.source];
        self.rows = table.mask.as_ref().map(|mask| {
            mask.iter()
                .take(table.data.nrows())
                .enumerate()
                .filter_map(|(row, &keep)| keep.then_some(row))
                .collect()
        });
        self.x_column = workspace.x_key.as_ref().and_then(|key| {
            table.data.columns().iter().position(|column| {
                clean_col(&column.name) == *key && (column.is_float() || column.is_datetime())
            })
        });
        self.origin = 0.0;
        self.x_index = XIndex::Missing;
        let Some(values) = self
            .x_column
            .and_then(|index| numeric_values(&table.data.column(index).data))
        else {
            return;
        };
        let count = self.rows.as_ref().map_or(values.len(), Vec::len);
        let raw_at = |row: usize| self.rows.as_ref().map_or(row, |rows| rows[row]);
        let mut ascending = true;
        let mut descending = true;
        let mut finite = true;
        let mut previous = None;
        let mut first = None;
        for row in 0..count {
            let value = values[raw_at(row)];
            if !value.is_finite() {
                finite = false;
                continue;
            }
            first.get_or_insert(value);
            if let Some(previous) = previous {
                ascending &= value >= previous;
                descending &= value <= previous;
            }
            previous = Some(value);
        }
        self.origin = values
            .iter()
            .copied()
            .find(|value| value.is_finite())
            .unwrap_or(0.0);
        self.x_index = if first.is_none() {
            XIndex::Missing
        } else if finite && ascending {
            XIndex::Ascending
        } else if finite && descending {
            XIndex::Descending
        } else {
            let mut rows: Vec<_> = (0..count)
                .filter(|&row| values[raw_at(row)].is_finite())
                .collect();
            if !ascending && !descending {
                rows.sort_unstable_by(|&a, &b| values[raw_at(a)].total_cmp(&values[raw_at(b)]));
            }
            XIndex::Indexed {
                rows,
                ascending: ascending || !descending,
            }
        };
    }

    fn nearest(&self, values: &[f64], target: f64) -> Option<usize> {
        if !target.is_finite() {
            return None;
        }
        let len = self.rows.as_ref().map_or(values.len(), Vec::len);
        let value_at = |row: usize| values[self.rows.as_ref().map_or(row, |rows| rows[row])];
        match &self.x_index {
            XIndex::Missing => None,
            XIndex::Ascending | XIndex::Descending => {
                let ascending = matches!(&self.x_index, XIndex::Ascending);
                let mut lo = 0;
                let mut hi = len;
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    if if ascending {
                        value_at(mid) < target
                    } else {
                        value_at(mid) > target
                    } {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                closest(lo, len, target, value_at)
            }
            XIndex::Indexed { rows, ascending } => {
                let insertion = rows.partition_point(|&row| {
                    if *ascending {
                        value_at(row) < target
                    } else {
                        value_at(row) > target
                    }
                });
                closest(insertion, rows.len(), target, |position| {
                    value_at(rows[position])
                })
                .map(|position| rows[position])
            }
        }
    }
}

fn numeric_values(data: &ColumnData) -> Option<&[f64]> {
    match data {
        ColumnData::Float(values) | ColumnData::DateTime(values) => Some(values),
        ColumnData::Str(_) => None,
    }
}

fn closest(
    index: usize,
    len: usize,
    target: f64,
    value_at: impl Fn(usize) -> f64,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if index == 0 {
        return Some(0);
    }
    if index == len {
        return Some(len - 1);
    }
    if (value_at(index - 1) - target).abs() <= (value_at(index) - target).abs() {
        Some(index - 1)
    } else {
        Some(index)
    }
}

fn count(value: usize) -> String {
    let raw = value.to_string();
    let mut result = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    result
}
