//! Readers ported from pyDatView's HAWC, FLEX and miscellaneous IO modules.
//!
//! HAWC binary data uses signed little-endian 16-bit samples and SEL channel
//! scaling. Mann boxes infer their dimensions from the filename; callers can
//! instead use [`fields::mann_with_options`] for explicit dimensions/spacing.
//! GNUPlot meshes expose both tables and two-dimensional fields. RAAW's
//! `DataCell` uses the shared nested MATLAB v5/v7.3 decoder in [`mat`].

pub mod bladed;
pub mod cactus;
mod common;
pub mod fields;
pub mod flex;
pub mod hawc;
pub mod mat;
mod mat_hdf;

use crate::registry::{FileFormat, Writer};

pub fn formats() -> Vec<FileFormat> {
    vec![
        FileFormat {
            name: "Tecplot ASCII file",
            extensions: &[".dat"],
            priority: 10,
            read: fields::tecplot,
        },
        FileFormat {
            name: "Bladed output file",
            extensions: &[".%*", ".$*"],
            priority: 10,
            read: bladed::read,
        },
        FileFormat {
            name: "HAWC2 dat file",
            extensions: &[".dat", ".sel"],
            priority: 30,
            read: hawc::dat,
        },
        FileFormat {
            name: "HAWC2 htc file",
            extensions: &[".htc"],
            priority: 30,
            read: hawc::htc,
        },
        FileFormat {
            name: "HAWC2 st file",
            extensions: &[".st", ".dat"],
            priority: 30,
            read: hawc::st,
        },
        FileFormat {
            name: "HAWC2 PC file",
            extensions: &[".dat", ".pc", ".txt"],
            priority: 30,
            read: hawc::pc,
        },
        FileFormat {
            name: "HAWC2 AE file",
            extensions: &[".dat", ".ae", ".txt"],
            priority: 30,
            read: hawc::ae,
        },
        FileFormat {
            name: "HAWCStab2 power file",
            extensions: &[".pwr", ".txt"],
            priority: 30,
            read: hawc::pwr,
        },
        FileFormat {
            name: "HAWCStab2 induction file",
            extensions: &[".ind", ".txt"],
            priority: 30,
            read: hawc::ind,
        },
        FileFormat {
            name: "HAWCStab2 Campbell file",
            extensions: &[".cmb"],
            priority: 30,
            read: hawc::cmb,
        },
        FileFormat {
            name: "HAWC2 Turbulence box",
            extensions: &[".u", ".v", ".w", ".bin"],
            priority: 30,
            read: fields::mann,
        },
        FileFormat {
            name: "FLEX blade file",
            extensions: &[".bld", ".bla", ".00x", ".00*"],
            priority: 40,
            read: flex::blade,
        },
        FileFormat {
            name: "FLEX profile file",
            extensions: &[".pro", ".00x", ".00*"],
            priority: 40,
            read: flex::profile,
        },
        FileFormat {
            name: "FLEX output file",
            extensions: &[".res", ".int"],
            priority: 40,
            read: flex::output,
        },
        FileFormat {
            name: "FLEX WaveKin file",
            extensions: &[".wko"],
            priority: 40,
            read: flex::wavekin,
        },
        FileFormat {
            name: "FLEX documentation file",
            extensions: &[".out", ".doc"],
            priority: 40,
            read: flex::doc,
        },
        FileFormat {
            name: "BModes output file",
            extensions: &[".out"],
            priority: 50,
            read: fields::bmodes,
        },
        FileFormat {
            name: "GNUPlot file",
            extensions: &[".dat", ".raw"],
            priority: 60,
            read: fields::gnuplot,
        },
        FileFormat {
            name: "CACTUS file",
            extensions: &[".in"],
            priority: 70,
            read: cactus::read,
        },
        FileFormat {
            name: "RAAW .mat file",
            extensions: &[".mat"],
            priority: 70,
            read: mat::raaw,
        },
    ]
}

/// Upstream dataframe converters do not define writers for these formats.
pub fn writers() -> Vec<Writer> {
    Vec::new()
}
