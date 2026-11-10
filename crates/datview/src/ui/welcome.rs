use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, UNIX_EPOCH};

use eframe::egui::{self, RichText};

use super::{icons, theme, widgets};
use crate::settings::Settings;

const POWER_SAMPLE: &[u8] = include_bytes!("../../../../examples/motherboard-power.csv");
const SIGNAL_SAMPLE: &[u8] = include_bytes!("../../../../examples/signals.csv");
const GAMING_SAMPLE: &[u8] = include_bytes!("../../../../examples/cpu-gaming-session.csv");
const BENCHMARK_ECO: &[u8] = include_bytes!("../../../../examples/cpu-benchmark-eco.csv");
const BENCHMARK_PERFORMANCE: &[u8] =
    include_bytes!("../../../../examples/cpu-benchmark-performance.csv");
const LOGO_LIGHT: &[u8] = include_bytes!("../../../../assets/brand/rustdatview-logo.svg");
const LOGO_DARK: &[u8] = include_bytes!("../../../../assets/brand/rustdatview-logo-dark.svg");

/// Actions are returned to the application, which owns the file and session flows.
pub(crate) enum WelcomeRequest {
    OpenFile,
    OpenRecent(PathBuf),
    OpenSession(PathBuf),
    OpenSample(Vec<PathBuf>),
    SampleError(String),
}

#[derive(Default)]
pub(crate) struct WelcomeView {
    files: Vec<RecentItem>,
    sessions: Vec<RecentItem>,
    logos: HashMap<(bool, u32), egui::TextureHandle>,
    last_refresh: Option<Instant>,
}

struct RecentItem {
    path: PathBuf,
    display_path: String,
    name: String,
    detail: String,
    available: bool,
}

impl WelcomeView {
    /// Draw the appropriate logo at the current pixel density. The same
    /// texture cache is shared by the welcome screen and the About dialog.
    pub(crate) fn show_logo(&mut self, ui: &mut egui::Ui, width: f32) {
        let width = width.max(1.0);
        let dark = ui.visuals().dark_mode;
        let pixels = (width * ui.ctx().pixels_per_point()).round().max(1.0) as u32;
        let key = (dark, pixels);
        if !self.logos.contains_key(&key) {
            if self.logos.len() >= 8 {
                self.logos.clear();
            }
            if let Some(image) = rasterize_logo(if dark { LOGO_DARK } else { LOGO_LIGHT }, pixels) {
                let texture = ui.ctx().load_texture(
                    format!("rustdatview-logo-{dark}-{pixels}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.logos.insert(key, texture);
            }
        }
        if let Some(texture) = self.logos.get(&key) {
            ui.image((texture.id(), egui::vec2(width, width * 256.0 / 806.0)));
        } else {
            ui.heading("rustDatView");
        }
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        settings: &mut Settings,
    ) -> Option<WelcomeRequest> {
        let refresh = self
            .last_refresh
            .is_none_or(|last| last.elapsed() >= Duration::from_secs(5));
        if refresh {
            self.last_refresh = Some(Instant::now());
        }
        refresh_recent(&mut self.files, &settings.recent_files, refresh);
        refresh_recent(&mut self.sessions, &settings.recent_sessions, refresh);
        let mut request = None;

        egui::ScrollArea::vertical().id_salt("welcome-scroll").show(ui, |ui| {
            let available = ui.available_width();
            let width = available.min(680.0);
            ui.add_space((ui.available_height() * 0.09).clamp(16.0, 76.0));
            ui.horizontal(|ui| {
                ui.add_space(((available - width) * 0.5).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.vertical_centered(|ui| {
                        self.show_logo(ui, width.min(330.0));
                        ui.add_space(14.0);
                        ui.heading(RichText::new("Open a log. See the story.").size(25.0).family(theme::semibold()));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Explore signals, compare runs, and find the numbers that matter.").weak());
                    });
                    ui.add_space(24.0);

                    let tokens = theme::Tokens::for_dark(ui.visuals().dark_mode);
                    egui::Frame::new()
                        .fill(tokens.surface)
                        .stroke(egui::Stroke::new(1.0, tokens.border_strong))
                        .corner_radius(12)
                        .inner_margin(24)
                        .show(ui, |ui| {
                            ui.set_min_width((width - 48.0).max(0.0));
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(icons::UPLOAD).family(theme::icon_family()).size(27.0).color(tokens.accent));
                                ui.add_space(5.0);
                                ui.heading("Drop files anywhere to get started");
                                ui.add_space(6.0);
                                ui.label(RichText::new("Or choose a file from your computer.").color(tokens.text_secondary));
                                ui.add_space(16.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(((ui.available_width() - 264.0) * 0.5).max(0.0));
                                    if ui.add_sized([116.0, 32.0], egui::Button::new(RichText::new("Open file…").color(if ui.visuals().dark_mode { tokens.app_bg } else { egui::Color32::WHITE })).fill(tokens.accent).stroke(egui::Stroke::NONE)).clicked() {
                                        request = Some(WelcomeRequest::OpenFile);
                                    }
                                    ui.menu_button("Open sample data", |ui| {
                                        if ui.button("CPU gaming session").clicked() {
                                            request = Some(open_samples(&[("cpu-gaming-session.csv", GAMING_SAMPLE)]));
                                            ui.close();
                                        }
                                        if ui.button("Compare two benchmark runs").clicked() {
                                            request = Some(open_samples(&[
                                                ("cpu-benchmark-eco.csv", BENCHMARK_ECO),
                                                ("cpu-benchmark-performance.csv", BENCHMARK_PERFORMANCE),
                                            ]));
                                            ui.close();
                                        }
                                        if ui.button("Signals & spectrum").clicked() {
                                            request = Some(open_samples(&[("signals.csv", SIGNAL_SAMPLE)]));
                                            ui.close();
                                        }
                                        if ui.button("Motherboard power").clicked() {
                                            request = Some(open_samples(&[("motherboard-power.csv", POWER_SAMPLE)]));
                                            ui.close();
                                        }
                                    });
                                });
                            });
                        });

                    let mut remove_file = None;
                    if !self.files.is_empty() {
                        ui.add_space(22.0);
                        widgets::section_label(ui, "RECENT FILES");
                        for (index, item) in self.files.iter().enumerate() {
                            let (open, remove) = recent_button(ui, item);
                            if open {
                                request = Some(WelcomeRequest::OpenRecent(item.path.clone()));
                            }
                            if remove {
                                remove_file = Some(index);
                            }
                        }
                    }
                    if let Some(index) = remove_file {
                        settings.recent_files.remove(index);
                    }
                    let mut remove_session = None;
                    if !self.sessions.is_empty() {
                        ui.add_space(18.0);
                        widgets::section_label(ui, "RECENT SESSIONS");
                        for (index, item) in self.sessions.iter().enumerate() {
                            let (open, remove) = recent_button(ui, item);
                            if open {
                                request = Some(WelcomeRequest::OpenSession(item.path.clone()));
                            }
                            if remove {
                                remove_session = Some(index);
                            }
                        }
                    }
                    if let Some(index) = remove_session {
                        settings.recent_sessions.remove(index);
                    }
                    ui.add_space(20.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("Ctrl+O  Open file     Ctrl+K  Commands     ?  Shortcuts").small().weak());
                    });
                });
            });
        });
        request
    }
}

fn open_samples(samples: &[(&str, &[u8])]) -> WelcomeRequest {
    let directory = std::env::temp_dir().join("rustDatView-samples");
    let result = (|| -> std::io::Result<Vec<PathBuf>> {
        std::fs::create_dir_all(&directory)?;
        samples
            .iter()
            .map(|(name, bytes)| {
                let path = directory.join(name);
                std::fs::write(&path, bytes)?;
                Ok(path)
            })
            .collect()
    })();
    match result {
        Ok(paths) => WelcomeRequest::OpenSample(paths),
        Err(error) => {
            WelcomeRequest::SampleError(format!("Could not prepare sample data: {error}"))
        }
    }
}

fn refresh_recent(items: &mut Vec<RecentItem>, paths: &[PathBuf], force: bool) {
    if !force
        && items.len() == paths.len()
        && items
            .iter()
            .zip(paths)
            .all(|(item, path)| item.path == *path)
    {
        return;
    }
    *items = paths
        .iter()
        .map(|path| {
            let metadata = std::fs::metadata(path)
                .ok()
                .filter(|metadata| metadata.is_file());
            RecentItem {
                path: path.clone(),
                display_path: path.display().to_string(),
                name: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Untitled")
                    .to_owned(),
                detail: file_detail(metadata.as_ref()),
                available: metadata.is_some(),
            }
        })
        .collect();
}

fn file_detail(metadata: Option<&std::fs::Metadata>) -> String {
    match metadata {
        Some(metadata) => {
            let mut detail = format_bytes(metadata.len());
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = modified.duration_since(UNIX_EPOCH) {
                    let date = dv_core::dataframe::format_datetime(elapsed.as_secs_f64());
                    if let Some(date) = date.split_whitespace().next() {
                        detail.push_str(" · Modified ");
                        detail.push_str(date);
                    }
                }
            }
            detail
        }
        None => "File unavailable".to_owned(),
    }
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

fn recent_button(ui: &mut egui::Ui, item: &RecentItem) -> (bool, bool) {
    let tokens = theme::Tokens::for_dark(ui.visuals().dark_mode);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(icons::FILE_TEXT)
                .family(theme::icon_family())
                .size(18.0)
                .color(if item.available {
                    tokens.accent
                } else {
                    tokens.text_muted
                }),
        );
        let width = (ui.available_width() - 48.0).max(80.0);
        let clicked = ui
            .allocate_ui_with_layout(
                egui::vec2(width, 44.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let clicked = ui
                        .add_enabled(
                            item.available,
                            egui::Button::new(RichText::new(&item.name).strong().color(
                                if item.available {
                                    tokens.text
                                } else {
                                    tokens.text_muted
                                },
                            ))
                            .frame(false),
                        )
                        .on_hover_text(&item.display_path)
                        .clicked();
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}  ·  {}", item.detail, item.display_path))
                                .small()
                                .color(tokens.text_secondary),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&item.display_path);
                    clicked
                },
            )
            .inner;
        let remove = !item.available
            && widgets::icon_button(
                ui,
                icons::CLOSE,
                "Remove unavailable item from recent items",
            )
            .clicked();
        (clicked, remove)
    })
    .inner
}

fn rasterize_logo(svg: &[u8], width: u32) -> Option<egui::ColorImage> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let scale = width as f32 / tree.size().width();
    let height = (tree.size().height() * scale).round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(egui::ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}
