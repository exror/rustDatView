//! Bundled fonts, installed-font selection, and the Graphite/Paper visual language.
use std::{collections::BTreeSet, sync::Arc};

use ab_glyph::Font as _;
use eframe::egui::{self, Color32, FontFamily, FontId, Stroke, TextStyle};
use resvg::usvg::fontdb;

use crate::settings::{SeriesPalette, Settings, ThemeChoice};

pub(crate) const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
pub(crate) const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
pub(crate) const JETBRAINS_MONO: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
pub(crate) const PHOSPHOR: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");

pub(crate) fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Phosphor is an explicit family as well as a fallback, for unambiguous icon buttons.
pub(crate) fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tokens {
    pub app_bg: Color32,
    pub panel_bg: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub grid: Color32,
    pub axis_text: Color32,
    pub crosshair: Color32,
}

impl Tokens {
    pub(crate) fn for_dark(dark: bool) -> Self {
        if dark {
            Self {
                app_bg: Color32::from_rgb(0x11, 0x13, 0x17),
                panel_bg: Color32::from_rgb(0x15, 0x18, 0x1d),
                surface: Color32::from_rgb(0x1a, 0x1e, 0x24),
                raised: Color32::from_rgb(0x20, 0x25, 0x2c),
                border: Color32::from_rgb(0x28, 0x2e, 0x36),
                border_strong: Color32::from_rgb(0x34, 0x3b, 0x45),
                text: Color32::from_rgb(0xe7, 0xea, 0xee),
                text_secondary: Color32::from_rgb(0xa6, 0xae, 0xba),
                text_muted: Color32::from_rgb(0x6f, 0x78, 0x84),
                accent: Color32::from_rgb(0x76, 0x92, 0xf5),
                accent_soft: Color32::from_rgba_unmultiplied(0x76, 0x92, 0xf5, 41),
                success: Color32::from_rgb(0x3f, 0xb6, 0x8b),
                warning: Color32::from_rgb(0xe5, 0xa9, 0x3b),
                error: Color32::from_rgb(0xf0, 0x61, 0x6d),
                grid: Color32::from_white_alpha(14),
                axis_text: Color32::from_rgb(0x8c, 0x95, 0xa1),
                crosshair: Color32::from_white_alpha(102),
            }
        } else {
            Self {
                app_bg: Color32::from_rgb(0xf5, 0xf6, 0xf8),
                panel_bg: Color32::from_rgb(0xfa, 0xfb, 0xfc),
                surface: Color32::WHITE,
                raised: Color32::WHITE,
                border: Color32::from_rgb(0xe2, 0xe5, 0xea),
                border_strong: Color32::from_rgb(0xcd, 0xd3, 0xdb),
                text: Color32::from_rgb(0x1a, 0x1e, 0x23),
                text_secondary: Color32::from_rgb(0x59, 0x62, 0x6e),
                text_muted: Color32::from_rgb(0x8b, 0x94, 0xa0),
                accent: Color32::from_rgb(0x42, 0x65, 0xda),
                accent_soft: Color32::from_rgba_unmultiplied(0x42, 0x65, 0xda, 31),
                success: Color32::from_rgb(0x1f, 0x9d, 0x6c),
                warning: Color32::from_rgb(0xc9, 0x87, 0x10),
                error: Color32::from_rgb(0xd9, 0x38, 0x48),
                grid: Color32::from_black_alpha(15),
                axis_text: Color32::from_rgb(0x6a, 0x73, 0x80),
                crosshair: Color32::from_black_alpha(89),
            }
        }
    }
}

pub(crate) fn is_dark(ctx: &egui::Context) -> bool {
    ctx.theme() == egui::Theme::Dark
}

/// The brand spectrum, adjusted for legibility on each chart background.
pub(crate) fn palette(dark: bool) -> [Color32; 10] {
    let colors = if dark {
        [
            0x7692f5, 0xff9960, 0x37bfd8, 0xf36c7e, 0xbb86e2, 0x45c888, 0xe6c750, 0x5cbde4,
            0xeb8bb5, 0xb8c1cc,
        ]
    } else {
        [
            0x4265da, 0xc95713, 0x0a829c, 0xd33249, 0x8744b8, 0x16884f, 0xa37b0a, 0x167eb0,
            0xbd497b, 0x6f7a87,
        ]
    };
    colors.map(rgb)
}

/// Series color selection is independent of the interface theme.
pub(crate) fn palette_for(dark: bool, choice: SeriesPalette) -> [Color32; 10] {
    match choice {
        SeriesPalette::Standard => palette(dark),
        SeriesPalette::Vivid => {
            let colors = if dark {
                [
                    0x38bdf8, 0xfb7185, 0xa3e635, 0xc084fc, 0xfbbf24, 0x2dd4bf, 0xf472b6, 0x818cf8,
                    0xf97316, 0xe2e8f0,
                ]
            } else {
                [
                    0x0079b5, 0xd52f56, 0x588b00, 0x8a3dd0, 0xb76e00, 0x008779, 0xbc3680, 0x4e57ce,
                    0xc44e00, 0x566577,
                ]
            };
            colors.map(rgb)
        }
    }
}

fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

struct Family {
    name: String,
    search_name: String,
    face: fontdb::ID,
    semibold: Option<fontdb::ID>,
}

pub(crate) struct Appearance {
    database: fontdb::Database,
    families: Vec<Family>,
    search: String,
    search_key: String,
    matches: Vec<usize>,
    applied_family: Option<String>,
    applied_theme: Option<ThemeChoice>,
    applied_scale: Option<f32>,
    styles_installed: bool,
    failed_family: Option<String>,
    pub(crate) font_error: Option<String>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self::new()
    }
}

impl Appearance {
    pub(crate) fn new() -> Self {
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
                let family = [fontdb::Family::Name(&name)];
                let face = database.query(&fontdb::Query {
                    families: &family,
                    ..Default::default()
                })?;
                let semibold = database.query(&fontdb::Query {
                    families: &family,
                    weight: fontdb::Weight::SEMIBOLD,
                    ..Default::default()
                });
                Some(Family {
                    search_name: name.to_lowercase(),
                    name,
                    face,
                    semibold,
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
            applied_theme: None,
            applied_scale: None,
            styles_installed: false,
            failed_family: None,
            font_error: None,
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

    /// Install both styles once, select the user's theme/scale, and apply the chosen font.
    /// System theme remains an egui preference, so later OS changes follow automatically.
    pub(crate) fn apply(&mut self, ctx: &egui::Context, settings: &Settings) -> Result<(), String> {
        if !self.styles_installed {
            ctx.set_style_of(egui::Theme::Dark, style(true));
            ctx.set_style_of(egui::Theme::Light, style(false));
            self.styles_installed = true;
        }
        if self.applied_theme != Some(settings.theme) {
            ctx.set_theme(match settings.theme {
                ThemeChoice::System => egui::ThemePreference::System,
                ThemeChoice::Dark => egui::ThemePreference::Dark,
                ThemeChoice::Light => egui::ThemePreference::Light,
            });
            self.applied_theme = Some(settings.theme);
        }
        let scale = if settings.scale.is_finite() {
            settings.scale.clamp(0.8, 2.0)
        } else {
            1.0
        };
        if self.applied_scale != Some(scale) {
            ctx.set_zoom_factor(scale);
            self.applied_scale = Some(scale);
        }
        if self.failed_family.as_deref() == Some(settings.font_family.as_str()) {
            if let Some(error) = &self.font_error {
                return Err(error.clone());
            }
        }
        match self.apply_font(ctx, &settings.font_family) {
            Ok(()) => {
                self.failed_family = None;
                Ok(())
            }
            Err(error) => {
                self.failed_family = Some(settings.font_family.clone());
                self.font_error = Some(error.clone());
                Err(error)
            }
        }
    }

    fn apply_font(&mut self, ctx: &egui::Context, family: &str) -> Result<(), String> {
        if self.applied_family.as_deref() == Some(family) {
            return Ok(());
        }
        // Even a missing persisted system font must leave all bundled fonts and icons installed.
        if self.applied_family.is_none() && !family.is_empty() {
            self.apply_font(ctx, "")?;
        }
        let mut fonts = bundled_fonts();
        if !family.is_empty() {
            let selected = self.families.iter().find(|candidate| candidate.name == family)
                .ok_or_else(|| format!("The installed font family ‘{family}’ was not found. Choose another font or bundled Inter."))?;
            let data = self
                .database
                .with_face_data(selected.face, font_data)
                .ok_or_else(|| format!("Could not read the installed font ‘{family}’."))?
                .map_err(|error| format!("Cannot use ‘{family}’: {error}"))?;
            fonts
                .font_data
                .insert("selected-regular".into(), Arc::new(data));
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, "selected-regular".into());
            let semibold_name =
                if let Some(face) = selected.semibold.filter(|face| *face != selected.face) {
                    if let Some(Ok(data)) = self.database.with_face_data(face, font_data) {
                        fonts
                            .font_data
                            .insert("selected-semibold".into(), Arc::new(data));
                        "selected-semibold"
                    } else {
                        "selected-regular"
                    }
                } else {
                    "selected-regular"
                };
            fonts
                .families
                .entry(semibold())
                .or_default()
                .insert(0, semibold_name.into());
        }
        ctx.set_fonts(fonts);
        self.applied_family = Some(family.to_owned());
        Ok(())
    }

    /// Select an installed family without committing an unreadable or unsupported face.
    pub(crate) fn font_picker(&mut self, ui: &mut egui::Ui, family: &mut String) {
        let mut selected = None;
        egui::ComboBox::from_id_salt("appearance-font-family")
            .width(260.0)
            .height(360.0)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .selected_text(if family.is_empty() {
                "Inter (bundled)"
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
                    .selectable_label(family.is_empty(), "Inter (bundled)")
                    .clicked()
                {
                    selected = Some(String::new());
                    ui.close();
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
                                ui.close();
                            }
                        }
                    });
                if self.matches.is_empty() {
                    ui.weak("No installed families match this search.");
                }
            });
        if let Some(selected) = selected {
            match self.apply_font(ui.ctx(), &selected) {
                Ok(()) => {
                    *family = selected;
                    self.failed_family = None;
                    self.font_error = None;
                }
                Err(error) => self.font_error = Some(error),
            }
        }
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

fn bundled_fonts() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("inter-regular", INTER_REGULAR),
        ("inter-semibold", INTER_SEMIBOLD),
        ("jetbrains-mono", JETBRAINS_MONO),
        ("phosphor", PHOSPHOR),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    let prop_fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mono_fallback = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let make = |first: &[&str], fallback: &[String]| {
        first
            .iter()
            .map(|name| (*name).to_owned())
            .chain(fallback.iter().cloned())
            .collect()
    };
    fonts.families.insert(
        FontFamily::Proportional,
        make(&["inter-regular", "phosphor"], &prop_fallback),
    );
    fonts.families.insert(
        FontFamily::Monospace,
        make(
            &["jetbrains-mono", "inter-regular", "phosphor"],
            &mono_fallback,
        ),
    );
    fonts.families.insert(
        semibold(),
        make(&["inter-semibold", "phosphor"], &prop_fallback),
    );
    fonts.families.insert(
        FontFamily::Name("chart".into()),
        make(&["inter-regular", "phosphor"], &prop_fallback),
    );
    fonts.families.insert(
        FontFamily::Name("chart-semibold".into()),
        make(&["inter-semibold", "phosphor"], &prop_fallback),
    );
    fonts.families.insert(
        icon_family(),
        make(&["phosphor", "inter-regular"], &prop_fallback),
    );
    fonts
}

fn style(dark: bool) -> egui::Style {
    let tokens = Tokens::for_dark(dark);
    let mut style = if dark {
        egui::Theme::Dark.default_style()
    } else {
        egui::Theme::Light.default_style()
    };
    for (text_style, size, family) in [
        (TextStyle::Small, 11.5, FontFamily::Proportional),
        (TextStyle::Body, 13.5, FontFamily::Proportional),
        (TextStyle::Button, 13.5, FontFamily::Proportional),
        (TextStyle::Heading, 16.0, semibold()),
        (TextStyle::Monospace, 12.5, FontFamily::Monospace),
    ] {
        style
            .text_styles
            .insert(text_style, FontId::new(size, family));
    }
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.combo_width = 160.0;
    style.spacing.slider_width = 180.0;

    let visuals = &mut style.visuals;
    visuals.panel_fill = tokens.panel_bg;
    visuals.window_fill = tokens.raised;
    visuals.extreme_bg_color = tokens.surface;
    visuals.faint_bg_color = tokens.surface;
    visuals.code_bg_color = tokens.surface;
    visuals.weak_text_color = Some(tokens.text_secondary);
    visuals.hyperlink_color = tokens.accent;
    visuals.warn_fg_color = tokens.warning;
    visuals.error_fg_color = tokens.error;
    visuals.window_stroke = Stroke::new(1.0, tokens.border);
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.window_shadow = egui::epaint::Shadow::NONE;
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 3],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 85 } else { 32 }),
    };
    visuals.selection.bg_fill = tokens.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, tokens.accent);
    visuals.widgets.noninteractive.bg_fill = tokens.panel_bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, tokens.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, tokens.text);
    visuals.widgets.inactive.bg_fill = tokens.surface;
    visuals.widgets.inactive.weak_bg_fill = tokens.surface;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, tokens.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, tokens.text);
    visuals.widgets.hovered.bg_fill = tokens.raised;
    visuals.widgets.hovered.weak_bg_fill = tokens.raised;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, tokens.border_strong);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, tokens.text);
    visuals.widgets.active.bg_fill = tokens.raised;
    visuals.widgets.active.weak_bg_fill = tokens.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, tokens.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, tokens.accent);
    visuals.widgets.open = visuals.widgets.hovered;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
        widget.expansion = 0.0;
    }
    visuals.slider_trailing_fill = true;
    style
}
