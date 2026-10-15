mod app;
mod export;

use std::path::PathBuf;

#[derive(Default)]
struct Launch {
    paths: Vec<PathBuf>,
    format: Option<String>,
    dayfirst: bool,
    export: Option<PathBuf>,
    mode: Option<String>,
    x: Option<usize>,
    ys: Vec<usize>,
    pipeline: Option<PathBuf>,
    plot_options: Option<serde_json::Value>,
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
                println!("rustDatView {}\nUsage: datview [OPTIONS] [FILE ...]\n\nOptions:\n  -f, --format NAME     Reader format (default: auto)\n  --dayfirst           Interpret ambiguous dates as day/month/year\n  --mode MODE          Regular, PDF, FFT, MinMax, Compare, Polar\n  --x INDEX            Zero-based table X column (including inserted Index)\n  --y INDEX[,INDEX]    Selected Y columns\n  --pipeline PATH      Load plugin pipeline JSON\n  --plot-options JSON  Serialized dv-plot PlotType options\n  --export PATH        Noninteractive SVG, PNG, PDF, EPS or Python export\n  -h, --help           Display help\n  -V, --version        Display version\n\nUse -- to open filenames beginning with a dash.\nFormats available:\n{}", env!("CARGO_PKG_VERSION"), dv_io::all_formats().iter().map(|f| f.name).collect::<Vec<_>>().join("\n"));
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("rustDatView {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-f" | "--format" => {
                let format = args.next().ok_or("--format requires a format name")?;
                if dv_io::find_format(&format).is_none() {
                    return Err(format!("Unknown format: {format}"));
                }
                launch.format = Some(format);
            }
            "--dayfirst" => launch.dayfirst = true,
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
                let mode = args.next().ok_or("--mode requires a mode")?;
                let mode = match mode.to_lowercase().as_str() {
                    "regular" => "Regular",
                    "pdf" => "PDF",
                    "fft" => "FFT",
                    "minmax" => "MinMax",
                    "compare" => "Compare",
                    "polar" => "Polar",
                    _ => return Err(format!("Unknown plot mode: {mode}")),
                };
                launch.mode = Some(mode.into());
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
                    .map(|v| v.parse().map_err(|_| "Invalid Y column index"))
                    .collect::<Result<Vec<_>, _>>()?
            }
            "--plot-options" => {
                launch.plot_options = Some(
                    serde_json::from_str(&args.next().ok_or("--plot-options requires JSON")?)
                        .map_err(|e| e.to_string())?,
                )
            }
            _ if arg.starts_with('-') => return Err(format!("Unknown option: {arg}")),
            _ => launch.paths.push(arg.into()),
        }
    }
    Ok(Some(launch))
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
        match app::headless_export(&launch, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("Export failed: {e}");
                std::process::exit(1);
            }
        }
    }
    eframe::run_native(
        "rustDatView",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 820.0])
                .with_drag_and_drop(true),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::DatView::new(cc, launch)))),
    )
}
