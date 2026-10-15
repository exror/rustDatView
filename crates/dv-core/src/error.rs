use thiserror::Error;

/// Generic datview error (port of `PyDatViewException` and friends).
#[derive(Debug, Error)]
pub enum DvError {
    /// User-facing error message (equivalent of `PyDatViewException`).
    #[error("{0}")]
    Msg(String),
    #[error("formula error: {0}")]
    Formula(String),
    #[error(transparent)]
    Io(#[from] crate::file::IoError),
}

impl DvError {
    pub fn msg(s: impl Into<String>) -> Self {
        DvError::Msg(s.into())
    }
}

pub type Result<T, E = DvError> = std::result::Result<T, E>;
