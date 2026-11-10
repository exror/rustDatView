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

/// Every settings section shares one label column, so controls line up.
fn grid<R>(ui: &mut egui::Ui, id: &str, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(egui::vec2(20.0, 8.0))
        .min_col_width(118.0)
        .show(ui, contents)
        .inner
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
            // The buttons stay visible below the scrolling sections.
            egui::ScrollArea::vertical()
                .id_salt("settings-scroll")
                .max_height((ui.available_height() - 44.0).max(120.0))
                .show(ui, |ui| {
                    let section = |ui: &mut egui::Ui, title: &str| {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new(title).strong().size(14.0));
                        ui.add_space(4.0);
                    };
                    section(ui, "Appearance");
                    grid(ui, "settings-appearance", |ui| {
                        ui.label("Theme");
                        if let Some(theme) = crate::ui::widgets::segmented(
                            ui,
                            settings.theme,
                            &[
                                (ThemeChoice::System, "System", "Follow the Windows app mode"),
                                (ThemeChoice::Dark, "Dark", ""),
                                (ThemeChoice::Light, "Light", ""),
                            ],
                        ) {
                            settings.theme = theme;
                        }
                        ui.end_row();
                        ui.label("Interface scale");
                        let mut percent = settings.scale * 100.0;
                        if ui.add(egui::Slider::new(&mut percent, 80.0..=200.0).step_by(5.0).suffix(" %")).changed() {
                            settings.scale = percent / 100.0;
                        }
                        ui.end_row();
                        ui.label("Font");
                        ui.vertical(|ui| {
                            appearance.font_picker(ui, &mut settings.font_family);
                            if let Some(error) = &appearance.font_error {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                        });
                        ui.end_row();
                    });

                    section(ui, "Charts");
                    grid(ui, "settings-charts", |ui| {
                        ui.label("Series colors");
                        egui::ComboBox::from_id_salt("series-palette")
                            .selected_text(match settings.palette {
                                SeriesPalette::Standard => "Standard",
                                SeriesPalette::Vivid => "Vivid",
                                SeriesPalette::ColorVisionFriendly => "Color-vision friendly",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut settings.palette, SeriesPalette::Standard, "Standard");
                                ui.selectable_value(&mut settings.palette, SeriesPalette::Vivid, "Vivid");
                                ui.selectable_value(&mut settings.palette, SeriesPalette::ColorVisionFriendly, "Color-vision friendly");
                            });
                        ui.end_row();
                        ui.label("Line width");
                        ui.add(egui::Slider::new(&mut settings.line_width, 0.5..=6.0).step_by(0.1).suffix(" px"));
                        ui.end_row();
                        ui.label("Significant digits");
                        ui.add(egui::Slider::new(&mut settings.precision, 1..=12))
                            .on_hover_text("Digits shown in readouts, legends and statistics");
                        ui.end_row();
                        ui.label("Gaps");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut settings.break_gaps, "Break lines after")
                                .on_hover_text("Do not draw a line across missing or widely spaced samples");
                            ui.add_enabled(
                                settings.break_gaps,
                                egui::DragValue::new(&mut settings.gap_factor).range(1.0..=1000.0).speed(0.2).suffix("×"),
                            )
                            .on_hover_text("Multiple of the median sample interval");
                            ui.label("the usual interval");
                        });
                        ui.end_row();
                        ui.label("Show");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut settings.show_sparklines, "Channel sparklines")
                                .on_hover_text("A small preview beside each channel in the sidebar");
                            ui.checkbox(&mut settings.overview, "Overview strip")
                                .on_hover_text("A full-range navigator below time series with at least 2,000 samples");
                        });
                        ui.end_row();
                    });

                    section(ui, "Data and startup");
                    grid(ui, "settings-data", |ui| {
                        ui.label("Dates");
                        ui.checkbox(&mut settings.dayfirst, "Day first when ambiguous")
                            .on_hover_text("Read 03/04 as 3 April rather than March 4. Applies to newly opened or reloaded files.");
                        ui.end_row();
                        ui.label("Startup");
                        ui.vertical(|ui| {
                            ui.checkbox(&mut settings.watch_default, "Follow files")
                                .on_hover_text("Watch opened files for new data and keep the latest samples in view");
                            ui.checkbox(&mut settings.restore_last_session, "Restore the last session")
                                .on_hover_text("Unless files are passed on the command line");
                        });
                        ui.end_row();
                        ui.label("Recent items");
                        ui.horizontal(|ui| {
                            let files = settings.recent_files.len();
                            let sessions = settings.recent_sessions.len();
                            ui.label(format!(
                                "{files} {} · {sessions} {}",
                                if files == 1 { "file" } else { "files" },
                                if sessions == 1 { "session" } else { "sessions" }
                            ));
                            if ui.add_enabled(files + sessions > 0, egui::Button::new("Clear")).clicked() {
                                settings.recent_files.clear();
                                settings.recent_sessions.clear();
                            }
                        });
                        ui.end_row();
                    });
                    ui.add_space(6.0);
                });
            ui.separator();
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui
                    .button("Reset to defaults")
                    .on_hover_text("Reset preferences; keep recent items")
                    .clicked()
                {
                    let files = std::mem::take(&mut settings.recent_files);
                    let sessions = std::mem::take(&mut settings.recent_sessions);
                    *settings = Settings::default();
                    settings.recent_files = files;
                    settings.recent_sessions = sessions;
                    appearance.font_error = None;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Done").clicked() {
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
