//! Load the synthetic telemetry corpus in release mode and report its shape and time axis.
use std::{path::PathBuf, time::Instant};

use dv_core::ColumnData;

fn main() {
    let directory = std::env::args_os().nth(1).map_or_else(
        || PathBuf::from("C:/Users/exror/AppData/Local/Temp/dvdata"),
        PathBuf::from,
    );
    let filter = std::env::args().nth(2);
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .expect("corpus directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| matches!(ext.to_str(), Some("csv" | "txt" | "log" | "hml")))
                && filter.as_ref().is_none_or(|filter| {
                    path.file_name()
                        .is_some_and(|name| name.to_string_lossy().contains(filter))
                })
        })
        .collect();
    paths.sort();
    println!("file\tformat\trows\tcolumns\ttime\tload_ms");
    for path in paths {
        let start = Instant::now();
        let result = dv_io::read_file(&path, None).and_then(|file| {
            let frames = file.to_dataframes()?;
            Ok((file, frames))
        });
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        match result {
            Ok((file, frames)) => {
                for frame in frames {
                    let time = frame
                        .df
                        .columns()
                        .iter()
                        .find(|c| matches!(c.data, ColumnData::DateTime(_)))
                        .or_else(|| {
                            frame.df.columns().iter().find(|c| {
                                let name = c.name.to_ascii_lowercase();
                                name.contains("time")
                                    || name == "x"
                                    || name.starts_with("x [")
                                    || name.starts_with("zeit")
                            })
                        })
                        .map_or("-", |c| c.name.as_str());
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{elapsed:.1}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        file.format_name(),
                        frame.df.nrows(),
                        frame.df.ncols(),
                        time
                    );
                }
            }
            Err(error) => println!(
                "{}\tERROR: {error}\t-\t-\t-\t{elapsed:.1}",
                path.file_name().unwrap_or_default().to_string_lossy()
            ),
        }
    }
}
