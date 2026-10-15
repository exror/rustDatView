//! Vector figure construction shared by SVG, PNG, PDF and EPS exports.
use crate::app::presentation::Annotation;
use serde::{Deserialize, Serialize};
use std::{fmt::Write as _, path::Path};

#[derive(Clone)]
pub struct Curve {
    pub name: String,
    pub sx: String,
    pub sy: String,
    pub points: Vec<[f64; 2]>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    pub log_x: bool,
    pub log_y: bool,
    pub flip_x: bool,
    pub flip_y: bool,
    pub subplots: bool,
    pub share_x: bool,
    pub grid: bool,
    pub legend: bool,
    pub lines: bool,
    pub markers: bool,
    pub step: bool,
    pub width: f32,
    pub marker_size: f32,
    pub title: String,
    pub x_label: String,
    pub y_label: String,
    pub limits: Option<[f64; 4]>,
    pub figure_width: u32,
    pub figure_height: u32,
    pub font_family: String,
    pub font_size: f32,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            log_x: false,
            log_y: false,
            flip_x: false,
            flip_y: false,
            subplots: false,
            share_x: true,
            grid: true,
            legend: true,
            lines: true,
            markers: false,
            step: false,
            width: 2.0,
            marker_size: 3.0,
            title: String::new(),
            x_label: String::new(),
            y_label: String::new(),
            limits: None,
            figure_width: 1200,
            figure_height: 800,
            font_family: String::new(),
            font_size: 14.0,
        }
    }
}

pub fn axis(value: f64, log: bool) -> f64 {
    if log {
        if value > 0.0 {
            value.log10()
        } else {
            f64::NAN
        }
    } else {
        value
    }
}
pub fn display_points(curve: &Curve, style: &Style) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(curve.points.len() * if style.step { 2 } else { 1 });
    let mut previous: Option<[f64; 2]> = None;
    for &[x, y] in &curve.points {
        let p = [axis(x, style.log_x), axis(y, style.log_y)];
        if style.step {
            if let Some(prev) = previous {
                out.push([p[0], prev[1]]);
            }
        }
        out.push(p);
        previous = Some(p);
    }
    out
}
pub fn color(index: usize) -> [u8; 3] {
    const COLORS: [[u8; 3]; 10] = [
        [31, 119, 180],
        [255, 127, 14],
        [44, 160, 44],
        [214, 39, 40],
        [148, 103, 189],
        [140, 86, 75],
        [227, 119, 194],
        [127, 127, 127],
        [188, 189, 34],
        [23, 190, 207],
    ];
    COLORS[index % COLORS.len()]
}
pub fn bounds(curves: &[Curve], style: &Style) -> [f64; 4] {
    if let Some([x0, x1, y0, y1]) = style.limits {
        return [
            axis(x0, style.log_x),
            axis(x1, style.log_x),
            axis(y0, style.log_y),
            axis(y1, style.log_y),
        ];
    }
    let mut b = [
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for c in curves {
        for &[x, y] in &c.points {
            let (x, y) = (axis(x, style.log_x), axis(y, style.log_y));
            if x.is_finite() && y.is_finite() {
                b[0] = b[0].min(x);
                b[1] = b[1].max(x);
                b[2] = b[2].min(y);
                b[3] = b[3].max(y);
            }
        }
    }
    for i in [0, 2] {
        if !b[i].is_finite() {
            b[i] = 0.0;
            b[i + 1] = 1.0;
        }
        let pad = if b[i] == b[i + 1] {
            b[i].abs().max(1.0) * 0.05
        } else {
            (b[i + 1] - b[i]) * 0.03
        };
        b[i] -= pad;
        b[i + 1] += pad;
    }
    b
}

#[derive(Clone)]
enum Element {
    Path(Vec<[f64; 2]>, [u8; 3], f32),
    Text(f64, f64, String, f32),
    Circle(f64, f64, f32, [u8; 3]),
    Rect(f64, f64, f64, f64, [u8; 3], f32),
}
struct Figure {
    width: u32,
    height: u32,
    elements: Vec<Element>,
    font_family: String,
    font_size: f32,
}
impl Figure {
    fn build(curves: &[Curve], s: &Style, annotations: &[&Annotation]) -> Result<Self, String> {
        if curves.is_empty() {
            return Err("No plotted curves to export".into());
        }
        if s.figure_width < 200 || s.figure_height < 150 {
            return Err("Figure dimensions must be at least 200 × 150".into());
        }
        let mut fig = Self {
            width: s.figure_width,
            height: s.figure_height,
            elements: Vec::new(),
            font_family: if s.font_family.is_empty() {
                "sans-serif".into()
            } else {
                s.font_family.clone()
            },
            font_size: s.font_size,
        };
        let count = if s.subplots { curves.len() } else { 1 };
        let h = s.figure_height as f64 / count as f64;
        if h < 100.0 {
            return Err("Increase figure height for this many subplots".into());
        }
        let global = bounds(curves, s);
        for panel in 0..count {
            let subset = if s.subplots {
                &curves[panel..panel + 1]
            } else {
                curves
            };
            let mut b = bounds(subset, s);
            if s.share_x {
                b[0] = global[0];
                b[1] = global[1];
            }
            if !b.iter().all(|x| x.is_finite()) || b[0] >= b[1] || b[2] >= b[3] {
                return Err("Invalid plot limits (logarithmic bounds must be positive)".into());
            }
            let (left, right, top, bottom) = (
                90.0,
                fig.width as f64 - 30.0,
                panel as f64 * h + 35.0,
                (panel + 1) as f64 * h - 65.0,
            );
            let map = |p: [f64; 2]| {
                let mut x = (p[0] - b[0]) / (b[1] - b[0]);
                let mut y = (p[1] - b[2]) / (b[3] - b[2]);
                if s.flip_x {
                    x = 1.0 - x;
                }
                if s.flip_y {
                    y = 1.0 - y;
                }
                [left + x * (right - left), bottom - y * (bottom - top)]
            };
            for annotation in annotations {
                if let Annotation::Region {
                    x_axis,
                    min,
                    max,
                    color,
                    ..
                } = annotation
                {
                    if *x_axis != subset[0].sx || !min.is_finite() || !max.is_finite() {
                        continue;
                    }
                    let a = axis(min.min(*max), s.log_x).max(b[0]);
                    let z = axis(min.max(*max), s.log_x).min(b[1]);
                    if a.is_finite() && z.is_finite() && a < z {
                        let a = map([a, b[2]])[0];
                        let z = map([z, b[2]])[0];
                        fig.elements.push(Element::Rect(
                            a.min(z),
                            top,
                            (z - a).abs(),
                            bottom - top,
                            *color,
                            0.15,
                        ));
                    }
                }
            }
            for tick in 0..=5 {
                let r = tick as f64 / 5.0;
                let x = b[0] + r * (b[1] - b[0]);
                let y = b[2] + r * (b[3] - b[2]);
                let xp = map([x, b[2]])[0];
                let yp = map([b[0], y])[1];
                if s.grid {
                    fig.elements.push(Element::Path(
                        vec![[xp, top], [xp, bottom]],
                        [220, 220, 220],
                        0.6,
                    ));
                    fig.elements.push(Element::Path(
                        vec![[left, yp], [right, yp]],
                        [220, 220, 220],
                        0.6,
                    ));
                }
                fig.elements.push(Element::Text(
                    xp - 20.0,
                    bottom + 20.0,
                    value_label(if s.log_x { 10f64.powf(x) } else { x }),
                    11.0,
                ));
                fig.elements.push(Element::Text(
                    4.0,
                    yp + 4.0,
                    value_label(if s.log_y { 10f64.powf(y) } else { y }),
                    11.0,
                ));
            }
            fig.elements.push(Element::Path(
                vec![
                    [left, top],
                    [left, bottom],
                    [right, bottom],
                    [right, top],
                    [left, top],
                ],
                [0, 0, 0],
                1.0,
            ));
            let sx = if s.x_label.is_empty() {
                &subset[0].sx
            } else {
                &s.x_label
            };
            let sy = if s.y_label.is_empty() {
                &subset[0].sy
            } else {
                &s.y_label
            };
            fig.elements
                .push(Element::Text(left, bottom + 45.0, sx.clone(), 13.0));
            fig.elements.push(Element::Text(
                left,
                top - 12.0,
                format!(
                    "{}{}{}",
                    s.title,
                    if s.title.is_empty() { "" } else { " — " },
                    sy
                ),
                14.0,
            ));
            for (local, c) in subset.iter().enumerate() {
                let index = if s.subplots { panel } else { local };
                let rgb = color(index);
                let pts = display_points(c, s);
                if s.lines {
                    let mut path = Vec::new();
                    let mut previous = None;
                    for p in pts {
                        if p.iter().all(|v| v.is_finite()) {
                            let q = map(p);
                            if let Some(prev) = previous {
                                if let Some((a, z)) = clip(prev, q, left, right, top, bottom) {
                                    if path.last().is_some_and(|last| *last != a) {
                                        fig.elements.push(Element::Path(
                                            std::mem::take(&mut path),
                                            rgb,
                                            s.width,
                                        ));
                                    }
                                    if path.is_empty() {
                                        path.push(a);
                                    }
                                    path.push(z);
                                } else if !path.is_empty() {
                                    fig.elements.push(Element::Path(
                                        std::mem::take(&mut path),
                                        rgb,
                                        s.width,
                                    ));
                                }
                            }
                            previous = Some(q);
                        } else {
                            previous = None;
                            if !path.is_empty() {
                                fig.elements.push(Element::Path(
                                    std::mem::take(&mut path),
                                    rgb,
                                    s.width,
                                ));
                            }
                        }
                    }
                    if !path.is_empty() {
                        fig.elements.push(Element::Path(path, rgb, s.width));
                    }
                }
                if s.markers {
                    for &p in &c.points {
                        let p = map([axis(p[0], s.log_x), axis(p[1], s.log_y)]);
                        if p[0] >= left && p[0] <= right && p[1] >= top && p[1] <= bottom {
                            fig.elements
                                .push(Element::Circle(p[0], p[1], s.marker_size, rgb));
                        }
                    }
                }
                if s.legend {
                    let yy = top + 16.0 + local as f64 * 18.0;
                    fig.elements.push(Element::Path(
                        vec![[right - 230.0, yy], [right - 205.0, yy]],
                        rgb,
                        s.width,
                    ));
                    fig.elements
                        .push(Element::Text(right - 200.0, yy + 4.0, c.name.clone(), 11.0));
                }
            }
            for annotation in annotations {
                match annotation {
                    Annotation::Point {
                        curve,
                        x,
                        y,
                        label,
                        color,
                        ..
                    } if subset.iter().any(|c| &c.name == curve) => {
                        let p = map([axis(*x, s.log_x), axis(*y, s.log_y)]);
                        if p.iter().all(|v| v.is_finite())
                            && p[0] >= left
                            && p[0] <= right
                            && p[1] >= top
                            && p[1] <= bottom
                        {
                            fig.elements.push(Element::Circle(p[0], p[1], 5., *color));
                            let tx = (p[0] + 12.).min(
                                (right - label.chars().count() as f64 * s.font_size as f64 * 0.48)
                                    .max(left + 8.),
                            );
                            let ty = (p[1] - 14.).max(top + 16.);
                            fig.elements
                                .push(Element::Path(vec![p, [tx, ty + 3.]], *color, 1.));
                            fig.elements.push(Element::Rect(
                                tx - 4.,
                                ty - s.font_size as f64,
                                ((label.chars().count() as f64 * s.font_size as f64 * 0.55) + 8.)
                                    .min(right - tx + 4.),
                                s.font_size as f64 + 6.,
                                [255, 255, 255],
                                0.94,
                            ));
                            fig.elements.push(Element::Text(tx, ty, label.clone(), 13.));
                        }
                    }
                    Annotation::Region {
                        x_axis,
                        min,
                        max,
                        label,
                        ..
                    } if *x_axis == subset[0].sx => {
                        let a = axis(min.min(*max), s.log_x).max(b[0]);
                        let z = axis(min.max(*max), s.log_x).min(b[1]);
                        if a.is_finite() && z.is_finite() && a < z {
                            let x = map([a, b[2]])[0].min(map([z, b[2]])[0]);
                            fig.elements
                                .push(Element::Text(x + 7., top + 18., label.clone(), 12.));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(fig)
    }
    fn svg(&self) -> String {
        let mut out=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/>",self.width,self.height,self.width,self.height);
        for e in &self.elements {
            match e {
                Element::Path(p, c, w) => {
                    let mut d = String::new();
                    for (i, [x, y]) in p.iter().enumerate() {
                        let _ = write!(d, "{}{} {} ", if i == 0 { "M" } else { "L" }, x, y);
                    }
                    let _=write!(out,"<path d=\"{d}\" fill=\"none\" stroke=\"rgb({},{},{})\" stroke-width=\"{w}\"/>",c[0],c[1],c[2]);
                }
                Element::Text(x, y, t, size) => {
                    let size = size * self.font_size / 14.;
                    let _=write!(out,"<text x=\"{x}\" y=\"{y}\" font-family=\"{}\" font-size=\"{size}\">{}</text>",xml(&self.font_family),xml(t));
                }
                Element::Circle(x, y, r, c) => {
                    let _ = write!(
                        out,
                        "<circle cx=\"{x}\" cy=\"{y}\" r=\"{r}\" fill=\"rgb({},{},{})\"/>",
                        c[0], c[1], c[2]
                    );
                }
                Element::Rect(x, y, w, h, color, alpha) => {
                    let _=write!(out,"<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"rgb({},{},{})\" fill-opacity=\"{alpha}\"/>",color[0],color[1],color[2]);
                }
            }
        }
        out.push_str("</svg>");
        out
    }
}
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn clip(a: [f64; 2], z: [f64; 2], l: f64, r: f64, t: f64, b: f64) -> Option<([f64; 2], [f64; 2])> {
    let d = [z[0] - a[0], z[1] - a[1]];
    let mut lo: f64 = 0.0;
    let mut hi: f64 = 1.0;
    for (p, q) in [
        (-d[0], a[0] - l),
        (d[0], r - a[0]),
        (-d[1], a[1] - t),
        (d[1], b - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let v = q / p;
            if p < 0.0 {
                lo = lo.max(v);
            } else {
                hi = hi.min(v);
            }
            if lo > hi {
                return None;
            }
        }
    }
    Some((
        [a[0] + lo * d[0], a[1] + lo * d[1]],
        [a[0] + hi * d[0], a[1] + hi * d[1]],
    ))
}

pub fn save(
    path: &Path,
    curves: &[Curve],
    style: &Style,
    annotations: &[&Annotation],
) -> Result<(), String> {
    let figure = Figure::build(curves, style, annotations)?;
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let tree = render_tree(&figure)?;
    let bytes = match ext.as_str() {
        "svg" => tree
            .to_string(&resvg::usvg::WriteOptions::default())
            .into_bytes(),
        "pdf" => svg2pdf::to_pdf(
            &tree,
            svg2pdf::ConversionOptions::default(),
            svg2pdf::PageOptions::default(),
        )
        .map_err(|e| e.to_string())?,
        "png" | "eps" => {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(figure.width, figure.height)
                .ok_or("Figure image allocation failed")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::identity(),
                &mut pixmap.as_mut(),
            );
            if ext == "png" {
                pixmap.encode_png().map_err(|e| e.to_string())?
            } else {
                eps_raster(&pixmap).into_bytes()
            }
        }
        _ => return Err("Choose an SVG, PNG, PDF or EPS filename".into()),
    };
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

fn render_tree(figure: &Figure) -> Result<resvg::usvg::Tree, String> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    resvg::usvg::Tree::from_str(&figure.svg(), &options).map_err(|e| e.to_string())
}
pub fn raster(
    curves: &[Curve],
    style: &Style,
    annotations: &[&Annotation],
) -> Result<eframe::egui::ColorImage, String> {
    let figure = Figure::build(curves, style, annotations)?;
    let tree = render_tree(&figure)?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(figure.width, figure.height)
        .ok_or("Figure image allocation failed")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(eframe::egui::ColorImage::from_rgba_premultiplied(
        [figure.width as usize, figure.height as usize],
        pixmap.data(),
    ))
}
fn eps_raster(pixmap: &resvg::tiny_skia::Pixmap) -> String {
    let (w, h) = (pixmap.width(), pixmap.height());
    let mut out=format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 {w} {h}\n%%LanguageLevel: 2\n%%EndComments\ngsave\n{w} {h} scale\n{w} {h} 8 [{w} 0 0 -{h} 0 {h}]\n{{currentfile {} string readhexstring pop}} false 3 colorimage\n",w*3);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for row in pixmap.data().chunks_exact(w as usize * 4) {
        for pixel in row.chunks_exact(4) {
            for &byte in &pixel[..3] {
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 15) as usize] as char);
            }
        }
        out.push('\n');
    }
    out.push_str("grestore\nshowpage\n%%EOF\n");
    out
}

pub fn value_label(value: f64) -> String {
    if value != 0. && (value.abs() >= 1e6 || value.abs() < 1e-3) {
        return format!("{value:.3e}");
    }
    let mut text = format!("{value:.4}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text.remove(0);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    fn curve() -> Curve {
        Curve {
            name: "SOC".into(),
            sx: "Time [s]".into(),
            sy: "Power [W]".into(),
            points: vec![[0., 1.], [2., 2.], [4., f64::NAN], [6., 3.], [8., 4.]],
        }
    }
    #[test]
    fn excluded_samples_do_not_bridge_in_shared_images() {
        let style = Style {
            grid: false,
            legend: false,
            limits: Some([0., 10., 0., 10.]),
            ..Style::default()
        };
        let figure = Figure::build(&[curve()], &style, &[]).unwrap();
        let paths = figure
            .elements
            .iter()
            .filter_map(|e| match e {
                Element::Path(points, rgb, _) if *rgb == color(0) => Some(points),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), 2);
        assert!(paths[0].last().unwrap()[0] < paths[1].first().unwrap()[0]);
    }
    #[test]
    fn highlighted_regions_clip_and_reverse_with_the_x_axis() {
        let style = Style {
            grid: false,
            legend: false,
            flip_x: true,
            limits: Some([0., 10., 0., 10.]),
            ..Style::default()
        };
        let region = Annotation::Region {
            mode: "Regular".into(),
            x_axis: "Time [s]".into(),
            min: 4.,
            max: -2.,
            label: "Workload".into(),
            color: [234, 153, 41],
        };
        let figure = Figure::build(&[curve()], &style, &[&region]).unwrap();
        let region = figure
            .elements
            .iter()
            .find_map(|e| match e {
                Element::Rect(x, y, w, h, _, alpha) if *alpha == 0.15 => Some([*x, *y, *w, *h]),
                _ => None,
            })
            .unwrap();
        let right = style.figure_width as f64 - 30.;
        let width = right - 90.;
        assert_eq!(
            region,
            [
                right - width * 0.4,
                35.,
                width * 0.4,
                style.figure_height as f64 - 100.
            ]
        );
    }
}
