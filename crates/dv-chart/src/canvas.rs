//! Shared vector primitives used by the live chart and SVG export.
use std::fmt::Write as _;

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Stroke, Vec2};

use crate::{Dash, FontSet};

#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    LeftTop,
    LeftCenter,
    CenterTop,
    CenterCenter,
    CenterBottom,
    RightCenter,
    RightTop,
}

/// A vector drawing target. Coordinates and sizes are in logical pixels.
pub trait Canvas {
    fn fill_rect(&mut self, rect: Rect, radius: f32, color: Color32);
    fn polyline(&mut self, points: &[Pos2], stroke: Stroke, dash: Dash);
    fn circle(&mut self, center: Pos2, radius: f32, fill: Color32, stroke: Stroke);
    fn text(
        &mut self,
        pos: Pos2,
        anchor: Anchor,
        text: &str,
        size: f32,
        color: Color32,
        semibold: bool,
    );
    fn measure_text(&mut self, text: &str, size: f32, semibold: bool) -> Vec2;
    fn set_clip(&mut self, clip: Option<Rect>);
}

pub struct EguiCanvas {
    base: Painter,
    painter: Painter,
    chart_font: bool,
    chart_bold: bool,
}
impl EguiCanvas {
    pub fn new(painter: Painter) -> Self {
        let (chart_font, chart_bold) = painter.ctx().fonts(|fonts| {
            let families = &fonts.definitions().families;
            (
                families.contains_key(&FontFamily::Name("chart".into())),
                families.contains_key(&FontFamily::Name("chart-semibold".into())),
            )
        });
        Self {
            base: painter.clone(),
            painter,
            chart_font,
            chart_bold,
        }
    }
    fn font(&self, size: f32, semibold: bool) -> FontId {
        if semibold && self.chart_bold {
            FontId::new(size, FontFamily::Name("chart-semibold".into()))
        } else if self.chart_font {
            FontId::new(size, FontFamily::Name("chart".into()))
        } else {
            FontId::proportional(size)
        }
    }
}
fn align(a: Anchor) -> Align2 {
    match a {
        Anchor::LeftTop => Align2::LEFT_TOP,
        Anchor::LeftCenter => Align2::LEFT_CENTER,
        Anchor::CenterTop => Align2::CENTER_TOP,
        Anchor::CenterCenter => Align2::CENTER_CENTER,
        Anchor::CenterBottom => Align2::CENTER_BOTTOM,
        Anchor::RightCenter => Align2::RIGHT_CENTER,
        Anchor::RightTop => Align2::RIGHT_TOP,
    }
}
impl Canvas for EguiCanvas {
    fn fill_rect(&mut self, rect: Rect, radius: f32, color: Color32) {
        self.painter.rect_filled(rect, radius, color);
    }
    fn polyline(&mut self, points: &[Pos2], stroke: Stroke, dash: Dash) {
        if points.len() < 2 {
            return;
        }
        if dash == Dash::Solid {
            self.painter.add(egui::Shape::line(points.to_vec(), stroke));
        } else {
            // Split into continuous dash intervals; the phase is carried over joins.
            let pattern: &[f32] = match dash {
                Dash::Dashed => &[7.0, 4.0],
                Dash::Dotted => &[1.5, 3.0],
                Dash::DashDot => &[7.0, 3.0, 1.5, 3.0],
                Dash::Solid => unreachable!(),
            };
            let mut phase = 0usize;
            let mut remaining = pattern[0];
            for pair in points.windows(2) {
                let delta = pair[1] - pair[0];
                let length = delta.length();
                if !length.is_finite() || length <= f32::EPSILON {
                    continue;
                }
                let dir = delta / length;
                let mut at = 0.0;
                while at < length {
                    let end = (at + remaining).min(length);
                    if phase % 2 == 0 {
                        self.painter
                            .line_segment([pair[0] + dir * at, pair[0] + dir * end], stroke);
                    }
                    remaining -= end - at;
                    at = end;
                    if remaining <= 0.001 {
                        phase = (phase + 1) % pattern.len();
                        remaining = pattern[phase];
                    }
                }
            }
        }
    }
    fn circle(&mut self, center: Pos2, radius: f32, fill: Color32, stroke: Stroke) {
        self.painter.circle(center, radius, fill, stroke);
    }
    fn text(
        &mut self,
        pos: Pos2,
        anchor: Anchor,
        text: &str,
        size: f32,
        color: Color32,
        semibold: bool,
    ) {
        self.painter
            .text(pos, align(anchor), text, self.font(size, semibold), color);
    }
    fn measure_text(&mut self, text: &str, size: f32, semibold: bool) -> Vec2 {
        self.painter
            .layout_no_wrap(text.to_owned(), self.font(size, semibold), Color32::WHITE)
            .size()
    }
    fn set_clip(&mut self, clip: Option<Rect>) {
        self.painter = match clip {
            Some(rect) => self.base.with_clip_rect(rect),
            None => self.base.clone(),
        };
    }
}

/// SVG implementation of the same canvas, using supplied font bytes for layout.
/// Text is emitted as `<text>` in `FontSet::family`; exporters that need
/// font-independent output convert it to outlines (usvg does this on write).
pub struct SvgCanvas {
    size: [f32; 2],
    family: String,
    regular: Option<FontArc>,
    bold: Option<FontArc>,
    mono: Option<FontArc>,
    body: String,
    defs: String,
    clipped: bool,
    next_clip: usize,
}
impl SvgCanvas {
    pub fn new(size: [f32; 2], fonts: &FontSet) -> Self {
        Self {
            size,
            family: fonts.family.clone(),
            regular: FontArc::try_from_vec(fonts.regular.to_vec()).ok(),
            bold: FontArc::try_from_vec(fonts.semibold.to_vec()).ok(),
            mono: FontArc::try_from_vec(fonts.mono.to_vec()).ok(),
            body: String::new(),
            defs: String::new(),
            clipped: false,
            next_clip: 0,
        }
    }
    pub fn finish(mut self) -> String {
        self.set_clip(None);
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><defs>{}</defs>{}</svg>",
            self.size[0], self.size[1], self.size[0], self.size[1], self.defs, self.body
        )
    }
    fn metrics(&self, text: &str, size: f32, bold: bool) -> (f32, f32, f32) {
        let font = if bold {
            self.bold.as_ref().or(self.regular.as_ref())
        } else {
            self.regular.as_ref()
        };
        let Some(font) = font else {
            return (
                text.chars().count() as f32 * size * 0.55,
                size * 1.2,
                size * 0.9,
            );
        };
        // SVG `font-size` is the em size; ab_glyph's `PxScale` is the
        // ascent-to-descent height, which is larger than the em for most
        // fonts. Convert, or every measured width comes out too narrow.
        let em_scale = |font: &FontArc| {
            PxScale::from(size * font.height_unscaled() / font.units_per_em().unwrap_or(1000.0))
        };
        let scaled = font.as_scaled(em_scale(font));
        let fallback = self
            .mono
            .as_ref()
            .map(|font| font.as_scaled(em_scale(font)));
        let mut width = 0.0;
        let mut previous = None;
        for c in text.chars() {
            let glyph = scaled.glyph_id(c);
            match fallback
                .as_ref()
                .filter(|fallback| glyph.0 == 0 && fallback.glyph_id(c).0 != 0)
            {
                Some(fallback) => {
                    let glyph = fallback.glyph_id(c);
                    if let Some((true, prev)) = previous {
                        width += fallback.kern(prev, glyph);
                    }
                    width += fallback.h_advance(glyph);
                    previous = Some((true, glyph));
                }
                None => {
                    if let Some((false, prev)) = previous {
                        width += scaled.kern(prev, glyph);
                    }
                    width += scaled.h_advance(glyph);
                    previous = Some((false, glyph));
                }
            }
        }
        let ascent = scaled.ascent();
        (width, ascent - scaled.descent(), ascent)
    }
}
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c == '\n' || c == '\t' || c == '\r' || !c.is_control() => out.push(c),
            _ => {}
        }
    }
    out
}
fn color(color: Color32) -> (String, f32) {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    (format!("#{r:02x}{g:02x}{b:02x}"), f32::from(a) / 255.0)
}
impl Canvas for SvgCanvas {
    fn fill_rect(&mut self, rect: Rect, radius: f32, fill: Color32) {
        let (fill, opacity) = color(fill);
        let _ = write!(self.body, "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" rx=\"{:.2}\" fill=\"{}\" fill-opacity=\"{:.4}\"/>", rect.left(), rect.top(), rect.width().max(0.0), rect.height().max(0.0), radius, fill, opacity);
    }
    fn polyline(&mut self, points: &[Pos2], stroke: Stroke, dash: Dash) {
        if points.len() < 2 {
            return;
        }
        let (stroke_color, opacity) = color(stroke.color);
        let dashes = match dash {
            Dash::Solid => "",
            Dash::Dashed => " stroke-dasharray=\"7 4\"",
            Dash::Dotted => " stroke-dasharray=\"1.5 3\"",
            Dash::DashDot => " stroke-dasharray=\"7 3 1.5 3\"",
        };
        let _ = write!(self.body, "<polyline fill=\"none\" stroke=\"{}\" stroke-opacity=\"{:.4}\" stroke-width=\"{:.3}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{} points=\"", stroke_color, opacity, stroke.width, dashes);
        for p in points {
            let _ = write!(self.body, "{:.3},{:.3} ", p.x, p.y);
        }
        self.body.push_str("\"/>");
    }
    fn circle(&mut self, center: Pos2, radius: f32, fill: Color32, stroke: Stroke) {
        let (fill, fill_opacity) = color(fill);
        let (stroke_color, stroke_opacity) = color(stroke.color);
        let _ = write!(self.body, "<circle cx=\"{:.3}\" cy=\"{:.3}\" r=\"{:.3}\" fill=\"{}\" fill-opacity=\"{:.4}\" stroke=\"{}\" stroke-opacity=\"{:.4}\" stroke-width=\"{:.3}\"/>", center.x, center.y, radius, fill, fill_opacity, stroke_color, stroke_opacity, stroke.width);
    }
    fn text(
        &mut self,
        pos: Pos2,
        anchor: Anchor,
        text: &str,
        size: f32,
        fill: Color32,
        semibold: bool,
    ) {
        let (w, h, ascent) = self.metrics(text, size, semibold);
        let x = match anchor {
            Anchor::LeftTop | Anchor::LeftCenter => pos.x,
            Anchor::CenterTop | Anchor::CenterCenter | Anchor::CenterBottom => pos.x - w * 0.5,
            Anchor::RightCenter | Anchor::RightTop => pos.x - w,
        };
        let top = match anchor {
            Anchor::LeftTop | Anchor::CenterTop | Anchor::RightTop => pos.y,
            Anchor::LeftCenter | Anchor::CenterCenter | Anchor::RightCenter => pos.y - h * 0.5,
            Anchor::CenterBottom => pos.y - h,
        };
        let (fill, opacity) = color(fill);
        let _ = write!(self.body, "<text x=\"{:.3}\" y=\"{:.3}\" font-family=\"{}\" font-size=\"{:.2}\" font-weight=\"{}\" fill=\"{}\" fill-opacity=\"{:.4}\">{}</text>", x, top + ascent, escape(&self.family), size, if semibold { 600 } else { 400 }, fill, opacity, escape(text));
    }
    fn measure_text(&mut self, text: &str, size: f32, semibold: bool) -> Vec2 {
        let (w, h, _) = self.metrics(text, size, semibold);
        Vec2::new(w, h)
    }
    fn set_clip(&mut self, clip: Option<Rect>) {
        if self.clipped {
            self.body.push_str("</g>");
            self.clipped = false;
        }
        if let Some(rect) = clip {
            let id = self.next_clip;
            self.next_clip += 1;
            let _ = write!(self.defs, "<clipPath id=\"clip{}\"><rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\"/></clipPath>", id, rect.left(), rect.top(), rect.width().max(0.0), rect.height().max(0.0));
            let _ = write!(self.body, "<g clip-path=\"url(#clip{})\">", id);
            self.clipped = true;
        }
    }
}
