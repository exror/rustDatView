use super::*;

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum Tool {
    #[default]
    Inspect,
    Point,
    Region,
}
#[derive(Clone)]
pub(crate) enum Annotation {
    Point {
        mode: String,
        curve: String,
        x: f64,
        y: f64,
        label: String,
        color: [u8; 3],
    },
    Region {
        mode: String,
        x_axis: String,
        min: f64,
        max: f64,
        label: String,
        color: [u8; 3],
    },
}
impl Annotation {
    pub fn mode(&self) -> &str {
        match self {
            Self::Point { mode, .. } | Self::Region { mode, .. } => mode,
        }
    }
    fn label_mut(&mut self) -> &mut String {
        match self {
            Self::Point { label, .. } | Self::Region { label, .. } => label,
        }
    }
    fn color_mut(&mut self) -> &mut [u8; 3] {
        match self {
            Self::Point { color, .. } | Self::Region { color, .. } => color,
        }
    }
}
#[derive(Clone, Copy, Default, PartialEq)]
enum FilterMode {
    #[default]
    Keep,
    Exclude,
}
pub(super) struct RangeRule {
    pub enabled: bool,
    pub mode: String,
    pub target: String,
    bounds: [Option<f64>; 4],
    action: FilterMode,
}
impl RangeRule {
    fn accepts(&self, x: f64, y: f64) -> bool {
        let [xmin, xmax, ymin, ymax] = self.bounds;
        let inside = x.is_finite()
            && y.is_finite()
            && xmin.is_none_or(|v| x >= v)
            && xmax.is_none_or(|v| x <= v)
            && ymin.is_none_or(|v| y >= v)
            && ymax.is_none_or(|v| y <= v);
        match self.action {
            FilterMode::Keep => inside,
            FilterMode::Exclude => !inside,
        }
    }
    pub fn matches(&self, mode: &str, curve: &str) -> bool {
        self.enabled && self.mode == mode && (self.target.is_empty() || self.target == curve)
    }
    fn summary(&self) -> String {
        let bound = |v: Option<f64>| v.map_or_else(|| "any".into(), |v| format!("{v:.4}"));
        format!(
            "{} · X {} … {} · Y {} … {}",
            if self.action == FilterMode::Keep {
                "Keep"
            } else {
                "Exclude"
            },
            bound(self.bounds[0]),
            bound(self.bounds[1]),
            bound(self.bounds[2]),
            bound(self.bounds[3])
        )
    }
}
pub(super) struct Presentation {
    pub annotations: Vec<Annotation>,
    pub rules: Vec<RangeRule>,
    pub tool: Tool,
    pub pending: Option<f64>,
    pub open: bool,
    label: String,
    color: [u8; 3],
    tab: usize,
    target: String,
    bounds: [String; 4],
    action: FilterMode,
    error: String,
}
impl Default for Presentation {
    fn default() -> Self {
        Self {
            annotations: Vec::new(),
            rules: Vec::new(),
            tool: Tool::Inspect,
            pending: None,
            open: true,
            label: String::new(),
            color: [234, 153, 41],
            tab: 0,
            target: String::new(),
            bounds: Default::default(),
            action: FilterMode::Keep,
            error: String::new(),
        }
    }
}
impl Presentation {
    pub fn filter_curve(&self, mode: &str, curve: &mut Curve) {
        let rules = self.rules.iter().filter(|r| r.matches(mode, &curve.name));
        if rules.clone().next().is_none() {
            return;
        }
        for point in &mut curve.points {
            if !rules.clone().all(|r| r.accepts(point[0], point[1])) {
                point[1] = f64::NAN;
            }
        }
    }
    pub fn point(&mut self, mode: &str, curve: &Curve, x: f64, y: f64) {
        let label = if self.label.trim().is_empty() {
            format!("{}: {y:.3} at {x:.3}", curve.sy)
        } else {
            self.label.clone()
        };
        self.annotations.push(Annotation::Point {
            mode: mode.into(),
            curve: curve.name.clone(),
            x,
            y,
            label,
            color: self.color,
        });
        self.tool = Tool::Inspect;
    }
    pub fn region(&mut self, mode: &str, x_axis: &str, x: f64) {
        if let Some(start) = self.pending.take() {
            if start == x {
                self.error = "Choose a second, different X position.".into();
                self.pending = Some(start);
                return;
            }
            let label = if self.label.trim().is_empty() {
                format!("{:.3} – {:.3}", start.min(x), start.max(x))
            } else {
                self.label.clone()
            };
            self.annotations.push(Annotation::Region {
                mode: mode.into(),
                x_axis: x_axis.into(),
                min: start.min(x),
                max: start.max(x),
                label,
                color: self.color,
            });
            self.tool = Tool::Inspect;
        } else {
            self.pending = Some(x);
        }
    }
    pub fn active<'a>(&'a self, mode: &'a str) -> impl Iterator<Item = &'a Annotation> {
        self.annotations.iter().filter(move |a| a.mode() == mode)
    }
}
fn optional_number(ui: &mut egui::Ui, label: &str, text: &mut String) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::TextEdit::singleline(text)
                .hint_text("No limit")
                .desired_width(120.),
        );
    });
}
impl DatView {
    pub(super) fn presentation_panel(&mut self, ctx: &egui::Context) {
        if self.page != Page::Plot || !self.presentation.open {
            return;
        }
        egui::SidePanel::right("presentation-inspector").default_width(290.).min_width(250.).resizable(true).show(ctx,|ui|{
            ui.horizontal(|ui|{ui.heading("Plot studio");if ui.small_button("Hide").clicked(){self.presentation.open=false;}});
            ui.label(egui::RichText::new("Prepare a figure worth sharing").small().weak());
            ui.add_space(10.);
            ui.horizontal(|ui|{ui.selectable_value(&mut self.presentation.tab,0,"Annotate");ui.selectable_value(&mut self.presentation.tab,1,"Filter");ui.selectable_value(&mut self.presentation.tab,2,"Share");});ui.separator();
            egui::ScrollArea::vertical().id_salt("presentation-settings").show(ui,|ui|{
                match self.presentation.tab {
                    0=>{
                        ui.label("Click to mark a sample or bracket a time region.");
                        ui.horizontal(|ui|{let before=self.presentation.tool;ui.selectable_value(&mut self.presentation.tool,Tool::Inspect,"Pan / zoom");ui.selectable_value(&mut self.presentation.tool,Tool::Point,"Point");ui.selectable_value(&mut self.presentation.tool,Tool::Region,"Region");if before!=self.presentation.tool{self.presentation.pending=None;if self.presentation.tool!=Tool::Inspect{self.measure_enabled=false;}}});
                        ui.label("Annotation text");ui.add(egui::TextEdit::singleline(&mut self.presentation.label).hint_text("e.g. SOC transient / benchmark run").desired_width(f32::INFINITY));
                        ui.horizontal(|ui|{ui.label("Highlight");ui.color_edit_button_srgb(&mut self.presentation.color);});
                        match self.presentation.tool {Tool::Point=>{ui.colored_label(Color32::from_rgb(35,112,191),"Click near a plotted sample. The marker snaps to it.");},Tool::Region=>{ui.colored_label(Color32::from_rgb(35,112,191),if self.presentation.pending.is_some(){"Click the end of the region. Esc cancels."}else{"Click the start of the region, then its end."});},Tool::Inspect=>{ui.small("Drag to pan · wheel to zoom · double-click to fit");}}
                        ui.separator();
                        let mut remove=None;
                        for (i,annotation) in self.presentation.annotations.iter_mut().enumerate(){
                            ui.push_id(i,|ui|{egui::Frame::group(ui.style()).show(ui,|ui|{
                                ui.horizontal(|ui|{ui.label(match annotation{Annotation::Point{..}=>"Point",Annotation::Region{..}=>"Time region"});ui.weak(annotation.mode());if ui.small_button("Remove").clicked(){remove=Some(i);}});
                                ui.text_edit_singleline(annotation.label_mut());ui.color_edit_button_srgb(annotation.color_mut());
                                match annotation{Annotation::Point{x,y,..}=>{ui.small(format!("X {x:.5} · Y {y:.5}"));},Annotation::Region{min,max,..}=>{ui.horizontal(|ui|{ui.label("Start");ui.add(egui::DragValue::new(min));ui.label("End");ui.add(egui::DragValue::new(max));});}}
                            });});ui.add_space(6.);
                        }
                        if let Some(i)=remove{self.presentation.annotations.remove(i);}
                        if self.presentation.annotations.is_empty(){ui.weak("No annotations yet. Highlights and labels are included in exported images.");}
                    },
                    1=>{
                        ui.label("Non-destructive display filters");ui.small("Rules use plotted X/Y values, including boundaries. Rejected samples leave gaps, not connecting lines. Source data, statistics and analysis are unchanged.");ui.add_space(8.);
                        egui::ComboBox::from_id_salt("filter-series").width(220.).selected_text(if self.presentation.target.is_empty(){"All selected series"}else{&self.presentation.target}).show_ui(ui,|ui|{
                            ui.selectable_value(&mut self.presentation.target,String::new(),"All selected series");
                            for curve in &self.curves{ui.selectable_value(&mut self.presentation.target,curve.name.clone(),&curve.name);}
                        });
                        ui.horizontal(|ui|{ui.selectable_value(&mut self.presentation.action,FilterMode::Keep,"Keep inside");ui.selectable_value(&mut self.presentation.action,FilterMode::Exclude,"Exclude inside");});
                        for (label,text) in ["X minimum","X maximum","Y minimum","Y maximum"].into_iter().zip(&mut self.presentation.bounds){optional_number(ui,label,text);}
                        if ui.button("Apply range filter").clicked(){
                            let bounds=self.presentation.bounds.iter().map(|text|if text.trim().is_empty(){Ok(None)}else{text.trim().parse::<f64>().ok().filter(|v|v.is_finite()).map(Some).ok_or("Enter finite numbers or leave bounds empty.")}).collect::<Result<Vec<_>,_>>();
                            match bounds {
                                Ok(bounds) if bounds.iter().all(Option::is_none)=>self.presentation.error="Set at least one X or Y bound.".into(),
                                Ok(bounds) if matches!((bounds[0],bounds[1]),(Some(a),Some(b)) if a>b)||matches!((bounds[2],bounds[3]),(Some(a),Some(b)) if a>b)=>self.presentation.error="Minimum must not exceed maximum.".into(),
                                Ok(bounds)=>{self.presentation.rules.push(RangeRule{enabled:true,mode:self.options.mode.clone(),target:self.presentation.target.clone(),bounds:bounds.try_into().unwrap(),action:self.presentation.action});self.presentation.error.clear();self.dirty=true;},
                                Err(error)=>self.presentation.error=error.into(),
                            }
                        }
                        ui.separator();let mut remove=None;
                        for (i,rule) in self.presentation.rules.iter_mut().enumerate(){ui.push_id(i,|ui|{egui::Frame::group(ui.style()).show(ui,|ui|{
                            ui.horizontal(|ui|{if ui.checkbox(&mut rule.enabled,&rule.mode).changed(){self.dirty=true;}if ui.small_button("Remove").clicked(){remove=Some(i);}});
                            ui.small(if rule.target.is_empty(){"All selected series"}else{&rule.target});ui.label(rule.summary());
                        });});ui.add_space(6.);}
                        if let Some(i)=remove{self.presentation.rules.remove(i);self.dirty=true;}
                        if !self.presentation.rules.is_empty()&&ui.button("Clear display filters").clicked(){self.presentation.rules.clear();self.dirty=true;}
                    },
                    _=>{
                        ui.heading("Image details");ui.label("Title");ui.add(egui::TextEdit::singleline(&mut self.options.style.title).hint_text("Motherboard power · workload comparison").desired_width(f32::INFINITY));
                        ui.label("X-axis label");ui.add(egui::TextEdit::singleline(&mut self.options.style.x_label).hint_text("Automatic").desired_width(f32::INFINITY));ui.label("Y-axis label");ui.add(egui::TextEdit::singleline(&mut self.options.style.y_label).hint_text("Power [W]").desired_width(f32::INFINITY));
                        ui.add_space(8.);egui::ComboBox::from_id_salt("share-resolution").selected_text(format!("{} × {}",self.options.style.figure_width,self.options.style.figure_height)).show_ui(ui,|ui|{for (w,h,label) in [(1200,800,"Compact · 1200 × 800"),(1920,1080,"Full HD · 1920 × 1080"),(2560,1440,"QHD · 2560 × 1440"),(3200,1800,"High resolution · 3200 × 1800")]{if ui.selectable_label(self.options.style.figure_width==w&&self.options.style.figure_height==h,label).clicked(){self.options.style.figure_width=w;self.options.style.figure_height=h;}}});
                        ui.checkbox(&mut self.options.style.legend,"Include legend");ui.checkbox(&mut self.options.style.grid,"Subtle grid");ui.checkbox(&mut self.options.style.subplots,"One panel per series");
                        ui.add_space(8.);if ui.button("Fonts & appearance…").clicked(){self.editor=Editor::Settings;}
                        if ui.add_sized([ui.available_width(),36.],egui::Button::new("Save annotated image…")).clicked(){self.figure_export();}
                        if ui.button("Copy image to clipboard").clicked(){
                            if self.dirty{self.rebuild();}
                            let annotations=self.presentation.active(&self.options.mode).collect::<Vec<_>>();
                            match export::raster(&self.curves,&self.options.style,&annotations){Ok(image)=>{ctx.copy_image(image);self.status="Figure copied — paste it into chat or an editor.".into();},Err(error)=>self.error(error)}
                        }
                        ui.small("PNG for chat · SVG/PDF for documents. Export includes current display filters and annotations, without application controls.");
                    }
                }
                if !self.presentation.error.is_empty(){ui.colored_label(Color32::RED,&self.presentation.error);}
                ui.add_space(12.);ui.weak("Annotations and filters are session-only. Save your image before closing.");
            });
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selective_ranges_keep_boundaries_and_leave_gaps() {
        let mut p = Presentation::default();
        p.rules.push(RangeRule {
            enabled: true,
            mode: "Regular".into(),
            target: "SOC".into(),
            bounds: [Some(1.), Some(3.), None, Some(10.)],
            action: FilterMode::Keep,
        });
        let make = |name: &str| Curve {
            name: name.into(),
            sx: "Time [s]".into(),
            sy: "Power [W]".into(),
            points: vec![[0., 5.], [1., 10.], [2., 11.], [3., 8.], [4., 9.]],
        };
        let mut soc = make("SOC");
        p.filter_curve("Regular", &mut soc);
        assert!(soc.points[0][1].is_nan());
        assert_eq!(soc.points[1], [1., 10.]);
        assert!(soc.points[2][1].is_nan());
        assert_eq!(soc.points[3], [3., 8.]);
        assert!(soc.points[4][1].is_nan());
        let mut core = make("Core");
        p.filter_curve("Regular", &mut core);
        assert_eq!(core.points, make("Core").points);
        p.rules[0].action = FilterMode::Exclude;
        let mut soc = make("SOC");
        p.filter_curve("Regular", &mut soc);
        assert_eq!(soc.points[0], [0., 5.]);
        assert!(soc.points[1][1].is_nan());
        assert_eq!(soc.points[2], [2., 11.]);
    }
    #[test]
    fn reverse_region_clicks_normalize_and_switch_mode_safely() {
        let mut p = Presentation::default();
        p.tool = Tool::Region;
        p.region("Regular", "Time [s]", 8.);
        p.region("Regular", "Time [s]", 2.);
        assert!(matches!(
            p.annotations[0],
            Annotation::Region {
                min: 2.,
                max: 8.,
                ..
            }
        ));
        assert!(p.active("FFT").next().is_none());
        assert!(p.tool == Tool::Inspect);
    }
}

fn screen_point(
    point: [f64; 2],
    style: &Style,
    transform: &egui_plot::PlotTransform,
) -> egui::Pos2 {
    let x = export::axis(point[0], style.log_x);
    let y = export::axis(point[1], style.log_y);
    transform.position_from_point(&egui_plot::PlotPoint::new(
        if style.flip_x { -x } else { x },
        if style.flip_y { -y } else { y },
    ))
}
impl Presentation {
    pub fn paint(
        &self,
        ui: &egui::Ui,
        transform: &egui_plot::PlotTransform,
        curves: &[Curve],
        style: &Style,
        mode: &str,
    ) {
        let frame = *transform.frame();
        let painter = ui.painter().with_clip_rect(frame);
        let font = egui::TextStyle::Body.resolve(ui.style());
        for annotation in self.active(mode) {
            match annotation {
                Annotation::Region {
                    x_axis,
                    min,
                    max,
                    label,
                    color,
                    ..
                } if curves.first().is_some_and(|c| &c.sx == x_axis) => {
                    let a = screen_point([min.min(*max), 1.], style, transform).x;
                    let z = screen_point([min.max(*max), 1.], style, transform).x;
                    if !a.is_finite() || !z.is_finite() {
                        continue;
                    }
                    let left = a.min(z).max(frame.left());
                    let right = a.max(z).min(frame.right());
                    if left >= right {
                        continue;
                    }
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(left, frame.top()),
                            egui::pos2(right, frame.bottom()),
                        ),
                        0.,
                        Color32::from_rgb(color[0], color[1], color[2]).gamma_multiply(0.16),
                    );
                    painter.text(
                        egui::pos2(left + 7., frame.top() + 8.),
                        egui::Align2::LEFT_TOP,
                        label,
                        font.clone(),
                        ui.visuals().strong_text_color(),
                    );
                }
                Annotation::Point {
                    curve,
                    x,
                    y,
                    label,
                    color,
                    ..
                } if curves.iter().any(|c| &c.name == curve) => {
                    let p = screen_point([*x, *y], style, transform);
                    if !frame.contains(p) {
                        continue;
                    }
                    let color = Color32::from_rgb(color[0], color[1], color[2]);
                    let galley = painter.layout_no_wrap(
                        label.clone(),
                        font.clone(),
                        ui.visuals().strong_text_color(),
                    );
                    let pos = egui::pos2(
                        (p.x + 12.)
                            .min((frame.right() - galley.size().x - 8.).max(frame.left() + 4.)),
                        (p.y - galley.size().y - 14.).max(frame.top() + 8.),
                    );
                    painter.line_segment(
                        [p, pos + egui::vec2(0., galley.size().y)],
                        egui::Stroke::new(1_f32, color),
                    );
                    painter.rect_filled(
                        egui::Rect::from_min_size(
                            pos - egui::vec2(4., 3.),
                            galley.size() + egui::vec2(8., 6.),
                        ),
                        4.,
                        ui.visuals().panel_fill.gamma_multiply(0.96),
                    );
                    painter.circle_filled(p, 5., color);
                    painter.galley(pos, galley, ui.visuals().strong_text_color());
                }
                _ => {}
            }
        }
        if self.tool == Tool::Region {
            if let Some(x) = self.pending {
                let x = screen_point([x, 1.], style, transform).x;
                if x.is_finite() {
                    painter.line_segment(
                        [egui::pos2(x, frame.top()), egui::pos2(x, frame.bottom())],
                        egui::Stroke::new(1.5_f32, Color32::from_rgb(234, 153, 41)),
                    );
                }
            }
        }
    }
    pub fn click(
        &mut self,
        response: &egui_plot::PlotResponse<()>,
        curves: &[Curve],
        style: &Style,
        mode: &str,
    ) {
        if self.tool == Tool::Inspect || !response.response.clicked() {
            return;
        }
        let Some(pointer) = response.response.interact_pointer_pos() else {
            return;
        };
        if !response.transform.frame().contains(pointer) {
            return;
        }
        if self.tool == Tool::Region {
            let point = response.transform.value_from_position(pointer);
            let x = if style.flip_x { -point.x } else { point.x };
            let x = if style.log_x { 10f64.powf(x) } else { x };
            if let Some(curve) = curves.first() {
                self.region(mode, &curve.sx, x);
            }
        } else {
            let nearest = curves
                .iter()
                .flat_map(|curve| {
                    curve
                        .points
                        .iter()
                        .filter(|p| p.iter().all(|v| v.is_finite()))
                        .map(move |point| (curve, point))
                })
                .filter_map(|(curve, point)| {
                    let p = screen_point(*point, style, &response.transform);
                    if response.transform.frame().contains(p) {
                        Some((p.distance_sq(pointer), curve, point))
                    } else {
                        None
                    }
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, curve, point)) = nearest {
                self.point(mode, curve, point[0], point[1]);
            }
        }
    }
}
