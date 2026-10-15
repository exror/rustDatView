//! Cached installed fonts and the desktop application's visual theme.
use ab_glyph::Font as _;
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};
use resvg::usvg::fontdb;
use std::{collections::BTreeSet, sync::Arc};

struct Family {
    name: String,
    search_name: String,
    face: fontdb::ID,
}

pub(super) struct Appearance {
    database: fontdb::Database,
    families: Vec<Family>,
    search: String,
    search_key: String,
    matches: Vec<usize>,
    applied_family: Option<String>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self::new()
    }
}

impl Appearance {
    pub(super) fn new() -> Self {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        Self::from_database(database)
    }

    fn from_database(database: fontdb::Database) -> Self {
        let names: BTreeSet<_> = database
            .faces()
            .flat_map(|face| face.families.iter().map(|(name, _)| name.clone()))
            .collect();
        let mut families: Vec<_> = names
            .into_iter()
            .filter_map(|name| {
                let face = database.query(&fontdb::Query {
                    families: &[fontdb::Family::Name(&name)],
                    ..Default::default()
                })?;
                Some(Family {
                    search_name: name.to_lowercase(),
                    name,
                    face,
                })
            })
            .collect();
        families.sort_by(|a, b| a.search_name.cmp(&b.search_name).then(a.name.cmp(&b.name)));
        Self {
            database,
            matches: (0..families.len()).collect(),
            families,
            search: String::new(),
            search_key: String::new(),
            applied_family: None,
        }
    }

    fn update_matches(&mut self) {
        let key = self.search.trim().to_lowercase();
        if self.search_key == key {
            return;
        }
        self.search_key = key;
        self.matches.clear();
        self.matches.extend(
            self.families
                .iter()
                .enumerate()
                .filter(|(_, family)| family.search_name.contains(&self.search_key))
                .map(|(index, _)| index),
        );
    }

    /// Changes only the primary proportional face; bundled glyph fallbacks stay intact.
    /// The family name, rather than a machine-local file path, is the persisted choice.
    pub(super) fn apply_font(&mut self, ctx: &egui::Context, family: &str) -> Result<(), String> {
        if self.applied_family.as_deref() == Some(family) {
            return Ok(());
        }
        let mut definitions = egui::FontDefinitions::default();
        if !family.is_empty() {
            let selected = self
                .families
                .iter()
                .find(|candidate| candidate.name == family)
                .ok_or_else(|| format!("The installed font family ‘{family}’ was not found. Choose another font or System default."))?;
            let data = self
                .database
                .with_face_data(selected.face, font_data)
                .ok_or_else(|| format!("Could not read the installed font ‘{family}’."))?
                .map_err(|error| format!("Cannot use ‘{family}’: {error}"))?;
            let name = "datview-selected-font".to_owned();
            definitions.font_data.insert(name.clone(), Arc::new(data));
            definitions
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, name);
        }
        ctx.set_fonts(definitions);
        self.applied_family = Some(family.to_owned());
        Ok(())
    }

    /// Returns whether the persisted font choice changed. A failed load keeps the old choice.
    pub(super) fn font_picker(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        family: &mut String,
    ) -> Result<bool, String> {
        let mut selected = None;
        egui::ComboBox::from_id_salt("appearance-font-family")
            .width(280.0)
            .height(360.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .selected_text(if family.is_empty() {
                "System default"
            } else {
                family.as_str()
            })
            .show_ui(ui, |ui| {
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Search installed fonts…"),
                    )
                    .changed()
                {
                    self.update_matches();
                }
                if ui
                    .selectable_label(family.is_empty(), "System default (bundled fallback)")
                    .clicked()
                {
                    selected = Some(String::new());
                    ui.close_menu();
                }
                ui.separator();
                let row_height = ui.spacing().interact_size.y;
                egui::ScrollArea::vertical()
                    .id_salt("installed-font-list")
                    .max_height(240.0)
                    .show_rows(ui, row_height, self.matches.len(), |ui, rows| {
                        for row in rows {
                            let entry = &self.families[self.matches[row]];
                            if ui
                                .selectable_label(*family == entry.name, &entry.name)
                                .clicked()
                            {
                                selected = Some(entry.name.clone());
                                ui.close_menu();
                            }
                        }
                    });
                if self.matches.is_empty() {
                    ui.weak("No installed families match this search.");
                }
            });
        if let Some(selected) = selected {
            self.apply_font(ctx, &selected)?;
            if *family != selected {
                *family = selected;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn font_data(bytes: &[u8], index: u32) -> Result<egui::FontData, String> {
    let font = ab_glyph::FontRef::try_from_slice_and_index(bytes, index)
        .map_err(|_| "this face is not a supported TrueType/OpenType font".to_owned())?;
    if !font.units_per_em().is_some_and(|units| units > 0.0)
        || !font.height_unscaled().is_finite()
        || font.height_unscaled() <= 0.0
        || !font
            .codepoint_ids()
            .any(|(id, _)| font.outline(id).is_some())
    {
        return Err("this face has no usable scalable outlines or font metrics".to_owned());
    }
    let mut data = egui::FontData::from_owned(bytes.to_vec());
    data.index = index;
    Ok(data)
}

pub(super) fn apply_theme(ctx: &egui::Context, dark: bool, font_size: f32) {
    let mut style = egui::Style::default();
    let size = if font_size.is_finite() {
        font_size.clamp(8.0, 32.0)
    } else {
        14.0
    };
    for (text_style, scale, family) in [
        (TextStyle::Small, 0.86, FontFamily::Proportional),
        (TextStyle::Body, 1.0, FontFamily::Proportional),
        (TextStyle::Button, 1.0, FontFamily::Proportional),
        (TextStyle::Heading, 1.45, FontFamily::Proportional),
        (TextStyle::Monospace, 0.95, FontFamily::Monospace),
    ] {
        style
            .text_styles
            .insert(text_style, FontId::new(size * scale, family));
    }
    style.spacing.item_spacing = egui::vec2(9.0, 7.0);
    style.spacing.button_padding = egui::vec2(11.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.interact_size.y = size + 14.0;
    style.spacing.combo_width = 160.0;
    style.spacing.slider_width = 180.0;

    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let (panel, surface, input, text, border, hover, active, accent) = if dark {
        (
            Color32::from_rgb(22, 29, 41),
            Color32::from_rgb(29, 39, 54),
            Color32::from_rgb(17, 24, 36),
            Color32::from_rgb(226, 232, 240),
            Color32::from_rgb(62, 77, 98),
            Color32::from_rgb(43, 62, 88),
            Color32::from_rgb(45, 79, 126),
            Color32::from_rgb(116, 173, 255),
        )
    } else {
        (
            Color32::from_rgb(245, 247, 250),
            Color32::from_rgb(252, 252, 250),
            Color32::WHITE,
            Color32::from_rgb(36, 49, 67),
            Color32::from_rgb(203, 213, 225),
            Color32::from_rgb(228, 238, 251),
            Color32::from_rgb(207, 226, 252),
            Color32::from_rgb(37, 99, 193),
        )
    };
    visuals.panel_fill = panel;
    visuals.window_fill = surface;
    visuals.extreme_bg_color = input;
    visuals.faint_bg_color = panel;
    visuals.code_bg_color = panel;
    visuals.hyperlink_color = accent;
    visuals.window_stroke = Stroke::new(1.0_f32, border);
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.selection.bg_fill = active;
    visuals.selection.stroke = Stroke::new(1.0_f32, accent);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, text);
    visuals.widgets.inactive.bg_fill = input;
    visuals.widgets.inactive.weak_bg_fill = input;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, text);
    visuals.widgets.hovered.bg_fill = hover;
    visuals.widgets.hovered.weak_bg_fill = hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, text);
    visuals.widgets.active.bg_fill = active;
    visuals.widgets.active.weak_bg_fill = active;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, accent);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
    }
    visuals.slider_trailing_fill = true;
    style.visuals = visuals;
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled_font() -> Vec<u8> {
        egui::FontDefinitions::default()
            .font_data
            .values()
            .find(|data| font_data(&data.font, data.index).is_ok())
            .expect("egui has a bundled outline font")
            .font
            .to_vec()
    }

    #[test]
    fn catalog_search_is_cached_case_insensitive_and_trimmed() {
        let mut database = fontdb::Database::new();
        database.load_font_data(bundled_font());
        let mut appearance = Appearance::from_database(database);
        assert!(!appearance.families.is_empty());
        let name = appearance.families[0].name.clone();
        appearance.search = format!("  {}  ", name.to_uppercase());
        appearance.update_matches();
        assert!(appearance.matches.contains(&0));
        let matches = appearance.matches.clone();
        appearance.update_matches();
        assert_eq!(appearance.matches, matches);
        appearance.search = "nonexistent family 837194".into();
        appearance.update_matches();
        assert!(appearance.matches.is_empty());
        appearance.search.clear();
        appearance.update_matches();
        assert_eq!(appearance.matches.len(), appearance.families.len());
    }

    #[test]
    fn unsupported_font_and_missing_family_do_not_replace_selection() {
        assert!(font_data(b"not a font", 0).is_err());
        let bytes = bundled_font();
        assert!(font_data(&bytes, u32::MAX).is_err());
        let mut appearance = Appearance::from_database(fontdb::Database::new());
        let ctx = egui::Context::default();
        appearance.apply_font(&ctx, "").unwrap();
        assert!(appearance.apply_font(&ctx, "Not installed").is_err());
        assert_eq!(appearance.applied_family.as_deref(), Some(""));
    }

    #[test]
    fn collection_face_index_is_retained_and_validated() {
        let bytes = bundled_font();
        // Put two valid copies of the bundled SFNT into a TrueType collection.
        let mut collection = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        let first = 20_u32;
        let second = first + bytes.len() as u32;
        collection.extend_from_slice(&first.to_be_bytes());
        collection.extend_from_slice(&second.to_be_bytes());
        for offset in [first, second] {
            let mut face = bytes.clone();
            let tables = u16::from_be_bytes([face[4], face[5]]) as usize;
            for table in 0..tables {
                let start = 12 + table * 16 + 8;
                let original = u32::from_be_bytes(face[start..start + 4].try_into().unwrap());
                face[start..start + 4].copy_from_slice(&(original + offset).to_be_bytes());
            }
            collection.extend_from_slice(&face);
        }
        let data = font_data(&collection, 1).unwrap();
        assert_eq!(data.index, 1);
        assert!(font_data(&collection, 2).is_err());
        let mut database = fontdb::Database::new();
        database.load_font_data(collection);
        let first_face = database.faces().find(|face| face.index == 0).unwrap().id;
        database.remove_face(first_face);
        let mut appearance = Appearance::from_database(database);
        let family = appearance.families[0].name.clone();
        assert_eq!(
            appearance
                .database
                .face(appearance.families[0].face)
                .unwrap()
                .index,
            1
        );
        appearance
            .apply_font(&egui::Context::default(), &family)
            .unwrap();
        assert_eq!(appearance.applied_family.as_deref(), Some(family.as_str()));
    }
}
