//! Figure output through the same chart layout and drawing engine as the UI.
use std::{
    path::Path,
    sync::{Arc, LazyLock},
};

use dv_chart::{ChartState, FontSet};
use eframe::egui::{self, Color32, ColorImage};
use serde::{Deserialize, Serialize};

use crate::{
    settings::Settings,
    ui::{
        chart_view::{self, ChartOptions},
        theme,
    },
    workspace::Workspace,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FigureFormat {
    #[default]
    Png,
    Svg,
    Pdf,
    Eps,
}

impl FigureFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Svg => "svg",
            Self::Pdf => "pdf",
            Self::Eps => "eps",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Svg => "SVG",
            Self::Pdf => "PDF",
            Self::Eps => "EPS",
        }
    }

    fn from_path(path: &Path) -> Result<Self, String> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if extension.eq_ignore_ascii_case("png") {
            Ok(Self::Png)
        } else if extension.eq_ignore_ascii_case("svg") {
            Ok(Self::Svg)
        } else if extension.eq_ignore_ascii_case("pdf") {
            Ok(Self::Pdf)
        } else if extension.eq_ignore_ascii_case("eps") {
            Ok(Self::Eps)
        } else {
            Err("Choose an SVG, PNG, PDF or EPS filename.".into())
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FigureTheme {
    #[default]
    Light,
    Dark,
    MatchApp,
}

impl FigureTheme {
    fn is_dark(self, app_dark: bool) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            Self::MatchApp => app_dark,
        }
    }
}

/// Portable figure preferences, kept independently of the interactive view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FigureOptions {
    pub format: FigureFormat,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub theme: FigureTheme,
    pub transparent: bool,
    pub include_legend: bool,
    pub include_title: bool,
    pub include_annotations: bool,
    pub include_selection: bool,
    pub title: String,
    pub subtitle: String,
    pub x_label: String,
    pub y_label: String,
}

impl Default for FigureOptions {
    fn default() -> Self {
        Self {
            format: FigureFormat::Png,
            width: 1600,
            height: 1000,
            scale: 1,
            theme: FigureTheme::Light,
            transparent: false,
            include_legend: true,
            include_title: true,
            include_annotations: true,
            include_selection: false,
            title: String::new(),
            subtitle: String::new(),
            x_label: String::new(),
            y_label: String::new(),
        }
    }
}

fn fonts() -> &'static FontSet {
    static FONTS: LazyLock<FontSet> = LazyLock::new(|| FontSet {
        regular: Arc::from(theme::INTER_REGULAR),
        semibold: Arc::from(theme::INTER_SEMIBOLD),
        mono: Arc::from(theme::JETBRAINS_MONO),
        family: "Inter".into(),
    });
    &FONTS
}
fn dimensions(figure: &FigureOptions) -> Result<[u32; 2], String> {
    let width = figure
        .width
        .checked_mul(figure.scale)
        .ok_or("Figure width is too large.")?;
    let height = figure
        .height
        .checked_mul(figure.scale)
        .ok_or("Figure height is too large.")?;
    if width == 0 || height == 0 {
        return Err("Figure dimensions and scale must be greater than zero.".into());
    }
    // This also bounds the memory used by the raster and EPS encoders.
    if width > 32_768 || height > 32_768 || u64::from(width) * u64::from(height) > 100_000_000 {
        return Err("Figure is too large. Reduce the dimensions or scale.".into());
    }
    Ok([width, height])
}

fn figure_svg(
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    figure: &FigureOptions,
    app_dark: bool,
    format: FigureFormat,
) -> Result<String, String> {
    let [width, height] = dimensions(figure)?;
    let mut chart = chart.clone();
    chart.legend = figure.include_legend;
    // A static figure has no hover, so legends need no room for live values.
    chart.legend_values = false;
    chart.crosshair = false;
    chart.tooltip = false;
    if figure.include_title {
        if !figure.title.is_empty() {
            chart.title.clone_from(&figure.title);
        }
        if !figure.subtitle.is_empty() {
            chart.subtitle.clone_from(&figure.subtitle);
        }
    } else {
        chart.title.clear();
        chart.subtitle.clear();
    }
    if !figure.x_label.is_empty() {
        chart.x_label.clone_from(&figure.x_label);
    }
    if !figure.y_label.is_empty() {
        chart.y_label.clone_from(&figure.y_label);
    }

    // Copy only persistent state, not the potentially large decimation caches.
    let mut view = state.persistent_snapshot();
    if !figure.include_selection {
        view.selection = None;
    }
    let dark = figure.theme.is_dark(app_dark);
    let svg = chart_view::with_chart(
        workspace,
        settings,
        &chart,
        dark,
        figure.include_annotations,
        |panes, config, marks, theme| {
            let mut theme = theme.clone();
            if figure.transparent && matches!(format, FigureFormat::Png | FigureFormat::Svg) {
                theme.surface = Color32::TRANSPARENT;
                theme.bg = Color32::TRANSPARENT;
            }
            dv_chart::render_svg(
                panes,
                &view,
                config,
                marks,
                &theme,
                fonts(),
                [figure.width as f32, figure.height as f32],
            )
        },
    );
    if figure.scale == 1 {
        Ok(svg)
    } else {
        // Scale the whole drawing, including text and strokes, not just its canvas.
        Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {} {}\">{svg}</svg>",
            figure.width, figure.height
        ))
    }
}

fn svg_tree(svg: &str) -> Result<resvg::usvg::Tree, String> {
    let mut options = resvg::usvg::Options::default();
    options.font_family = fonts().family.clone();
    static FONTDB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut database = resvg::usvg::fontdb::Database::new();
        database.load_font_data(fonts().regular.to_vec());
        database.load_font_data(fonts().semibold.to_vec());
        database.load_font_data(fonts().mono.to_vec());
        database.load_font_data(theme::PHOSPHOR.to_vec());
        // Keep bundled Inter first, but permit the same system fallback as the UI.
        database.load_system_fonts();
        Arc::new(database)
    });
    options.fontdb = Arc::clone(&FONTDB);
    resvg::usvg::Tree::from_str(svg, &options)
        .map_err(|error| format!("Could not parse the figure: {error}"))
}

fn pixmap(tree: &resvg::usvg::Tree, size: [u32; 2]) -> Result<resvg::tiny_skia::Pixmap, String> {
    let mut image = resvg::tiny_skia::Pixmap::new(size[0], size[1])
        .ok_or("Could not allocate the figure image.")?;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::identity(),
        &mut image.as_mut(),
    );
    Ok(image)
}

/// EPS has no transparency. Composite the premultiplied raster on white before
/// writing its Level 2 colorimage, instead of turning transparent areas black.
fn eps(image: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    let (width, height) = (image.width(), image.height());
    let header = format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 {width} {height}\n%%LanguageLevel: 2\n%%EndComments\ngsave\n{width} {height} scale\n{width} {height} 8 [{width} 0 0 -{height} 0 {height}]\n{{currentfile {} string readhexstring pop}} false 3 colorimage\n",
        (width * 3).min(65_535),
    );
    let mut output =
        Vec::with_capacity(header.len() + image.data().len() / 4 * 6 + height as usize + 32);
    output.extend_from_slice(header.as_bytes());
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for row in image.data().chunks_exact(width as usize * 4) {
        for pixel in row.chunks_exact(4) {
            let background = 255 - pixel[3];
            for &component in &pixel[..3] {
                let value = component.saturating_add(background);
                output.push(HEX[(value >> 4) as usize]);
                output.push(HEX[(value & 15) as usize]);
            }
        }
        output.push(b'\n');
    }
    output.extend_from_slice(b"grestore\nshowpage\n%%EOF\n");
    output
}

/// Render a figure in any supported file format. SVG text is converted to
/// outlines so the file looks identical without Inter installed; PDF embeds
/// the exact font faces loaded into the SVG tree.
pub(crate) fn render(
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    figure: &FigureOptions,
    app_dark: bool,
    format: FigureFormat,
) -> Result<Vec<u8>, String> {
    let svg = figure_svg(workspace, state, settings, chart, figure, app_dark, format)?;
    let tree = svg_tree(&svg)?;
    match format {
        FigureFormat::Svg => Ok(tree
            .to_string(&resvg::usvg::WriteOptions::default())
            .into_bytes()),
        FigureFormat::Pdf => svg2pdf::to_pdf(
            &tree,
            svg2pdf::ConversionOptions::default(),
            svg2pdf::PageOptions::default(),
        )
        .map_err(|error| format!("Could not create the PDF: {error}")),
        FigureFormat::Png => pixmap(&tree, dimensions(figure)?)?
            .encode_png()
            .map_err(|error| format!("Could not encode the PNG: {error}")),
        FigureFormat::Eps => Ok(eps(&pixmap(&tree, dimensions(figure)?)?)),
    }
}

pub(crate) fn raster(
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    figure: &FigureOptions,
    app_dark: bool,
) -> Result<ColorImage, String> {
    let size = dimensions(figure)?;
    let svg = figure_svg(
        workspace,
        state,
        settings,
        chart,
        figure,
        app_dark,
        figure.format,
    )?;
    let image = pixmap(&svg_tree(&svg)?, size)?;
    Ok(ColorImage::from_rgba_premultiplied(
        [size[0] as usize, size[1] as usize],
        image.data(),
    ))
}

/// Headless/default export; format is inferred from the filename.
pub(crate) fn save(
    path: &Path,
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
) -> Result<(), String> {
    save_with_options(
        path,
        workspace,
        state,
        settings,
        chart,
        &FigureOptions::default(),
        false,
    )
}

pub(crate) fn save_with_options(
    path: &Path,
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    figure: &FigureOptions,
    app_dark: bool,
) -> Result<(), String> {
    let format = FigureFormat::from_path(path)?;
    let bytes = render(workspace, state, settings, chart, figure, app_dark, format)?;
    std::fs::write(path, bytes)
        .map_err(|error| format!("Could not save {}: {error}", path.display()))
}

/// Queue the exact figure raster for the native clipboard.
pub(crate) fn copy(
    ctx: &egui::Context,
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    figure: &FigureOptions,
    app_dark: bool,
) -> Result<(), String> {
    ctx.copy_image(raster(workspace, state, settings, chart, figure, app_dark)?);
    Ok(())
}

#[derive(Clone, Default)]
struct Preview {
    key: String,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn preview(
    ctx: &egui::Context,
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    figure: &FigureOptions,
    app_dark: bool,
    area: egui::Vec2,
) -> Preview {
    let id = egui::Id::unique("figure_preview");
    let pixels_per_point = ctx.pixels_per_point();
    let area = [
        (area.x * pixels_per_point).max(64.0).round() as u32,
        (area.y * pixels_per_point).max(64.0).round() as u32,
    ];
    let key = match serde_json::to_string(&(
        workspace.generation,
        &workspace.panes,
        &workspace.annotations,
        &workspace.x_key,
        &workspace.mode,
        state,
        settings,
        chart,
        figure,
        app_dark,
        area,
    )) {
        Ok(key) => key,
        Err(error) => {
            return Preview {
                error: Some(format!("Could not prepare preview: {error}")),
                ..Preview::default()
            }
        }
    };
    if let Some(cached) = ctx.data(|data| data.get_temp::<Preview>(id)) {
        if cached.key == key {
            return cached;
        }
    }
    let result = (|| {
        let [width, height] = dimensions(figure)?;
        let factor = (f64::from(area[0]) / f64::from(width))
            .min(f64::from(area[1]) / f64::from(height))
            .min(1.0);
        let size = [
            (f64::from(width) * factor).round().max(1.0) as u32,
            (f64::from(height) * factor).round().max(1.0) as u32,
        ];
        let svg = figure_svg(
            workspace,
            state,
            settings,
            chart,
            figure,
            app_dark,
            figure.format,
        )?;
        let tree = svg_tree(&svg)?;
        let mut image = resvg::tiny_skia::Pixmap::new(size[0], size[1])
            .ok_or("Could not allocate the figure preview.")?;
        let transform = resvg::tiny_skia::Transform::from_scale(
            size[0] as f32 / width as f32,
            size[1] as f32 / height as f32,
        );
        resvg::render(&tree, transform, &mut image.as_mut());
        Ok::<_, String>(ColorImage::from_rgba_premultiplied(
            [size[0] as usize, size[1] as usize],
            image.data(),
        ))
    })();
    let cache = match result {
        Ok(image) => Preview {
            key,
            texture: Some(ctx.load_texture("figure preview", image, egui::TextureOptions::LINEAR)),
            error: None,
        },
        Err(error) => Preview {
            key,
            texture: None,
            error: Some(error),
        },
    };
    ctx.data_mut(|data| data.insert_temp(id, cache.clone()));
    cache
}

/// A Python script reproducing the current chart: files, processing steps,
/// formulas, plotted channels, mode and the main chart styling.
pub(crate) fn python_script(
    workspace: &Workspace,
    chart: &ChartOptions,
    mut options: dv_plot::ScriptOptions,
) -> Result<String, String> {
    use crate::workspace::Mode;
    use dv_core::common::clean_col;
    use dv_plot::{CompareMode, PlotType};
    // One selection per plotted series, so removed or hidden-by-source series
    // are not resurrected in the script.
    let mut selections = Vec::new();
    for series in &workspace.series {
        let Some(table) = workspace.tables.tabs.get(series.source) else {
            continue;
        };
        let columns = table.data.columns();
        let position = |key: &str| {
            columns
                .iter()
                .position(|column| clean_col(&column.name) == key)
        };
        if let (Some(x), Some(y)) = (
            workspace.x_key.as_deref().and_then(position),
            position(&series.key),
        ) {
            if !selections.contains(&(series.source, x, y)) {
                selections.push((series.source, x, y));
            }
        }
    }
    if selections.is_empty() {
        return Err("Plot at least one channel to export a script.".into());
    }
    options.grid = chart.grid;
    options.legend = chart.legend;
    options.log_x = chart.log_x;
    options.log_y = chart.log_y;
    options.flip_x = chart.flip_x;
    options.flip_y = chart.flip_y;
    options.title.clone_from(&chart.title);
    options.x_label.clone_from(&chart.x_label);
    options.y_label.clone_from(&chart.y_label);
    let plot_type = match workspace.mode {
        Mode::Signals | Mode::Compare => PlotType::Regular,
        Mode::Distribution => PlotType::Pdf(workspace.options.pdf.clone()),
        Mode::Spectrum => PlotType::Fft(workspace.options.fft.clone()),
        Mode::Normalized => PlotType::MinMax(workspace.options.normalized.clone()),
        Mode::Polar => PlotType::Polar(workspace.options.polar.clone()),
    };
    let active = workspace
        .sources
        .iter()
        .filter(|source| source.active)
        .count();
    let comparison = (workspace.mode == Mode::Compare).then(|| {
        let mode = workspace.options.compare_mode.unwrap_or(if active <= 1 {
            CompareMode::OneTableColumns
        } else if workspace.selected.len() == 1 {
            CompareMode::TablesOneColumn
        } else {
            CompareMode::TablesSimilarColumns
        });
        (mode, workspace.options.compare_method)
    });
    let script = dv_plot::PythonScripter {
        files: workspace.tables.unique_filenames(),
        selections,
        options,
        plot_type,
        comparison,
    };
    script
        .generate(&workspace.tables.tabs, &workspace.pipeline)
        .map_err(|error| error.to_string())
}

/// Returns a status or error for the app's toast/status area when an action is
/// completed. The caller owns `open` and the serializable `figure` preferences.
pub(crate) fn show_dialog(
    ctx: &egui::Context,
    open: &mut bool,
    figure: &mut FigureOptions,
    workspace: &Workspace,
    state: &ChartState,
    settings: &Settings,
    chart: &ChartOptions,
    app_dark: bool,
) -> Option<Result<String, String>> {
    use crate::ui::widgets;
    if !*open {
        return None;
    }
    let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
    let mut result = None;
    let mut close = false;
    let screen = ctx.content_rect();
    let size = egui::vec2(
        (screen.width() - 80.0).clamp(640.0, 1040.0),
        (screen.height() - 80.0).clamp(440.0, 640.0),
    );
    let modal = egui::Modal::new(egui::Id::unique("export_figure")).show(ctx, |ui| {
        ui.set_width(size.x);
        ui.set_height(size.y);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Export figure")
                    .family(theme::semibold())
                    .size(17.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, crate::ui::icons::CLOSE, "Close · Esc").clicked() {
                    close = true;
                }
            });
        });
        ui.add_space(6.0);
        let body_height = size.y - 86.0;
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(262.0, body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("figure-options")
                        .max_height(body_height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_width(250.0);
                            widgets::section_label(ui, "Format");
                            if let Some(format) = widgets::segmented(
                                ui,
                                figure.format,
                                &[
                                    (
                                        FigureFormat::Png,
                                        "PNG",
                                        "Raster image for chat, slides and documents",
                                    ),
                                    (
                                        FigureFormat::Svg,
                                        "SVG",
                                        "Vector graphics; text is converted to outlines",
                                    ),
                                    (
                                        FigureFormat::Pdf,
                                        "PDF",
                                        "Vector document with embedded fonts",
                                    ),
                                    (
                                        FigureFormat::Eps,
                                        "EPS",
                                        "PostScript with an embedded raster",
                                    ),
                                ],
                            ) {
                                figure.format = format;
                            }
                            ui.add_space(10.0);
                            widgets::section_label(ui, "Size");
                            let preset = match (figure.width, figure.height) {
                                (1920, 1080) => "Slide · 1920 × 1080",
                                (1600, 1000) => "Document · 1600 × 1000",
                                (1200, 1200) => "Square · 1200 × 1200",
                                (1200, 600) => "Wide · 1200 × 600",
                                _ => "Custom",
                            };
                            egui::ComboBox::from_id_salt("figure_size")
                                .width(250.0)
                                .selected_text(preset)
                                .show_ui(ui, |ui| {
                                    for (label, width, height) in [
                                        ("Slide · 1920 × 1080", 1920, 1080),
                                        ("Document · 1600 × 1000", 1600, 1000),
                                        ("Square · 1200 × 1200", 1200, 1200),
                                        ("Wide · 1200 × 600", 1200, 600),
                                    ] {
                                        if ui
                                            .selectable_label(
                                                (figure.width, figure.height) == (width, height),
                                                label,
                                            )
                                            .clicked()
                                        {
                                            figure.width = width;
                                            figure.height = height;
                                        }
                                    }
                                });
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::DragValue::new(&mut figure.width)
                                        .range(200..=16_384)
                                        .speed(10)
                                        .suffix(" px"),
                                );
                                ui.label("×");
                                ui.add(
                                    egui::DragValue::new(&mut figure.height)
                                        .range(150..=16_384)
                                        .speed(10)
                                        .suffix(" px"),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("Resolution").color(tokens.text_secondary),
                                );
                                if let Some(scale) = widgets::segmented(
                                    ui,
                                    figure.scale,
                                    &[
                                        (1, "1×", ""),
                                        (2, "2×", "Sharper on high-DPI screens"),
                                        (3, "3×", "Print quality"),
                                    ],
                                ) {
                                    figure.scale = scale;
                                }
                            });
                            ui.add_space(10.0);
                            widgets::section_label(ui, "Appearance");
                            if let Some(choice) = widgets::segmented(
                                ui,
                                figure.theme,
                                &[
                                    (FigureTheme::Light, "Light", "Best for documents and print"),
                                    (FigureTheme::Dark, "Dark", "Dark background"),
                                    (
                                        FigureTheme::MatchApp,
                                        "Match app",
                                        "Use the current app theme",
                                    ),
                                ],
                            ) {
                                figure.theme = choice;
                            }
                            ui.add_enabled(
                                matches!(figure.format, FigureFormat::Png | FigureFormat::Svg),
                                egui::Checkbox::new(
                                    &mut figure.transparent,
                                    "Transparent background",
                                ),
                            );
                            ui.add_space(10.0);
                            widgets::section_label(ui, "Include");
                            ui.checkbox(&mut figure.include_title, "Title");
                            ui.checkbox(&mut figure.include_legend, "Legend");
                            ui.checkbox(&mut figure.include_annotations, "Annotations");
                            ui.checkbox(&mut figure.include_selection, "Selected range");
                            ui.add_space(10.0);
                            widgets::section_label(ui, "Labels");
                            for (label, value, hint) in [
                                ("Title", &mut figure.title, "Chart title"),
                                (
                                    "Subtitle",
                                    &mut figure.subtitle,
                                    "e.g. BIOS 1.2 · 25 °C ambient",
                                ),
                                ("X axis", &mut figure.x_label, "Uses the chart label"),
                                ("Y axis", &mut figure.y_label, "Uses each pane's unit"),
                            ] {
                                ui.label(egui::RichText::new(label).color(tokens.text_secondary));
                                ui.add(
                                    egui::TextEdit::singleline(value)
                                        .hint_text(hint)
                                        .desired_width(246.0),
                                );
                            }
                        });
                },
            );
            ui.add_space(12.0);
            let preview_size = egui::vec2(ui.available_width(), body_height);
            egui::Frame::new()
                .fill(tokens.app_bg)
                .stroke(egui::Stroke::new(1.0, tokens.border))
                .corner_radius(8)
                .show(ui, |ui| {
                    ui.set_min_size(preview_size - egui::vec2(2.0, 2.0));
                    ui.centered_and_justified(|ui| {
                        let preview = preview(
                            ctx,
                            workspace,
                            state,
                            settings,
                            chart,
                            figure,
                            app_dark,
                            preview_size - egui::vec2(24.0, 24.0),
                        );
                        if let Some(texture) = preview.texture {
                            ui.image((texture.id(), texture.size_vec2() / ctx.pixels_per_point()));
                        } else if let Some(error) = preview.error {
                            ui.colored_label(tokens.error, error);
                        }
                    });
                });
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let pixels = dimensions(figure)
                .map(|[w, h]| format!("{w} × {h} px"))
                .unwrap_or_default();
            ui.label(egui::RichText::new(pixels).color(tokens.text_muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = egui::Button::new(egui::RichText::new("Save…").color(Color32::WHITE))
                    .fill(tokens.accent)
                    .min_size(egui::vec2(92.0, 30.0));
                if ui.add(save).clicked() {
                    let extension = figure.format.extension();
                    if let Some(mut path) = rfd::FileDialog::new()
                        .add_filter(figure.format.label(), &[extension])
                        .set_file_name(format!("figure.{extension}"))
                        .save_file()
                    {
                        if path.extension().is_none() {
                            path.set_extension(extension);
                        }
                        let saved = save_with_options(
                            &path, workspace, state, settings, chart, figure, app_dark,
                        );
                        if saved.is_ok() {
                            close = true;
                        }
                        result =
                            Some(saved.map(|()| format!("Saved figure to {}", path.display())));
                    }
                }
                if ui
                    .add(egui::Button::new("Copy to clipboard").min_size(egui::vec2(0.0, 30.0)))
                    .clicked()
                {
                    result = Some(
                        copy(ctx, workspace, state, settings, chart, figure, app_dark)
                            .map(|()| "Figure copied to the clipboard".into()),
                    );
                }
            });
        });
    });
    if close || modal.should_close() {
        *open = false;
    }
    result
}
