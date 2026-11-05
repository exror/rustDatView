use std::{f64::consts::TAU, sync::Arc, time::Duration};

use dv_chart::{
    AxisFormat, ChartConfig, ChartState, ChartTheme, Dash, Draw, FontSet, Pane, PaneAction, Scale,
    Series, Tool,
};
use eframe::egui::{self, Color32, FontFamily, FontId, RichText};

const INTER_REGULAR: &[u8] = include_bytes!("../../datview/assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../datview/assets/fonts/Inter-SemiBold.ttf");
const JETBRAINS_MONO: &[u8] =
    include_bytes!("../../datview/assets/fonts/JetBrainsMono-Regular.ttf");

const SAMPLES: usize = 1_000_000;
const SAMPLE_PERIOD: f64 = 0.001;
// 2025-01-01 00:00:00 UTC. DateTime axes interpret epoch seconds as UTC-naive.
const START_TIME: f64 = 1_735_689_600.0;
const EXPORT_SIZE: [u32; 2] = [1600, 1000];

struct Signal {
    label: &'static str,
    unit: &'static str,
}

const SIGNALS: [Signal; 9] = [
    Signal {
        label: "CPU package",
        unit: "W",
    },
    Signal {
        label: "CPU cores",
        unit: "W",
    },
    Signal {
        label: "GPU board",
        unit: "W",
    },
    Signal {
        label: "Core 0 effective",
        unit: "MHz",
    },
    Signal {
        label: "Core 1 effective",
        unit: "MHz",
    },
    Signal {
        label: "Core 2 effective",
        unit: "MHz",
    },
    Signal {
        label: "GPU clock",
        unit: "MHz",
    },
    Signal {
        label: "CPU package",
        unit: "°C",
    },
    Signal {
        label: "GPU hotspot",
        unit: "°C",
    },
];

#[derive(Clone, Copy, Debug, PartialEq)]
struct PaneSettings {
    y_scale: Scale,
    include_zero: bool,
    removed: bool,
}

fn initial_pane_settings() -> [PaneSettings; 3] {
    [
        PaneSettings {
            y_scale: Scale::Linear,
            include_zero: true,
            removed: false,
        },
        PaneSettings {
            y_scale: Scale::Linear,
            include_zero: false,
            removed: false,
        },
        PaneSettings {
            y_scale: Scale::Linear,
            include_zero: false,
            removed: false,
        },
    ]
}

struct Samples {
    elapsed: Vec<f64>,
    datetime: Vec<f64>,
    values: [Vec<f64>; SIGNALS.len()],
}

impl Samples {
    fn generate(count: usize) -> Self {
        let mut elapsed = Vec::with_capacity(count);
        let mut datetime = Vec::with_capacity(count);
        let mut values: [Vec<f64>; SIGNALS.len()] =
            std::array::from_fn(|_| Vec::with_capacity(count));
        let mut noise_state = 0x91e1_0da5_u32;

        for i in 0..count {
            let t = i as f64 * SAMPLE_PERIOD;
            let phase = (t % 180.0) / 180.0;
            let load = smoothstep(0.035, 0.12, phase) * (1.0 - smoothstep(0.72, 0.85, phase));
            let slow = (TAU * t / 81.0).sin();
            let cycle = (TAU * t / 17.0).sin();
            let detail = (TAU * t / 3.7).sin();
            noise_state = noise_state
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            let noise = f64::from(noise_state >> 8) / f64::from(0x00ff_ffff_u32) * 2.0 - 1.0;

            let row = [
                42.0 + 124.0 * load + 5.0 * cycle + 1.2 * detail + noise * 0.8,
                22.0 + 86.0 * load + 3.4 * cycle + 1.6 * detail + noise * 0.6,
                17.0 + 32.0 * load + 3.2 * slow + 1.1 * detail + noise * 0.4,
                1050.0 + 3300.0 * load + 95.0 * cycle + 44.0 * detail + noise * 18.0,
                980.0 + 3100.0 * load + 83.0 * slow - 42.0 * detail + noise * 16.0,
                920.0 + 2870.0 * load - 88.0 * cycle + 36.0 * detail + noise * 15.0,
                620.0 + 1660.0 * load + 72.0 * slow + 28.0 * detail + noise * 11.0,
                40.0 + 29.0 * load + 1.8 * slow + 0.4 * detail + noise * 0.12,
                37.0 + 19.0 * load - 1.5 * slow + 0.3 * cycle + noise * 0.1,
            ];
            elapsed.push(t);
            datetime.push(START_TIME + t);
            for (column, value) in values.iter_mut().zip(row) {
                column.push(value);
            }
        }

        Self {
            elapsed,
            datetime,
            values,
        }
    }

    fn panes(
        &self,
        datetime: bool,
        theme: &ChartTheme,
        settings: &[PaneSettings; 3],
    ) -> Vec<Pane<'_>> {
        let x = if datetime {
            &self.datetime
        } else {
            &self.elapsed
        };
        // The same signal retains its palette slot; the theme supplies its legible variant.
        let series = |index: usize| Series {
            id: index as u64 + 1,
            label: SIGNALS[index].label,
            unit: SIGNALS[index].unit,
            x,
            y: &self.values[index],
            generation: if datetime { 2 } else { 1 },
            color: theme.series_palette[index],
            width: 1.6,
            dash: Dash::Solid,
            draw: Draw::Lines,
            marker_radius: 2.5,
            visible: true,
            overlay: false,
        };

        [
            (1, "POWER", "W", 0..3),
            (2, "CLOCK", "MHz", 3..7),
            (3, "TEMPERATURE", "°C", 7..9),
        ]
        .into_iter()
        .zip(settings)
        .filter_map(|((id, title, y_label, indices), settings)| {
            if settings.removed {
                return None;
            }
            Some(Pane {
                id,
                title,
                y_label,
                y_scale: settings.y_scale,
                y_format: AxisFormat::Number,
                y_flip: false,
                include_zero: settings.include_zero,
                series: indices.map(&series).collect(),
                weight: 1.0,
            })
        })
        .collect()
    }
}

fn smoothstep(start: f64, end: f64, value: f64) -> f64 {
    let t = ((value - start) / (end - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn chart_config(datetime: bool) -> ChartConfig<'static> {
    ChartConfig {
        x_label: if datetime {
            "Time · UTC"
        } else {
            "Elapsed time"
        },
        x_scale: Scale::Linear,
        x_format: if datetime {
            AxisFormat::DateTime
        } else {
            AxisFormat::Duration
        },
        x_flip: false,
        link_x: true,
        grid: true,
        legend: true,
        legend_values: true,
        crosshair: true,
        overview: true,
        tooltip: true,
        equal_aspect: false,
        gap_factor: Some(10.0),
        decimate: true,
        precision: 4,
        title: "",
        subtitle: "",
    }
}

// Rebase the view and selection while discarding history and caches in the old domain.
fn switch_x_domain(state: &mut ChartState, datetime: bool) {
    let offset = if datetime { START_TIME } else { -START_TIME };
    let shifted = |[start, end]: [f64; 2]| [start + offset, end + offset];
    let mut next = state.persistent_snapshot();
    next.x_view = next.x_view.map(shifted);
    next.selection = next.selection.map(shifted);
    for view in next.pane_x_views.values_mut() {
        *view = shifted(*view);
    }
    *state = next;
}

fn apply_pane_action(
    settings: &mut [PaneSettings; 3],
    state: &mut ChartState,
    action: PaneAction,
) -> bool {
    let id = match action {
        PaneAction::ToggleLog(id) | PaneAction::ToggleIncludeZero(id) | PaneAction::Remove(id) => {
            id
        }
        _ => return false,
    };
    let index = match id {
        1 => 0,
        2 => 1,
        3 => 2,
        _ => return false,
    };
    let pane = &mut settings[index];
    if pane.removed {
        return false;
    }
    match action {
        PaneAction::ToggleLog(_) => {
            pane.y_scale = if pane.y_scale == Scale::Linear {
                Scale::Log10
            } else {
                Scale::Linear
            };
            state.y_views.remove(&id);
        }
        PaneAction::ToggleIncludeZero(_) => pane.include_zero = !pane.include_zero,
        PaneAction::Remove(_) => {
            pane.removed = true;
            state.y_views.remove(&id);
            state.pane_x_views.remove(&id);
            state.pane_weights.remove(&id);
            state.highlighted = None;
            if state.maximized == Some(id) {
                state.maximized = None;
            }
        }
        _ => return false,
    }
    true
}

fn restore_panes(settings: &mut [PaneSettings; 3], state: &mut ChartState) {
    for pane in settings {
        pane.removed = false;
    }
    state.maximized = None;
}

#[derive(Clone, Copy)]
struct UiColors {
    background: Color32,
    panel: Color32,
    border: Color32,
    secondary: Color32,
    accent: Color32,
    success: Color32,
    error: Color32,
}

impl UiColors {
    fn for_theme(dark: bool) -> Self {
        if dark {
            Self {
                background: Color32::from_rgb(17, 19, 23),
                panel: Color32::from_rgb(21, 24, 29),
                border: Color32::from_rgb(40, 46, 54),
                secondary: Color32::from_rgb(166, 174, 186),
                accent: Color32::from_rgb(91, 140, 255),
                success: Color32::from_rgb(63, 182, 139),
                error: Color32::from_rgb(240, 97, 109),
            }
        } else {
            Self {
                background: Color32::from_rgb(245, 246, 248),
                panel: Color32::from_rgb(250, 251, 252),
                border: Color32::from_rgb(226, 229, 234),
                secondary: Color32::from_rgb(89, 98, 110),
                accent: Color32::from_rgb(47, 107, 255),
                success: Color32::from_rgb(31, 157, 108),
                error: Color32::from_rgb(217, 56, 72),
            }
        }
    }
}

fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

fn configure_fonts_and_style(ctx: &egui::Context) -> FontSet {
    let mut definitions = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("inter-regular", INTER_REGULAR),
        ("inter-semibold", INTER_SEMIBOLD),
        ("jetbrains-mono", JETBRAINS_MONO),
    ] {
        definitions.font_data.insert(
            name.to_owned(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    definitions
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "inter-regular".into());
    definitions
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains-mono".into());
    let regular_fonts = definitions
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    definitions
        .families
        .insert(FontFamily::Name("chart".into()), regular_fonts.clone());
    let mut semibold_fonts = regular_fonts;
    semibold_fonts.insert(0, "inter-semibold".into());
    definitions.families.insert(
        FontFamily::Name("chart-semibold".into()),
        semibold_fonts.clone(),
    );
    definitions.families.insert(semibold(), semibold_fonts);
    ctx.set_fonts(definitions);

    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        let colors = UiColors::for_theme(dark);
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        style.visuals.panel_fill = colors.background;
        style.visuals.selection.bg_fill = colors.accent.gamma_multiply(0.25);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, colors.accent);
        style.visuals.widgets.noninteractive.corner_radius = 6.into();
        style.visuals.widgets.inactive.corner_radius = 6.into();
        style.visuals.widgets.hovered.corner_radius = 6.into();
        style.visuals.widgets.active.corner_radius = 6.into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(13.5));
        style
            .text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.5));
        style
            .text_styles
            .insert(egui::TextStyle::Small, FontId::proportional(11.5));
        style
            .text_styles
            .insert(egui::TextStyle::Heading, FontId::new(16.0, semibold()));
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, FontId::monospace(12.5));
        ctx.set_style_of(theme, style);
    }
    ctx.set_theme(egui::Theme::Dark);

    FontSet {
        regular: Arc::from(INTER_REGULAR),
        semibold: Arc::from(INTER_SEMIBOLD),
        mono: Arc::from(JETBRAINS_MONO),
        family: "Inter".into(),
    }
}

fn export_figure(
    panes: &[Pane<'_>],
    state: &ChartState,
    config: &ChartConfig<'_>,
    theme: &ChartTheme,
    fonts: &FontSet,
) -> Result<(), String> {
    let svg = dv_chart::render_svg(
        panes,
        state,
        config,
        &[],
        theme,
        fonts,
        [EXPORT_SIZE[0] as f32, EXPORT_SIZE[1] as f32],
    );
    std::fs::write("demo.svg", &svg).map_err(|err| format!("Could not write demo.svg: {err}"))?;

    let mut options = resvg::usvg::Options {
        font_family: fonts.family.clone(),
        ..Default::default()
    };
    let database = options.fontdb_mut();
    database.load_font_data(fonts.regular.as_ref().to_vec());
    database.load_font_data(fonts.semibold.as_ref().to_vec());
    database.load_font_data(fonts.mono.as_ref().to_vec());
    database.set_sans_serif_family(fonts.family.clone());
    database.set_serif_family(fonts.family.clone());
    database.set_monospace_family("JetBrains Mono");
    let tree = resvg::usvg::Tree::from_str(&svg, &options)
        .map_err(|err| format!("Saved demo.svg, but could not parse it for PNG export: {err}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(EXPORT_SIZE[0], EXPORT_SIZE[1])
        .ok_or_else(|| "Saved demo.svg, but could not allocate the PNG image".to_owned())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png("demo.png")
        .map_err(|err| format!("Saved demo.svg, but could not write demo.png: {err}"))?;
    Ok(())
}

struct Notice {
    text: String,
    is_error: bool,
}

struct Demo {
    samples: Samples,
    state: ChartState,
    pane_settings: [PaneSettings; 3],
    fonts: FontSet,
    datetime: bool,
    dark: bool,
    notice: Option<Notice>,
}

impl Demo {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            samples: Samples::generate(SAMPLES),
            state: ChartState::default(),
            pane_settings: initial_pane_settings(),
            fonts: configure_fonts_and_style(&cc.egui_ctx),
            datetime: false,
            dark: true,
            notice: None,
        }
    }
}

impl eframe::App for Demo {
    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        ctx.request_repaint_after(Duration::from_secs(1));
        let mut colors = UiColors::for_theme(self.dark);
        let mut export_requested = false;
        let mut theme_changed = false;

        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(colors.panel)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .inner_margin(egui::Margin::symmetric(22, 14)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("SYNTHETIC TELEMETRY").size(10.5).color(colors.accent));
                        ui.label(RichText::new("Telemetry at 1 ms").font(FontId::new(21.0, semibold())));
                        ui.label(RichText::new("Synthetic signals · linked time axis").size(12.0).color(colors.secondary));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(
                            self.pane_settings.iter().any(|pane| !pane.removed),
                            egui::Button::new("Export SVG + PNG"),
                        ).on_hover_text("Save the current chart to demo.svg and demo.png in the working directory").clicked() {
                            export_requested = true;
                        }
                        if ui.button(if self.dark { "Light theme" } else { "Dark theme" }).clicked() {
                            self.dark = !self.dark;
                            theme_changed = true;
                        }
                    });
                });
            });

        if theme_changed {
            ctx.set_theme(if self.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            colors = UiColors::for_theme(self.dark);
            ctx.request_repaint();
        }

        egui::Panel::top("tools")
            .frame(
                egui::Frame::new()
                    .fill(colors.panel)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .inner_margin(egui::Margin::symmetric(22, 8)),
            )
            .show(root, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("TOOL").size(10.5).color(colors.secondary));
                    ui.selectable_value(&mut self.state.tool, Tool::Select, "Select")
                        .on_hover_text("Drag to select a time range");
                    ui.selectable_value(&mut self.state.tool, Tool::Pan, "Pan")
                        .on_hover_text("Drag to pan the linked time axes and the active pane");
                    ui.selectable_value(&mut self.state.tool, Tool::Zoom, "Zoom")
                        .on_hover_text("Drag a box to zoom");
                    ui.separator();
                    if ui.button("Fit all").on_hover_text("Show the full time range and auto-fit each pane").clicked() {
                        self.state.fit_all();
                    }
                    if ui.add_enabled(!self.state.y_views.is_empty(), egui::Button::new("Auto Y"))
                        .on_hover_text("Auto-fit each pane without changing the time range")
                        .clicked()
                    {
                        self.state.y_views.clear();
                    }
                    if ui.add_enabled(self.state.selection.is_some(), egui::Button::new("Clear selection")).clicked() {
                        self.state.selection = None;
                    }
                    if self.pane_settings.iter().any(|pane| pane.removed)
                        && ui.button("Restore panes").on_hover_text("Show all removed panes").clicked()
                    {
                        restore_panes(&mut self.pane_settings, &mut self.state);
                    }
                    ui.separator();
                    if ui.checkbox(&mut self.datetime, "Date & time on X")
                        .on_hover_text("Switch between elapsed time and UTC, keeping the current view and selection")
                        .changed()
                    {
                        switch_x_domain(&mut self.state, self.datetime);
                    }
                });
            });

        let theme = if self.dark {
            ChartTheme::dark()
        } else {
            ChartTheme::light()
        };
        let config = chart_config(self.datetime);
        let panes = self
            .samples
            .panes(self.datetime, &theme, &self.pane_settings);
        if export_requested {
            self.notice = Some(
                match export_figure(&panes, &self.state, &config, &theme, &self.fonts) {
                    Ok(()) => {
                        let directory = std::env::current_dir()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|_| ".".to_owned());
                        Notice {
                            text: format!("Saved demo.svg and demo.png to {directory}"),
                            is_error: false,
                        }
                    }
                    Err(text) => Notice {
                        text,
                        is_error: true,
                    },
                },
            );
        }

        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(colors.panel)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .inner_margin(egui::Margin::symmetric(22, 8)),
            )
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    if let Some(notice) = &self.notice {
                        ui.label(RichText::new(&notice.text).size(11.5).color(if notice.is_error { colors.error } else { colors.success }));
                    } else {
                        ui.label(RichText::new("Scroll to zoom · Double-click to fit · Click a legend to hide a signal").size(11.5).color(colors.secondary));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let timing = frame.info().cpu_usage
                            .map(|seconds| format!("Frame {:.1} ms", seconds * 1000.0))
                            .unwrap_or_else(|| "Frame —".to_owned());
                        ui.label(RichText::new(timing).size(11.5).color(colors.secondary))
                            .on_hover_text("Previous frame CPU time, including rendering but excluding vertical-sync wait");
                        ui.separator();
                        ui.label(RichText::new("1,000,000 points per signal · 1 ms").size(11.5).color(colors.secondary));
                    });
                });
            });

        let mut pane_action = None;
        let mut restore_requested = false;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(colors.background)
                    .inner_margin(egui::Margin::symmetric(18, 12)),
            )
            .show(root, |ui| {
                if panes.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);
                        ui.heading("No panes to display");
                        ui.label("Restore the panes to explore and export the signals.");
                        restore_requested = ui.button("Restore panes").clicked();
                    });
                } else {
                    pane_action = dv_chart::show(ui, &mut self.state, &panes, &config, &[], &theme)
                        .pane_action;
                }
            });
        if restore_requested {
            restore_panes(&mut self.pane_settings, &mut self.state);
            ctx.request_repaint();
        } else if let Some(action) = pane_action {
            if apply_pane_action(&mut self.pane_settings, &mut self.state, action) {
                ctx.request_repaint();
            }
        }
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1320.0, 900.0])
            .with_min_inner_size([880.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "rustDatView · Chart demo",
        options,
        Box::new(|cc| Ok(Box::new(Demo::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_deterministic_synchronized_and_spaced_at_one_millisecond() {
        let first = Samples::generate(128);
        let second = Samples::generate(128);
        assert_eq!(first.elapsed, second.elapsed);
        assert_eq!(first.datetime, second.datetime);
        assert_eq!(first.values, second.values);
        assert_eq!(first.elapsed.len(), 128);
        for (index, (&elapsed, &datetime)) in first.elapsed.iter().zip(&first.datetime).enumerate()
        {
            assert_eq!(elapsed, index as f64 * SAMPLE_PERIOD);
            assert_eq!(datetime, START_TIME + elapsed);
        }
        for column in &first.values {
            assert_eq!(column.len(), first.elapsed.len());
            assert!(column.iter().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn generated_workload_has_visible_power_clock_and_temperature_changes() {
        let samples = Samples::generate(30_001);
        assert!(samples.values[0][30_000] > samples.values[0][5_000] + 100.0);
        assert!(samples.values[3][30_000] > samples.values[3][5_000] + 2_000.0);
        assert!(samples.values[7][30_000] > samples.values[7][5_000] + 20.0);
    }

    #[test]
    fn changing_time_domain_preserves_the_view_selection_and_signal_settings() {
        let mut state = ChartState::default();
        state.tool = Tool::Zoom;
        state.x_view = Some([12.5, 32.5]);
        state.pane_x_views.insert(1, [14.0, 20.0]);
        state.y_views.insert(1, [1.0, 50.0]);
        state.pane_weights.insert(1, 1.5);
        state.maximized = Some(1);
        state.selection = Some([16.0, 18.0]);
        state.hidden.insert(3);

        switch_x_domain(&mut state, true);
        assert_eq!(state.x_view, Some([START_TIME + 12.5, START_TIME + 32.5]));
        assert_eq!(
            state.selection,
            Some([START_TIME + 16.0, START_TIME + 18.0])
        );
        assert_eq!(
            state.pane_x_views[&1],
            [START_TIME + 14.0, START_TIME + 20.0]
        );
        assert_eq!(state.y_views[&1], [1.0, 50.0]);
        assert_eq!(state.pane_weights[&1], 1.5);
        assert_eq!(state.maximized, Some(1));
        assert!(state.hidden.contains(&3));
        assert_eq!(state.tool, Tool::Zoom);

        switch_x_domain(&mut state, false);
        assert_eq!(state.x_view, Some([12.5, 32.5]));
        assert_eq!(state.selection, Some([16.0, 18.0]));
        assert_eq!(state.pane_x_views[&1], [14.0, 20.0]);
    }

    #[test]
    fn pane_actions_update_the_rendered_panes_and_clean_up_removed_views() {
        let mut settings = initial_pane_settings();
        let mut state = ChartState::default();
        state.y_views.insert(1, [2.0, 30.0]);
        state.y_views.insert(2, [500.0, 4000.0]);
        state.pane_x_views.insert(2, [1.0, 3.0]);
        state.pane_weights.insert(2, 2.0);
        state.maximized = Some(2);

        assert!(apply_pane_action(
            &mut settings,
            &mut state,
            PaneAction::ToggleLog(1)
        ));
        assert_eq!(settings[0].y_scale, Scale::Log10);
        assert!(!state.y_views.contains_key(&1));
        assert!(state.y_views.contains_key(&2));
        assert!(apply_pane_action(
            &mut settings,
            &mut state,
            PaneAction::ToggleIncludeZero(1)
        ));
        assert!(!settings[0].include_zero);
        assert!(apply_pane_action(
            &mut settings,
            &mut state,
            PaneAction::Remove(2)
        ));
        assert!(!state.y_views.contains_key(&2));
        assert!(!state.pane_x_views.contains_key(&2));
        assert!(!state.pane_weights.contains_key(&2));
        assert_eq!(state.maximized, None);

        let samples = Samples::generate(16);
        let panes = samples.panes(false, &ChartTheme::dark(), &settings);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].id, 1);
        assert_eq!(panes[0].y_scale, Scale::Log10);
        assert!(!panes[0].include_zero);
        assert_eq!(panes[1].id, 3);
        assert!(!apply_pane_action(
            &mut settings,
            &mut state,
            PaneAction::ToggleLog(2)
        ));
        assert!(!apply_pane_action(
            &mut settings,
            &mut state,
            PaneAction::Remove(4)
        ));

        restore_panes(&mut settings, &mut state);
        let panes = samples.panes(false, &ChartTheme::dark(), &settings);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].y_scale, Scale::Log10);
        assert!(!panes[0].include_zero);
    }
}
