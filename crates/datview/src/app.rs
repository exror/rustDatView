use dv_core::{ColumnData, Table, TableList};
use dv_plot::{Action, Pipeline, PlotData, PlotType, Plugin, PythonScripter};
use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{Duration, Instant},
};
#[path = "editors.rs"]
mod editors;
#[path = "selection.rs"]
mod selection;
#[path = "view.rs"]
mod view;
use selection::{shared_columns, SelectionMode};
#[path = "fields.rs"]
mod fields;
#[path = "channels.rs"]
mod channels;
#[path = "appearance.rs"]
mod appearance;
#[path = "presentation.rs"]
pub(crate) mod presentation;
use crate::export::{self, Curve, Style};
use serde_json::Value;

#[derive(Clone, Default, Serialize, Deserialize)]
struct Selection {
    selected: bool,
    x: usize,
    ys: Vec<usize>,
    filter: String,
}
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Options {
    style: Style,
    dayfirst: bool,
    live: bool,
    watch: bool,
    dark: bool,
    font_size: f32,
    mode: String,
    plot_options: Value,
    pipeline: Value,
    selection_mode: SelectionMode,
    field_style: fields::FieldStyle,
    info_columns: Vec<String>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            style: Style::default(),
            dayfirst: false,
            live: true,
            watch: false,
            dark: false,
            font_size: 14.0,
            mode: "Regular".into(),
            selection_mode: SelectionMode::Auto,
            field_style: fields::FieldStyle::default(),
            info_columns: vec![
                "N".into(),
                "Mean".into(),
                "Std".into(),
                "Min".into(),
                "Max".into(),
                "Median".into(),
                "Integral".into(),
                "TI".into(),
            ],
            plot_options: serde_json::to_value(PlotType::Regular).unwrap_or(Value::Null),
            pipeline: Value::Null,
        }
    }
}
#[derive(PartialEq)]
enum Page {
    Plot,
    Data,
    Info,
    Fields,
}
#[derive(PartialEq)]
enum Editor {
    None,
    Formula,
    RenameTable,
    RenameColumn,
    Pipeline,
    Fit,
    Damping,
    Script,
    Settings,
}

pub struct DatView {
    tables: TableList,
    selections: BTreeMap<usize, Selection>,
    formats: Vec<dv_io::FileFormat>,
    format: Option<String>,
    options: Options,
    appearance: appearance::Appearance,
    channel_search: BTreeMap<usize, channels::SearchCache>,
    presentation: presentation::Presentation,
    pipeline: Pipeline,
    plot_type: PlotType,
    plotted: Vec<PlotData>,
    curves: Vec<Curve>,
    overlays: Vec<Curve>,
    vertices: Vec<Vec<egui_plot::PlotPoint>>,
    overlay_vertices: Vec<Vec<egui_plot::PlotPoint>>,
    vertex_style: Option<(bool, bool, bool, bool, bool)>,
    dirty: bool,
    status: String,
    errors: Vec<String>,
    page: Page,
    editor: Editor,
    table_filter: String,
    edit_name: String,
    formula: String,
    replace_formula: bool,
    common_only: bool,
    merge_interpolate: bool,
    compare_mode: String,
    compare_method: String,
    measurements: Vec<[f64; 2]>,
    measure_enabled: bool,
    tool_curve: usize,
    fit_model: String,
    fit_options: Value,
    tool_result: String,
    plugin_name: String,
    plugin_config: Value,
    script: String,
    script_options: Value,
    field_index: usize,
    fields: Vec<dv_core::Field2D>,
    field_texture: Option<egui::TextureHandle>,
    field_raster: Option<fields::FieldRaster>,
    last_watch: Instant,
    mtimes: BTreeMap<String, std::time::SystemTime>,
    data_row: usize,
    limits_text: String,
}
impl DatView {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: crate::Launch) -> Self {
        let mut options: Options = cc
            .storage
            .and_then(|s| eframe::get_value(s, "options"))
            .unwrap_or_default();
        if launch.dayfirst {
            options.dayfirst = true;
        }
        if let Some(mode) = &launch.mode {
            options.mode = mode.clone();
            options.plot_options =
                serde_json::to_value(view::mode_type(mode)).unwrap_or(Value::Null);
        }
        if let Some(value) = &launch.plot_options {
            options.plot_options = value.clone();
        }
        let mut initial_errors = Vec::new();
        let plot_type = match serde_json::from_value(options.plot_options.clone()) {
            Ok(p) => p,
            Err(e) => {
                initial_errors.push(format!("Invalid plot options: {e}"));
                PlotType::Regular
            }
        };
        let pipeline = if let Some(path) = &launch.pipeline {
            match Pipeline::load(path) {
                Ok(p) => p,
                Err(e) => {
                    initial_errors.push(e.to_string());
                    Pipeline::default()
                }
            }
        } else {
            match serde_json::from_value(options.pipeline.clone()) {
                Ok(p) => p,
                Err(e) if !options.pipeline.is_null() => {
                    initial_errors.push(format!("Invalid persisted pipeline: {e}"));
                    Pipeline::default()
                }
                Err(_) => Pipeline::default(),
            }
        };
        appearance::apply_theme(&cc.egui_ctx, options.dark, options.font_size);
        let mut appearance = appearance::Appearance::new();
        if let Err(error)=appearance.apply_font(&cc.egui_ctx,&options.style.font_family){initial_errors.push(error);}
        let mut app = Self {
            tables: TableList::default(),
            selections: BTreeMap::new(),
            formats: dv_io::all_formats(),
            format: launch.format,
            options,
            appearance,
            channel_search: BTreeMap::new(),
            presentation: presentation::Presentation::default(),
            pipeline,
            plot_type,
            plotted: Vec::new(),
            curves: Vec::new(),
            overlays: Vec::new(),
            dirty: true,
            status: "Open files or drag them here. Ctrl-drop appends; ordinary drop replaces."
                .into(),
            errors: initial_errors,
            page: Page::Plot,
            editor: Editor::None,
            table_filter: String::new(),
            edit_name: String::new(),
            formula: String::new(),
            replace_formula: false,
            common_only: false,
            merge_interpolate: false,
            compare_mode: "OneTableColumns".into(),
            compare_method: "Relative".into(),
            measurements: Vec::new(),
            measure_enabled: false,
            tool_curve: 0,
            fit_model: "predef: gaussian".into(),
            fit_options: serde_json::to_value(dv_tools::curve_fitting::FitOptions::default())
                .unwrap_or(Value::Null),
            tool_result: String::new(),
            plugin_name: "Filter".into(),
            plugin_config: Value::Null,
            script: String::new(),
            script_options: serde_json::to_value(dv_plot::ScriptOptions::default())
                .unwrap_or(Value::Null),
            field_index: 0,
            fields: Vec::new(),
            field_texture: None,
            field_raster: None,
            last_watch: Instant::now(),
            mtimes: BTreeMap::new(),
            data_row: 0,
            limits_text: String::new(),
            vertices: Vec::new(),
            overlay_vertices: Vec::new(),
            vertex_style: None,
        };
        app.tables.dayfirst = app.options.dayfirst;
        if !launch.paths.is_empty() {
            app.load(launch.paths, false);
        }
        for selection in app.selections.values_mut() {
            if let Some(x) = launch.x {
                selection.x = x;
            }
            if !launch.ys.is_empty() {
                selection.ys = launch.ys.clone();
            }
        }
        app
    }
    fn error(&mut self, error: impl ToString) {
        self.errors.push(error.to_string());
    }
    fn selected(&self) -> Vec<usize> {
        self.selections
            .iter()
            .filter(|(i, s)| **i < self.tables.len() && s.selected)
            .map(|(&i, _)| i)
            .collect()
    }
    fn primary(&self) -> Option<usize> {
        self.selected().first().copied()
    }
    fn sync_selection(&mut self) {
        self.channel_search.clear();
        self.selections.retain(|i, _| *i < self.tables.len());
        for (i, t) in self.tables.tabs.iter().enumerate() {
            let s = self.selections.entry(i).or_insert_with(|| Selection {
                selected: true,
                x: selection::default_x(t),
                ys: selection::default_ys(t,selection::default_x(t)),
                filter: String::new(),
            });
            s.x = s.x.min(t.ncols().saturating_sub(1));
            s.ys.retain(|j| *j < t.ncols());
        }
        let names = self.tables.get_display_tab_names();
        self.tables.set_active_names(&names);
        self.dirty = true;
    }
    fn load(&mut self, mut paths: Vec<PathBuf>, add: bool) {
        paths.retain(|p| !p.is_dir());
        paths.sort();
        if !add {
            self.selections.clear();
            self.overlays.clear();
            self.measurements.clear();
            self.presentation = presentation::Presentation::default();
        }
        self.tables.dayfirst = self.options.dayfirst;
        let files = paths
            .into_iter()
            .map(|p| (p, self.format.clone()))
            .collect::<Vec<_>>();
        let (_, warnings) =
            self.tables
                .load_tables_from_files(&files, add, false, &dv_io::read_file);
        self.errors
            .extend(warnings.into_iter().filter(|w| !w.is_empty()));
        self.pipeline.apply(&mut self.tables.tabs, true);
        self.errors.extend(self.pipeline.errors.clone());
        self.sync_selection();
        self.capture_mtimes();
        self.fields.clear();
        self.field_texture = None;
        self.status = format!("{} tables loaded", self.tables.len());
    }
    fn capture_mtimes(&mut self) {
        self.mtimes = self
            .tables
            .unique_filenames()
            .into_iter()
            .filter_map(|p| {
                std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(|t| (p, t))
            })
            .collect();
    }
    fn reload(&mut self, selected_only: bool) {
        let indices = if selected_only {
            self.selected()
        } else {
            (0..self.tables.len()).collect()
        };
        let mut files = Vec::new();
        let formulas = self.tables.store_formulas();
        for i in indices {
            if i >= self.tables.len() {
                continue;
            }
            let filename = self.tables.tabs[i].filename.clone();
            if files.contains(&filename) {
                continue;
            }
            files.push(filename);
            match self.tables.reload_one_tab(i, &dv_io::read_file) {
                Ok(w) if !w.is_empty() => self.error(w),
                Err(e) => self.error(e),
                _ => {}
            }
        }
        self.errors.extend(self.tables.apply_formulas(&formulas));
        self.pipeline.apply(&mut self.tables.tabs, true);
        self.errors.extend(self.pipeline.errors.clone());
        self.sync_selection();
        self.capture_mtimes();
        self.fields.clear();
        self.field_texture = None;
    }
    fn open_dialog(&mut self, add: bool) {
        if let Some(paths) = rfd::FileDialog::new().pick_files() {
            self.load(paths, add);
        }
    }
    fn plot_ids(&self) -> Vec<(usize, usize, usize)> {
        let selected = self.selected();
        if let Some(mapped) = shared_columns(&self.tables, &selected, self.options.selection_mode) {
            let primary = &self.selections[&selected[0]];
            let Some(x) = mapped[0].iter().position(|&i| i == primary.x) else {
                return Vec::new();
            };
            return selected
                .iter()
                .enumerate()
                .flat_map(|(row, &table)| {
                    primary
                        .ys
                        .iter()
                        .filter_map(|&y| {
                            mapped[0]
                                .iter()
                                .position(|&i| i == y)
                                .map(|j| (table, mapped[row][x], mapped[row][j]))
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        selected
            .iter()
            .flat_map(|&table| {
                let s = &self.selections[&table];
                s.ys.iter().map(move |&y| (table, s.x, y))
            })
            .collect()
    }
    fn rebuild(&mut self) {
        self.plotted.clear();
        self.curves.clear();
        self.vertices.clear();
        self.vertex_style = None;
        for (i, x, y) in self.plot_ids() {
            match PlotData::from_table(&self.tables.tabs[i], x, y, Some(&self.pipeline)).and_then(
                |mut p| {
                    p.it = i;
                    p.transform(&self.plot_type)?;
                    Ok(p)
                },
            ) {
                Ok(p) => self.plotted.push(p),
                Err(e) => self.errors.push(e.to_string()),
            }
        }
        if self.options.mode == "Compare" {
            let mode = serde_json::from_value::<dv_plot::CompareMode>(Value::String(
                self.compare_mode.clone(),
            ));
            let method = serde_json::from_value::<dv_plot::CompareMethod>(Value::String(
                self.compare_method.clone(),
            ));
            match (mode, method) {
                (Ok(m), Ok(t)) => match dv_plot::plotdata::compare(&self.plotted, m, t) {
                    Ok(p) => self.plotted = p,
                    Err(e) => self.errors.push(e.to_string()),
                },
                (Err(e), _) | (_, Err(e)) => self.errors.push(e.to_string()),
            }
        }
        let single_table=self.plotted.first().is_none_or(|first|self.plotted.iter().all(|p|p.it==first.it));
        for p in &self.plotted {
            let x = column_values(&p.x);
            let y = column_values(&p.y);
            let mut curve=Curve {
                name: if single_table{p.sy.clone()}else{format!("{} — {}",p.st,p.sy)},
                sx: p.sx.clone(),
                sy: p.sy.clone(),
                points: x.into_iter().zip(y).map(|(x,y)|[x,y]).collect(),
            };
            self.presentation.filter_curve(&self.options.mode,&mut curve);
            self.curves.push(curve);
        }
        self.dirty = false;
    }
    fn table_export(&mut self) {
        let Some(i) = self.primary() else {
            self.error("Select a table to export");
            return;
        };
        let writers = dv_io::writers();
        let mut dialog = rfd::FileDialog::new().set_file_name("table.csv");
        for w in &writers {
            let ext = w
                .extensions
                .iter()
                .map(|e| e.trim_start_matches('.'))
                .collect::<Vec<_>>();
            dialog = dialog.add_filter(w.name, &ext);
        }
        if let Some(path) = dialog.save_file() {
            let t = &self.tables.tabs[i];
            let mut df = t.data_without_index();
            if let Some(mask) = &t.mask {
                df = df.filter_rows(mask);
            }
            match dv_io::write_dataframe(&df, &path, None) {
                Ok(()) => self.status = "Table exported".into(),
                Err(e) => self.error(e),
            }
        }
    }
    fn figure_export(&mut self) {
        if self.dirty {
            self.rebuild();
        }
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("telemetry.png")
            .add_filter("PNG image", &["png"])
            .add_filter("Vector figure", &["svg", "pdf", "eps"])
            .save_file()
        {
            let mut curves = self.curves.clone();
            curves.extend(self.overlays.clone());
            let annotations=self.presentation.active(&self.options.mode).collect::<Vec<_>>();
            match export::save(&path, &curves, &self.options.style,&annotations) {
                Ok(()) => self.status = "Figure exported".into(),
                Err(e) => self.error(e),
            }
        }
    }
    fn generate_script(&mut self) {
        let mut scripter = PythonScripter::default();
        scripter.files = self.tables.unique_filenames();
        scripter.selections = self.plot_ids();
        scripter.plot_type = self.plot_type.clone();
        match serde_json::from_value(self.script_options.clone()) {
            Ok(o) => scripter.options = o,
            Err(e) => {
                self.error(e);
                return;
            }
        }
        if self.options.mode == "Compare" {
            match (
                serde_json::from_value(Value::String(self.compare_mode.clone())),
                serde_json::from_value(Value::String(self.compare_method.clone())),
            ) {
                (Ok(mode), Ok(method)) => scripter.comparison = Some((mode, method)),
                (Err(e), _) | (_, Err(e)) => {
                    self.error(e);
                    return;
                }
            }
        }
        let style = &self.options.style;
        scripter.options.grid = style.grid;
        scripter.options.log_x = style.log_x;
        scripter.options.log_y = style.log_y;
        scripter.options.line_width = style.width as f64;
        scripter.options.marker_size = style.marker_size as f64;
        scripter.options.title.clone_from(&style.title);
        scripter.options.x_label.clone_from(&style.x_label);
        scripter.options.y_label.clone_from(&style.y_label);
        scripter.options.x_limits = style.limits.map(|b| (b[0], b[1]));
        scripter.options.y_limits = style.limits.map(|b| (b[2], b[3]));
        scripter.options.flip_x = style.flip_x;
        scripter.options.flip_y = style.flip_y;
        scripter.options.legend = style.legend;
        scripter.options.step = style.step;
        scripter.options.line_styles = vec![if style.lines { "-" } else { "None" }.into()];
        scripter.options.markers = vec![if style.markers { "o" } else { "" }.into()];
        if style.subplots {
            scripter.options.subplot_rows = scripter.selections.len().max(1);
            scripter.options.subplot_cols = 1;
            scripter.options.selection_axes = (0..scripter.selections.len()).collect();
        }
        match scripter.generate(&self.tables.tabs, &self.pipeline) {
            Ok(s) => self.script = s,
            Err(e) => self.error(e),
        }
    }
    fn menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx,|ui|{
            egui::menu::bar(ui,|ui|{
                ui.menu_button("File",|ui|{
                    if ui.button("Open…  Ctrl+O").clicked(){self.open_dialog(false);ui.close_menu();}
                    if ui.button("Add files…").clicked(){self.open_dialog(true);ui.close_menu();}
                    if ui.button("Reload all  F5").clicked(){self.reload(false);ui.close_menu();}
                    if ui.button("Export table…").clicked(){self.table_export();ui.close_menu();}
                    if ui.button("Save figure…").clicked(){self.figure_export();ui.close_menu();}
                    if ui.button("Export Python script…").clicked(){self.generate_script();self.editor=Editor::Script;ui.close_menu();}
                    if ui.button("Options…").clicked(){self.editor=Editor::Settings;ui.close_menu();}
                    if ui.button("Quit").clicked(){ctx.send_viewport_cmd(egui::ViewportCommand::Close);}
                });
                ui.menu_button("Data",|ui|{
                    if ui.button("Formula column…").clicked(){self.editor=Editor::Formula;ui.close_menu();}
                    if ui.button("Plugin pipeline…").clicked(){self.editor=Editor::Pipeline;ui.close_menu();}
                    if ui.button("Rename table…").clicked(){self.editor=Editor::RenameTable;ui.close_menu();}
                    if ui.button("Rename column…").clicked(){self.editor=Editor::RenameColumn;ui.close_menu();}
                    if ui.button("Delete selected columns").clicked(){for i in self.selected(){let ys=self.selections[&i].ys.clone();self.tables.tabs[i].delete_columns(&ys);}self.sync_selection();ui.close_menu();}
                });
                ui.menu_button("Tools",|ui|{
                    if ui.button("Curve fitting…").clicked(){self.editor=Editor::Fit;ui.close_menu();}
                    if ui.button("Damping / logarithmic decrement…").clicked(){self.editor=Editor::Damping;ui.close_menu();}
                    ui.checkbox(&mut self.measure_enabled,"Measure plot (two clicks)");
                    if ui.button("Clear measurements and fits").clicked(){self.measurements.clear();self.overlays.clear();}
                });
                ui.menu_button("Help",|ui|{ui.label("rustDatView — a Rust implementation of pyDatView");ui.label("Select tables and X/Y columns at left. Pan/zoom with the mouse.\nDouble-click resets plot bounds. Ctrl-drop appends files.\nFormulas use {column name}; settings and pipeline persist on exit.");});
            });
            ui.horizontal_wrapped(|ui|{
                if ui.button("Open").clicked(){self.open_dialog(false);}
                if ui.button("Add").clicked(){self.open_dialog(true);}
                if ui.button("Reload").clicked(){self.reload(false);}
                egui::ComboBox::from_id_salt("format").selected_text(self.format.as_deref().unwrap_or("Auto format")).show_ui(ui,|ui|{
                    ui.selectable_value(&mut self.format,None,"Auto format");for f in &self.formats {ui.selectable_value(&mut self.format,Some(f.name.to_string()),f.name);}
                });
                ui.checkbox(&mut self.options.dayfirst,"Day first");
                if ui.checkbox(&mut self.options.live,"Live plot").changed(){self.dirty=true;}
                ui.checkbox(&mut self.options.watch,"Watch files");
                if ui.button("Plot").clicked(){self.rebuild();}
                ui.separator();
                ui.selectable_value(&mut self.page,Page::Plot,"1D plot");ui.selectable_value(&mut self.page,Page::Data,"Table");
                ui.selectable_value(&mut self.page,Page::Info,"File info");ui.selectable_value(&mut self.page,Page::Fields,"2D fields");
            });
        });
    }
    fn selection_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("selection")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.heading("Data sources");
                ui.add(egui::TextEdit::singleline(&mut self.table_filter).hint_text("Find a table…").desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    if ui.button("All").clicked() {
                        for s in self.selections.values_mut() {
                            s.selected = true;
                        }
                        self.dirty = true;
                    }
                    if ui.button("None").clicked() {
                        for s in self.selections.values_mut() {
                            s.selected = false;
                        }
                        self.dirty = true;
                    }
                    if ui.button("Sort").clicked() {
                        self.tables.sort_by_name();
                        self.selections.clear();
                        self.sync_selection();
                    }
                });
                let names = self.tables.get_display_tab_names();
                egui::ScrollArea::vertical()
                    .id_salt("tables")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for (i, name) in names.iter().enumerate() {
                            if !name
                                .to_lowercase()
                                .contains(&self.table_filter.to_lowercase())
                            {
                                continue;
                            }
                            let s = self.selections.entry(i).or_default();
                            if ui
                                .checkbox(
                                    &mut s.selected,
                                    format!("{name} ({})", self.tables.tabs[i].shapestring()),
                                )
                                .changed()
                            {
                                self.dirty = true;
                            }
                        }
                    });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Merge").clicked() {
                        let idx = self.selected();
                        if !idx.is_empty() {
                            let x = idx.iter().map(|i| self.selections[i].x).collect::<Vec<_>>();
                            self.tables.merge_tabs(
                                Some(&idx),
                                if self.merge_interpolate {
                                    Some(&x)
                                } else {
                                    None
                                },
                                dv_core::common::Extrap::Bounded,
                            );
                            self.sync_selection();
                        }
                    }
                    if ui.button("VStack").clicked() {
                        let idx = self.selected();
                        if !idx.is_empty() {
                            self.tables.vstack(Some(&idx), self.common_only);
                            self.sync_selection();
                        }
                    }
                    if ui.button("Transpose").clicked() {
                        for i in self.selected() {
                            self.tables.tabs[i].transpose();
                        }
                        self.sync_selection();
                    }
                    if ui.button("Delete").clicked() {
                        let idx = self.selected();
                        self.tables.delete_tabs(&idx);
                        self.selections.clear();
                        self.sync_selection();
                    }
                    if ui.button("Reload selected").clicked() {
                        self.reload(true);
                    }
                });
                ui.checkbox(
                    &mut self.merge_interpolate,
                    "Merge by selected X (interpolate)",
                );
                ui.checkbox(&mut self.common_only, "Stack common columns only");
                if let Some(i) = self.primary() {
                    ui.horizontal(|ui| {
                        if ui.button("↑").clicked() && i > 0 {
                            self.tables.swap(i, i - 1);
                            self.selections.clear();
                            self.sync_selection();
                        }
                        if ui.button("↓").clicked() && i + 1 < self.tables.len() {
                            self.tables.swap(i, i + 1);
                            self.selections.clear();
                            self.sync_selection();
                        }
                        let mut file_names =
                            self.tables.naming == dv_core::table::Naming::FileNames;
                        if ui.checkbox(&mut file_names, "File names").changed() {
                            self.tables.naming = if file_names {
                                dv_core::table::Naming::FileNames
                            } else {
                                dv_core::table::Naming::Ellude
                            };
                        }
                    });
                }
                ui.separator();
                ui.heading("Series");
                let before = self.options.selection_mode;
                egui::ComboBox::from_id_salt("selection-mode")
                    .selected_text(match before {
                        SelectionMode::Auto => "Auto",
                        SelectionMode::Common => "Common columns",
                        SelectionMode::Independent => "Independent tables",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.options.selection_mode,
                            SelectionMode::Auto,
                            "Auto",
                        );
                        ui.selectable_value(
                            &mut self.options.selection_mode,
                            SelectionMode::Common,
                            "Common columns",
                        );
                        ui.selectable_value(
                            &mut self.options.selection_mode,
                            SelectionMode::Independent,
                            "Independent tables",
                        );
                    });
                if before != self.options.selection_mode {
                    self.dirty = true;
                }
                let mut selected = self.selected();
                let common = shared_columns(&self.tables, &selected, self.options.selection_mode);
                if common.is_some() && selected.len() > 1 {
                    ui.label(
                        "Primary-table choices apply to matching channels in all selected tables.",
                    );
                    selected.truncate(1);
                }
                egui::ScrollArea::vertical()
                    .id_salt("columns")
                    .show(ui, |ui| {
                        for i in selected {
                            self.channel_picker(ui,i,common.as_ref().map(|m|m[0].as_slice()));
                        }
                    });
            });
    }
}
fn column_values(data: &ColumnData) -> Vec<f64> {
    match data {
        ColumnData::Float(v) | ColumnData::DateTime(v) => v.clone(),
        ColumnData::Str(v) => {
            let mut categories: BTreeMap<&str, usize> = BTreeMap::new();
            v.iter()
                .map(|s| {
                    let next = categories.len();
                    *categories.entry(s).or_insert(next) as f64
                })
                .collect()
        }
    }
}

impl eframe::App for DatView {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (open, reload, add) = ctx.input(|input| {
            (
                input.modifiers.command && input.key_pressed(egui::Key::O),
                input.key_pressed(egui::Key::F5),
                input.modifiers.ctrl,
            )
        });
        if open {
            self.open_dialog(add);
        }
        if reload {
            self.reload(false);
        }
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty() {
            self.load(dropped, add);
        }
        if self.options.watch {
            ctx.request_repaint_after(Duration::from_secs(1));
            if self.last_watch.elapsed() > Duration::from_secs(1) {
                self.last_watch = Instant::now();
                let changed = self.mtimes.iter().any(|(p, old)| {
                    std::fs::metadata(p)
                        .and_then(|m| m.modified())
                        .is_ok_and(|time| time != *old)
                });
                if changed {
                    self.reload(false);
                }
            }
        }
        if ctx.input(|i|i.key_pressed(egui::Key::Escape)){self.presentation.tool=presentation::Tool::Inspect;self.presentation.pending=None;}
        self.menu(ctx);
        self.selection_panel(ctx);
        self.presentation_panel(ctx);
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if self.dirty {
                    ui.label("Plot pending");
                }
                if !self.errors.is_empty() {
                    ui.colored_label(Color32::RED, format!("{} messages", self.errors.len()));
                }
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| match self.page {
            Page::Plot => self.draw_plot(ui),
            Page::Data => self.data_view(ui),
            Page::Info => self.info_view(ui),
            Page::Fields => self.fields_view(ui, ctx),
        });
        self.editor_window(ctx);
        if !self.errors.is_empty() {
            egui::Window::new("Messages")
                .default_width(600.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(350.0)
                        .show(ui, |ui| {
                            for error in &self.errors {
                                ui.colored_label(Color32::RED, error);
                                ui.separator();
                            }
                        });
                    if ui.button("Dismiss").clicked() {
                        self.errors.clear();
                    }
                });
        }
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.options.plot_options = serde_json::to_value(&self.plot_type).unwrap_or(Value::Null);
        self.options.pipeline = serde_json::to_value(&self.pipeline).unwrap_or(Value::Null);
        eframe::set_value(storage, "options", &self.options);
    }
}

fn combo(ui: &mut egui::Ui, label: &str, value: &mut String, choices: &[&str]) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(label)
        .selected_text(format!("{label}: {value}"))
        .show_ui(ui, |ui| {
            for &name in choices {
                changed |= ui.selectable_value(value, name.into(), name).changed();
            }
        });
    changed
}
fn json_edit(ui: &mut egui::Ui, value: &mut Value) -> bool {
    let mut changed = false;
    match value {
        Value::Bool(v) => {
            changed = ui.checkbox(v, "Enabled").changed();
        }
        Value::String(v) => {
            changed = ui.text_edit_singleline(v).changed();
        }
        Value::Number(n) => {
            if let Some(mut v) = n.as_i64() {
                if ui.add(egui::DragValue::new(&mut v)).changed() {
                    *n = v.into();
                    changed = true;
                }
            } else if let Some(mut v) = n.as_u64() {
                if ui.add(egui::DragValue::new(&mut v)).changed() {
                    *n = v.into();
                    changed = true;
                }
            } else if let Some(mut v) = n.as_f64() {
                if ui.add(egui::DragValue::new(&mut v).speed(0.01)).changed() {
                    if let Some(new) = serde_json::Number::from_f64(v) {
                        *n = new;
                        changed = true;
                    }
                }
            }
        }
        Value::Object(map) => {
            ui.vertical(|ui| {
                let mut remove = None;
                for (key, v) in map.iter_mut() {
                    ui.push_id(key, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(key);
                            changed |= json_edit(ui, v);
                            if ui.small_button("×").clicked() {
                                remove = Some(key.clone());
                            }
                        });
                    });
                }
                if let Some(k) = remove {
                    map.remove(&k);
                    changed = true;
                }
                let key_id = ui.make_persistent_id("new key");
                let value_id = ui.make_persistent_id("new value");
                let mut key = ui
                    .ctx()
                    .data_mut(|d| d.get_temp::<String>(key_id))
                    .unwrap_or_default();
                let mut text = ui
                    .ctx()
                    .data_mut(|d| d.get_temp::<String>(value_id))
                    .unwrap_or("1".into());
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut key)
                            .hint_text("new key")
                            .desired_width(100.0),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text("JSON value")
                            .desired_width(100.0),
                    );
                    if ui.small_button("+").clicked() && !key.is_empty() {
                        match serde_json::from_str(&text) {
                            Ok(v) => {
                                map.insert(key.clone(), v);
                                changed = true;
                            }
                            Err(e) => {
                                ui.colored_label(Color32::RED, e.to_string());
                            }
                        }
                    }
                });
                ui.ctx().data_mut(|d| {
                    d.insert_temp(key_id, key);
                    d.insert_temp(value_id, text);
                });
            });
        }
        Value::Array(values) => {
            ui.vertical(|ui| {
                let mut remove = None;
                for (i, v) in values.iter_mut().enumerate() {
                    ui.push_id(i, |ui| {
                        ui.horizontal(|ui| {
                            changed |= json_edit(ui, v);
                            if ui.small_button("×").clicked() {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if let Some(i) = remove {
                    values.remove(i);
                    changed = true;
                }
                if ui.small_button("+ item").clicked() {
                    values.push(Value::from(0));
                    changed = true;
                }
            });
        }
        Value::Null => {
            let id = ui.make_persistent_id("optional value");
            let mut text = ui
                .ctx()
                .data_mut(|d| d.get_temp::<String>(id))
                .unwrap_or("null".into());
            ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("null or JSON value")
                    .desired_width(150.0),
            );
            if ui.small_button("Set").clicked() {
                match serde_json::from_str(&text) {
                    Ok(v) => {
                        *value = v;
                        changed = true;
                    }
                    Err(e) => {
                        ui.colored_label(Color32::RED, e.to_string());
                    }
                }
            }
            ui.ctx().data_mut(|d| d.insert_temp(id, text));
        }
    }
    changed
}

pub fn headless_export(launch: &crate::Launch, path: &std::path::Path) -> Result<(), String> {
    if launch.paths.is_empty() {
        return Err("At least one input file is required for --export".into());
    }
    let mut tables = TableList::default();
    tables.dayfirst = launch.dayfirst;
    let files = launch
        .paths
        .iter()
        .cloned()
        .map(|p| (p, launch.format.clone()))
        .collect::<Vec<_>>();
    let (_, warnings) = tables.load_tables_from_files(&files, false, false, &dv_io::read_file);
    if !warnings.is_empty() {
        return Err(warnings.join("\n"));
    }
    let mut pipeline = match &launch.pipeline {
        Some(p) => Pipeline::load(p).map_err(|e| e.to_string())?,
        None => Pipeline::default(),
    };
    pipeline.apply(&mut tables.tabs, true);
    if !pipeline.errors.is_empty() {
        return Err(pipeline.errors.join("\n"));
    }
    let kind = match &launch.plot_options {
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
        None => view::mode_type(launch.mode.as_deref().unwrap_or("Regular")),
    };
    let mut selections = Vec::new();
    let mut plotted = Vec::new();
    for (i, t) in tables.tabs.iter().enumerate() {
        let x = launch.x.unwrap_or_else(||selection::default_x(t));
        let ys = if launch.ys.is_empty() {
            selection::default_ys(t,x)
        } else {
            launch.ys.clone()
        };
        for y in ys {
            let mut p =
                PlotData::from_table(t, x, y, Some(&pipeline)).map_err(|e| e.to_string())?;
            p.it = i;
            p.transform(&kind).map_err(|e| e.to_string())?;
            plotted.push(p);
            selections.push((i, x, y));
        }
    }
    let compare_mode = if tables.tabs.len() == 1 {
        dv_plot::CompareMode::OneTableColumns
    } else if launch.ys.len() > 1 {
        dv_plot::CompareMode::TablesSameColumns
    } else {
        dv_plot::CompareMode::TablesOneColumn
    };
    if path.extension().is_some_and(|e| e == "py") {
        let mut s = PythonScripter::default();
        s.files = tables.unique_filenames();
        s.selections = selections;
        s.plot_type = kind;
        if launch.mode.as_deref() == Some("Compare") {
            s.comparison = Some((compare_mode, dv_plot::CompareMethod::Relative));
        }
        let script = s
            .generate(&tables.tabs, &pipeline)
            .map_err(|e| e.to_string())?;
        return std::fs::write(path, script).map_err(|e| e.to_string());
    }
    if launch.mode.as_deref() == Some("Compare") {
        plotted =
            dv_plot::plotdata::compare(&plotted, compare_mode, dv_plot::CompareMethod::Relative)
                .map_err(|e| e.to_string())?;
    }
    let curves = plotted
        .iter()
        .map(|p| Curve {
            name: format!("{} — {}", p.st, p.sy),
            sx: p.sx.clone(),
            sy: p.sy.clone(),
            points: column_values(&p.x)
                .into_iter()
                .zip(column_values(&p.y))
                .map(|(x, y)| [x, y])
                .collect(),
        })
        .collect::<Vec<_>>();
    export::save(path, &curves, &Style::default(),&[])
}
