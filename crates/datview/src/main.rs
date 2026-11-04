mod actions;
mod app;
mod channels;
mod export;
mod loader;
mod panes;
mod selection;
mod series;
mod session;
mod settings;
mod ui;
mod workspace;

use std::path::PathBuf;
use workspace::Mode;

#[derive(Default)]
pub(crate) struct Launch {
    pub paths: Vec<PathBuf>,
    pub format: Option<String>,
    pub dayfirst: bool,
    pub export: Option<PathBuf>,
    pub mode: Option<Mode>,
    pub x: Option<usize>,
    pub ys: Vec<usize>,
    pub pipeline: Option<PathBuf>,
    pub plot_options: Option<serde_json::Value>,
    pub session: Option<PathBuf>,
    pub follow: bool,
}

fn arguments() -> Result<Option<Launch>, String> {
    let mut launch = Launch::default();
    let mut args = std::env::args().skip(1);
    let mut positional = false;
    while let Some(arg) = args.next() {
        if positional {
            launch.paths.push(arg.into());
            continue;
        }
        match arg.as_str() {
            "--" => positional = true,
            "-h" | "--help" => {
                let formats = dv_io::all_formats()
                    .iter()
                    .map(|f| f.name)
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("rustDatView {}\nUsage: datview [OPTIONS] [FILE ...]\n\nOptions:\n  -f, --format NAME     Reader format (default: auto)\n  --dayfirst            Interpret ambiguous dates as day/month/year\n  --mode MODE           Signals/Regular, Distribution/PDF, Spectrum/FFT,\n                        Normalized/MinMax, Compare, Polar\n  --x INDEX             Zero-based X column (including Index)\n  --y INDEX[,INDEX]     Y column indices\n  --pipeline PATH       Load a processing pipeline JSON\n  --plot-options JSON   Serialized dv-plot PlotType options\n  --session PATH        Open a .dvsession workspace\n  --follow              Watch input files for changes\n  --export PATH         Export SVG, PNG, PDF, EPS or Python without a window\n  -h, --help            Display help\n  -V, --version         Display version\n\nUse -- before filenames beginning with a dash.\nFormats available: {formats}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("rustDatView {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-f" | "--format" => {
                let value = args.next().ok_or("--format requires a format name")?;
                if dv_io::find_format(&value).is_none() {
                    return Err(format!("Unknown format: {value}"));
                }
                launch.format = Some(value);
            }
            "--dayfirst" => launch.dayfirst = true,
            "--follow" => launch.follow = true,
            "--session" => {
                launch.session = Some(args.next().ok_or("--session requires a path")?.into())
            }
            "--export" => {
                launch.export = Some(
                    args.next()
                        .ok_or("--export requires an output path")?
                        .into(),
                )
            }
            "--pipeline" => {
                launch.pipeline = Some(args.next().ok_or("--pipeline requires a JSON path")?.into())
            }
            "--mode" => {
                let value = args.next().ok_or("--mode requires a mode")?;
                launch.mode = Some(match value.to_ascii_lowercase().as_str() {
                    "regular" | "signals" => Mode::Signals,
                    "pdf" | "distribution" => Mode::Distribution,
                    "fft" | "spectrum" => Mode::Spectrum,
                    "minmax" | "normalized" | "normalised" => Mode::Normalized,
                    "compare" => Mode::Compare,
                    "polar" => Mode::Polar,
                    _ => return Err(format!("Unknown plot mode: {value}")),
                });
            }
            "--x" => {
                launch.x = Some(
                    args.next()
                        .ok_or("--x requires a column index")?
                        .parse()
                        .map_err(|_| "Invalid X column index")?,
                )
            }
            "--y" => {
                launch.ys = args
                    .next()
                    .ok_or("--y requires column indices")?
                    .split(',')
                    .map(|part| part.parse().map_err(|_| "Invalid Y column index"))
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--plot-options" => {
                launch.plot_options = Some(
                    serde_json::from_str(&args.next().ok_or("--plot-options requires JSON")?)
                        .map_err(|e| format!("Invalid plot options: {e}"))?,
                )
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option: {arg}")),
            _ => launch.paths.push(arg.into()),
        }
    }
    Ok(Some(launch))
}

fn icon() -> eframe::egui::IconData {
    const SIZE: u32 = 256;
    let svg = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/brand/rustdatview-mark.svg"
    ));
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default())
        .expect("bundled rustDatView icon must be a valid SVG");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE)
        .expect("bundled rustDatView icon must fit in memory");
    let scale = resvg::tiny_skia::Transform::from_scale(
        SIZE as f32 / tree.size().width(),
        SIZE as f32 / tree.size().height(),
    );
    resvg::render(&tree, scale, &mut pixmap.as_mut());
    let rgba = pixmap
        .data()
        .chunks_exact(4)
        .flat_map(|pixel| {
            eframe::egui::Color32::from_rgba_premultiplied(pixel[0], pixel[1], pixel[2], pixel[3])
                .to_srgba_unmultiplied()
        })
        .collect();
    eframe::egui::IconData {
        rgba,
        width: SIZE,
        height: SIZE,
    }
}

fn main() -> eframe::Result {
    let launch = match arguments() {
        Ok(Some(launch)) => launch,
        Ok(None) => return Ok(()),
        Err(error) => {
            eprintln!("{error}\nUse --help for usage.");
            std::process::exit(2);
        }
    };
    if let Some(path) = &launch.export {
        if let Err(error) = app::headless_export(&launch, path) {
            eprintln!("Export failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    eframe::run_native(
        "rustDatView",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1440.0, 900.0])
                .with_min_inner_size([900.0, 600.0])
                .with_drag_and_drop(true)
                .with_icon(icon()),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::DatView::new(cc, launch)))),
    )
}
