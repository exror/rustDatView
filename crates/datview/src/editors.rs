use super::*;

impl DatView {
    pub(super) fn editor_window(&mut self, ctx: &egui::Context) {
        if self.editor == Editor::None {
            return;
        }
        let title = match self.editor {
            Editor::Formula => "Formula column",
            Editor::RenameTable => "Rename table",
            Editor::RenameColumn => "Rename column",
            Editor::Pipeline => "Plugin pipeline",
            Editor::Fit => "Curve fitting",
            Editor::Damping => "Damping and statistics",
            Editor::Script => "Python script export",
            Editor::Settings => "Options and styling",
            Editor::None => return,
        };
        let mut open = true;
        egui::Window::new(title)
            .open(&mut open)
            .default_width(650.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(if self.editor == Editor::Settings {
                        (ctx.screen_rect().height() - 160.0).max(100.0)
                    } else {
                        650.0
                    })
                    .show(ui, |ui| match self.editor {
                        Editor::Formula => self.formula_editor(ui),
                        Editor::RenameTable | Editor::RenameColumn => self.rename_editor(ui),
                        Editor::Pipeline => self.pipeline_editor(ui),
                        Editor::Fit => self.fit_editor(ui),
                        Editor::Damping => self.damping_editor(ui),
                        Editor::Script => self.script_editor(ui),
                        Editor::Settings => self.settings_editor(ui, ctx),
                        Editor::None => {}
                    });
            });
        if !open {
            self.editor = Editor::None;
        }
    }
    fn formula_editor(&mut self, ui: &mut egui::Ui) {
        ui.label("Applies to selected tables. Reference columns as {column name}.\nNumeric functions: sin, cos, exp, log, sqrt, abs; arithmetic and array expressions.");
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.edit_name);
        });
        ui.horizontal(|ui| {
            ui.label("Formula");
            ui.text_edit_singleline(&mut self.formula);
        });
        ui.checkbox(&mut self.replace_formula, "Replace first selected Y column");
        if let Some(i) = self.primary() {
            let names = self.tables.tabs[i].columns();
            egui::ComboBox::from_id_salt("formula-column")
                .selected_text("Insert column reference")
                .show_ui(ui, |ui| {
                    for name in names {
                        if ui.button(&name).clicked() {
                            self.formula.push_str(&format!("{{{name}}}"));
                        }
                    }
                });
        }
        if ui.button("Apply formula").clicked() {
            if self.edit_name.trim().is_empty() {
                self.error("A formula column requires a name");
                return;
            }
            for i in self.selected() {
                let x = self.selections[&i].ys.first().copied();
                let result = if self.replace_formula {
                    match x {
                        Some(j) => self.tables.tabs[i].set_column_by_formula(
                            &self.edit_name,
                            &self.formula,
                            j,
                        ),
                        None => Err(dv_core::DvError::msg("Select a Y column to replace")),
                    }
                } else {
                    self.tables.tabs[i].add_column_by_formula(&self.edit_name, &self.formula, x)
                };
                if let Err(e) = result {
                    self.error(e);
                }
            }
            self.sync_selection();
        }
        ui.separator();
        if let Some(i) = self.primary() {
            for f in self.tables.tabs[i].formulas.clone() {
                ui.horizontal(|ui| {
                    if ui.button("Edit").clicked() {
                        self.edit_name = f.name.clone();
                        self.formula = f.formula.clone();
                        self.replace_formula = true;
                        self.selections.get_mut(&i).unwrap().ys = vec![f.pos];
                    }
                    ui.monospace(format!("{} = {}", f.name, f.formula));
                });
            }
        }
    }
    fn rename_editor(&mut self, ui: &mut egui::Ui) {
        ui.label(if self.editor == Editor::RenameTable {
            "Renames the first selected table"
        } else {
            "Renames the first selected Y column in every selected table"
        });
        ui.text_edit_singleline(&mut self.edit_name);
        if ui.button("Rename").clicked() {
            if self.edit_name.trim().is_empty() {
                self.error("Enter a nonempty name");
                return;
            }
            if self.editor == Editor::RenameTable {
                if let Some(i) = self.primary() {
                    if let Err(e) = self.tables.rename_table(i, &self.edit_name) {
                        self.error(e);
                    }
                }
            } else {
                for i in self.selected() {
                    if let Some(&j) = self.selections[&i].ys.first() {
                        self.tables.tabs[i].rename_column(j, &self.edit_name);
                    }
                }
            }
            self.sync_selection();
        }
    }
    fn pipeline_editor(&mut self, ui: &mut egui::Ui) {
        ui.label("Actions run in order. Data actions modify tables; plot actions transform selected curves.\nReload original files before reapplying destructive actions to recover original data.");
        let mut remove = None;
        let mut disable = None;
        let mut up = None;
        let mut down = None;
        let mut changed = false;
        let mut add_tables = None;
        for (i, action) in self.pipeline.actions.iter_mut().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut action.enabled, &action.name).changed() {
                        if !action.enabled {
                            disable = Some(i);
                        }
                        changed = true;
                    }
                    if ui.button("↑").clicked() && i > 0 {
                        up = Some(i);
                    }
                    if ui.button("↓").clicked() {
                        down = Some(i);
                    }
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
                egui::CollapsingHeader::new("Parameters").show(ui, |ui| {
                    let mut value = serde_json::to_value(&action.plugin).unwrap_or(Value::Null);
                    if json_edit(ui, &mut value) {
                        match serde_json::from_value(value) {
                            Ok(p) => {
                                action.plugin = p;
                                changed = true;
                            }
                            Err(e) => self.errors.push(e.to_string()),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("X column for new tables");
                    ui.add(egui::DragValue::new(&mut action.x_column));
                    if ui.button("Create new tables").clicked() {
                        add_tables = Some(i);
                    }
                });
            });
        }
        if let Some(i) = disable {
            match self.pipeline.cancel(i, &mut self.tables.tabs) {
                Ok(()) => self.sync_selection(),
                Err(e) => {
                    self.pipeline.actions[i].enabled = true;
                    self.error(e);
                }
            }
        }
        if let Some(i) = remove {
            match self.pipeline.remove(i, &mut self.tables.tabs, true) {
                Ok(_) => changed = true,
                Err(e) => self.error(e),
            }
        }
        if let Some(i) = add_tables {
            let indices = self.selected();
            let input = indices
                .iter()
                .map(|&i| self.tables.tabs[i].clone())
                .collect::<Vec<_>>();
            let added = self.pipeline.actions[i].add_tables(&input);
            self.tables.tabs.extend(added);
            self.pipeline.collect_errors();
            self.errors.extend(self.pipeline.errors.clone());
            self.sync_selection();
        }
        if let Some(i) = up {
            self.pipeline.actions.swap(i, i - 1);
            changed = true;
        }
        if let Some(i) = down {
            if i + 1 < self.pipeline.actions.len() {
                self.pipeline.actions.swap(i, i + 1);
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
        }
        ui.separator();
        let old = self.plugin_name.clone();
        combo(
            ui,
            "Plugin",
            &mut self.plugin_name,
            &[
                "Filter",
                "Sampler",
                "Binning",
                "Outliers",
                "Mask",
                "Units",
                "RenameChannels",
                "RadialAverage",
                "RadialConcat",
                "Concat",
            ],
        );
        if old != self.plugin_name || self.plugin_config.is_null() {
            self.plugin_config =
                serde_json::to_value(plugin_default(&self.plugin_name)).unwrap_or_default();
        }
        ui.label("Filter names: Moving average, Low pass 1st order, High pass 1st order.\nSampler names: Replace, Insert, Remove, Every n, Linspace, Time-based, Delta x.\nUnit flavors: SI or WE. Channel rename flavors: OF23 or FldAero.\nRadial averaging: constantwindow (seconds) or periods (rotor periods).");
        json_edit(ui, &mut self.plugin_config);
        if ui.button("Add action").clicked() {
            match serde_json::from_value::<Plugin>(self.plugin_config.clone()) {
                Ok(p) => {
                    self.pipeline
                        .append(Action::new(self.plugin_name.clone(), p), false);
                    self.dirty = true;
                }
                Err(e) => self.error(e),
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Apply data actions").clicked() {
                self.pipeline.apply(&mut self.tables.tabs, true);
                self.errors.extend(self.pipeline.errors.clone());
                self.sync_selection();
            }
            if ui.button("Save pipeline…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("pipeline.json")
                    .save_file()
                {
                    match serde_json::to_vec_pretty(&self.pipeline) {
                        Ok(bytes) => {
                            if let Err(e) = std::fs::write(path, bytes) {
                                self.error(e);
                            }
                        }
                        Err(e) => self.error(e),
                    }
                }
            }
            if ui.button("Load pipeline…").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    match std::fs::read(path)
                        .map_err(|e| e.to_string())
                        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
                    {
                        Ok(p) => {
                            self.pipeline = p;
                            self.dirty = true;
                        }
                        Err(e) => self.error(e),
                    }
                }
            }
        });
        for error in &self.pipeline.errors {
            ui.colored_label(Color32::RED, error);
        }
    }
    fn curve_picker(&mut self, ui: &mut egui::Ui) {
        egui::ComboBox::from_id_salt("tool curve")
            .selected_text(
                self.curves
                    .get(self.tool_curve)
                    .map(|c| c.name.as_str())
                    .unwrap_or("Select curve"),
            )
            .show_ui(ui, |ui| {
                for (i, c) in self.curves.iter().enumerate() {
                    ui.selectable_value(&mut self.tool_curve, i, &c.name);
                }
            });
    }
    fn fit_editor(&mut self, ui: &mut egui::Ui) {
        if self.dirty {
            self.rebuild();
        }
        self.curve_picker(ui);
        let models = dv_tools::curve_fitting::models();
        egui::ComboBox::from_id_salt("fit model")
            .selected_text(&self.fit_model)
            .show_ui(ui, |ui| {
                for model in &models {
                    if ui
                        .selectable_value(&mut self.fit_model, model.id.clone(), &model.label)
                        .clicked()
                    {
                        self.tool_result = model.formula.clone();
                    }
                }
            });
        ui.label("Model ID, or custom expression: eval: {a}*x+{b}");
        ui.text_edit_singleline(&mut self.fit_model);
        ui.label("Guesses, bounds and constants use coefficient names. Polynomial order and exponents apply to the corresponding fitter.");
        json_edit(ui, &mut self.fit_options);
        if ui.button("Fit selected curve").clicked() {
            let result = (|| -> Result<_, String> {
                let p = self
                    .plotted
                    .get(self.tool_curve)
                    .ok_or("Select a plotted curve")?;
                let (x, y) = p.numeric_xy().map_err(|e| e.to_string())?;
                let options =
                    serde_json::from_value(self.fit_options.clone()).map_err(|e| e.to_string())?;
                dv_tools::curve_fitting::model_fit(&self.fit_model, x, y, &options)
                    .map_err(|e| e.to_string())
            })();
            match result {
                Ok(f) => {
                    let p = &self.plotted[self.tool_curve];
                    self.overlays.push(Curve {
                        name: "Fit".into(),
                        sx: p.sx.clone(),
                        sy: p.sy.clone(),
                        points: f
                            .x
                            .iter()
                            .copied()
                            .zip(f.y_fit.iter().copied())
                            .map(|(x, y)| [x, y])
                            .collect(),
                    });
                    self.tool_result = format!(
                        "{}\nR² = {}\nRMSE = {}\nIterations = {}\n{}",
                        f.formula,
                        f.r2,
                        f.rmse,
                        f.iterations,
                        f.coefficients
                            .iter()
                            .map(|(n, v)| format!("{n} = {v}"))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                }
                Err(e) => self.error(e),
            }
        }
        if ui.button("Add last fit as table").clicked() {
            if let Some(c) = self.overlays.last() {
                let df = dv_core::DataFrame::from_floats(
                    vec![c.sx.clone(), c.sy.clone()],
                    vec![
                        c.points.iter().map(|p| p[0]).collect(),
                        c.points.iter().map(|p| p[1]).collect(),
                    ],
                );
                match df {
                    Ok(df) => {
                        self.tables.append(Table::from_df(df, "Fit"));
                        self.sync_selection();
                    }
                    Err(e) => self.error(e),
                }
            }
        }
        ui.monospace(&self.tool_result);
        if ui.button("Copy results").clicked() {
            ui.ctx().copy_text(self.tool_result.clone());
        }
    }
    fn damping_editor(&mut self, ui: &mut egui::Ui) {
        if self.dirty {
            self.rebuild();
        }
        self.curve_picker(ui);
        if ui
            .button("Estimate damping (logarithmic decrement)")
            .clicked()
        {
            let result = (|| -> Result<_, String> {
                let p = self
                    .plotted
                    .get(self.tool_curve)
                    .ok_or("Select a plotted curve")?;
                let (t, x) = p.numeric_xy().map_err(|e| e.to_string())?;
                dv_tools::damping::freq_damp_estimator(x, t, "fromPeaks").map_err(|e| e.to_string())
            })();
            match result {
                Ok(r) => {
                    let p = &self.plotted[self.tool_curve];
                    let (t, _) = p.numeric_xy().unwrap();
                    self.overlays.push(Curve {
                        name: "Damping model".into(),
                        sx: p.sx.clone(),
                        sy: p.sy.clone(),
                        points: t
                            .iter()
                            .copied()
                            .zip(r.x_model.iter().copied())
                            .map(|(x, y)| [x, y])
                            .collect(),
                    });
                    self.tool_result=format!("Natural frequency: {} Hz\nDamped frequency: {} Hz\nDamping ratio ζ: {}\nLog decrement: {}\nζ range: {} .. {}\nζ mean: {}",r.fn_,r.fd,r.zeta,r.logdec,r.zeta_min,r.zeta_max,r.zeta_mean);
                }
                Err(e) => self.error(e),
            }
        }
        ui.monospace(&self.tool_result);
        egui::CollapsingHeader::new("Equivalent fatigue load").show(ui,|ui|{
            ui.label("Compute damage-equivalent load using rainflow cycles. Parameters: Wöhler exponent, equivalent period [s], histogram bins.");
            ui.horizontal(|ui|{ui.label("m, Teq, bins");ui.text_edit_singleline(&mut self.formula);});
            if ui.button("Calculate equivalent load").clicked(){
                let result=(||->Result<f64,String>{let params=self.formula.split(',').map(|s|s.trim().parse::<f64>()).collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;if params.len()!=3||params[2]<1.0{return Err("Enter m, Teq, bins (for example 3,1,100)".into());}let p=self.plotted.get(self.tool_curve).ok_or("Select a curve")?;p.equivalent_load(params[0],params[1],params[2]as usize).map_err(|e|e.to_string())})();match result{Ok(v)=>self.tool_result=format!("Equivalent load: {v}"),Err(e)=>self.error(e)}
            }
        });
    }
    fn script_editor(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Script generation options").show(ui, |ui| {
            json_edit(ui, &mut self.script_options);
        });
        ui.horizontal(|ui| {
            if ui.button("Regenerate").clicked() {
                self.generate_script();
            }
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(self.script.clone());
            }
            if ui.button("Save Python…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_file_name("datview_plot.py")
                    .add_filter("Python", &["py"])
                    .save_file()
                {
                    if let Err(e) = std::fs::write(path, &self.script) {
                        self.error(e);
                    }
                }
            }
        });
        ui.add(
            egui::TextEdit::multiline(&mut self.script)
                .font(egui::TextStyle::Monospace)
                .code_editor()
                .desired_rows(25)
                .desired_width(f32::INFINITY),
        );
    }
    fn settings_editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Make DatView yours");
        ui.weak("A comfortable workspace and clear, share-ready figures.");
        ui.add_space(6.0);
        self.startup_settings(ui, ctx);
        let mut changed = false;
        let mut theme_changed = false;
        ui.group(|ui| {
            ui.strong("Appearance");
            ui.horizontal(|ui| {
                ui.label("Theme");
                theme_changed |= ui.selectable_value(&mut self.options.dark, false, "Light").changed();
                theme_changed |= ui.selectable_value(&mut self.options.dark, true, "Dark").changed();
            });
            ui.horizontal(|ui| {
                ui.label("Font");
                match self.appearance.font_picker(ui, ctx, &mut self.options.style.font_family) {
                    Ok(font_changed) => changed |= font_changed,
                    Err(error) => self.error(error),
                }
            });
            ui.weak("Installed families are shared by the interface and exported figures.");
            ui.horizontal(|ui| {
                ui.label("Interface size");
                theme_changed |= ui.add(
                    egui::Slider::new(&mut self.options.font_size, 8.0..=32.0)
                        .suffix(" pt")
                ).changed();
            });
            ui.label(egui::RichText::new("CPU package · 72.5 °C   |   Core clock · 5,200 MHz")
                .size(self.options.font_size));
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong("Share-ready figure");
            ui.weak("These settings control the exported image, not the desktop window.");
            egui::Grid::new("figure-label-settings")
                .num_columns(2)
                .spacing([16.0, 9.0])
                .show(ui, |ui| {
                    ui.label("Title");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.options.style.title)
                        .hint_text("e.g. CPU temperature under sustained load")
                        .desired_width(360.0)).changed();
                    ui.end_row();
                    ui.label("X-axis label");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.options.style.x_label)
                        .hint_text("Automatic from plotted channels")
                        .desired_width(360.0)).changed();
                    ui.end_row();
                    ui.label("Y-axis label");
                    changed |= ui.add(egui::TextEdit::singleline(&mut self.options.style.y_label)
                        .hint_text("Automatic from plotted channels")
                        .desired_width(360.0)).changed();
                    ui.end_row();
                    ui.label("Figure text size");
                    changed |= ui.add(egui::Slider::new(&mut self.options.style.font_size, 8.0..=48.0)
                        .suffix(" pt")).changed();
                    ui.end_row();
                    ui.label("Resolution");
                    let dimensions = (self.options.style.figure_width, self.options.style.figure_height);
                    let label = match dimensions {
                        (1200, 800) => "1200 × 800 · Standard",
                        (1920, 1080) => "1920 × 1080 · Full HD",
                        (2560, 1440) => "2560 × 1440 · QHD",
                        (3200, 1800) => "3200 × 1800 · High resolution",
                        _ => "Custom",
                    };
                    egui::ComboBox::from_id_salt("share-resolution")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (width, height, name) in [
                                (1200, 800, "1200 × 800 · Standard"),
                                (1920, 1080, "1920 × 1080 · Full HD"),
                                (2560, 1440, "2560 × 1440 · QHD"),
                                (3200, 1800, "3200 × 1800 · High resolution"),
                            ] {
                                if ui.selectable_label(dimensions == (width, height), name).clicked() {
                                    self.options.style.figure_width = width;
                                    self.options.style.figure_height = height;
                                    changed = true;
                                }
                            }
                        });
                    ui.end_row();
                    ui.label("Custom dimensions");
                    ui.horizontal(|ui| {
                        changed |= ui.add(egui::DragValue::new(&mut self.options.style.figure_width)
                            .range(200..=16384).suffix(" px")).changed();
                        ui.label("×");
                        changed |= ui.add(egui::DragValue::new(&mut self.options.style.figure_height)
                            .range(150..=16384).suffix(" px")).changed();
                    });
                    ui.end_row();
                });
        });
        ui.add_space(6.0);
        ui.group(|ui| {
            ui.strong("Plot styling");
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut self.options.style.grid, "Grid").changed();
                changed |= ui.checkbox(&mut self.options.style.legend, "Legend").changed();
                changed |= ui.checkbox(&mut self.options.style.lines, "Lines").changed();
                changed |= ui.checkbox(&mut self.options.style.markers, "Markers").changed();
                changed |= ui.checkbox(&mut self.options.style.step, "Step lines").changed();
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Line width");
                changed |= ui.add(egui::DragValue::new(&mut self.options.style.width)
                    .range(0.1..=20.0).speed(0.1).suffix(" px")).changed();
                ui.label("Marker size");
                changed |= ui.add(egui::DragValue::new(&mut self.options.style.marker_size)
                    .range(0.5..=30.0).speed(0.1).suffix(" px")).changed();
            });
        });
        ui.collapsing("Advanced styling", |ui| {
            ui.weak("All plot/export style fields. Axis limits use raw data coordinates.");
            let mut style = serde_json::to_value(&self.options.style).unwrap_or(Value::Null);
            if json_edit(ui, &mut style) {
                match serde_json::from_value::<Style>(style) {
                    Ok(mut style) => {
                        if let Err(error) = self.appearance.apply_font(ctx, &style.font_family) {
                            style.font_family.clone_from(&self.options.style.font_family);
                            self.error(error);
                        }
                        self.options.style = style;
                        changed = true;
                    }
                    Err(error) => self.error(error),
                }
            }
        });
        if theme_changed {
            appearance::apply_theme(ctx, self.options.dark, self.options.font_size);
        }
        self.dirty |= changed;
        ui.add_space(6.0);
        ui.weak("Settings are autosaved as a recoverable draft, not silently restored. Pipelines are saved separately.");
        if ui.button("Reset options to defaults").clicked() {
            self.apply_settings(Options::default(), ctx);
        }
    }
}
fn plugin_default(name: &str) -> Plugin {
    match name {
        "Sampler" => Plugin::Sampler {
            name: "Every n".into(),
            param: vec![2.0],
        },
        "Binning" => Plugin::Binning {
            bins: 50,
            min: 0.0,
            max: 1.0,
        },
        "Outliers" => Plugin::Outliers {
            median_deviation: 5.0,
        },
        "Mask" => Plugin::Mask {
            expression: String::new(),
        },
        "Units" => Plugin::Units {
            flavor: "SI".into(),
        },
        "RenameChannels" => Plugin::RenameChannels {
            flavor: "OF23".into(),
        },
        "RadialAverage" => Plugin::RadialAverage {
            method: "constantwindow".into(),
            param: 2.0,
        },
        "RadialConcat" => Plugin::RadialConcat,
        "Concat" => Plugin::Concat { common_only: false },
        _ => Plugin::Filter {
            name: "Moving average".into(),
            param: 100.0,
        },
    }
}
