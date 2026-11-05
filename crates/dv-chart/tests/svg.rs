use std::sync::Arc;

use dv_chart::{
    render_svg, AxisFormat, ChartConfig, ChartState, ChartTheme, Dash, Draw, FontSet, Mark, Pane,
    Scale, Series,
};
use egui::Color32;

#[test]
fn transparent_svg_with_clipped_series_and_annotations_rasterizes() {
    let fonts = FontSet {
        regular: Arc::from(&include_bytes!("../../datview/assets/fonts/Inter-Regular.ttf")[..]),
        semibold: Arc::from(&include_bytes!("../../datview/assets/fonts/Inter-SemiBold.ttf")[..]),
        mono: Arc::from(
            &include_bytes!("../../datview/assets/fonts/JetBrainsMono-Regular.ttf")[..],
        ),
        family: "Inter".to_owned(),
    };
    let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
    let y = [1.0, 3.0, f64::NAN, 2.0, 5.0, 1.0];
    let pane = Pane {
        id: 7,
        title: "POWER",
        y_label: "W",
        y_scale: Scale::Linear,
        y_format: AxisFormat::Number,
        y_flip: false,
        include_zero: true,
        weight: 1.0,
        series: vec![Series {
            id: 11,
            label: "Package & cores",
            unit: "W",
            x: &x,
            y: &y,
            generation: 1,
            color: Color32::from_rgb(91, 140, 255),
            width: 2.0,
            dash: Dash::Dashed,
            draw: Draw::LinesPoints,
            marker_radius: 3.0,
            visible: true,
            overlay: false,
        }],
    };
    let marks = [
        Mark::Point {
            pane: Some(7),
            x: 1.0,
            y: 3.0,
            label: "Peak <1>".to_owned(),
            color: Color32::LIGHT_RED,
        },
        Mark::Region {
            x0: 3.0,
            x1: 4.0,
            label: "Load & hold".to_owned(),
            color: Color32::LIGHT_BLUE,
        },
        Mark::VLine {
            x: 4.0,
            label: "Event".to_owned(),
            color: Color32::LIGHT_GREEN,
        },
        Mark::HLine {
            pane: 7,
            y: 2.0,
            label: "Limit".to_owned(),
            color: Color32::YELLOW,
        },
    ];
    let config = ChartConfig {
        title: "Power",
        subtitle: "A < B",
        x_label: "Time [s]",
        overview: false,
        ..Default::default()
    };
    let mut theme = ChartTheme::light();
    theme.surface = Color32::TRANSPARENT;
    let svg = render_svg(
        &[pane],
        &ChartState::default(),
        &config,
        &marks,
        &theme,
        &fonts,
        [800.0, 480.0],
    );
    assert!(svg.contains("clipPath") && svg.contains("font-family=\"Inter\""));
    assert!(
        svg.contains("Peak &lt;1&gt;")
            && svg.contains("Load &amp; hold")
            && svg.contains("A &lt; B")
    );
    assert!(svg.contains("stroke-dasharray=\"7 4\""));
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_font_data(fonts.regular.to_vec());
    options.fontdb_mut().load_font_data(fonts.semibold.to_vec());
    let tree = resvg::usvg::Tree::from_str(&svg, &options).expect("valid SVG");
    assert_eq!(tree.size().width(), 800.0);
    assert_eq!(tree.size().height(), 480.0);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(800, 480).expect("pixmap");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let painted = pixmap.pixels().iter().filter(|p| p.alpha() > 0).count();
    assert!(
        painted > 2_000 && painted < 800 * 480 / 2,
        "painted pixels: {painted}"
    );
}
