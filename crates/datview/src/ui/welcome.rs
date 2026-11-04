use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use eframe::egui::{self, RichText};

use crate::settings::Settings;

const POWER_SAMPLE: &[u8] = include_bytes!("../../../../examples/motherboard-power.csv");
const SIGNAL_SAMPLE: &[u8] = include_bytes!("../../../../examples/signals.csv");
const LOGO_LIGHT: &[u8] = include_bytes!("../../../../assets/brand/rustdatview-logo.svg");
const LOGO_DARK: &[u8] = include_bytes!("../../../../assets/brand/rustdatview-logo-dark.svg");

/// Actions are returned to the application, which owns the file and session flows.
pub(crate) enum WelcomeRequest {
    OpenFile,
    OpenRecent(PathBuf),
    OpenSession(PathBuf),
    OpenSample(PathBuf),
    SampleError(String),
}

#[derive(Default)]
pub(crate) struct WelcomeView {
    files: Vec<RecentItem>,
    sessions: Vec<RecentItem>,
    logos: HashMap<(bool, u32), egui::TextureHandle>,
}

struct RecentItem {
    path: PathBuf,
    display_path: String,
    name: String,
    detail: String,
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
        settings: &Settings,
    ) -> Option<WelcomeRequest> {
        refresh_recent(&mut self.files, &settings.recent_files);
        refresh_recent(&mut self.sessions, &settings.recent_sessions);
        let mut request = None;

        egui::ScrollArea::vertical().id_salt("welcome-scroll").show(ui, |ui| {
            let available = ui.available_width();
            let width = available.min(620.0);
            ui.add_space((ui.available_height() * 0.08).clamp(12.0, 56.0));
            ui.horizontal(|ui| {
                ui.add_space(((available - width) * 0.5).max(0.0));
                ui.vertical(|ui| {
                    ui.set_width(width);
                    ui.vertical_centered(|ui| {
                        self.show_logo(ui, width.min(310.0));
                        ui.add_space(12.0);
                        ui.heading(RichText::new("Open a log. See the story.").size(25.0).strong());
                        ui.add_space(8.0);
                        ui.label(RichText::new("Explore signals, compare runs, and find the numbers that matter.").weak());
                    });
                    ui.add_space(24.0);

                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                        .corner_radius(10)
                        .inner_margin(24)
                        .show(ui, |ui| {
                            ui.set_min_width((width - 48.0).max(0.0));
                            ui.vertical_centered(|ui| {
                                ui.heading("Drop files anywhere to get started");
                                ui.add_space(6.0);
                                ui.label(RichText::new("Or choose a file from your computer.").weak());
                                ui.add_space(14.0);
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("Open file…").clicked() {
                                        request = Some(WelcomeRequest::OpenFile);
                                    }
                                    ui.menu_button("Open sample data", |ui| {
                                        if ui.button("Motherboard power and temperature").clicked() {
                                            request = Some(open_sample("motherboard-power.csv", POWER_SAMPLE));
                                            ui.close();
                                        }
                                        if ui.button("Signals and reference").clicked() {
                                            request = Some(open_sample("signals.csv", SIGNAL_SAMPLE));
                                            ui.close();
                                        }
                                    });
                                });
                            });
                        });

                    if !self.files.is_empty() {
                        ui.add_space(22.0);
                        ui.strong("Recent files");
                        for item in &self.files {
                            if recent_button(ui, item) {
                                request = Some(WelcomeRequest::OpenRecent(item.path.clone()));
                            }
                        }
                    }
                    if !self.sessions.is_empty() {
                        ui.add_space(18.0);
                        ui.strong("Recent sessions");
                        for item in &self.sessions {
                            if recent_button(ui, item) {
                                request = Some(WelcomeRequest::OpenSession(item.path.clone()));
                            }
                        }
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

fn open_sample(name: &str, bytes: &[u8]) -> WelcomeRequest {
    let directory = std::env::temp_dir().join("rustDatView-samples");
    let path = directory.join(name);
    match std::fs::create_dir_all(&directory).and_then(|()| std::fs::write(&path, bytes)) {
        Ok(()) => WelcomeRequest::OpenSample(path),
        Err(error) => {
            WelcomeRequest::SampleError(format!("Could not prepare sample data: {error}"))
        }
    }
}

fn refresh_recent(items: &mut Vec<RecentItem>, paths: &[PathBuf]) {
    if items.len() == paths.len()
        && items
            .iter()
            .zip(paths)
            .all(|(item, path)| item.path == *path)
    {
        return;
    }
    *items = paths
        .iter()
        .map(|path| RecentItem {
            path: path.clone(),
            display_path: path.display().to_string(),
            name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Untitled")
                .to_owned(),
            detail: file_detail(path),
        })
        .collect();
}

fn file_detail(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let mut detail = format_bytes(metadata.len());
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = modified.duration_since(UNIX_EPOCH) {
                    detail.push_str(" · ");
                    detail.push_str(&dv_core::dataframe::format_datetime(elapsed.as_secs_f64()));
                }
            }
            detail
        }
        Err(_) => "File unavailable".to_owned(),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn recent_button(ui: &mut egui::Ui, item: &RecentItem) -> bool {
    ui.horizontal(|ui| {
        let clicked = ui
            .vertical(|ui| {
                let clicked = ui
                    .link(&item.name)
                    .on_hover_text(&item.display_path)
                    .clicked();
                ui.label(RichText::new(&item.display_path).small().weak());
                clicked
            })
            .inner;
        ui.label(RichText::new(&item.detail).small().weak());
        clicked
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
