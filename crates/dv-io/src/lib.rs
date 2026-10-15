//! File formats of datview, port of `pydatview/io` (weio).
//!
//! Each group module exposes `formats()` (readers) and `writers()`;
//! [`registry`] assembles them, sorts readers by priority and implements
//! format auto-detection.

pub mod generic;
pub mod misc_formats;
pub mod openfast_input;
pub mod openfast_output;
pub mod postpro;
pub mod registry;

pub use registry::{
    all_formats, detect_format, find_format, read_file, write_dataframe, writers, FileFormat,
    Writer,
};
