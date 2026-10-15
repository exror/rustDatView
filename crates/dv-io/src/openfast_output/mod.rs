//! OpenFAST output, linearization, summary and TurbSim readers and writers.
mod binary;
mod linearization;
pub mod mini_yaml;
mod output;
mod short_descr;
mod summary;
mod turbsim;
mod turbsim_ts;

pub use linearization::{
    EigenAnalysis, FastLinearizationFile, LinearMatrix, LinearizationOptions, OperatingPoint,
};
pub use output::{
    channel_units, find_driver_file, load_ascii_output, load_binary_output, write_ascii,
    write_binary, write_outb, FastOutputFile,
};
pub use short_descr::short_description;
pub use summary::{
    FastSummaryFile, NodeDisplacements, SummaryGraph, SummaryGraphElement, SummaryGraphMode,
    SummaryGraphNode, SummaryModes,
};
pub use turbsim::{TurbSimFile, TurbSimOptions};
pub use turbsim_ts::TurbSimTsFile;

use crate::registry::{FileFormat, Writer};
use dv_core::{FileObject, IoError};
use std::path::Path;
use std::sync::Arc;

fn read_output(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(FastOutputFile::read(path)?))
}
fn read_linearization(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(FastLinearizationFile::read(path)?))
}
fn read_summary(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(FastSummaryFile::read(path)?))
}
fn read_turbsim(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(TurbSimFile::read(path)?))
}
fn read_turbsim_ts(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(TurbSimTsFile::read(path)?))
}

pub fn formats() -> Vec<FileFormat> {
    vec![
        FileFormat {
            name: "FAST output file",
            extensions: &[".out", ".outb", ".elm", ".elev", ".dbg", ".dbg2"],
            priority: 20,
            read: read_output,
        },
        FileFormat {
            name: "FAST linearization output",
            extensions: &[".lin"],
            priority: 20,
            read: read_linearization,
        },
        FileFormat {
            name: "FAST summary file",
            extensions: &[".sum", ".yaml"],
            priority: 20,
            read: read_summary,
        },
        FileFormat {
            name: "TurbSim time series",
            extensions: &[".txt"],
            priority: 20,
            read: read_turbsim_ts,
        },
        FileFormat {
            name: "TurbSim binary",
            extensions: &[".bts"],
            priority: 20,
            read: read_turbsim,
        },
    ]
}
pub fn writers() -> Vec<Writer> {
    vec![Writer {
        name: "outb",
        extensions: &[".outb"],
        write: write_outb,
    }]
}

#[cfg(test)]
mod tests;
