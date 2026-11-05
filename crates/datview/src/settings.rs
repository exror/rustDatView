//! User preferences. Session/view state is stored separately.
use std::path::PathBuf;

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::ui::theme::Appearance;

const STORAGE_KEY: &str = "rustdatview.settings.v1";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SeriesPalette {
    #[default]
    Standard,
    Vivid,
    ColorVisionFriendly,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub theme: ThemeChoice,
    /// Zoom relative to the operating system's display scale.
    pub scale: f32,
    /// An empty family selects bundled Inter; nonempty names refer to installed fonts.
    pub font_family: String,
    pub line_width: f32,
    pub palette: SeriesPalette,
    pub show_sparklines: bool,
    pub overview: bool,
    pub break_gaps: bool,
    pub gap_factor: f64,
    pub precision: usize,
    pub dayfirst: bool,
    pub watch_default: bool,
    pub restore_last_session: bool,
    pub recent_files: Vec<PathBuf>,
    pub recent_sessions: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            scale: 1.0,
            font_family: String::new(),
            line_width: 1.6,
            palette: SeriesPalette::Standard,
            show_sparklines: true,
            overview: true,
            break_gaps: true,
            gap_factor: 10.0,
            precision: 4,
            dayfirst: false,
            watch_default: false,
            restore_last_session: false,
            recent_files: Vec::new(),
            recent_sessions: Vec::new(),
        }
    }
}

impl Settings {
    /// Only the new preferences key is read; obsolete preset/draft keys are ignored.
    pub(crate) fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let mut settings: Self = storage
            .and_then(|storage| storage.get_string(STORAGE_KEY))
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        settings.sanitize();
        settings
    }

    pub(crate) fn save(&self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = serde_json::to_string(self) {
            storage.set_string(STORAGE_KEY, json);
        }
    }

    fn sanitize(&mut self) {
        self.scale = if self.scale.is_finite() {
            self.scale.clamp(0.8, 2.0)
        } else {
            1.0
        };
        self.line_width = if self.line_width.is_finite() {
            self.line_width.clamp(0.5, 6.0)
        } else {
            1.6
        };
        self.gap_factor = if self.gap_factor.is_finite() {
            self.gap_factor.clamp(1.0, 1000.0)
        } else {
            10.0
        };
        self.precision = self.precision.clamp(1, 12);
    }
}

/// Preferences take effect immediately. The caller persists them through `save`
/// and applies non-appearance preferences to the workspace/chart.
pub(crate) fn show(
    ctx: &egui::Context,
    settings: &mut Settings,
    appearance: &mut Appearance,
    open: &mut bool,
) {
    if !*open {
        return;
    }
    let mut close = false;
    egui::Window::new("Settings")
        .open(open)
        .anchor(egui::Align2::RIGHT_TOP, [-24.0, 72.0])
        .default_width(480.0)
        .default_height(700.0)
        .min_width(390.0)
        .collapsible(false)
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .show(ui, |ui| {
                    ui.heading("Appearance");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Theme");
                        ui.selectable_value(&mut settings.theme, ThemeChoice::System, "System");
                        ui.selectable_value(&mut settings.theme, ThemeChoice::Dark, "Dark");
                        ui.selectable_value(&mut settings.theme, ThemeChoice::Light, "Light");
                    });
                    let mut percent = settings.scale * 100.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut percent, 80.0..=200.0)
                                .step_by(5.0)
                                .suffix("%")
                                .text("Interface scale"),
                        )
                        .changed()
                    {
                        settings.scale = percent / 100.0;
                    }
                    ui.horizontal(|ui| {
                        ui.label("Interface font");
                        appearance.font_picker(ui, &mut settings.font_family);
                    });
                    if let Some(error) = &appearance.font_error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                    ui.weak("Bundled fonts remain available as fallbacks.");

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.heading("Chart defaults");
                    ui.add_space(4.0);
                    ui.add(
                        egui::Slider::new(&mut settings.line_width, 0.5..=6.0)
                            .step_by(0.1)
                            .suffix(" pt")
                            .text("Line width"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Series palette");
                        egui::ComboBox::from_id_salt("series-palette")
                            .selected_text(match settings.palette {
                                SeriesPalette::Standard => "Standard",
                                SeriesPalette::Vivid => "Vivid",
                                SeriesPalette::ColorVisionFriendly => "Color-vision friendly",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut settings.palette,
                                    SeriesPalette::Standard,
                                    "Standard",
                                );
                                ui.selectable_value(
                                    &mut settings.palette,
                                    SeriesPalette::Vivid,
                                    "Vivid",
                                );
                                ui.selectable_value(
                                    &mut settings.palette,
                                    SeriesPalette::ColorVisionFriendly,
                                    "Color-vision friendly",
                                );
                            });
                    });
                    ui.weak("Choose distinct series colors for charts and channel markers.");
                    ui.checkbox(&mut settings.show_sparklines, "Show channel sparklines")
                        .on_hover_text("Show a small preview beside each channel.");
                    ui.checkbox(&mut settings.overview, "Show overview strip")
                        .on_hover_text("Show a full-range navigator for time series with at least 2,000 samples.");
                    ui.checkbox(&mut settings.break_gaps, "Break lines at gaps")
                        .on_hover_text("Avoid drawing a line across missing or widely spaced samples.");
                    ui.add_enabled_ui(settings.break_gaps, |ui| {
                        ui.add(
                            egui::Slider::new(&mut settings.gap_factor, 1.0..=1000.0)
                                .logarithmic(true)
                                .suffix("×")
                                .text("Gap threshold"),
                        );
                        ui.weak("A gap exceeds this multiple of the median sample interval.");
                    });
                    ui.add(
                        egui::Slider::new(&mut settings.precision, 1..=12)
                            .text("Significant digits"),
                    );

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.heading("Data and startup");
                    ui.add_space(4.0);
                    ui.checkbox(&mut settings.dayfirst, "Parse ambiguous dates day first")
                        .on_hover_text("For example, read 03/04 as 3 April rather than March 4. Applies to newly opened or reloaded files.");
                    ui.checkbox(&mut settings.watch_default, "Follow files by default")
                        .on_hover_text("Enable Follow when the application starts.");
                    ui.checkbox(
                        &mut settings.restore_last_session,
                        "Restore the last session on startup",
                    )
                    .on_hover_text("Open the most recently saved or opened session on startup, unless files were passed on the command line.");
                    ui.weak("Without automatic restore, sessions are opened explicitly.");

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.heading("Recent items");
                    ui.horizontal(|ui| {
                        let files = settings.recent_files.len();
                        let sessions = settings.recent_sessions.len();
                        ui.label(format!(
                            "{files} {} · {sessions} {}",
                            if files == 1 { "file" } else { "files" },
                            if sessions == 1 { "session" } else { "sessions" }
                        ));
                        if ui
                            .add_enabled(
                                files + sessions > 0,
                                egui::Button::new("Clear recent items"),
                            )
                            .clicked()
                        {
                            settings.recent_files.clear();
                            settings.recent_sessions.clear();
                        }
                    });
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button("Reset to defaults")
                            .on_hover_text("Reset preferences; keep recent items.")
                            .clicked()
                        {
                            let files = std::mem::take(&mut settings.recent_files);
                            let sessions = std::mem::take(&mut settings.recent_sessions);
                            *settings = Settings::default();
                            settings.recent_files = files;
                            settings.recent_sessions = sessions;
                            appearance.font_error = None;
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
                });
        });
    if close {
        *open = false;
    }
    // Font load errors remain visible in the dialog and keep the previous usable face.
    let _ = appearance.apply(ctx, settings);
}
