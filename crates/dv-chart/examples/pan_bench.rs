//! CPU-side pan benchmark: ten series of one million samples each, including
//! egui tessellation. Run with `cargo run --release --example pan_bench`.
use std::time::Instant;

use dv_chart::{AxisFormat, ChartConfig, ChartState, ChartTheme, Dash, Draw, Pane, Scale, Series};
use egui::{Color32, Pos2, Rect, Vec2};

fn main() {
    const SAMPLES: usize = 1_000_000;
    const FRAMES: usize = 80;
    let x: Vec<f64> = (0..SAMPLES).map(|i| i as f64 * 0.001).collect();
    let ys: Vec<Vec<f64>> = (0..10)
        .map(|series| {
            x.iter()
                .map(|t| {
                    100.0 + 20.0 * (t * (0.02 + series as f64 * 0.002)).sin() + (t * 3.0).sin()
                })
                .collect()
        })
        .collect();
    let theme = ChartTheme::dark();
    let panes: Vec<_> = (0..3)
        .map(|pane| {
            let start = if pane == 0 {
                0
            } else if pane == 1 {
                4
            } else {
                7
            };
            let end = if pane == 0 {
                4
            } else if pane == 1 {
                7
            } else {
                10
            };
            Pane {
                id: pane as u64,
                title: "Telemetry",
                y_label: "W",
                y_scale: Scale::Linear,
                y_format: AxisFormat::Number,
                y_flip: false,
                include_zero: false,
                weight: 1.0,
                series: (start..end)
                    .map(|i| Series {
                        id: i as u64,
                        label: "Signal",
                        unit: "W",
                        x: &x,
                        y: &ys[i],
                        generation: 1,
                        color: theme.series_palette[i],
                        width: 1.6,
                        dash: Dash::Solid,
                        draw: Draw::Lines,
                        marker_radius: 2.0,
                        visible: true,
                        overlay: false,
                    })
                    .collect(),
            }
        })
        .collect();
    let config = ChartConfig {
        x_format: AxisFormat::Duration,
        gap_factor: Some(10.0),
        tooltip: false,
        ..Default::default()
    };
    let mut state = ChartState::default();
    let context = egui::Context::default();
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0))),
        ..Default::default()
    };
    let mut times = Vec::with_capacity(FRAMES);
    let run = |state: &mut ChartState| {
        let output = context.run_ui(input(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                dv_chart::show(ui, state, &panes, &config, &[], &theme);
            });
        });
        let _meshes = context.tessellate(output.shapes, output.pixels_per_point);
    };
    let start = Instant::now();
    run(&mut state);
    println!(
        "Initial indexing and frame: {:.2} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    for i in 0..FRAMES {
        let start_x = 100.0 + i as f64 * 0.5;
        state.x_view = Some([start_x, start_x + 500.0]);
        let start = Instant::now();
        run(&mut state);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!("10 × 1,000,000 samples; 1200 × 800 points; {FRAMES} distinct pan views");
    println!("CPU frame time including tessellation: median {:.2} ms, mean {:.2} ms, p95 {:.2} ms, max {:.2} ms", times[FRAMES / 2], mean, times[FRAMES * 95 / 100], times[FRAMES - 1]);
    println!(
        "GPU submission, presentation and vsync are not measured; background color: {:?}",
        Color32::TRANSPARENT
    );
}
