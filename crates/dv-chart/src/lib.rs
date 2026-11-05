//! Interactive, pane-based charts and resolution-independent SVG export.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use egui::Color32;
use serde::{Deserialize, Serialize};

pub mod canvas;
pub mod decimate;
pub mod format;
mod interaction;
mod render;
mod theme;
pub mod ticks;

pub use theme::ChartTheme;

pub type SeriesId = u64;
pub type PaneId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Scale {
    #[default]
    Linear,
    Log10,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AxisFormat {
    Number,
    DateTime,
    Duration,
    Category(Arc<[String]>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Dash {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Draw {
    #[default]
    Lines,
    Points,
    LinesPoints,
    Steps,
}

pub struct Series<'a> {
    pub id: SeriesId,
    pub label: &'a str,
    pub unit: &'a str,
    pub x: &'a [f64],
    pub y: &'a [f64],
    pub generation: u64,
    pub color: Color32,
    pub width: f32,
    pub dash: Dash,
    pub draw: Draw,
    pub marker_radius: f32,
    pub visible: bool,
    pub overlay: bool,
}

pub struct Pane<'a> {
    pub id: PaneId,
    pub title: &'a str,
    pub y_label: &'a str,
    pub y_scale: Scale,
    pub y_format: AxisFormat,
    pub y_flip: bool,
    pub include_zero: bool,
    pub series: Vec<Series<'a>>,
    pub weight: f32,
}

#[derive(Clone)]
pub struct ChartConfig<'a> {
    pub x_label: &'a str,
    pub x_scale: Scale,
    pub x_format: AxisFormat,
    pub x_flip: bool,
    pub link_x: bool,
    pub grid: bool,
    pub legend: bool,
    pub legend_values: bool,
    pub crosshair: bool,
    pub overview: bool,
    pub tooltip: bool,
    pub equal_aspect: bool,
    pub gap_factor: Option<f64>,
    pub decimate: bool,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub precision: usize,
}

impl Default for ChartConfig<'_> {
    fn default() -> Self {
        Self {
            x_label: "",
            x_scale: Scale::Linear,
            x_format: AxisFormat::Number,
            x_flip: false,
            link_x: true,
            grid: true,
            legend: true,
            legend_values: true,
            crosshair: true,
            overview: true,
            tooltip: false,
            equal_aspect: false,
            gap_factor: Some(10.0),
            decimate: true,
            title: "",
            subtitle: "",
            precision: 4,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Mark {
    Point {
        pane: Option<PaneId>,
        x: f64,
        y: f64,
        label: String,
        color: Color32,
    },
    Region {
        x0: f64,
        x1: f64,
        label: String,
        color: Color32,
    },
    VLine {
        x: f64,
        label: String,
        color: Color32,
    },
    HLine {
        pane: PaneId,
        y: f64,
        label: String,
        color: Color32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Tool {
    #[default]
    Select,
    Pan,
    Zoom,
}

/// Persistent view state. Transient caches and pointer state are never serialized.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ChartState {
    pub x_view: Option<[f64; 2]>,
    pub pane_x_views: BTreeMap<PaneId, [f64; 2]>,
    pub y_views: BTreeMap<PaneId, [f64; 2]>,
    #[serde(default)]
    pub pane_weights: BTreeMap<PaneId, f32>,
    #[serde(default)]
    pub maximized: Option<PaneId>,
    pub selection: Option<[f64; 2]>,
    pub tool: Tool,
    pub hidden: BTreeSet<SeriesId>,
    #[serde(skip)]
    pub highlighted: Option<SeriesId>,
    #[serde(skip)]
    runtime: interaction::Runtime,
}

impl ChartState {
    /// Fit the complete X domain and restore auto-Y on every pane.
    pub fn fit_all(&mut self) {
        self.x_view = None;
        self.pane_x_views.clear();
        self.y_views.clear();
    }
    /// Set a transient cursor selected by another view. The pointer takes precedence
    /// while it is over the chart; `None` clears the external cursor.
    pub fn set_cursor(&mut self, x: Option<f64>) {
        self.runtime.external_cursor = x.filter(|value| value.is_finite());
    }
    /// Return the externally selected cursor, if any.
    pub fn cursor(&self) -> Option<f64> {
        self.runtime.external_cursor
    }
    /// Copy only session-persisted fields, without copying transient geometry.
    pub fn persistent_snapshot(&self) -> Self {
        Self {
            x_view: self.x_view,
            pane_x_views: self.pane_x_views.clone(),
            y_views: self.y_views.clone(),
            pane_weights: self.pane_weights.clone(),
            maximized: self.maximized,
            selection: self.selection,
            tool: self.tool,
            hidden: self.hidden.clone(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct Hover {
    pub x: f64,
    pub pane: Option<PaneId>,
    pub values: Vec<(SeriesId, usize, f64, f64)>,
}

#[derive(Clone, Copy, Debug)]
pub struct Click {
    pub pane: Option<PaneId>,
    pub series: Option<SeriesId>,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub pane: Option<PaneId>,
    pub series: Option<SeriesId>,
    pub x: f64,
    pub y: f64,
}
/// Pane commands are emitted to the owner. Auto-Y, Y lock and maximize are also
/// applied directly to `ChartState`; the owner handles the other variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneAction {
    ToggleLog(PaneId),
    AutoY(PaneId),
    ToggleIncludeZero(PaneId),
    ToggleLockY(PaneId),
    ToggleMaximize(PaneId),
    Remove(PaneId),
}

pub struct ChartResponse {
    pub hover: Option<Hover>,
    pub clicked: Option<Click>,
    pub context: Option<Context>,
    pub selection_changed: bool,
    pub view_changed: bool,
    pub legend_clicked: Option<SeriesId>,
    pub pane_action: Option<PaneAction>,
    pub response: egui::Response,
}

#[derive(Clone)]
pub struct FontSet {
    pub regular: Arc<[u8]>,
    pub semibold: Arc<[u8]>,
    pub mono: Arc<[u8]>,
    pub family: String,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut ChartState,
    panes: &[Pane<'_>],
    config: &ChartConfig<'_>,
    marks: &[Mark],
    theme: &ChartTheme,
) -> ChartResponse {
    render::show(ui, state, panes, config, marks, theme)
}

pub fn render_svg(
    panes: &[Pane<'_>],
    state: &ChartState,
    config: &ChartConfig<'_>,
    marks: &[Mark],
    theme: &ChartTheme,
    fonts: &FontSet,
    size: [f32; 2],
) -> String {
    render::svg(panes, state, config, marks, theme, fonts, size)
}
