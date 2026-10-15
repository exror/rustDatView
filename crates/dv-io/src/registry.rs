//! Format registry and auto-detection (port of `io/__init__.py`,
//! `io/file_formats.py` and `io/converters.py`).

use std::path::Path;
use std::sync::Arc;

use dv_core::{DataFrame, FileObject, IoError};

/// Reader function: opens a file or returns [`IoError::WrongFormat`] when the
/// file is not of this format.
pub type ReadFn = fn(&Path) -> Result<Arc<dyn FileObject>, IoError>;
/// Writer function for a dataframe.
pub type WriteFn = fn(&DataFrame, &Path) -> Result<(), IoError>;

/// A readable file format (port of `FileFormat`).
#[derive(Clone)]
pub struct FileFormat {
    /// Display name (weio `formatName()`), unique across formats.
    pub name: &'static str,
    /// Lower-case extensions including the dot (weio `defaultExtensions()`);
    /// entries containing `*` are glob patterns, e.g. `".out*"`.
    pub extensions: &'static [&'static str],
    /// Detection order: lower is tried first (stable for equal priorities).
    pub priority: i32,
    pub read: ReadFn,
}

impl std::fmt::Debug for FileFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileFormat({}, {:?})", self.name, self.extensions)
    }
}

impl FileFormat {
    /// True if `ext` (lower-case, with dot) matches this format's extensions.
    pub fn matches_extension(&self, ext: &str) -> bool {
        self.extensions.iter().any(|e| {
            if e.contains('*') {
                glob_match(e, ext)
            } else {
                *e == ext
            }
        })
    }
    /// File-dialog filter string, e.g. `"CSV file (*.csv,*.txt)"`.
    pub fn wildcard(&self) -> String {
        let exts: Vec<String> = self.extensions.iter().map(|e| format!("*{e}")).collect();
        format!("{} ({})", self.name, exts.join(","))
    }
}

/// A dataframe writer (port of `converters.writeDataFrameToFormat`).
#[derive(Clone)]
pub struct Writer {
    /// Short name, e.g. `"csv"`, `"outb"`, `"parquet"`.
    pub name: &'static str,
    /// Extensions (lower-case, with dot) mapped to this writer.
    pub extensions: &'static [&'static str],
    pub write: WriteFn,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Writer({}, {:?})", self.name, self.extensions)
    }
}

/// `*` matches any (possibly empty) sequence of characters.
fn glob_match(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            if !rest.starts_with(p) {
                return false;
            }
            rest = &rest[p.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else if let Some(k) = rest.find(p) {
            rest = &rest[k + p.len()..];
        } else {
            return false;
        }
    }
    rest.is_empty()
}

/// All readers, sorted by priority (stable).
pub fn all_formats() -> Vec<FileFormat> {
    let mut v = Vec::new();
    v.extend(crate::generic::formats());
    v.extend(crate::openfast_input::formats());
    v.extend(crate::openfast_output::formats());
    v.extend(crate::misc_formats::formats());
    v.sort_by_key(|f| f.priority);
    v
}

/// All dataframe writers.
pub fn writers() -> Vec<Writer> {
    let mut v = Vec::new();
    v.extend(crate::generic::writers());
    v.extend(crate::openfast_output::writers());
    v.extend(crate::misc_formats::writers());
    v.extend(crate::openfast_input::writers());
    v
}

/// Format with the given display name.
pub fn find_format(name: &str) -> Option<FileFormat> {
    all_formats().into_iter().find(|f| f.name == name)
}

fn lower_ext(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

fn check_file(path: &Path) -> Result<(), IoError> {
    let md = std::fs::metadata(path).map_err(|_| IoError::NotFound(path.to_path_buf()))?;
    if !md.is_file() {
        return Err(IoError::NotFound(path.to_path_buf()));
    }
    if md.len() == 0 {
        return Err(IoError::Empty(path.to_path_buf()));
    }
    Ok(())
}

/// Detect the format of `path` (port of `detectFormat`): formats whose
/// extensions match are tried in priority order; [`IoError::WrongFormat`]
/// moves on to the next one, any other error is returned.
pub fn detect_format(path: &Path) -> Result<(FileFormat, Arc<dyn FileObject>), IoError> {
    check_file(path)?;
    let ext = lower_ext(path);
    for f in all_formats() {
        if !f.matches_extension(&ext) {
            continue;
        }
        match (f.read)(path) {
            Ok(fo) => return Ok((f, fo)),
            Err(IoError::WrongFormat(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(IoError::FormatNotDetected(path.to_path_buf()))
}

/// Read `path` with the named format, or auto-detect when `format` is `None`.
/// Matches the [`dv_core::table::Reader`] signature.
pub fn read_file(path: &Path, format: Option<&str>) -> Result<Arc<dyn FileObject>, IoError> {
    match format {
        None => detect_format(path).map(|(_, fo)| fo),
        Some(name) => {
            check_file(path)?;
            let f = find_format(name)
                .ok_or_else(|| IoError::Other(format!("unknown file format `{name}`")))?;
            (f.read)(path)
        }
    }
}

/// Write a dataframe, choosing the writer by name (`Some("csv")`) or by the
/// extension of `path` (`None`; unknown extensions default to csv).
pub fn write_dataframe(df: &DataFrame, path: &Path, format: Option<&str>) -> Result<(), IoError> {
    let ws = writers();
    let w = match format {
        Some(name) => ws.iter().find(|w| w.name == name),
        None => {
            let ext = lower_ext(path);
            ws.iter()
                .find(|w| w.extensions.contains(&ext.as_str()))
                .or_else(|| ws.iter().find(|w| w.name == "csv"))
        }
    };
    let w = w.ok_or_else(|| {
        IoError::Other(format!(
            "File format not supported for dataframe export `{}`",
            format.unwrap_or("csv")
        ))
    })?;
    (w.write)(df, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob() {
        assert!(glob_match(".out*", ".outb"));
        assert!(glob_match(".out*", ".out"));
        assert!(!glob_match(".out*", ".csv"));
        assert!(glob_match("*", ".anything"));
        assert!(glob_match(".*.txt", ".a.txt"));
    }

    #[test]
    fn format_names_are_unique() {
        let f = all_formats();
        let mut names: Vec<&str> = f.iter().map(|f| f.name).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len(), "duplicate format names");
    }
}
