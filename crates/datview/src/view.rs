use crate::app::*;
use crate::export::{self, Curve};
use dv_plot::{FftOptions, MinMaxOptions, PdfOptions, PlotType, PolarOptions};
use eframe::egui::{self, Color32};
use egui_plot::{Legend, Line, Plot, PlotBounds, PlotPoints, Points};

impl DatView {
    fn plot_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let old = self.options.mode.clone();
            for name in ["Regular", "PDF", "FFT", "MinMax", "Compare", "Polar"] {
                ui.selectable_value(&mut self.options.mode, name.into(), name);
            }
            if old != self.options.mode {
                self.plot_type = mode_type(&self.options.mode);
                self.options.plot_options =
                    serde_json::to_value(&self.plot_type).unwrap_or_default();
                self.overlays.clear();
                self.dirty = true;
                self.presentation.pending=None;
                self.presentation.tool=presentation::Tool::Inspect;
            }
            ui.separator();
            let s = &mut self.options.style;
            ui.menu_button("Axes & lines",|ui|{
            for (value, label) in [
                (&mut s.log_x, "Log X"),
                (&mut s.log_y, "Log Y"),
                (&mut s.flip_x, "Flip X"),
                (&mut s.flip_y, "Flip Y"),
                (&mut s.subplots, "Subplots"),
                (&mut s.share_x, "Share X"),
                (&mut s.grid, "Grid"),
                (&mut s.legend, "Legend"),
                (&mut s.lines, "Lines"),
                (&mut s.markers, "Points"),
                (&mut s.step, "Step"),
            ] {
                ui.checkbox(value, label);
            }
            });
            if ui.selectable_label(self.presentation.open,"Plot studio").clicked(){self.presentation.open=!self.presentation.open;}
        });
        if self.options.mode == "Compare" {
            ui.horizontal(|ui| {
                self.dirty |= combo(
                    ui,
                    "comparison mode",
                    &mut self.compare_mode,
                    &[
                        "TablesOneColumn",
                        "OneTableColumns",
                        "TablesSameColumns",
                        "TablesSimilarColumns",
                    ],
                );
                self.dirty |= combo(
                    ui,
                    "comparison method",
                    &mut self.compare_method,
                    &["Relative", "AbsRelative", "Ratio", "Absolute", "YY"],
                );
            });
        }
        let before = self.options.plot_options.clone();
        if self.options.mode != "Compare" && self.options.mode != "Regular" {
            egui::CollapsingHeader::new("Mode options")
                .default_open(true)
                .show(ui, |ui| {
                    json_edit(ui, &mut self.options.plot_options);
                });
        }
        if before != self.options.plot_options {
            match serde_json::from_value(self.options.plot_options.clone()) {
                Ok(t) => {
                    self.plot_type = t;
                    self.dirty = true;
                }
                Err(e) => self.error(e),
            }
        }
        ui.horizontal_wrapped(|ui|{
            ui.label("Limits: xmin,xmax,ymin,ymax (blank = auto)");ui.add(egui::TextEdit::singleline(&mut self.limits_text).desired_width(180.0));
            if ui.button("Set").clicked(){
                if self.limits_text.trim().is_empty(){self.options.style.limits=None;}
                else {let nums=self.limits_text.split(',').map(|x|x.trim().parse::<f64>()).collect::<Result<Vec<_>,_>>();
                    match nums {Ok(v) if v.len()==4&&v.iter().all(|x|x.is_finite())&&v[0]<v[1]&&v[2]<v[3]&&(!self.options.style.log_x||v[0]>0.0)&&(!self.options.style.log_y||v[2]>0.0)=>self.options.style.limits=Some([v[0],v[1],v[2],v[3]]),_=>self.error("Enter four finite, increasing bounds; logarithmic limits must be positive")}
                }
            }
            if ui.button("Auto").clicked(){self.options.style.limits=None;self.limits_text.clear();}
            if ui.button("Style…").clicked(){self.editor=Editor::Settings;}
            if ui.button("Save figure…").clicked(){self.figure_export();}
            if self.presentation.tool!=presentation::Tool::Inspect {
                ui.colored_label(Color32::from_rgb(35,112,191),match self.presentation.tool{presentation::Tool::Point=>"Click a sample to annotate",_=>if self.presentation.pending.is_some(){"Click region end · Esc cancels"}else{"Click region start · Esc cancels"}});
            }
        });
    }
    pub(super) fn draw_plot(&mut self, ui: &mut egui::Ui) {
        self.plot_controls(ui);
        if self.dirty && self.options.live {
            self.rebuild();
        }
        if self.curves.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Open data, select tables and Y columns, then press Plot.");
            });
            return;
        }
        let s = &self.options.style;
        if !s.title.is_empty() {
            ui.heading(&s.title);
        }
        let key = (s.log_x, s.log_y, s.flip_x, s.flip_y, s.step);
        if self.vertex_style != Some(key)
            || self.vertices.len() != self.curves.len()
            || self.overlay_vertices.len() != self.overlays.len()
        {
            let prepare = |c: &Curve| {
                export::display_points(c, s)
                    .into_iter()
                    .map(|p| {
                        egui_plot::PlotPoint::new(
                            if s.flip_x { -p[0] } else { p[0] },
                            if s.flip_y { -p[1] } else { p[1] },
                        )
                    })
                    .collect()
            };
            self.vertices = self.curves.iter().map(&prepare).collect();
            self.overlay_vertices = self.overlays.iter().map(prepare).collect();
            self.vertex_style = Some(key);
        }
        let count = if s.subplots { self.curves.len() } else { 1 };
        let height = ((ui.available_height() - 150.0) / count as f32).max(160.0);
        let measurements = &mut self.measurements;
        let measure_enabled = self.measure_enabled;
        egui::ScrollArea::vertical()
            .id_salt("plots")
            .show(ui, |ui| {
                for panel in 0..count {
                    let curves = if s.subplots {
                        &self.curves[panel..panel + 1]
                    } else {
                        &self.curves[..]
                    };
                    let sx = if s.x_label.is_empty() {
                        &curves[0].sx
                    } else {
                        &s.x_label
                    };
                    let sy = if s.y_label.is_empty() {
                        &curves[0].sy
                    } else {
                        &s.y_label
                    };
                    let mut plot = Plot::new(("plot", panel, s.log_x, s.log_y, s.flip_x, s.flip_y))
                        .height(height)
                        .show_grid(s.grid)
                        .x_axis_label(sx)
                        .y_axis_label(sy);
                    if s.legend {
                        plot = plot.legend(Legend::default());
                    }
                    if s.share_x && s.subplots {
                        plot = plot.link_axis(egui::Id::new("shared-x"), [true, false]);
                    }
                    if self.options.mode == "Polar" {
                        plot = plot.data_aspect(1.0);
                    }
                    let (logx, logy, flipx, flipy) = (s.log_x, s.log_y, s.flip_x, s.flip_y);
                    let xdata = self
                        .plotted
                        .get(if s.subplots { panel } else { 0 })
                        .map(|p| &p.x);
                    let ydata = self
                        .plotted
                        .get(if s.subplots { panel } else { 0 })
                        .map(|p| &p.y);
                    plot = plot.x_axis_formatter(move |mark, _| {
                        axis_label(mark.value, logx, flipx, xdata)
                    });
                    plot = plot.y_axis_formatter(move |mark, _| {
                        axis_label(mark.value, logy, flipy, ydata)
                    });
                    plot = plot.label_formatter(move |name, p| {
                        format!(
                            "{name}\nX: {}\nY: {}",
                            axis_label(p.x, logx, flipx, xdata),
                            axis_label(p.y, logy, flipy, ydata)
                        )
                    });
                    let mut click = None;
                    let response = plot.show(ui, |plot_ui| {
                        if s.limits.is_some() {
                            let b = export::bounds(curves, s);
                            if b.iter().all(|v| v.is_finite()) && b[0] < b[1] && b[2] < b[3] {
                                let xmin = if flipx { -b[1] } else { b[0] };
                                let xmax = if flipx { -b[0] } else { b[1] };
                                let ymin = if flipy { -b[3] } else { b[2] };
                                let ymax = if flipy { -b[2] } else { b[3] };
                                plot_ui.set_plot_bounds(PlotBounds::from_min_max(
                                    [xmin, ymin],
                                    [xmax, ymax],
                                ));
                            }
                        }
                        for (j, c) in curves.iter().enumerate() {
                            let index = if s.subplots { panel } else { j };
                            let rgb = export::color(index);
                            let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                            let pts = &self.vertices[index];
                            if s.lines {
                                plot_ui.line(
                                    Line::new(&c.name, PlotPoints::Borrowed(pts))
                                        .color(color)
                                        .width(s.width),
                                );
                            }
                            if s.markers {
                                plot_ui.points(
                                    Points::new(&c.name, PlotPoints::Borrowed(pts))
                                        .color(color)
                                        .radius(s.marker_size),
                                );
                            }
                        }
                        for (i, c) in self.overlays.iter().enumerate() {
                            if !s.subplots || c.sx == curves[0].sx && c.sy == curves[0].sy {
                                plot_ui.line(
                                    Line::new(
                                        &c.name,
                                        PlotPoints::Borrowed(&self.overlay_vertices[i]),
                                    )
                                    .color(Color32::RED)
                                    .width(s.width + 1.0),
                                );
                            }
                        }
                        for (j, p) in measurements.iter().enumerate() {
                            let mut q = [export::axis(p[0], logx), export::axis(p[1], logy)];
                            if flipx {
                                q[0] = -q[0];
                            }
                            if flipy {
                                q[1] = -q[1];
                            }
                            plot_ui.points(
                                Points::new(format!("M{}", j + 1), vec![q])
                                    .radius(6.0_f32)
                                    .color(Color32::BLACK),
                            );
                        }
                        if measure_enabled {
                            click = plot_ui.pointer_coordinate();
                        }
                    });
                    self.presentation.paint(ui,&response.transform,curves,s,&self.options.mode);
                    self.presentation.click(&response,curves,s,&self.options.mode);
                    if measure_enabled && self.presentation.tool==presentation::Tool::Inspect && response.response.clicked() {
                        if let Some(p) = click {
                            let x = if logx {
                                10f64.powf(if flipx { -p.x } else { p.x })
                            } else if flipx {
                                -p.x
                            } else {
                                p.x
                            };
                            let y = if logy {
                                10f64.powf(if flipy { -p.y } else { p.y })
                            } else if flipy {
                                -p.y
                            } else {
                                p.y
                            };
                            let pd = self.plotted.get(if s.subplots {
                                panel
                            } else {
                                self.tool_curve.min(self.plotted.len().saturating_sub(1))
                            });
                            if let Some(pd) = pd {
                                if let Ok(m) = pd.measure(x, y) {
                                    if measurements.len() == 2 {
                                        measurements.clear();
                                    }
                                    measurements.push([m.x, m.y]);
                                }
                            }
                        }
                    }
                }
                if measurements.len() == 2 {
                    let dx = measurements[1][0] - measurements[0][0];
                    let dy = measurements[1][1] - measurements[0][1];
                    ui.label(format!("Δx={dx:.6}   Δy={dy:.6}   slope={:.6}", dy / dx));
                    if ui.button("Statistics in measurement interval").clicked() {
                        self.tool_result = self
                            .plotted
                            .iter()
                            .map(|p| {
                                match p.measurement_stats(measurements[0][0], measurements[1][0]) {
                                    Ok(stats) => format!("{} — {}\n{stats:#?}", p.st, p.sy),
                                    Err(e) => e.to_string(),
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        self.editor = Editor::Damping;
                    }
                }
                egui::CollapsingHeader::new("Statistics")
                    .default_open(true)
                    .show(ui, |ui| {
                        if self.presentation.rules.iter().any(|r|r.enabled&&r.mode==self.options.mode){ui.weak("Source statistics — before non-destructive display filters.");}
                        ui.menu_button("Displayed statistics", |ui| {
                            for &name in INFO_COLUMNS {
                                let mut selected =
                                    self.options.info_columns.iter().any(|s| s == name);
                                if ui.checkbox(&mut selected, name).changed() {
                                    if selected {
                                        self.options.info_columns.push(name.into());
                                    } else {
                                        self.options.info_columns.retain(|s| s != name);
                                    }
                                }
                            }
                        });
                        egui::ScrollArea::horizontal().show(ui, |ui| {
                            egui::Grid::new("stats").striped(true).show(ui, |ui| {
                                ui.strong("Curve");
                                for title in &self.options.info_columns {
                                    ui.strong(title);
                                }
                                ui.end_row();
                                for p in &self.plotted {
                                    ui.label(format!("{} — {}", p.st, p.sy));
                                    for field in &self.options.info_columns {
                                        ui.label(dv_core::common::pretty_num(info_value(p, field)));
                                    }
                                    ui.end_row();
                                }
                            });
                        });
                        if ui.button("Full statistics").clicked() {
                            self.tool_result = self
                                .plotted
                                .iter()
                                .map(|p| {
                                    format!(
                                        "{} — {}\nCurrent: {:#?}\nOriginal: {:#?}",
                                        p.st,
                                        p.sy,
                                        p.stats(),
                                        p.original_stats()
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.editor = Editor::Damping;
                        }
                    });
            });
    }
    pub(super) fn data_view(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.primary() else {
            ui.label("Select a table");
            return;
        };
        let t = &self.tables.tabs[i];
        ui.heading(&t.active_name);
        ui.label(format!("{} rows × {} columns", t.nrows(), t.ncols()));
        ui.horizontal(|ui| {
            ui.label("First row");
            ui.add(egui::DragValue::new(&mut self.data_row).range(0..=t.nrows().saturating_sub(1)));
        });
        egui::ScrollArea::both().show(ui, |ui| {
            egui::Grid::new("data").striped(true).show(ui, |ui| {
                for c in t.data.columns() {
                    ui.strong(&c.name);
                }
                ui.end_row();
                for row in self.data_row..(self.data_row + 200).min(t.nrows()) {
                    for c in t.data.columns() {
                        ui.label(c.data.display(row));
                    }
                    ui.end_row();
                }
            });
        });
    }
    pub(super) fn info_view(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::both().show(ui, |ui| {
            for (filename, object) in self.tables.unique_fileobjects() {
                ui.collapsing(filename, |ui| {
                    ui.monospace(object.info());
                });
            }
            if let Some(i) = self.primary() {
                let t = &self.tables.tabs[i];
                ui.heading("Table metadata");
                ui.monospace(format!(
                    "Name: {}\nFormat: {}\nShape: {}\nMask: {}\nFormulas: {:#?}",
                    t.name(),
                    t.fileformat_name,
                    t.shapestring(),
                    t.mask_string,
                    t.formulas
                ));
            }
        });
    }
}
const INFO_COLUMNS: &[&str] = &[
    "N",
    "Mean",
    "Std",
    "Min",
    "Max",
    "Median",
    "Variance",
    "Abs max",
    "TI",
    "X min",
    "X max",
    "X at min",
    "X at max",
    "X range",
    "Y range",
    "Δx",
    "Integral",
    "Integral mean",
    "Integral x",
    "Integral x normalized",
    "Integral x²",
    "Original N",
    "Original mean",
    "Original std",
    "Original min",
    "Original max",
];
fn info_value(p: &dv_plot::PlotData, name: &str) -> f64 {
    let s = p.stats();
    let o = p.original_stats();
    match name {
        "N" => s.n as f64,
        "Mean" => s.mean,
        "Std" => s.std,
        "Min" => s.y_min,
        "Max" => s.y_max,
        "Median" => s.median,
        "Variance" => s.variance,
        "Abs max" => s.abs_max,
        "TI" => s.turbulence_intensity,
        "X min" => s.x_min,
        "X max" => s.x_max,
        "X at min" => s.x_at_y_min,
        "X at max" => s.x_at_y_max,
        "X range" => s.x_range,
        "Y range" => s.y_range,
        "Δx" => s.dx,
        "Integral" => s.integral,
        "Integral mean" => s.integral_average,
        "Integral x" => s.integral_x,
        "Integral x normalized" => s.integral_x_normalized,
        "Integral x²" => s.integral_x2,
        "Original N" => o.n as f64,
        "Original mean" => o.mean,
        "Original std" => o.std,
        "Original min" => o.y_min,
        "Original max" => o.y_max,
        _ => f64::NAN,
    }
}
fn axis_label(value: f64, log: bool, flip: bool, data: Option<&dv_core::ColumnData>) -> String {
    let value = if flip { -value } else { value };
    let value = if log { 10f64.powf(value) } else { value };
    match data {
        Some(dv_core::ColumnData::DateTime(_)) => dv_core::dataframe::format_datetime(value),
        Some(dv_core::ColumnData::Str(v)) => {
            let mut categories = Vec::new();
            for s in v {
                if !categories.contains(&s.as_str()) {
                    categories.push(s.as_str());
                }
            }
            categories
                .get(value.round().max(0.0) as usize)
                .unwrap_or(&"")
                .to_string()
        }
        _ => export::value_label(value),
    }
}
pub(crate) fn mode_type(mode: &str) -> PlotType {
    match mode {
        "PDF" => PlotType::Pdf(PdfOptions::default()),
        "FFT" => PlotType::Fft(FftOptions::default()),
        "MinMax" => PlotType::MinMax(MinMaxOptions::default()),
        "Polar" => PlotType::Polar(PolarOptions::default()),
        _ => PlotType::Regular,
    }
}
