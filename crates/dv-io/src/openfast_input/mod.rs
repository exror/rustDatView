//! OpenFAST key/value inputs and module tables, linked input decks, deterministic
//! wind files, and ROSCO controller/performance files.
//!
//! Tables retain their original column names and units; `FileObject` conversion
//! uses upstream `name_[unit]` names and the specialized blade/polar quantities.

mod deck;
mod input;
mod rosco;
mod specialized;
mod wind;

pub use deck::FastInputDeck;
pub use input::{FastInputFile, InputEntry, InputTable, InputValue};
pub use rosco::{RoscoDisconFile, RoscoPerformanceFile};
pub use wind::{FastWindFile, StepDuration, StepWindOptions, WIND_COLUMNS};

use crate::registry::{FileFormat, Writer};
use dv_core::{FileObject, IoError};
use std::{path::Path, sync::Arc};

fn read_input(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(FastInputFile::read(path)?))
}
fn read_wind(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(FastWindFile::read(path)?))
}
fn read_discon(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(RoscoDisconFile::read(path)?))
}
fn read_performance(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(Arc::new(RoscoPerformanceFile::read(path)?))
}

pub fn formats() -> Vec<FileFormat> {
    vec![
        FileFormat {
            name: "FAST input file",
            extensions: &[".dat", ".fst", ".txt", ".fstf", ".dvr"],
            priority: 20,
            read: read_input,
        },
        FileFormat {
            name: "FAST determ. wind file",
            extensions: &[".wnd"],
            priority: 20,
            read: read_wind,
        },
        FileFormat {
            name: "ROSCO DISCON file",
            extensions: &[".in"],
            priority: 50,
            read: read_discon,
        },
        FileFormat {
            name: "ROSCO Performance file",
            extensions: &[".txt"],
            priority: 50,
            read: read_performance,
        },
    ]
}
pub fn writers() -> Vec<Writer> {
    vec![Writer {
        name: "wnd",
        extensions: &[".wnd"],
        write: wind::write_wind_dataframe,
    }]
}
