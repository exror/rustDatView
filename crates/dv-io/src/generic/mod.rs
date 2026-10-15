//! Generic tabular readers and CSV/Parquet exporters.
//!
//! NetCDF supports classic/CDF-2 and NetCDF-4 through a safe Rust HDF5 decoder.
//! MATLAB supports level-5 and v7.3 numeric/text arrays and nested cells/structs
//! via the shared MAT decoder; sparse/object arrays are explicitly rejected.
//! Pickle decoding never executes Python: built-in tables, NumPy numeric/string
//! arrays and pandas BlockManager tables are supported, not arbitrary objects.

pub mod csv;
pub mod excel;
pub(crate) mod hdf;
pub mod matlab;
pub mod netcdf;
mod netcdf_hdf;
pub mod parquet;
pub mod pickle;
pub mod tdms;
pub mod vtk;

use crate::registry::{FileFormat, Writer};
use dv_core::{DataFrame, Field2D, FileObject, IoError, NamedFrame};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
struct GenericFile {
    path: PathBuf,
    format: &'static str,
    frames: Vec<NamedFrame>,
    fields: Option<Vec<Field2D>>,
    detail: String,
}

impl FileObject for GenericFile {
    fn format_name(&self) -> &str {
        self.format
    }
    fn filename(&self) -> &Path {
        &self.path
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        Ok(self.frames.clone())
    }
    fn to_2d_fields(&self) -> Option<Result<Vec<Field2D>, IoError>> {
        self.fields.as_ref().map(|f| Ok(f.clone()))
    }
    fn info(&self) -> String {
        let tables = self
            .frames
            .iter()
            .map(|f| {
                format!(
                    "{}: {} rows, {} columns",
                    f.name,
                    f.df.nrows(),
                    f.df.ncols()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("{}\n{}\n{}", self.format, tables, self.detail)
    }
}

fn object(
    path: &Path,
    format: &'static str,
    frames: Vec<NamedFrame>,
    fields: Option<Vec<Field2D>>,
    detail: String,
) -> Arc<dyn FileObject> {
    Arc::new(GenericFile {
        path: path.into(),
        format,
        frames,
        fields,
        detail,
    })
}

fn frame(columns: Vec<dv_core::Column>) -> Result<DataFrame, IoError> {
    DataFrame::from_columns(columns).map_err(|e| IoError::BrokenFormat(e.to_string()))
}

pub(crate) fn broken(e: impl std::fmt::Display) -> IoError {
    IoError::BrokenFormat(e.to_string())
}

pub fn formats() -> Vec<FileFormat> {
    vec![
        FileFormat {
            name: "CSV file",
            extensions: &[".csv", ".txt"],
            priority: 0,
            read: csv::read,
        },
        FileFormat {
            name: "Excel file",
            extensions: &[".xls", ".xlsx"],
            priority: 0,
            read: excel::read,
        },
        FileFormat {
            name: "Parquet file",
            extensions: &[".parquet"],
            priority: 60,
            read: parquet::read,
        },
        FileFormat {
            name: "NetCDF file (<=2D)",
            extensions: &[".nc"],
            priority: 60,
            read: netcdf::read,
        },
        FileFormat {
            name: "TDMS file",
            extensions: &[".tdms"],
            priority: 60,
            read: tdms::read,
        },
        FileFormat {
            name: "VTK file",
            extensions: &[".vtk", ".vtp", ".vti", ".vtr", ".vts", ".vtu"],
            priority: 60,
            read: vtk::read,
        },
        FileFormat {
            name: "Matlab mat file",
            extensions: &[".mat"],
            priority: 60,
            read: matlab::read,
        },
        FileFormat {
            name: "Pickle file",
            extensions: &[".pkl"],
            priority: 60,
            read: pickle::read,
        },
    ]
}

pub fn writers() -> Vec<Writer> {
    vec![
        Writer {
            name: "csv",
            extensions: &[".csv", ".txt"],
            write: csv::write,
        },
        Writer {
            name: "parquet",
            extensions: &[".parquet"],
            write: parquet::write,
        },
    ]
}
