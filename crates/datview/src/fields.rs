use super::*;
use egui_plot::{Plot, PlotImage, PlotPoint};

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct FieldStyle {
    pub cmap: String,
    pub common_range: bool,
    pub manual_range: bool,
    pub min: f64,
    pub max: f64,
    pub polar: bool,
    pub degrees: bool,
}
impl Default for FieldStyle {
    fn default() -> Self {
        Self {
            cmap: "viridis".into(),
            common_range: false,
            manual_range: false,
            min: 0.0,
            max: 1.0,
            polar: false,
            degrees: true,
        }
    }
}

pub(super) struct FieldRaster {
    range: (f64, f64),
    bounds: [f64; 4],
    palette: Vec<Color32>,
}
impl FieldRaster {
    fn new(fields: &[dv_core::Field2D], index: usize, style: &FieldStyle) -> Result<Self, String> {
        let field = &fields[index];
        if field.x.is_empty()
            || field.y.is_empty()
            || field.x.len().checked_mul(field.y.len()) != Some(field.m.len())
        {
            return Err("Invalid field dimensions".into());
        }
        let ordered = |axis: &[f64]| {
            axis.iter().all(|v| v.is_finite())
                && (axis.windows(2).all(|v| v[0] <= v[1]) || axis.windows(2).all(|v| v[0] >= v[1]))
        };
        if !ordered(&field.x) || !ordered(&field.y) {
            return Err("Field coordinates must be finite and monotonic.".into());
        }
        let mut range = (f64::INFINITY, f64::NEG_INFINITY);
        if style.manual_range {
            range = (style.min, style.max);
        } else {
            for f in fields
                .iter()
                .filter(|f| style.common_range || std::ptr::eq(*f, field))
            {
                for &v in &f.m {
                    if v.is_finite() {
                        range.0 = range.0.min(v);
                        range.1 = range.1.max(v);
                    }
                }
            }
        }
        if !range.0.is_finite() || !range.1.is_finite() || range.0 > range.1 {
            return Err(
                "Color range must be finite and increasing; field may contain no finite values."
                    .into(),
            );
        }
        let palette = dv_tools::colors::cmap_colors(256, &style.cmap)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|rgb| {
                Color32::from_rgb(
                    (rgb[0] * 255.) as u8,
                    (rgb[1] * 255.) as u8,
                    (rgb[2] * 255.) as u8,
                )
            })
            .collect();
        let bounds = [
            field.x[0].min(field.x[field.x.len() - 1]),
            field.x[0].max(field.x[field.x.len() - 1]),
            field.y[0].min(field.y[field.y.len() - 1]),
            field.y[0].max(field.y[field.y.len() - 1]),
        ];
        Ok(Self {
            range,
            bounds,
            palette,
        })
    }
}
fn nearest(axis: &[f64], value: f64) -> Option<usize> {
    if axis.is_empty() || !value.is_finite() {
        return None;
    }
    let ascending = axis[0] <= axis[axis.len() - 1];
    let next = axis.partition_point(|&x| if ascending { x < value } else { x > value });
    if next == 0 {
        return Some(0);
    }
    if next == axis.len() {
        return Some(next - 1);
    }
    Some(
        if (axis[next - 1] - value).abs() <= (axis[next] - value).abs() {
            next - 1
        } else {
            next
        },
    )
}
/// Inverse Cartesian mapping for polar rasters. Do not extend incomplete
/// angular sectors or paint values outside the measured radial range.
fn polar_cell(field: &dv_core::Field2D, x: f64, y: f64, degrees: bool) -> Option<(usize, usize)> {
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
impl DatView {
    pub(super) fn fields_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui.button("Extract 2D fields from selected files").clicked() {
            self.fields.clear();
            self.field_texture = None;
            let mut files = Vec::new();
            for i in self.selected() {
                let t = &self.tables.tabs[i];
                if files.contains(&t.filename) {
                    continue;
                }
                files.push(t.filename.clone());
                if let Some(fo) = &t.fileobject {
                    match fo.to_2d_fields() {
                        Some(Ok(mut fields)) => self.fields.append(&mut fields),
                        Some(Err(e)) => self.errors.push(e.to_string()),
                        None => self
                            .errors
                            .push(format!("{} does not provide 2D fields", fo.format_name())),
                    }
                }
            }
            self.field_index = 0;
        }
        if self.fields.is_empty() {
            ui.label("Extract available scalar fields from selected files.");
            return;
        }
        self.field_index = self.field_index.min(self.fields.len() - 1);
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("field")
                .selected_text(&self.fields[self.field_index].name)
                .show_ui(ui, |ui| {
                    for (i, field) in self.fields.iter().enumerate() {
                        changed |= ui
                            .selectable_value(&mut self.field_index, i, &field.name)
                            .changed();
                    }
                });
            let s = &mut self.options.field_style;
            changed |= combo(
                ui,
                "Color map",
                &mut s.cmap,
                &[
                    "viridis",
                    "plasma",
                    "inferno",
                    "magma",
                    "cividis",
                    "coolwarm",
                    "RdBu",
                    "jet",
                    "gray",
                    "viridis_r",
                ],
            );
            changed |= ui
                .checkbox(&mut s.common_range, "Common color range")
                .changed();
            changed |= ui.checkbox(&mut s.manual_range, "Manual range").changed();
            if s.manual_range {
                changed |= ui
                    .add(egui::DragValue::new(&mut s.min).prefix("Min "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut s.max).prefix("Max "))
                    .changed();
            }
            changed |= ui
                .checkbox(&mut s.polar, "Polar (X=angle, Y=radius)")
                .changed();
            if s.polar {
                changed |= ui.checkbox(&mut s.degrees, "Degrees").changed();
            }
        });
        if changed {
            self.field_texture = None;
        }
        let s = &self.options.field_style;
        let field = &self.fields[self.field_index];
        if self.field_texture.is_none() {
            match FieldRaster::new(&self.fields, self.field_index, s) {
                Ok(raster) => self.field_raster = Some(raster),
                Err(e) => {
                    ui.colored_label(Color32::RED, e);
                    return;
                }
            }
        }
        let raster = self
            .field_raster
            .as_ref()
            .expect("raster parameters exist before texture creation");
        let (min, max) = raster.range;
        let [xmin, xmax, ymin, ymax] = raster.bounds;
        let palette = &raster.palette;
        let color = |value: f64| {
            if !value.is_finite() {
                Color32::TRANSPARENT
            } else {
                let p = if max == min {
                    0.5
                } else {
                    ((value - min) / (max - min)).clamp(0., 1.)
                };
                palette[(p * 255.).round() as usize]
            }
        };
        if self.field_texture.is_none() {
            let (size, pixels) = if s.polar {
                let n = 512usize;
                let pixels = (0..n)
                    .flat_map(|j| (0..n).map(move |i| (i, j)))
                    .map(|(i, j)| {
                        let x = ((i as f64 + 0.5) / n as f64 * 2. - 1.) * ymax;
                        let y = (1. - (j as f64 + 0.5) / n as f64 * 2.) * ymax;
                        polar_cell(field, x, y, s.degrees)
                            .map_or(Color32::TRANSPARENT, |(i, j)| color(field.value(i, j)))
                    })
                    .collect();
                ([n, n], pixels)
            } else {
                let nx = field.x.len().min(1024).max(2);
                let ny = field.y.len().min(1024).max(2);
                let pixels = (0..ny)
                    .flat_map(|j| (0..nx).map(move |i| (i, j)))
                    .map(|(i, j)| {
                        let x = xmin + (xmax - xmin) * i as f64 / (nx - 1) as f64;
                        let y = ymax - (ymax - ymin) * j as f64 / (ny - 1) as f64;
                        let (a, b) = (nearest(&field.x, x).unwrap(), nearest(&field.y, y).unwrap());
                        color(field.value(a, b))
                    })
                    .collect();
                ([nx, ny], pixels)
            };
            self.field_texture = Some(ctx.load_texture(
                "scalar-field",
                egui::ColorImage { size, pixels },
                egui::TextureOptions::NEAREST,
            ));
        }
        let height = (ui.available_height() - 70.).max(180.);
        ui.horizontal(|ui| {
            if let Some(texture) = &self.field_texture {
                let (center, size) = if s.polar {
                    (
                        PlotPoint::new(0., 0.),
                        egui::vec2((2. * ymax) as f32, (2. * ymax) as f32),
                    )
                } else {
                    (
                        PlotPoint::new((xmin + xmax) / 2., (ymin + ymax) / 2.),
                        egui::vec2(
                            (xmax - xmin).max(f64::EPSILON) as f32,
                            (ymax - ymin).max(f64::EPSILON) as f32,
                        ),
                    )
                };
                let mut plot = Plot::new("scalar-field-plot")
                    .height(height)
                    .width((ui.available_width() - 95.).max(100.))
                    .x_axis_label(if s.polar { "X" } else { &field.sx })
                    .y_axis_label(if s.polar { "Y" } else { &field.sy });
                if s.polar {
                    plot = plot.data_aspect(1.);
                }
                plot.show(ui, |p| {
                    p.image(PlotImage::new(&field.name, texture.id(), center, size))
                });
            }
            ui.vertical(|ui| {
                ui.label(format!("{max:.5}"));
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(24., (height - 55.).max(60.)),
                    egui::Sense::hover(),
                );
                for j in 0..256 {
                    let top = rect.top() + rect.height() * j as f32 / 256.;
                    let bottom = rect.top() + rect.height() * (j + 1) as f32 / 256.;
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left(), top),
                            egui::pos2(rect.right(), bottom),
                        ),
                        0.,
                        palette[255 - j],
                    );
                }
                ui.label(format!("{min:.5}"));
            });
        });
        ui.label(&field.name);
        if ui.button("Export field matrix CSV…").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("field.csv")
                .save_file()
            {
                let mut columns = vec![dv_core::Column::float(&field.sx, field.x.clone())];
                for (j, y) in field.y.iter().enumerate() {
                    columns.push(dv_core::Column::float(
                        format!("{}={y}", field.sy),
                        (0..field.x.len()).map(|i| field.value(i, j)).collect(),
                    ));
                }
                match dv_core::DataFrame::from_columns(columns)
                    .map_err(|e| e.to_string())
                    .and_then(|df| {
                        dv_io::write_dataframe(&df, &path, Some("csv")).map_err(|e| e.to_string())
                    }) {
                    Ok(()) => self.status = "Field exported".into(),
                    Err(e) => self.error(e),
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polar_geometry_masks_unmeasured_sector_and_radius() {
        let field = dv_core::Field2D {
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
}
