//! File-object contract shared by the readers in `dv-io` and the GUI.
//!
//! Port of `pydatview/io/file.py` (`File`, the error classes) and of
//! `pydatview/Fields2D.py`.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::dataframe::DataFrame;

/// Errors raised by file readers (port of the weio exception classes).
#[derive(Debug, Error)]
pub enum IoError {
    #[error("file not found: {0}")]
    NotFound(PathBuf),
    #[error("file is empty: {0}")]
    Empty(PathBuf),
    /// The reader does not recognise the file; format auto-detection moves on
    /// to the next candidate format.
    #[error("wrong format: {0}")]
    WrongFormat(String),
    /// The file was recognised but is inconsistent; detection stops here.
    #[error("broken format: {0}")]
    BrokenFormat(String),
    #[error("file format not detected for: {0}")]
    FormatNotDetected(PathBuf),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// A dataframe with the name it has inside its file.
///
/// An empty name means "the file contains a single, unnamed table"
/// (Python readers returning a DataFrame rather than a dict).
#[derive(Debug, Clone)]
pub struct NamedFrame {
    pub name: String,
    pub df: DataFrame,
}

impl NamedFrame {
    pub fn new(name: impl Into<String>, df: DataFrame) -> Self {
        Self {
            name: name.into(),
            df,
        }
    }
    pub fn unnamed(df: DataFrame) -> Self {
        Self {
            name: String::new(),
            df,
        }
    }
}

/// A 2D scalar field (port of one entry of `Fields2D`).
///
/// `m` is stored row-major with shape `(x.len(), y.len())`:
/// `m[i * y.len() + j]` is the value at `(x[i], y[j])`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field2D {
    /// Field name, including unit in brackets when known, e.g. `"u [m/s]"`.
    pub name: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    /// Axis labels, including unit in brackets when known.
    pub sx: String,
    pub sy: String,
    pub m: Vec<f64>,
}

impl Field2D {
    pub fn value(&self, i: usize, j: usize) -> f64 {
        self.m[i * self.y.len() + j]
    }
}

/// A file opened by one of the `dv-io` readers.
///
/// Port of the weio `File` base class as consumed by pyDatView: tables via
/// `toDataFrame`, optional 2D fields via `to2DFields`, and a textual
/// description for the file-info panel.
pub trait FileObject: Send + Sync + std::fmt::Debug {
    /// Name of the format, e.g. `"OpenFAST output"` (weio `formatName()`).
    fn format_name(&self) -> &str;
    /// Path the object was read from.
    fn filename(&self) -> &Path;
    /// Tables contained in the file (weio `toDataFrame`).
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError>;
    /// 2D fields contained in the file (weio `to2DFields`); `None` when the
    /// format has no 2D-field representation.
    fn to_2d_fields(&self) -> Option<Result<Vec<Field2D>, IoError>> {
        None
    }
    /// Human-readable summary of the file content (weio `__repr__`), shown in
    /// the file-info panel.
    fn info(&self) -> String {
        format!("{} file: {}", self.format_name(), self.filename().display())
    }
}
