use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use dv_core::common::clean_col;
use dv_plot::PlotType;
use eframe::egui::{self, Align, Layout, RichText, Stroke};

use crate::{
    actions::{self, ActionContext, ActionId, Menu, ACTIONS},
    export,
    session::{Restore, Session},
    settings::{self, Settings},
    ui::{
        self,
        chart_view::ChartOptions,
        icons,
        sidebar::Sidebar,
        stats_dock::StatsDock,
        table_view::TableView,
        theme::{self, Appearance},
        widgets,
    },
    workspace::{Mode, Workspace},
    Launch,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Chart,
    Table,
    Info,
    Fields,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Dialog {
    #[default]
    None,
    Settings,
    Shortcuts,
    About,
    OpenFormat,
    Export,
}

struct Toast {
    message: String,
    error: bool,
    created: Instant,
    action: Option<ToastAction>,
}

/// A follow-up offered by a notification, e.g. undoing an automatic change.
enum ToastAction {
    /// Return these sources (by stable key) to absolute time.
    UndoAlign(Vec<String>),
}

impl ToastAction {
    fn label(&self) -> &'static str {
        match self {
            Self::UndoAlign(_) => "Undo",
        }
    }
}

struct SessionOverrides {
    mode: Option<Mode>,
    plot_options: Option<serde_json::Value>,
    pipeline: Option<PathBuf>,
    follow: bool,
}

pub(crate) struct DatView {
    workspace: Workspace,
    settings: Settings,
    appearance: Appearance,
    sidebar: Sidebar,
    inspector: ui::inspector::Inspector,
    welcome: ui::welcome::WelcomeView,
    brand_icon: egui::TextureHandle,
    chart: dv_chart::ChartState,
    chart_options: ChartOptions,
    figure_options: export::FigureOptions,
    stats: StatsDock,
    table: TableView,
    info: ui::info_view::InfoView,
    fields: ui::fields_view::FieldsView,
    view: View,
    dialog: Dialog,
    sidebar_visible: bool,
    stats_visible: bool,
    inspector_visible: bool,
    palette_open: bool,
    palette_query: String,
    palette_focus: bool,
    palette_index: usize,
    reader_format: String,
    cursor: Option<f64>,
    chart_context: Option<dv_chart::Context>,
    session_path: Option<PathBuf>,
    restore: Option<Restore>,
    session_overrides: Option<SessionOverrides>,
    startup_selection: Option<(Option<usize>, Vec<usize>)>,
    deferred_paths: Vec<PathBuf>,
    startup_format: Option<String>,
    loading_started: Option<Instant>,
    last_title: String,
    seen_problems: usize,
    toasts: Vec<Toast>,
    /// Source count at the last automatic-alignment check; `usize::MAX`
    /// suppresses the check for the load in progress (sessions keep their own).
    checked_sources: usize,
    formula: ui::dialogs::formula::FormulaDialog,
    analysis: ui::dialogs::analysis::AnalysisWindow,
    processing: ui::dialogs::pipeline::PipelineDialog,
    data_export: ui::dialogs::exports::DataExportDialog,
    script_export: ui::dialogs::exports::ScriptDialog,
    /// Log Y was switched on automatically by entering Spectrum mode.
    spectrum_log_y: bool,
}

impl DatView {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        let mut settings = Settings::load(cc.storage);
        if launch.dayfirst {
            settings.dayfirst = true;
        }
        let mut appearance = Appearance::new();
        let appearance_error = appearance.apply(&cc.egui_ctx, &settings).err();
        let mut workspace = Workspace::new(settings.dayfirst);
        workspace.follow = launch.follow || settings.watch_default;
        let mut chart_options = ChartOptions::default();
        if let Some(options) = launch.plot_options.as_ref() {
            if let Err(error) = set_plot_options(&mut workspace, options) {
                workspace.problems.push(error);
            }
        }
        if let Some(mode) = launch.mode {
            workspace.mode = mode;
        }
        if workspace.mode == Mode::Spectrum {
            chart_options.log_y = true;
        }
        if let Some(path) = &launch.pipeline {
            match dv_plot::Pipeline::load(path) {
                Ok(pipeline) => workspace.pipeline = pipeline,
                Err(error) => workspace
                    .problems
                    .push(format!("Could not load processing pipeline: {error}")),
            }
        }
        if let Some(error) = appearance_error {
            workspace.problems.push(error);
        }
        let session_overrides = SessionOverrides {
            mode: launch.mode,
            plot_options: launch.plot_options.clone(),
            pipeline: launch.pipeline.clone(),
            follow: launch.follow,
        };
        let selection = if launch.x.is_some() || !launch.ys.is_empty() {
            Some((launch.x, launch.ys))
        } else {
            None
        };
        let icon = crate::icon();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        let brand_icon =
            cc.egui_ctx
                .load_texture("rustdatview-mark", image, egui::TextureOptions::LINEAR);
        let mut app = Self {
            workspace,
            settings,
            appearance,
            sidebar: Sidebar::default(),
            inspector: ui::inspector::Inspector::default(),
            welcome: ui::welcome::WelcomeView::default(),
            brand_icon,
            chart: dv_chart::ChartState::default(),
            chart_options,
            figure_options: export::FigureOptions::default(),
            stats: StatsDock::default(),
            table: TableView::default(),
            info: ui::info_view::InfoView::default(),
            fields: ui::fields_view::FieldsView::default(),
            view: View::Chart,
            dialog: Dialog::None,
            sidebar_visible: true,
            stats_visible: true,
            inspector_visible: false,
            palette_open: false,
            palette_query: String::new(),
            palette_focus: false,
            palette_index: 0,
            reader_format: String::new(),
            cursor: None,
            chart_context: None,
            session_path: None,
            restore: None,
            session_overrides: None,
            startup_selection: selection,
            deferred_paths: Vec::new(),
            startup_format: launch.format,
            loading_started: None,
            last_title: String::new(),
            seen_problems: 0,
            toasts: Vec::new(),
            checked_sources: 0,
            formula: Default::default(),
            analysis: Default::default(),
            processing: Default::default(),
            data_export: Default::default(),
            script_export: Default::default(),
            spectrum_log_y: false,
        };
        let session = launch.session.or_else(|| {
            if app.settings.restore_last_session && launch.paths.is_empty() {
                app.settings.recent_sessions.first().cloned()
            } else {
                None
            }
        });
        if let Some(path) = session {
            app.session_overrides = Some(session_overrides);
            app.deferred_paths = launch.paths;
            app.open_session(path);
        } else if !launch.paths.is_empty() {
            app.open_paths(launch.paths, false, app.startup_format.clone());
        }
        app
    }

    fn action_context(&self) -> ActionContext {
        ActionContext {
            has_sources: !self.workspace.tables.is_empty(),
            has_series: !self.workspace.series.is_empty(),
            has_fields: self.fields.available(),
        }
    }

    fn notify(&mut self, message: impl Into<String>, error: bool) {
        self.toasts.push(Toast {
            message: message.into(),
            error,
            created: Instant::now(),
            action: None,
        });
    }

    /// Separate recordings opened together are compared from their starts.
    /// Overlapping recordings (simultaneous loggers) keep absolute time.
    fn align_separate_recordings(&mut self) {
        let Some(indices) = self.workspace.separate_recordings() else { return };
        let mut keys = Vec::with_capacity(indices.len());
        for index in indices {
            let source = &mut self.workspace.sources[index];
            source.align_start = true;
            keys.push(source.stable_key());
        }
        let count = keys.len();
        self.chart.fit_all();
        self.chart.selection = None;
        self.workspace.rebuild();
        self.toasts.push(Toast {
            message: format!("{count} separate recordings aligned at their start"),
            error: false,
            created: Instant::now(),
            action: Some(ToastAction::UndoAlign(keys)),
        });
    }

    fn run_toast_action(&mut self, action: ToastAction) {
        match action {
            ToastAction::UndoAlign(keys) => {
                for source in &mut self.workspace.sources {
                    if keys.contains(&source.stable_key()) {
                        source.align_start = false;
                    }
                }
                self.chart.fit_all();
                self.chart.selection = None;
                self.workspace.rebuild();
            }
        }
    }

    fn remember(list: &mut Vec<PathBuf>, path: PathBuf) {
        list.retain(|entry| entry != &path);
        list.insert(0, path);
        list.truncate(12);
    }

    fn open_paths(&mut self, paths: Vec<PathBuf>, add: bool, format: Option<String>) {
        if paths.is_empty() {
            return;
        }
        for path in &paths {
            Self::remember(&mut self.settings.recent_files, path.clone());
        }
        if !add {
            self.chart = dv_chart::ChartState::default();
            self.restore = None;
            self.session_path = None;
            self.cursor = None;
        }
        self.workspace.load(paths, add, format);
    }

    fn open_dialog(&mut self, add: bool, format: Option<String>) {
        if let Some(paths) = rfd::FileDialog::new()
            .set_title(if add { "Add files" } else { "Open files" })
            .pick_files()
        {
            self.open_paths(paths, add, format);
        }
    }

    fn handle_welcome(&mut self, request: ui::welcome::WelcomeRequest) {
        use ui::welcome::WelcomeRequest;
        match request {
            WelcomeRequest::OpenFile => self.open_dialog(false, None),
            WelcomeRequest::OpenRecent(path) => self.open_paths(vec![path], false, None),
            WelcomeRequest::OpenSample(paths) => self.open_paths(paths, false, None),
            WelcomeRequest::OpenSession(path) => self.open_session(path),
            WelcomeRequest::SampleError(error) => self.workspace.problems.push(error),
        }
    }

    fn open_session(&mut self, path: PathBuf) {
        self.checked_sources = usize::MAX;
        match Session::load(&path).and_then(|session| session.begin(&mut self.workspace)) {
            Ok(restore) => {
                Self::remember(&mut self.settings.recent_sessions, path.clone());
                self.session_path = Some(path);
                self.restore = Some(restore);
                self.view = View::Chart;
            }
            Err(error) => self
                .workspace
                .problems
                .push(format!("Could not open session: {error}")),
        }
    }

    fn save_session(&mut self, save_as: bool) {
        let path = if save_as {
            None
        } else {
            self.session_path.clone()
        }
        .or_else(|| {
            rfd::FileDialog::new()
                .set_title("Save session")
                .add_filter("rustDatView session", &["dvsession"])
                .set_file_name("workspace.dvsession")
                .save_file()
        });
        let Some(path) = path else {
            return;
        };
        match Session::capture(
            &self.workspace,
            &self.chart,
            &self.chart_options,
            &self.figure_options,
            &self.stats.settings,
            self.restore.as_ref(),
        )
        .and_then(|session| session.save(&path))
        {
            Ok(()) => {
                Self::remember(&mut self.settings.recent_sessions, path.clone());
                self.session_path = Some(path);
                let unsaved = self.workspace.sources.iter().zip(&self.workspace.tables.tabs).filter(|(source, table)| {
                    source.path.as_os_str().is_empty()
                        && source.recipe.is_none()
                        && !self.workspace.pipeline.actions.iter().any(|action| action.generated_table(table))
                }).count();
                if unsaved > 0 {
                    self.workspace.problems.push("Fit results are not saved in sessions; re-run the fit after opening it.".into());
                }
                self.notify("Session saved", false);
            }
            Err(error) => self
                .workspace
                .problems
                .push(format!("Could not save session: {error}")),
        }
    }

    fn locate_source(&mut self, original: PathBuf) {
        let Some(replacement) = rfd::FileDialog::new()
            .set_title(format!("Locate {}", original.display()))
            .pick_file()
        else {
            return;
        };
        let snapshot = Session::capture(
            &self.workspace,
            &self.chart,
            &self.chart_options,
            &self.figure_options,
            &self.stats.settings,
            self.restore.as_ref(),
        );
        match snapshot.and_then(|mut session| {
            session.relocate(&original, replacement)?;
            session.begin(&mut self.workspace)
        }) {
            Ok(restore) => self.restore = Some(restore),
            Err(error) => self
                .workspace
                .problems
                .push(format!("Could not locate source: {error}")),
        }
    }

    pub(crate) fn run_action(&mut self, id: ActionId, ctx: &egui::Context) {
        match id {
            ActionId::Open => self.open_dialog(false, None),
            ActionId::AddFiles => self.open_dialog(true, None),
            ActionId::OpenWithFormat => self.dialog = Dialog::OpenFormat,
            ActionId::Reload => self.workspace.reload(),
            ActionId::SaveSession => self.save_session(false),
            ActionId::SaveSessionAs => self.save_session(true),
            ActionId::OpenSession => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("rustDatView session", &["dvsession"])
                    .pick_file()
                {
                    self.open_session(path);
                }
            }
            ActionId::Settings => self.dialog = Dialog::Settings,
            ActionId::ExportFigure => self.dialog = Dialog::Export,
            ActionId::CopyFigure => match export::copy(
                ctx,
                &self.workspace,
                &self.chart,
                &self.settings,
                &self.chart_options,
                &self.figure_options,
                theme::is_dark(ctx),
            ) {
                Ok(()) => self.notify("Figure copied", false),
                Err(error) => self
                    .workspace
                    .problems
                    .push(format!("Could not copy figure: {error}")),
            },
            ActionId::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            ActionId::ViewChart => self.view = View::Chart,
            ActionId::ViewTable => self.view = View::Table,
            ActionId::ViewInfo => self.view = View::Info,
            ActionId::ViewFields => self.view = View::Fields,
            ActionId::ModeSignals => self.set_mode(Mode::Signals),
            ActionId::ModeDistribution => self.set_mode(Mode::Distribution),
            ActionId::ModeSpectrum => self.set_mode(Mode::Spectrum),
            ActionId::ModeNormalized => self.set_mode(Mode::Normalized),
            ActionId::ModeCompare => self.set_mode(Mode::Compare),
            ActionId::ModePolar => self.set_mode(Mode::Polar),
            ActionId::ToggleStats => self.stats_visible = !self.stats_visible,
            ActionId::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            ActionId::ToggleInspector => self.inspector_visible = !self.inspector_visible,
            ActionId::ToggleFollow => self.workspace.follow = !self.workspace.follow,
            ActionId::Search => {
                self.sidebar_visible = true;
                self.sidebar.focus_search = true;
            }
            ActionId::Palette => {
                self.palette_open = true;
                self.palette_focus = true;
                self.palette_query.clear();
                self.palette_index = 0;
            }
            ActionId::Shortcuts => self.dialog = Dialog::Shortcuts,
            ActionId::About => self.dialog = Dialog::About,
            ActionId::Fit => self.chart.fit_all(),
            ActionId::NewChannel => self.formula.open_new(),
            ActionId::Processing => self.processing.open(&self.workspace),
            ActionId::ExportData => self.data_export.open(&self.workspace, &self.chart),
            ActionId::ExportScript => self.script_export.open(),
            ActionId::CurveFit => self.analysis.open_tool(ui::dialogs::analysis::Tool::Fit),
            ActionId::Damping => self
                .analysis
                .open_tool(ui::dialogs::analysis::Tool::Damping),
            ActionId::Fatigue => self
                .analysis
                .open_tool(ui::dialogs::analysis::Tool::Fatigue),
            ActionId::StatsReport => self.analysis.open_tool(ui::dialogs::analysis::Tool::Report),
        }
    }

    fn set_mode(&mut self, mode: Mode) {
        if self.workspace.mode == mode {
            return;
        }
        // Spectra are read on a log scale (pyDatView does the same); undo it
        // when leaving, unless the user had log Y on already.
        if self.workspace.mode == Mode::Spectrum && self.spectrum_log_y {
            self.chart_options.log_y = false;
            self.spectrum_log_y = false;
        }
        if mode == Mode::Spectrum && !self.chart_options.log_y {
            self.chart_options.log_y = true;
            self.spectrum_log_y = true;
        }
        self.workspace.mode = mode;
        self.chart_options.pane_log_y.clear();
        // X means something else in every mode (time, frequency, value, angle).
        self.chart.x_view = None;
        self.chart.pane_x_views.clear();
        self.chart.y_views.clear();
        self.chart.selection = None;
        self.chart.set_cursor(None);
        self.cursor = None;
        self.workspace.rebuild();
    }

    fn update_loading(&mut self, ctx: &egui::Context) {
        let followed_view = if self.workspace.follow {
            self.chart.x_view.and_then(|view| {
                data_end(&self.workspace)
                    .filter(|end| {
                        (view[1] - end).abs() <= ((view[1] - view[0]).abs() * 0.01).max(1e-9)
                    })
                    .map(|end| (view, end))
            })
        } else {
            None
        };
        let changed = self.workspace.poll();
        if changed {
            self.workspace.rebuild();
            if let Some((view, old_end)) = followed_view {
                if let Some(end) = data_end(&self.workspace).filter(|end| *end > old_end) {
                    self.chart.x_view = Some([end - (view[1] - view[0]), end]);
                }
            }
            ctx.request_repaint();
        }
        let source_count = self.workspace.sources.len();
        if self.workspace.loading.is_none() && self.restore.as_ref().is_none_or(|restore| !restore.is_pending()) {
            if source_count > self.checked_sources {
                self.align_separate_recordings();
            }
            self.checked_sources = source_count;
        }
        if let Some(restore) = &mut self.restore {
            if let Some(view) = restore.finish(&mut self.workspace) {
                self.chart = view.chart;
                self.chart_options = view.chart_options;
                self.figure_options = view.figure_options;
                self.stats.restore_settings(view.stats);
                if let Some(overrides) = self.session_overrides.take() {
                    if let Some(value) = &overrides.plot_options {
                        if let Err(error) = set_plot_options(&mut self.workspace, value) {
                            self.workspace.problems.push(error);
                        }
                    }
                    if let Some(mode) = overrides.mode {
                        self.workspace.mode = mode;
                    }
                    if self.workspace.mode == Mode::Spectrum
                        && (overrides.mode.is_some() || overrides.plot_options.is_some())
                    {
                        self.chart_options.log_y = true;
                        self.chart_options.pane_log_y.clear();
                    }
                    if let Some(path) = overrides.pipeline {
                        match dv_plot::Pipeline::load(path) {
                            Ok(pipeline) => self.workspace.pipeline = pipeline,
                            Err(error) => self
                                .workspace
                                .problems
                                .push(format!("Could not load processing pipeline: {error}")),
                        }
                    }
                    self.workspace.follow |= overrides.follow;
                    self.workspace.rebuild();
                }
                if !self.deferred_paths.is_empty() {
                    let paths = std::mem::take(&mut self.deferred_paths);
                    self.open_paths(paths, true, self.startup_format.clone());
                }
            }
        }
        if self.workspace.loading.is_none()
            && !self.workspace.tables.is_empty()
            && self.restore.as_ref().is_none_or(|r| !r.is_pending())
        {
            if let Some((x, ys)) = self.startup_selection.take() {
                if let Err(error) = apply_indices(&mut self.workspace, x, &ys) {
                    self.workspace.problems.push(error);
                }
                self.workspace.rebuild();
            }
        }
        if self.workspace.loading.is_some() {
            self.loading_started.get_or_insert_with(Instant::now);
        } else {
            self.loading_started = None;
        }
        if self.workspace.loading.is_some() || self.workspace.follow {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.workspace.problems.len() < self.seen_problems {
            self.seen_problems = 0;
        }
        for error in self.workspace.problems[self.seen_problems..].to_vec() {
            self.notify(error, true);
        }
        self.seen_problems = self.workspace.problems.len();
        // Notifications offering an action stay long enough to use it.
        self.toasts.retain(|toast| {
            let lifetime = if toast.action.is_some() { 10 } else { 4 };
            toast.error || toast.created.elapsed() < Duration::from_secs(lifetime)
        });
        if self.toasts.iter().any(|toast| !toast.error) {
            ctx.request_repaint_after(Duration::from_millis(300));
        }
        let title = match self.workspace.sources.as_slice() {
            [] => "rustDatView".to_owned(),
            [source] => format!("{} — rustDatView", source.name),
            [source, rest @ ..] => format!("{} + {} more — rustDatView", source.name, rest.len()),
        };
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
    }

    fn topbar(&mut self, root: &mut egui::Ui, ctx: &egui::Context) {
        const MODES: [(Mode, &str, &str); 6] = [
            (
                Mode::Signals,
                "Signals",
                "Signals over X (pyDatView: Regular) · Alt+1",
            ),
            (
                Mode::Distribution,
                "Distribution",
                "Probability density of each signal (PDF) · Alt+2",
            ),
            (
                Mode::Spectrum,
                "Spectrum",
                "Frequency content (FFT) · Alt+3",
            ),
            (
                Mode::Normalized,
                "Normalized",
                "Signals scaled to their min/max range (MinMax) · Alt+4",
            ),
            (
                Mode::Compare,
                "Compare",
                "Difference or ratio against a reference signal · Alt+5",
            ),
            (Mode::Polar, "Polar", "Angle/radius view · Alt+6"),
        ];
        const VIEWS: [(View, &str, &str); 4] = [
            (View::Chart, "Chart", "Interactive chart · Ctrl+1"),
            (View::Table, "Table", "Raw data · Ctrl+2"),
            (View::Info, "Info", "File and table details · Ctrl+3"),
            (View::Fields, "Fields", "2D fields · Ctrl+4"),
        ];
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let action_context = self.action_context();
        let has_sources = !self.workspace.tables.is_empty();
        let mut chosen = None;
        let frame = egui::Frame::new()
            .fill(tokens.panel_bg)
            .inner_margin(egui::Margin::symmetric(12, 6))
            .stroke(Stroke::new(1.0, tokens.border));
        egui::Panel::top("topbar").frame(frame).show(root, |ui| {
            ui.horizontal_centered(|ui| {
                ui.image((self.brand_icon.id(), egui::vec2(22.0, 22.0)));
                ui.label(
                    RichText::new("rustDatView")
                        .family(theme::semibold())
                        .size(14.0)
                        .color(tokens.text),
                );
                ui.add_space(10.0);
                egui::MenuBar::new().ui(ui, |ui| {
                    for menu in Menu::ALL {
                        let entries: Vec<_> =
                            ACTIONS.iter().filter(|entry| entry.menu == menu).collect();
                        if entries.is_empty() {
                            continue;
                        }
                        ui.menu_button(menu.label(), |ui| {
                            ui.set_min_width(230.0);
                            let mut group = entries[0].group;
                            for action in entries {
                                if action.group != group {
                                    ui.separator();
                                    group = action.group;
                                }
                                let mut button = egui::Button::new(action.label);
                                if let Some(shortcut) = &action.shortcut {
                                    button = button.shortcut_text(shortcut.text(ctx));
                                }
                                if ui
                                    .add_enabled(action.is_enabled(&action_context), button)
                                    .clicked()
                                {
                                    chosen = Some(action.id);
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, icons::PALETTE, "Command palette · Ctrl+K")
                        .clicked()
                    {
                        chosen = Some(ActionId::Palette);
                    }
                    if !has_sources {
                        return;
                    }
                    ui.add_space(6.0);
                    let views = if self.fields.available() { &VIEWS[..] } else { &VIEWS[..3] };
                    if let Some(view) = widgets::segmented(ui, self.view, views) {
                        self.view = view;
                    }
                    if self.view == View::Chart {
                        ui.add_space(14.0);
                        if let Some(mode) = widgets::segmented(ui, self.workspace.mode, &MODES) {
                            self.set_mode(mode);
                        }
                    }
                });
            });
        });
        if let Some(id) = chosen {
            if let Some(action) = actions::find(id) {
                action.execute(self, ctx, &action_context);
            }
        }
    }

    fn statusbar(&mut self, root: &mut egui::Ui, ctx: &egui::Context) {
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let mut clear_problems = false;
        let mut copy_problems = false;
        let mut locate = None;
        let missing_count = self
            .restore
            .as_ref()
            .map_or(0, |restore| restore.unresolved().len());
        egui::Panel::bottom("statusbar")
            .frame(
                egui::Frame::new()
                    .fill(tokens.panel_bg)
                    .inner_margin(egui::Margin::symmetric(12, 5))
                    .stroke(Stroke::new(1.0, tokens.border)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(loading) = &self.workspace.loading {
                        ui.spinner();
                        let elapsed = self
                            .loading_started
                            .map_or(0.0, |started| started.elapsed().as_secs_f32());
                        ui.label(
                            RichText::new(format!("{loading}  ·  {elapsed:.1} s"))
                                .size(11.5)
                                .color(tokens.text_secondary),
                        );
                    } else if let Some((source, table)) = self
                        .workspace
                        .sources
                        .first()
                        .zip(self.workspace.tables.tabs.first())
                    {
                        let rows = table.nrows();
                        let channels = table.ncols().saturating_sub(1);
                        let mut summary = format!(
                            "{}  ·  {} {}  ·  {} {}",
                            source.name,
                            format_count(rows),
                            if rows == 1 { "row" } else { "rows" },
                            format_count(channels),
                            if channels == 1 { "channel" } else { "channels" }
                        );
                        if let Some((duration, hz)) = sampling_summary(&self.workspace) {
                            summary.push_str(&format!(
                                "  ·  {}  ·  {}",
                                dv_chart::format::format_value(hz, "Hz", 3),
                                format_duration(duration)
                            ));
                        }
                        ui.label(
                            RichText::new(summary)
                                .size(11.5)
                                .color(tokens.text_secondary),
                        );
                    } else {
                        ui.label(RichText::new("Ready").size(11.5).color(tokens.text_muted));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.workspace.problems.is_empty() || missing_count != 0 {
                            egui::containers::menu::MenuButton::from_button(
                                egui::Button::new((
                                    RichText::new(icons::WARNING)
                                        .family(theme::icon_family())
                                        .color(tokens.warning),
                                    self.workspace.problems.len().max(missing_count).to_string(),
                                ))
                                .small()
                                .corner_radius(6)
                                .stroke(Stroke::new(1.0, tokens.border_strong)),
                            )
                            .ui(ui, |ui| {
                                    ui.set_max_width(460.0);
                                    ui.heading("Problems");
                                    egui::ScrollArea::vertical()
                                        .max_height(240.0)
                                        .show(ui, |ui| {
                                            ui.vertical(|ui| {
                                                for message in &self.workspace.problems {
                                                    ui.add(egui::Label::new(message).wrap());
                                                    ui.separator();
                                                }
                                            });
                                        });
                                    if let Some(restore) = &self.restore {
                                        for missing in restore.unresolved() {
                                            ui.horizontal(|ui| {
                                                if missing.entry.generated {
                                                    ui.label(&missing.reason);
                                                } else {
                                                    ui.label(
                                                        missing
                                                            .entry
                                                            .source
                                                            .path
                                                            .display()
                                                            .to_string(),
                                                    )
                                                    .on_hover_text(&missing.reason);
                                                    if ui.button("Locate…").clicked() {
                                                        locate =
                                                            Some(missing.entry.source.path.clone());
                                                        ui.close();
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    ui.horizontal(|ui| {
                                        if ui.button("Copy").clicked() {
                                            copy_problems = true;
                                            ui.close();
                                        }
                                        if ui.button("Clear").clicked() {
                                            clear_problems = true;
                                            ui.close();
                                        }
                                    });
                            })
                            .0
                            .on_hover_text("Show problems");
                        }
                        let following = self.workspace.follow;
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(if following { "●  Following" } else { "Follow" })
                                        .size(11.5)
                                        .color(if following { tokens.accent } else { tokens.text_secondary }),
                                )
                                .small()
                                .corner_radius(6)
                                .fill(if following { tokens.accent_soft } else { tokens.surface })
                                .stroke(Stroke::new(1.0, if following { tokens.accent } else { tokens.border_strong })),
                            )
                            .on_hover_text("Watch source files for new data and keep the latest samples in view.")
                            .clicked()
                        {
                            self.workspace.follow = !following;
                        }
                        if let Some(cursor) = self.cursor.filter(|_| !self.workspace.series.is_empty()) {
                            let value = ui::chart_view::x_readout(&self.workspace, cursor, self.settings.precision);
                            let label = self.workspace.x_key.as_ref()
                                .and_then(|key| self.workspace.channels.iter().find(|channel| &channel.key == key))
                                .map(|channel| channel.label.as_str())
                                .filter(|label| !label.is_empty() && self.workspace.mode == Mode::Signals);
                            let text = match label {
                                Some(label) => format!("{label} = {value}"),
                                None => value,
                            };
                            ui.label(RichText::new(text).size(11.5).color(tokens.text_secondary));
                        }
                    });
                });
            });
        if copy_problems {
            let mut messages = self.workspace.problems.clone();
            if let Some(restore) = &self.restore {
                for missing in restore.unresolved() {
                    if !messages.iter().any(|message| message.contains(&missing.reason)) {
                        messages.push(missing.reason.clone());
                    }
                }
            }
            ctx.copy_text(messages.join("\n"));
        }
        if clear_problems {
            self.workspace.problems.clear();
            self.seen_problems = 0;
            self.toasts.retain(|t| !t.error);
        }
        if let Some(original) = locate {
            self.locate_source(original);
        }
    }

    fn chart_context_menu(&mut self, response: &dv_chart::ChartResponse, ctx: &egui::Context) {
        use crate::panes::{Annotation, LineDraw, PaneSettings, RangeRule, SeriesStyle};

        if let Some(context) = response.context {
            self.chart_context = Some(context);
        }
        let Some(context) = self.chart_context else { return };
        let mut rebuild = false;
        let mut copy_figure = false;
        response.response.context_menu(|ui| {
            let palette = theme::palette_for(theme::is_dark(ctx), self.settings.palette);
            let target = context.series.and_then(|id| self.workspace.series.iter().find(|s| s.id == id))
                .map(|s| {
                    let [r, g, b, _] = ui::chart_view::series_color(&self.workspace, &palette, s).to_srgba_unmultiplied();
                    (s.id, s.key.clone(), s.label.clone(), [r, g, b], s.dash,
                        self.workspace.mode == Mode::Signals && s.family == "Flags")
                });
            let anchor = context.series.and_then(|id| self.workspace.series.iter().find(|s| s.id == id))
                .or_else(|| self.workspace.series.iter().find(|s| context.pane == Some(ui::chart_view::pane_id(&s.pane))));
            let point_id = anchor.map(|s| s.id);
            let pane = anchor.map(|s| s.pane.clone());
            let unit = anchor.map(|s| s.unit.clone()).unwrap_or_default();
            if let Some((id, key, label, current_color, current_dash, flag)) = &target {
                ui.strong(label);
                if ui.button(if self.chart.hidden.contains(id) { "Show series" } else { "Hide series" }).clicked() {
                    if !self.chart.hidden.remove(id) { self.chart.hidden.insert(*id); }
                    ui.close();
                }
                if ui.button("Show only this series").clicked() {
                    self.chart.hidden = self.workspace.series.iter().filter(|s| s.id != *id).map(|s| s.id).collect();
                    ui.close();
                }
                if ui.button("Remove series").clicked() {
                    self.workspace.panes.removed.insert(*id, label.clone());
                    self.chart.hidden.remove(id);
                    rebuild = true;
                    ui.close();
                }
                ui.menu_button("Style", |ui| {
                    let mut style = self.workspace.panes.style_for(*id, key).cloned().unwrap_or_default();
                    let mut changed = false;
                    let mut color = *current_color;
                    ui.horizontal(|ui| {
                        ui.label("Color");
                        if ui.color_edit_button_srgb(&mut color).changed() {
                            style.color = Some(color);
                            changed = true;
                        }
                    });
                    let mut width = style.width.unwrap_or(self.settings.line_width);
                    if ui.add(egui::Slider::new(&mut width, 0.5..=6.0).text("Width")).changed() {
                        style.width = Some(width);
                        changed = true;
                    }
                    ui.menu_button("Dash", |ui| {
                        for (dash, name) in [(0, "Solid"), (1, "Dashed"), (2, "Dotted"), (3, "Dash-dot")] {
                            if ui.selectable_label(style.dash.unwrap_or(*current_dash) == dash, name).clicked() {
                                style.dash = Some(dash);
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                    ui.menu_button("Draw", |ui| {
                        let default = if *flag { LineDraw::Steps } else { LineDraw::Lines };
                        for (draw, name) in [(LineDraw::Lines, "Lines"), (LineDraw::Points, "Points"),
                            (LineDraw::LinesPoints, "Lines + points"), (LineDraw::Steps, "Steps")] {
                            if ui.selectable_label(style.draw.unwrap_or(default) == draw, name).clicked() {
                                style.draw = Some(draw);
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                    if ui.button("Reset style").clicked() {
                        style = SeriesStyle::default();
                        changed = true;
                    }
                    if changed {
                        self.workspace.panes.styles.insert(PaneSettings::series_style_key(*id), style);
                        rebuild = true;
                    }
                });
                ui.separator();
            }
            if ui.add_enabled(point_id.is_some() && context.x.is_finite() && context.y.is_finite(), egui::Button::new("Add point annotation here")).clicked() {
                if let Some(id) = point_id {
                    self.workspace.annotations.push(Annotation::Point {
                        mode: self.workspace.mode, curve: RangeRule::series_target(id),
                        x: context.x, y: context.y, label: "Point".into(), color: ui::inspector::MARK_COLOR,
                    });
                }
                ui.close();
            }
            if ui.add_enabled(self.chart.selection.is_some(), egui::Button::new("Add region from selection")).clicked() {
                if let Some([a, b]) = self.chart.selection {
                    if let Some(region) = Annotation::region(self.workspace.mode, self.workspace.x_key.clone().unwrap_or_default(),
                        a, b, "Selection".into(), ui::inspector::MARK_COLOR) {
                        self.workspace.annotations.push(region);
                    }
                }
                ui.close();
            }
            if ui.add_enabled(pane.is_some() && context.y.is_finite(), egui::Button::new("Add reference line at this value")).clicked() {
                if let Some(pane) = &pane {
                    self.workspace.annotations.push(Annotation::HLine {
                        mode: self.workspace.mode, pane: pane.clone(), y: context.y,
                        label: "Reference".into(), color: ui::inspector::MARK_COLOR,
                    });
                }
                ui.close();
            }
            if ui.add_enabled(context.x.is_finite(), egui::Button::new("Add event marker here")).clicked() {
                self.workspace.annotations.push(Annotation::VLine {
                    mode: self.workspace.mode, x_axis: self.workspace.x_key.clone().unwrap_or_default(),
                    x: context.x, label: "Event".into(), color: ui::inspector::MARK_COLOR,
                });
                ui.close();
            }
            ui.separator();
            if ui.add_enabled(self.chart.selection.is_some(), egui::Button::new("Zoom to selection")).clicked() {
                if let Some([a, b]) = self.chart.selection {
                    let range = [a.min(b), a.max(b)];
                    if self.chart_options.link_x {
                        self.chart.x_view = Some(range);
                    } else {
                        for series in &self.workspace.series {
                            self.chart.pane_x_views.insert(ui::chart_view::pane_id(&series.pane), range);
                        }
                    }
                }
                ui.close();
            }
            if ui.button("Fit all").clicked() { self.chart.fit_all(); ui.close(); }
            if ui.button("Fit Y in every pane").clicked() { self.chart.y_views.clear(); ui.close(); }
            if ui.add_enabled(context.x.is_finite() && context.y.is_finite(), egui::Button::new("Copy X and Y"))
                .on_hover_text("Copy the position under the cursor as tab-separated text")
                .clicked()
            {
                let x = ui::chart_view::x_readout(&self.workspace, context.x, self.settings.precision);
                let y = dv_chart::format::format_value(context.y, &unit, self.settings.precision);
                ctx.copy_text(format!("{x}\t{y}"));
                ui.close();
            }
            ui.separator();
            if ui.button("Copy figure").clicked() { copy_figure = true; ui.close(); }
            if ui.button("Export figure…").clicked() { self.dialog = Dialog::Export; ui.close(); }
        });
        if rebuild { self.workspace.rebuild(); }
        if copy_figure { self.run_action(ActionId::CopyFigure, ctx); }
    }

    fn command_palette(&mut self, ctx: &egui::Context) {
        if !self.palette_open {
            return;
        }
        enum Pick {
            Action(ActionId),
            Channel(String),
            Source(usize),
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let state = self.action_context();
        let query = self.palette_query.trim().to_lowercase();
        let mut choices: Vec<(String, String, &str, Pick)> = Vec::new();
        for action in ACTIONS {
            if action.is_enabled(&state) && fuzzy(&action.label.to_lowercase(), &query) {
                let hint = action
                    .shortcut
                    .as_ref()
                    .map(|s| s.text(ctx))
                    .unwrap_or_default();
                choices.push((
                    action.label.to_owned(),
                    hint,
                    action.menu.label(),
                    Pick::Action(action.id),
                ));
            }
        }
        if !query.is_empty() {
            for index in self
                .workspace
                .search(&self.palette_query)
                .into_iter()
                .take(30)
            {
                let channel = &self.workspace.channels[index];
                let shown = if self.workspace.selected.contains(&channel.key) {
                    "Hide"
                } else {
                    "Plot"
                };
                choices.push((
                    format!("{shown} {}", channel.label),
                    channel.unit.clone(),
                    "Channel",
                    Pick::Channel(channel.key.clone()),
                ));
            }
            for (index, source) in self.workspace.sources.iter().enumerate() {
                if fuzzy(&source.name.to_lowercase(), &query) {
                    let verb = if source.active {
                        "Hide source"
                    } else {
                        "Show source"
                    };
                    choices.push((
                        format!("{verb} {}", source.name),
                        String::new(),
                        "Source",
                        Pick::Source(index),
                    ));
                }
            }
        }
        let (up, down, enter, escape) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if down {
            self.palette_index += 1;
        }
        if up {
            self.palette_index = self.palette_index.saturating_sub(1);
        }
        self.palette_index = self.palette_index.min(choices.len().saturating_sub(1));
        let mut pick = None;
        let modal = egui::Modal::new(egui::Id::unique("command-palette"))
            .area(
                egui::Modal::default_area(egui::Id::unique("command-palette-area"))
                    .anchor(egui::Align2::CENTER_TOP, [0.0, 90.0]),
            )
            .show(ctx, |ui| {
                ui.set_width(560.0);
                let response = widgets::search_field(
                    ui,
                    &mut self.palette_query,
                    "Type a command or channel name…",
                );
                if response.changed() {
                    self.palette_index = 0;
                }
                if self.palette_focus {
                    response.request_focus();
                    self.palette_focus = false;
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if choices.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("No matching commands or channels")
                                    .color(tokens.text_muted),
                            );
                        }
                        for (index, (label, hint, kind, _)) in choices.iter().enumerate() {
                            let (rect, row) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 30.0),
                                egui::Sense::click(),
                            );
                            let active = index == self.palette_index;
                            if active || row.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    6.0,
                                    if active {
                                        tokens.accent_soft
                                    } else {
                                        tokens.surface
                                    },
                                );
                            }
                            if active && (up || down) {
                                row.scroll_to_me(None);
                            }
                            let painter = ui.painter();
                            painter.text(
                                rect.left_center() + egui::vec2(10.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                label,
                                egui::FontId::proportional(13.0),
                                tokens.text,
                            );
                            let right = painter.text(
                                rect.right_center() - egui::vec2(10.0, 0.0),
                                egui::Align2::RIGHT_CENTER,
                                hint,
                                egui::FontId::proportional(11.5),
                                tokens.text_muted,
                            );
                            painter.text(
                                egui::pos2(right.left() - 14.0, rect.center().y),
                                egui::Align2::RIGHT_CENTER,
                                *kind,
                                egui::FontId::proportional(11.0),
                                tokens.text_muted.gamma_multiply(0.8),
                            );
                            if row.clicked() {
                                pick = Some(index);
                            }
                        }
                    });
                if enter && !choices.is_empty() {
                    pick = Some(self.palette_index);
                }
            });
        if escape || modal.should_close() {
            self.palette_open = false;
        }
        if let Some(index) = pick {
            self.palette_open = false;
            match choices.swap_remove(index).3 {
                Pick::Action(id) => {
                    if let Some(action) = actions::find(id) {
                        action.execute(self, ctx, &state);
                    }
                }
                Pick::Channel(key) => {
                    self.workspace.toggle(&key);
                    self.workspace.rebuild();
                }
                Pick::Source(index) => {
                    if let Some(source) = self.workspace.sources.get_mut(index) {
                        source.active = !source.active;
                        self.workspace.rebuild();
                    }
                }
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Windows (unlike modals) do not close on Escape by themselves.
        if matches!(
            self.dialog,
            Dialog::Settings | Dialog::Shortcuts | Dialog::About | Dialog::OpenFormat
        ) && (matches!(self.dialog, Dialog::Settings) || !ctx.text_edit_focused())
            && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.dialog = Dialog::None;
            return;
        }
        match self.dialog {
            Dialog::None => {}
            Dialog::Settings => {
                let before = self.settings.clone();
                let mut open = true;
                settings::show(ctx, &mut self.settings, &mut self.appearance, &mut open);
                if self.settings != before {
                    self.workspace.tables.dayfirst = self.settings.dayfirst;
                    if let Err(error) = self.appearance.apply(ctx, &self.settings) {
                        self.workspace.problems.push(error);
                    }
                }
                if !open {
                    self.dialog = Dialog::None;
                }
            }
            Dialog::Shortcuts => {
                let mut open = true;
                egui::Window::new("Keyboard shortcuts")
                    .open(&mut open)
                    .collapsible(false)
                    .default_width(470.0)
                    .default_height(670.0)
                    .show(ctx, |ui| {
                        ui.weak("Shortcuts are grouped by their menu.");
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .max_height(600.0)
                            .show(ui, |ui| {
                                for menu in Menu::ALL {
                                    if !ACTIONS.iter().any(|action| {
                                        action.menu == menu && action.shortcut.is_some()
                                    }) {
                                        continue;
                                    }
                                    widgets::section_label(ui, menu.label());
                                    egui::Grid::new(("shortcut-grid", menu.label()))
                                        .num_columns(2)
                                        .min_col_width(190.0)
                                        .spacing([20.0, 5.0])
                                        .show(ui, |ui| {
                                            for action in ACTIONS.iter().filter(|action| action.menu == menu) {
                                                if let Some(shortcut) = &action.shortcut {
                                                    ui.label(action.label);
                                                    ui.horizontal(|ui| {
                                                        ui.spacing_mut().item_spacing.x = 3.0;
                                                        for key in shortcut.text(ctx).split('+') {
                                                            widgets::kbd_hint(ui, key);
                                                        }
                                                        if let Some(alternate) = &action.alternate_shortcut {
                                                            ui.weak("or");
                                                            for key in alternate.text(ctx).split('+') {
                                                                widgets::kbd_hint(ui, key);
                                                            }
                                                        }
                                                    });
                                                    ui.end_row();
                                                }
                                            }
                                        });
                                    ui.add_space(12.0);
                                }
                            });
                    });
                if !open {
                    self.dialog = Dialog::None;
                }
            }
            Dialog::About => {
                let mut open = true;
                egui::Window::new("About rustDatView")
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .default_width(380.0)
                    .show(ctx, |ui| {
                        ui.add_space(10.0);
                        self.welcome.show_logo(ui, 260.0);
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                                .strong(),
                        );
                        ui.label("A fast, beautiful data viewer for engineers.");
                        ui.add_space(8.0);
                        ui.hyperlink_to(
                            "rustDatView on GitHub",
                            "https://github.com/exror/rustDatView",
                        );
                    });
                if !open {
                    self.dialog = Dialog::None;
                }
            }
            Dialog::OpenFormat => {
                let mut open = true;
                let mut choose = false;
                egui::Window::new("Open with format")
                    .open(&mut open)
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label("Choose the reader for this file.");
                        egui::ComboBox::from_id_salt("reader-format")
                            .width(320.0)
                            .selected_text(if self.reader_format.is_empty() {
                                "Auto detect"
                            } else {
                                &self.reader_format
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.reader_format,
                                    String::new(),
                                    "Auto detect",
                                );
                                for format in dv_io::all_formats() {
                                    ui.selectable_value(
                                        &mut self.reader_format,
                                        format.name.to_owned(),
                                        format.name,
                                    );
                                }
                            });
                        if ui.button("Choose file…").clicked() {
                            choose = true;
                        }
                    });
                if choose {
                    let format =
                        (!self.reader_format.is_empty()).then(|| self.reader_format.clone());
                    self.open_dialog(false, format);
                    open = false;
                }
                if !open {
                    self.dialog = Dialog::None;
                }
            }
            Dialog::Export => {
                let mut open = true;
                if let Some(result) = export::show_dialog(
                    ctx,
                    &mut open,
                    &mut self.figure_options,
                    &self.workspace,
                    &self.chart,
                    &self.settings,
                    &self.chart_options,
                    theme::is_dark(ctx),
                ) {
                    match result {
                        Ok(message) => self.notify(message, false),
                        Err(error) => self.workspace.problems.push(error),
                    }
                }
                if !open {
                    self.dialog = Dialog::None;
                }
            }
        }
    }

    fn toast_overlay(&mut self, root: &egui::Ui, ctx: &egui::Context) {
        if self.toasts.is_empty() {
            return;
        }
        let tokens = theme::Tokens::for_dark(theme::is_dark(ctx));
        let mut dismissed = None;
        let mut acted = None;
        egui::Area::new(root.make_persistent_id("notifications"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-18.0, -44.0])
            .show(ctx, |ui| {
                for (index, toast) in self.toasts.iter().enumerate().rev().take(3) {
                    egui::Frame::new()
                        .fill(tokens.raised)
                        .stroke(Stroke::new(1.0, tokens.border_strong))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.set_width(360.0);
                            ui.horizontal_top(|ui| {
                                ui.label(
                                    RichText::new(if toast.error { icons::WARNING_CIRCLE } else { icons::CHECK_CIRCLE })
                                        .family(theme::icon_family())
                                        .size(16.0)
                                        .color(if toast.error { tokens.error } else { tokens.success }),
                                );
                                let text_width = if toast.action.is_some() { 220.0 } else { 280.0 };
                                ui.allocate_ui_with_layout(
                                    egui::vec2(text_width, 0.0),
                                    Layout::top_down(Align::Min),
                                    |ui| { ui.add(egui::Label::new(&toast.message).wrap()); },
                                );
                                if let Some(action) = &toast.action {
                                    if ui.button(RichText::new(action.label()).color(tokens.accent)).clicked() {
                                        acted = Some(index);
                                    }
                                }
                                if widgets::icon_button(ui, icons::CLOSE, "Dismiss notification").clicked() {
                                    dismissed = Some(index);
                                }
                            });
                        });
                    ui.add_space(5.0);
                }
            });
        if let Some(index) = acted {
            if let Some(action) = self.toasts.remove(index).action {
                self.run_toast_action(action);
            }
        } else if let Some(index) = dismissed {
            self.toasts.remove(index);
        }
    }
}

impl eframe::App for DatView {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.save(storage);
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.update_loading(&ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if !dropped.is_empty() {
            let add = ctx.input(|i| i.modifiers.ctrl || i.modifiers.shift);
            self.open_paths(dropped, add, None);
        }
        self.fields.sync(&mut self.workspace);
        if self.view == View::Fields && !self.fields.available() {
            self.view = View::Chart;
        }
        self.topbar(root, &ctx);
        self.statusbar(root, &ctx);
        let has_sources = !self.workspace.tables.is_empty();
        if has_sources && self.sidebar_visible {
            let tokens = theme::Tokens::for_dark(theme::is_dark(&ctx));
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(280.0)
                .min_size(220.0)
                .frame(
                    egui::Frame::new()
                        .fill(tokens.panel_bg)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .stroke(Stroke::new(1.0, tokens.border)),
                )
                .show(root, |ui| {
                    let result = self.sidebar.show(ui, &mut self.workspace, &self.settings);
                    if result.add {
                        self.open_dialog(true, None);
                    }
                    if let Some(source) = result.reload {
                        self.workspace.reload_source(source);
                    }
                    if let Some(source) = result.table {
                        self.table.source = source;
                        self.view = View::Table;
                    }
                    if result.info {
                        self.view = View::Info;
                    }
                    if let Some(source) = result.export {
                        self.data_export.open_source(&self.workspace, &self.chart, source);
                    }
                    match result.formula {
                        Some(ui::sidebar::FormulaRequest::New) => self.formula.open_new(),
                        Some(ui::sidebar::FormulaRequest::Template(template, key)) => self
                            .formula
                            .open_template(&self.workspace, template, Some(&key)),
                        Some(ui::sidebar::FormulaRequest::Edit(key)) => {
                            self.formula.open_edit(&self.workspace, &key);
                        }
                        None => {}
                    }
                    if result.processing {
                        self.processing.open(&self.workspace);
                    }
                });
        }
        if has_sources && self.stats_visible && self.view == View::Chart {
            let tokens = theme::Tokens::for_dark(theme::is_dark(&ctx));
            // Room for a toolbar, a header and about six rows; never fewer than three.
            egui::Panel::bottom("stats")
                .resizable(true)
                .default_size(220.0)
                .min_size(170.0)
                .frame(
                    egui::Frame::new()
                        .fill(tokens.panel_bg)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .stroke(Stroke::new(1.0, tokens.border)),
                )
                .show(root, |ui| {
                    let palette =
                        theme::palette_for(theme::is_dark(ui.ctx()), self.settings.palette);
                    if let Some(error) = self.stats.show(
                        ui,
                        &self.workspace,
                        &self.chart,
                        self.chart_options.link_x,
                        self.settings.precision,
                        &palette,
                    ) {
                        self.workspace.problems.push(error);
                    }
                });
        }
        if has_sources && self.inspector_visible && self.view == View::Chart {
            let tokens = theme::Tokens::for_dark(theme::is_dark(&ctx));
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(330.0)
                .min_size(280.0)
                .max_size(560.0)
                .frame(
                    egui::Frame::new()
                        .fill(tokens.panel_bg)
                        .inner_margin(egui::Margin::symmetric(12, 10))
                        .stroke(Stroke::new(1.0, tokens.border)),
                )
                .show(root, |ui| {
                    if self.inspector.show(ui, &mut self.workspace, &mut self.chart, &self.settings, &self.chart_options, self.cursor) {
                        self.inspector_visible = false;
                    }
                });
        }
        egui::CentralPanel::default().show(root, |ui| {
            if !has_sources {
                if let Some(request) = self.welcome.show(ui, &mut self.settings) {
                    self.handle_welcome(request);
                }
            } else {
                match self.view {
                    View::Chart => {
                        if ui::mode_bar::show(ui, &mut self.workspace) {
                            self.workspace.rebuild();
                        }
                        let response = ui::chart_view::show(
                            ui,
                            &mut self.workspace,
                            &mut self.chart,
                            &self.settings,
                            &mut self.chart_options,
                            &mut self.inspector_visible,
                        );
                        self.cursor = response.hover.as_ref().map(|hover| hover.x).or(self.cursor);
                        self.chart_context_menu(&response, &ctx);
                    }
                    View::Table => {
                        if let Some(x) = self.table.show(ui, &self.workspace, self.cursor) {
                            self.cursor = Some(x);
                        }
                    }
                    View::Info => self.info.show(ui, &self.workspace),
                    View::Fields => {
                        if let Some(message) = self.fields.show(ui) {
                            let error = message.starts_with("Could not");
                            self.notify(message, error);
                        }
                    }
                }
            }
        });
        let state = self.action_context();
        actions::handle_shortcuts(self, &ctx, &state);
        self.command_palette(&ctx);
        self.dialogs(&ctx);
        let accent = theme::Tokens::for_dark(theme::is_dark(&ctx)).accent;
        match self.formula.show(&ctx, &mut self.workspace, accent) {
            Some(ui::dialogs::formula::FormulaOutcome::Added(message)) => {
                self.notify(message, false)
            }
            Some(ui::dialogs::formula::FormulaOutcome::Failed(error)) => self.notify(error, true),
            None => {}
        }
        for outcome in [
            self.data_export.show(&ctx, &self.workspace, &self.chart),
            self.script_export
                .show(&ctx, &self.workspace, &self.chart_options),
        ]
        .into_iter()
        .flatten()
        {
            match outcome {
                ui::dialogs::exports::ExportOutcome::Done(message) => self.notify(message, false),
                ui::dialogs::exports::ExportOutcome::Failed(error) => self.notify(error, true),
            }
        }
        match self.processing.show(&ctx, &mut self.workspace) {
            Some(ui::dialogs::pipeline::PipelineOutcome::Applied(message)) => {
                self.notify(message, false)
            }
            Some(ui::dialogs::pipeline::PipelineOutcome::Failed(error)) => self.notify(error, true),
            None => {}
        }
        match self.analysis.show(&ctx, &mut self.workspace, &self.chart) {
            Some(ui::dialogs::analysis::AnalysisOutcome::Notify(message)) => {
                self.notify(message, false)
            }
            Some(ui::dialogs::analysis::AnalysisOutcome::AddTable(df, name)) => {
                self.workspace.add_generated_table(df, &name);
                self.notify(format!("Added “{name}” as a source"), false);
            }
            None => {}
        }
        self.toast_overlay(root, &ctx);
    }
}

fn data_end(workspace: &Workspace) -> Option<f64> {
    workspace
        .series
        .iter()
        .filter_map(|series| {
            series
                .x
                .iter()
                .rev()
                .copied()
                .find(|value| value.is_finite())
        })
        .max_by(f64::total_cmp)
}

fn sampling_summary(workspace: &Workspace) -> Option<(f64, f64)> {
    use dv_core::{
        units::{parse_unit, UnitFamily},
        ColumnData,
    };
    let key = workspace.x_key.as_ref()?;
    let table = workspace.tables.tabs.first()?;
    let column = table
        .data
        .columns()
        .iter()
        .find(|column| clean_col(&column.name) == *key)?;
    let factor = match &column.data {
        ColumnData::DateTime(_) => 1.0,
        ColumnData::Float(_) => {
            let unit = workspace
                .channels
                .iter()
                .find(|channel| &channel.key == key)
                .map(|channel| parse_unit(&channel.unit))?;
            if unit.family != UnitFamily::Time {
                return None;
            }
            unit.to_base
        }
        ColumnData::Str(_) => return None,
    };
    let values = match &column.data {
        ColumnData::Float(values) | ColumnData::DateTime(values) => values.as_slice(),
        ColumnData::Str(_) => return None,
    };
    let first = values.iter().copied().find(|v| v.is_finite())?;
    let last = values.iter().rev().copied().find(|v| v.is_finite())?;
    let duration = (last - first) * factor;
    if duration <= 0.0 || values.len() < 2 {
        return None;
    }
    Some((duration, (values.len() - 1) as f64 / duration))
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.round().max(0.0) as u64;
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    let remainder = seconds % 60;
    if hours > 0 {
        format!("{hours} h {minutes:02} min")
    } else if minutes > 0 && remainder > 0 {
        format!("{minutes} min {remainder} s")
    } else if minutes > 0 {
        format!("{minutes} min")
    } else {
        format!("{remainder} s")
    }
}

fn fuzzy(name: &str, query: &str) -> bool {
    query.split('|').any(|alternative| {
        alternative.split_whitespace().all(|token| {
            let mut chars = name.chars();
            token.chars().all(|wanted| chars.any(|c| c == wanted))
        })
    })
}

fn format_count(value: usize) -> String {
    let text = value.to_string();
    let mut output = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index != 0 && (text.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}

fn apply_indices(workspace: &mut Workspace, x: Option<usize>, ys: &[usize]) -> Result<(), String> {
    let table = workspace.tables.tabs.first().ok_or("No table was loaded")?;
    let columns = table.data.columns();
    let x_key = x
        .map(|index| {
            columns
                .get(index)
                .map(|column| clean_col(&column.name))
                .ok_or_else(|| format!("X column index {index} is out of range"))
        })
        .transpose()?;
    let y_keys = ys
        .iter()
        .map(|&index| {
            columns
                .get(index)
                .map(|column| clean_col(&column.name))
                .ok_or_else(|| format!("Y column index {index} is out of range"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(key) = x_key {
        workspace.set_x(&key);
    }
    if !ys.is_empty() {
        workspace.selected = y_keys;
    }
    Ok(())
}

fn set_plot_options(workspace: &mut Workspace, value: &serde_json::Value) -> Result<(), String> {
    let kind: PlotType =
        serde_json::from_value(value.clone()).map_err(|e| format!("Invalid plot options: {e}"))?;
    match kind {
        PlotType::Regular => workspace.mode = Mode::Signals,
        PlotType::Pdf(options) => {
            workspace.mode = Mode::Distribution;
            workspace.options.pdf = options;
        }
        PlotType::Fft(options) => {
            workspace.mode = Mode::Spectrum;
            workspace.options.fft = options;
        }
        PlotType::MinMax(options) => {
            workspace.mode = Mode::Normalized;
            workspace.options.normalized = options;
        }
        PlotType::Polar(options) => {
            workspace.mode = Mode::Polar;
            workspace.options.polar = options;
        }
    }
    Ok(())
}

pub(crate) fn headless_export(launch: &Launch, path: &Path) -> Result<(), String> {
    if launch.paths.is_empty() && launch.session.is_none() {
        return Err("At least one input file or session is required for --export".into());
    }
    let mut workspace = Workspace::new(launch.dayfirst);
    let mut chart = dv_chart::ChartState::default();
    let mut chart_options = ChartOptions::default();
    if let Some(session) = &launch.session {
        let session = Session::load(session)?;
        let mut restore = session.begin(&mut workspace)?;
        while restore.is_pending() {
            workspace.poll();
            if let Some(view) = restore.finish(&mut workspace) {
                chart = view.chart;
                chart_options = view.chart_options;
            }
            if restore.is_pending() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        if !restore.unresolved().is_empty() {
            return Err(restore
                .unresolved()
                .iter()
                .map(|missing| missing.reason.as_str())
                .collect::<Vec<_>>()
                .join("\n"));
        }
    }
    if let Some(pipeline) = &launch.pipeline {
        workspace.pipeline = dv_plot::Pipeline::load(pipeline).map_err(|e| e.to_string())?;
    }
    if !launch.paths.is_empty() {
        workspace.load(
            launch.paths.clone(),
            launch.session.is_some(),
            launch.format.clone(),
        );
        while workspace.loading.is_some() {
            workspace.poll();
            if workspace.loading.is_some() {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    if !workspace.problems.is_empty() {
        return Err(workspace.problems.join("\n"));
    }
    if let Some(value) = &launch.plot_options {
        set_plot_options(&mut workspace, value)?;
    }
    if let Some(mode) = launch.mode {
        workspace.mode = mode;
    }
    if workspace.mode == Mode::Spectrum && (launch.mode.is_some() || launch.plot_options.is_some())
    {
        chart_options.log_y = true;
        chart_options.pane_log_y.clear();
    }
    apply_indices(&mut workspace, launch.x, &launch.ys)?;
    workspace.rebuild();
    while workspace.loading.is_some() {
        workspace.poll();
        workspace.rebuild();
        if workspace.loading.is_some() {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    if !workspace.problems.is_empty() {
        return Err(workspace.problems.join("\n"));
    }
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("py"))
    {
        let text = export::python_script(
            &workspace,
            &chart_options,
            dv_plot::ScriptOptions::default(),
        )?;
        return std::fs::write(path, text).map_err(|e| e.to_string());
    }
    export::save(
        path,
        &workspace,
        &chart,
        &Settings::default(),
        &chart_options,
    )
}
