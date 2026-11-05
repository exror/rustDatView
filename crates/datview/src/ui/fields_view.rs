use std::sync::Arc;

use dv_core::{Field2D, FileObject};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::workspace::Workspace;

const MAPS: &[&str] = &[
    "viridis", "plasma", "inferno", "magma", "cividis", "coolwarm", "RdBu", "jet", "gray",
];

#[derive(Clone, Copy, PartialEq)]
enum RangeMode {
    Automatic,
    Common,
    Manual,
}

pub(crate) struct FieldsView {
    revision: Option<u64>,
    files: Vec<Arc<dyn FileObject>>,
    fields: Vec<(String, Field2D)>,
    index: usize,
    map: &'static str,
    range_mode: RangeMode,
    min: f64,
    max: f64,
    polar: bool,
    degrees: bool,
    texture: Option<egui::TextureHandle>,
    image: Option<egui::ColorImage>,
    palette: Vec<Color32>,
    range: (f64, f64),
    error: Option<String>,
}

impl Default for FieldsView {
    fn default() -> Self {
        Self {
            revision: None,
            files: Vec::new(),
            fields: Vec::new(),
            index: 0,
            map: "viridis",
            range_mode: RangeMode::Automatic,
            min: 0.0,
            max: 1.0,
            polar: false,
            degrees: true,
            texture: None,
            image: None,
            palette: Vec::new(),
            range: (0.0, 1.0),
            error: None,
        }
    }
}

impl FieldsView {
    pub(crate) fn sync(&mut self, workspace: &mut Workspace) {
        if self.revision == Some(workspace.data_revision) {
            return;
        }
        self.revision = Some(workspace.data_revision);
        let mut files: Vec<Arc<dyn FileObject>> = Vec::new();
        for table in &workspace.tables.tabs {
            if let Some(file) = &table.fileobject {
                if !files.iter().any(|f| Arc::ptr_eq(f, file)) {
                    files.push(file.clone());
                }
            }
        }
        if files.len() == self.files.len()
            && files
                .iter()
                .zip(&self.files)
                .all(|(a, b)| Arc::ptr_eq(a, b))
        {
            return;
        }
        self.fields.clear();
        for file in &files {
            match file.to_2d_fields() {
                Some(Ok(fields)) => self.fields.extend(fields.into_iter().map(|f| {
                    (
                        file.filename()
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        f,
                    )
                })),
                Some(Err(error)) => workspace.problems.push(format!(
                    "Could not read fields from {}: {error}",
                    file.filename().display()
                )),
                None => {}
            }
        }
        self.files = files;
        self.index = 0;
        self.texture = None;
        self.image = None;
        self.error = None;
    }
    pub(crate) fn available(&self) -> bool {
        !self.fields.is_empty()
    }

    fn rebuild(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let field = &self.fields[self.index].1;
        validate(field)?;
        let range = if self.range_mode == RangeMode::Manual {
            (self.min, self.max)
        } else {
            let mut range = (f64::INFINITY, f64::NEG_INFINITY);
            for (_, f) in self
                .fields
                .iter()
                .filter(|(_, f)| self.range_mode == RangeMode::Common || std::ptr::eq(f, field))
            {
                for &v in &f.m {
                    if v.is_finite() {
                        range.0 = range.0.min(v);
                        range.1 = range.1.max(v);
                    }
                }
            }
            range
        };
        if !range.0.is_finite() || !range.1.is_finite() || range.0 > range.1 {
            return Err(
                "Color range must be finite and increasing; the field may have no finite values."
                    .into(),
            );
        }
        let palette: Vec<_> = dv_tools::colors::cmap_colors(256, self.map)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|c| {
                Color32::from_rgb(
                    (c[0] * 255.0) as u8,
                    (c[1] * 255.0) as u8,
                    (c[2] * 255.0) as u8,
                )
            })
            .collect();
        let bounds = bounds(field, self.polar);
        let size = if self.polar {
            [512, 512]
        } else {
            [field.x.len().clamp(2, 1024), field.y.len().clamp(2, 1024)]
        };
        let mut pixels = Vec::with_capacity(size[0] * size[1]);
        for j in 0..size[1] {
            for i in 0..size[0] {
                let x = bounds[0] + (bounds[1] - bounds[0]) * (i as f64 + 0.5) / size[0] as f64;
                let y = bounds[3] - (bounds[3] - bounds[2]) * (j as f64 + 0.5) / size[1] as f64;
                let cell = if self.polar {
                    polar_cell(field, x, y, self.degrees)
                } else {
                    nearest(&field.x, x).zip(nearest(&field.y, y))
                };
                let color = cell
                    .map(|(a, b)| field.value(a, b))
                    .filter(|v| v.is_finite())
                    .map_or(Color32::TRANSPARENT, |v| {
                        let p = if range.0 == range.1 {
                            0.5
                        } else {
                            ((v - range.0) / (range.1 - range.0)).clamp(0.0, 1.0)
                        };
                        palette[(p * 255.0).round() as usize]
                    });
                pixels.push(color);
            }
        }
        let image = egui::ColorImage::new(size, pixels);
        self.texture =
            Some(ctx.load_texture("scalar-field", image.clone(), egui::TextureOptions::NEAREST));
        self.image = Some(image);
        self.palette = palette;
        self.range = range;
        Ok(())
    }

    pub(crate) fn show(&mut self, ui: &mut egui::Ui) -> Option<String> {
        if !self.available() {
            ui.label("No 2D fields are available.");
            return None;
        }
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Field");
            egui::ComboBox::from_id_salt("field-picker")
                .selected_text(format!(
                    "{} · {}",
                    self.fields[self.index].0, self.fields[self.index].1.name
                ))
                .show_ui(ui, |ui| {
                    for (i, (source, field)) in self.fields.iter().enumerate() {
                        changed |= ui
                            .selectable_value(
                                &mut self.index,
                                i,
                                format!("{source} · {}", field.name),
                            )
                            .changed();
                    }
                });
            ui.label("Color map");
            egui::ComboBox::from_id_salt("field-map")
                .selected_text(self.map)
                .show_ui(ui, |ui| {
                    for &map in MAPS {
                        changed |= ui.selectable_value(&mut self.map, map, map).changed();
                    }
                });
            ui.label("Range");
            egui::ComboBox::from_id_salt("field-range")
                .selected_text(match self.range_mode {
                    RangeMode::Automatic => "Automatic",
                    RangeMode::Common => "Common",
                    RangeMode::Manual => "Manual",
                })
                .show_ui(ui, |ui| {
                    changed |= ui
                        .selectable_value(&mut self.range_mode, RangeMode::Automatic, "Automatic")
                        .changed();
                    changed |= ui
                        .selectable_value(
                            &mut self.range_mode,
                            RangeMode::Common,
                            "Common to all fields",
                        )
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.range_mode, RangeMode::Manual, "Manual")
                        .changed();
                });
            if self.range_mode == RangeMode::Manual {
                changed |= ui
                    .add(egui::DragValue::new(&mut self.min).prefix("Min "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut self.max).prefix("Max "))
                    .changed();
            }
            changed |= ui
                .checkbox(&mut self.polar, "Polar")
                .on_hover_text("X is angle; Y is radius")
                .changed();
            if self.polar {
                egui::ComboBox::from_id_salt("angle-unit")
                    .selected_text(if self.degrees { "Degrees" } else { "Radians" })
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut self.degrees, true, "Degrees")
                            .changed();
                        changed |= ui
                            .selectable_value(&mut self.degrees, false, "Radians")
                            .changed();
                    });
            }
        });
        if changed {
            self.texture = None;
            self.image = None;
            self.error = None;
        }
        if self.texture.is_none() && self.error.is_none() {
            if let Err(error) = self.rebuild(ui.ctx()) {
                self.error = Some(error);
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
            return None;
        }
        let field = &self.fields[self.index].1;
        let mut result = None;
        ui.horizontal(|ui| {
            if ui.button("Export matrix CSV…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("CSV", &["csv"])
                    .set_file_name("field.csv")
                    .save_file()
                {
                    result = Some(match export_csv(field, &path) {
                        Ok(()) => "Field matrix exported".into(),
                        Err(e) => format!("Could not export field matrix: {e}"),
                    });
                }
            }
            if ui.button("Export image PNG…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("PNG", &["png"])
                    .set_file_name("field.png")
                    .save_file()
                {
                    if let Some(image) = &self.image {
                        result = Some(match export_png(image, &path) {
                            Ok(()) => "Field image exported".into(),
                            Err(e) => format!("Could not export field image: {e}"),
                        });
                    }
                }
            }
        });
        let size = Vec2::new(
            ui.available_width().max(250.0),
            ui.available_height().max(200.0),
        );
        let (outer, response) = ui.allocate_exact_size(size, Sense::hover());
        let mut plot = Rect::from_min_max(
            outer.min + Vec2::new(80.0, 20.0),
            outer.max - Vec2::new(125.0, 65.0),
        );
        if self.polar {
            let side = plot.width().min(plot.height());
            plot = Rect::from_center_size(plot.center(), Vec2::splat(side));
        }
        let painter = ui.painter();
        let text = ui.visuals().weak_text_color();
        let font = FontId::proportional(11.5);
        let b = bounds(field, self.polar);
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                plot,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        for tick in dv_chart::ticks::nice_ticks(b[0], b[1], 6) {
            let x = plot.left() + ((tick - b[0]) / (b[1] - b[0])) as f32 * plot.width();
            painter.line_segment(
                [
                    Pos2::new(x, plot.bottom()),
                    Pos2::new(x, plot.bottom() + 4.0),
                ],
                Stroke::new(1.0, text),
            );
            painter.text(
                Pos2::new(x, plot.bottom() + 7.0),
                Align2::CENTER_TOP,
                dv_chart::format::format_value(tick, "", 4),
                font.clone(),
                text,
            );
        }
        for tick in dv_chart::ticks::nice_ticks(b[2], b[3], 6) {
            let y = plot.bottom() - ((tick - b[2]) / (b[3] - b[2])) as f32 * plot.height();
            painter.line_segment(
                [Pos2::new(plot.left() - 4.0, y), Pos2::new(plot.left(), y)],
                Stroke::new(1.0, text),
            );
            painter.text(
                Pos2::new(plot.left() - 7.0, y),
                Align2::RIGHT_CENTER,
                dv_chart::format::format_value(tick, "", 4),
                font.clone(),
                text,
            );
        }
        painter.text(
            Pos2::new(plot.center().x, plot.bottom() + 30.0),
            Align2::CENTER_TOP,
            if self.polar { "X" } else { &field.sx },
            font.clone(),
            text,
        );
        painter.text(
            Pos2::new(plot.left(), plot.top() - 4.0),
            Align2::LEFT_BOTTOM,
            if self.polar { "Y" } else { &field.sy },
            font.clone(),
            text,
        );
        let bar = Rect::from_min_size(
            Pos2::new(plot.right() + 24.0, plot.top()),
            Vec2::new(18.0, plot.height()),
        );
        for j in 0..256 {
            let y0 = bar.top() + bar.height() * j as f32 / 256.0;
            let y1 = bar.top() + bar.height() * (j + 1) as f32 / 256.0;
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(bar.left(), y0), Pos2::new(bar.right(), y1)),
                0.0,
                self.palette[255 - j],
            );
        }
        for tick in dv_chart::ticks::nice_ticks(self.range.0, self.range.1, 6) {
            let p = if self.range.0 == self.range.1 {
                0.5
            } else {
                (tick - self.range.0) / (self.range.1 - self.range.0)
            };
            let y = bar.bottom() - p as f32 * bar.height();
            painter.text(
                Pos2::new(bar.right() + 5.0, y),
                Align2::LEFT_CENTER,
                dv_chart::format::format_value(tick, "", 4),
                font.clone(),
                text,
            );
        }
        if let Some(pos) = response.hover_pos().filter(|p| plot.contains(*p)) {
            let x = b[0] + (pos.x - plot.left()) as f64 / plot.width() as f64 * (b[1] - b[0]);
            let y = b[3] - (pos.y - plot.top()) as f64 / plot.height() as f64 * (b[3] - b[2]);
            let cell = if self.polar {
                polar_cell(field, x, y, self.degrees)
            } else {
                nearest(&field.x, x).zip(nearest(&field.y, y))
            };
            if let Some((i, j)) = cell {
                response.on_hover_text(format!(
                    "{}: {}\n{}: {}\n{}: {}",
                    field.sx,
                    dv_chart::format::format_value(field.x[i], "", 5),
                    field.sy,
                    dv_chart::format::format_value(field.y[j], "", 5),
                    field.name,
                    dv_chart::format::format_value(field.value(i, j), "", 5)
                ));
            }
        }
        result
    }
}

fn validate(field: &Field2D) -> Result<(), String> {
    if field.x.is_empty()
        || field.y.is_empty()
        || field.x.len().checked_mul(field.y.len()) != Some(field.m.len())
    {
        return Err("Invalid field dimensions.".into());
    }
    let ordered = |a: &[f64]| {
        a.iter().all(|v| v.is_finite())
            && (a.windows(2).all(|v| v[0] <= v[1]) || a.windows(2).all(|v| v[0] >= v[1]))
    };
    if !ordered(&field.x) || !ordered(&field.y) {
        return Err("Field coordinates must be finite and monotonic.".into());
    }
    Ok(())
}
fn bounds(field: &Field2D, polar: bool) -> [f64; 4] {
    if polar {
        let r = field.y[0]
            .abs()
            .max(field.y[field.y.len() - 1].abs())
            .max(f64::EPSILON);
        [-r, r, -r, r]
    } else {
        let expand = |a: &[f64]| {
            let lo = a[0].min(a[a.len() - 1]);
            let hi = a[0].max(a[a.len() - 1]);
            if lo == hi {
                let d = lo.abs().max(1.0) * 0.5;
                (lo - d, hi + d)
            } else {
                (lo, hi)
            }
        };
        let (x0, x1) = expand(&field.x);
        let (y0, y1) = expand(&field.y);
        [x0, x1, y0, y1]
    }
}
fn nearest(axis: &[f64], value: f64) -> Option<usize> {
    if axis.is_empty() || !value.is_finite() {
        return None;
    }
    let ascending = axis[0] <= axis[axis.len() - 1];
    let next = axis.partition_point(|&x| if ascending { x < value } else { x > value });
    if next == 0 {
        Some(0)
    } else if next == axis.len() {
        Some(next - 1)
    } else if (axis[next - 1] - value).abs() <= (axis[next] - value).abs() {
        Some(next - 1)
    } else {
        Some(next)
    }
}
fn polar_cell(field: &Field2D, x: f64, y: f64, degrees: bool) -> Option<(usize, usize)> {
    let r = x.hypot(y);
    let rmin = field.y.first()?.min(*field.y.last()?);
    let rmax = field.y.first()?.max(*field.y.last()?);
    if r < rmin || r > rmax {
        return None;
    }
    let mut angle = y.atan2(x);
    if degrees {
        angle = angle.to_degrees();
    }
    let period = if degrees {
        360.0
    } else {
        std::f64::consts::TAU
    };
    let amin = field.x.first()?.min(*field.x.last()?);
    let amax = field.x.first()?.max(*field.x.last()?);
    angle = (angle - amin).rem_euclid(period) + amin;
    if angle > amax {
        return None;
    }
    Some((nearest(&field.x, angle)?, nearest(&field.y, r)?))
}
fn export_csv(field: &Field2D, path: &std::path::Path) -> Result<(), String> {
    let mut columns = vec![dv_core::Column::float(&field.sx, field.x.clone())];
    for (j, y) in field.y.iter().enumerate() {
        columns.push(dv_core::Column::float(
            format!("{}={y}", field.sy),
            (0..field.x.len()).map(|i| field.value(i, j)).collect(),
        ));
    }
    let df = dv_core::DataFrame::from_columns(columns).map_err(|e| e.to_string())?;
    dv_io::write_dataframe(&df, path, Some("csv")).map_err(|e| e.to_string())
}
fn export_png(image: &egui::ColorImage, path: &std::path::Path) -> Result<(), String> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(image.size[0] as u32, image.size[1] as u32)
        .ok_or("Invalid image size")?;
    for (dst, src) in pixmap.data_mut().chunks_exact_mut(4).zip(&image.pixels) {
        dst.copy_from_slice(&src.to_array());
    }
    pixmap.save_png(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polar_geometry_masks_unmeasured_sector_and_radius() {
        let field = Field2D {
            name: "v".into(),
            sx: "deg".into(),
            sy: "r".into(),
            x: vec![0., 45., 90.],
            y: vec![1., 2.],
            m: vec![1.; 6],
        };
        assert_eq!(polar_cell(&field, 0., 1.5, true), Some((2, 0)));
        assert_eq!(polar_cell(&field, -1.5, 0., true), None);
        assert_eq!(polar_cell(&field, 0., 0., true), None);
        assert_eq!(polar_cell(&field, 3., 0., true), None);
    }
    #[test]
    fn nearest_handles_descending_and_edges() {
        assert_eq!(nearest(&[4., 2., 0.], 3.2), Some(0));
        assert_eq!(nearest(&[4., 2., 0.], -1.), Some(2));
        assert_eq!(nearest(&[], 0.), None);
    }
    #[test]
    fn csv_export_preserves_field_orientation() {
        let field = Field2D {
            name: "value".into(),
            sx: "x".into(),
            sy: "y".into(),
            x: vec![1.0, 2.0],
            y: vec![3.0, 4.0],
            m: vec![10.0, 11.0, 20.0, 21.0],
        };
        let path = std::env::temp_dir().join(format!("datview-fields-{}.csv", std::process::id()));
        export_csv(&field, &path).expect("write matrix");
        let csv = std::fs::read_to_string(&path).expect("read matrix");
        std::fs::remove_file(path).expect("remove matrix");
        let rows: Vec<_> = csv.lines().collect();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].contains("y=3") && rows[0].contains("y=4"));
        let numbers = |line: &str| {
            line.split(',')
                .map(|s| s.parse::<f64>().expect("number"))
                .collect::<Vec<_>>()
        };
        assert_eq!(numbers(rows[1]), vec![1.0, 10.0, 11.0]);
        assert_eq!(numbers(rows[2]), vec![2.0, 20.0, 21.0]);
    }
}
