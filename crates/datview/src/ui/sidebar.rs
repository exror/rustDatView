use std::collections::{BTreeMap, HashMap, HashSet};

use dv_core::{common::clean_col, ColumnData};
use eframe::egui::{self, Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::{
    settings::Settings,
    ui::{icons, theme, widgets},
    workspace::Workspace,
};

#[derive(Default)]
pub(crate) struct Sidebar {
    pub query: String,
    pub focus_search: bool,
    group_by_unit: bool,
    show_index: bool,
    initialized: bool,
    last_clicked: Option<String>,
    previews: HashMap<(u64, String), Preview>,
    presence_signature: Vec<(u64, bool)>,
    presence: HashMap<String, usize>,
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
    pub changed: bool,
    pub formula: Option<FormulaRequest>,
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
        if signature != self.presence_signature {
            self.presence_signature = signature;
            self.presence.clear();
            self.previews.clear();
            for (table, source) in workspace.tables.tabs.iter().zip(&workspace.sources) {
                if source.active {
                    let keys: HashSet<_> = table
                        .data
                        .columns()
                        .iter()
                        .map(|column| clean_col(&column.name))
                        .collect();
                    for key in keys {
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
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            widgets::section_label(ui, "Sources");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, icons::PLUS, "Add files · Ctrl+Shift+O").clicked() {
                    output.add = true;
                }
            });
        });
        let mut remove = None;
        for (index, source) in workspace.sources.iter_mut().enumerate() {
            let Some(table) = workspace.tables.tabs.get(index) else {
                continue;
            };
            egui::Frame::new()
                .fill(tokens.surface)
                .stroke(Stroke::new(1.0, tokens.border))
                .corner_radius(6)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .checkbox(&mut source.active, "")
                            .on_hover_text("Include this source in the chart")
                            .changed()
                        {
                            output.changed = true;
                        }
                        let text =
                            egui::RichText::new(&source.name)
                                .strong()
                                .color(if source.active {
                                    tokens.text
                                } else {
                                    tokens.text_muted
                                });
                        let response = ui.add(egui::Label::new(text).truncate());
                        response
                            .clone()
                            .on_hover_text(source.path.display().to_string());
                        response.context_menu(|ui| {
                            ui.label(egui::RichText::new("Source").strong());
                            if ui.text_edit_singleline(&mut source.name).changed() {
                                output.changed = true;
                            }
                            if ui
                                .checkbox(&mut source.align_start, "Align start to zero")
                                .on_hover_text(
                                    "Subtract the first X value before applying the offset",
                                )
                                .changed()
                            {
                                output.changed = true;
                            }
                            ui.horizontal(|ui| {
                                ui.label("X offset");
                                if ui
                                    .add(egui::DragValue::new(&mut source.offset).speed(0.1))
                                    .changed()
                                {
                                    output.changed = true;
                                }
                            });
                            if ui.button("Reload source").clicked() {
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
                            ui.separator();
                            if ui.button("Remove from workspace").clicked() {
                                remove = Some(index);
                                ui.close();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(24.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "{} rows  ·  {} channels",
                                count(table.nrows()),
                                count(table.ncols().saturating_sub(1))
                            ))
                            .size(11.5)
                            .color(tokens.text_muted),
                        );
                    });
                });
            ui.add_space(3.0);
        }
        if let Some(index) = remove {
            workspace.remove_source(index);
            output.changed = true;
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
                            if !ui.is_rect_visible(rect) {
                                continue;
                            }
                            let painter = ui.painter().with_clip_rect(rect);
                            if response.hovered() {
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
                                26.0
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
                                painter.text(
                                    Pos2::new(
                                        rect.right() - unit_width - spark_width - 3.0,
                                        rect.center().y,
                                    ),
                                    Align2::RIGHT_CENTER,
                                    format!("{availability}/{active_sources}"),
                                    FontId::proportional(10.0),
                                    tokens.text_muted,
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
                                let cache_key = workspace
                                    .tables
                                    .tabs
                                    .iter()
                                    .zip(&workspace.sources)
                                    .find(|(table, source)| {
                                        source.active
                                            && table.data.columns().iter().any(|column| {
                                                clean_col(&column.name) == channel.key
                                            })
                                    })
                                    .map(|(table, _)| (table.identity(), channel.key.clone()));
                                if let Some(cache_key) = cache_key {
                                    if !self.previews.contains_key(&cache_key) {
                                        if let Some((key, value)) = preview(workspace, &channel.key)
                                        {
                                            self.previews.insert(key, value);
                                        }
                                    }
                                    if let Some(p) = self.previews.get(&cache_key) {
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
                                            "\n{}min {:.4}  ·  mean {:.4}  ·  max {:.4}",
                                            if p.sampled { "Preview: " } else { "" },
                                            p.min,
                                            p.mean,
                                            p.max
                                        ));
                                    }
                                }
                            }
                            response.clone().on_hover_text(tooltip);
                            if response.clicked() {
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
