//! Port of `pydatview/plotdata.py`, `pipeline.py`, `plugins/` and `scripter.py`.
pub mod pipeline;
pub mod plotdata;
pub mod plugins;
pub mod scripter;

pub use pipeline::{Action, ActionKind, Pipeline};
pub use plotdata::{
    compare, CompareMethod, CompareMode, FftOptions, Measurement, MinMaxOptions, PdfOptions,
    PlotData, PlotStats, PlotType, PolarOptions,
};
pub use plugins::Plugin;
pub use scripter::{PythonScripter, ScriptOptions};
