//! Core data model for datview, a Rust port of pyDatView.
//!
//! - [`DataFrame`]: ordered, possibly duplicated, named columns of equal length
//!   (float, string or datetime), the Rust stand-in for a pandas DataFrame.
//! - [`Table`] / [`TableList`]: port of `pydatview/Tables.py`.
//! - [`formula`]: evaluator for the `{Column}`-based expressions used for
//!   formula columns and masks.
//! - [`file`]: the file-object contract implemented by `dv-io` readers.
//! - [`common`]: port of `pydatview/common.py` helpers.

pub mod common;
pub mod dataframe;
pub mod error;
pub mod file;
pub mod formula;
pub mod table;

pub use dataframe::{Column, ColumnData, DataFrame};
pub use error::{DvError, Result};
pub use file::{Field2D, FileObject, IoError, NamedFrame};
pub use table::{Formula, Naming, Table, TableList};
