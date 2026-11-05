//! Format registry and auto-detection (port of `io/__init__.py`,
//! `io/file_formats.py` and `io/converters.py`).

use std::io::Read;
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
    /// Normal detection order: lower is tried first (stable for equal
    /// priorities). Distinctive legacy text signatures take precedence.
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
/// A bounded, decoded probe prevents rejecting large unrelated text only after
/// reading it all. Invalid probe text is not evidence of a specialized format.
pub(crate) fn text_prefix(path: &Path) -> Result<Option<String>, IoError> {
    const PROBE_BYTES: u64 = 64 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(PROBE_BYTES)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Err(IoError::Empty(path.to_path_buf()));
    }
    if bytes.len() == PROBE_BYTES as usize {
        // Do not split a multibyte character at the probe boundary.
        let end = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
            let newline = if bytes[0] == 0xff {
                [b'\n', 0]
            } else {
                [0, b'\n']
            };
            bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .rposition(|pair| pair == newline.as_slice())
                .map(|i| 2 + (i + 1) * 2)
        } else {
            bytes.iter().rposition(|&byte| byte == b'\n').map(|i| i + 1)
        };
        if let Some(end) = end {
            bytes.truncate(end);
        }
    }
    Ok(crate::generic::csv::decode_text(&bytes)
        .ok()
        .map(|(text, _)| text.into_owned()))
}

fn distinctive_text_format(text: &str) -> Option<&'static str> {
    if crate::openfast_output::looks_like_turbsim_ts(text) {
        Some("TurbSim time series")
    } else if crate::openfast_input::looks_like_rosco_performance(text) {
        Some("ROSCO Performance file")
    } else if let Some(ind) = crate::misc_formats::hawc::stab_header_kind(text) {
        Some(if ind {
            "HAWCStab2 induction file"
        } else {
            "HAWCStab2 power file"
        })
    } else if let Some(pc) = crate::misc_formats::hawc::polar_header_kind(text) {
        Some(if pc { "HAWC2 PC file" } else { "HAWC2 AE file" })
    } else if crate::openfast_input::looks_like_fast_input(text) {
        Some("FAST input file")
    } else {
        None
    }
}
fn read_candidate(
    format: &FileFormat,
    path: &Path,
    csv_options: Option<&crate::generic::csv::CsvOptions>,
) -> Result<Arc<dyn FileObject>, IoError> {
    match csv_options {
        Some(options) if format.name == "CSV file" => {
            crate::generic::csv::read_with_options(path, options)
        }
        _ => (format.read)(path),
    }
}

/// Detect the format of `path`. Distinctive legacy `.txt` signatures are
/// tried before the generic reader; otherwise matching extensions are tried
/// in priority order. [`IoError::WrongFormat`] moves on to the next reader,
/// while any other error is returned.
pub fn detect_format(path: &Path) -> Result<(FileFormat, Arc<dyn FileObject>), IoError> {
    detect_format_with_options(path, None)
}

fn detect_format_with_options(
    path: &Path,
    csv_options: Option<&crate::generic::csv::CsvOptions>,
) -> Result<(FileFormat, Arc<dyn FileObject>), IoError> {
    check_file(path)?;
    let ext = lower_ext(path);
    let formats = all_formats();
    // CSV is intentionally broad. Only strong legacy signatures take
    // precedence over it; other text goes directly to the generic reader.
    if ext == ".txt" {
        if let Some(name) = text_prefix(path)?
            .as_deref()
            .and_then(distinctive_text_format)
        {
            if let Some(f) = formats.iter().find(|f| f.name == name) {
                match read_candidate(f, path, csv_options) {
                    Ok(fo) => return Ok((f.clone(), fo)),
                    Err(IoError::WrongFormat(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }
    for f in formats {
        if !f.matches_extension(&ext) {
            continue;
        }
        match read_candidate(&f, path, csv_options) {
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
    read_file_inner(path, format, None)
}

/// Read with the normal detection order, applying these options only if the
/// selected reader is CSV. Explicitly named non-CSV formats are unchanged.
pub fn read_file_with_csv_options(
    path: &Path,
    format: Option<&str>,
    options: &crate::generic::csv::CsvOptions,
) -> Result<Arc<dyn FileObject>, IoError> {
    read_file_inner(path, format, Some(options))
}

fn read_file_inner(
    path: &Path,
    format: Option<&str>,
    csv_options: Option<&crate::generic::csv::CsvOptions>,
) -> Result<Arc<dyn FileObject>, IoError> {
    match format {
        None => detect_format_with_options(path, csv_options).map(|(_, fo)| fo),
        Some(name) => {
            check_file(path)?;
            let f = find_format(name)
                .ok_or_else(|| IoError::Other(format!("unknown file format `{name}`")))?;
            read_candidate(&f, path, csv_options)
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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(extension: &str, contents: &[u8]) -> Self {
            let path = std::env::temp_dir().join(format!(
                "datview-detection-{}-{}.{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
                extension
            ));
            std::fs::write(&path, contents).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

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
    #[test]
    fn csv_options_apply_to_auto_and_named_reads() {
        let fixture = Fixture::new(
            "csv",
            b"Timestamp,Power [W]\n03/04/2023 12:00:00,1\n04/05/2023 12:00:00,2\n",
        );
        let options = crate::generic::csv::CsvOptions {
            dayfirst: true,
            ..Default::default()
        };
        let first_timestamp = |file: Arc<dyn FileObject>| {
            let frames = file.to_dataframes().unwrap();
            let column = frames[0].df.column(0);
            assert!(column.is_datetime());
            column.as_f64()[0]
        };
        let expected = |month, day| {
            chrono::NaiveDate::from_ymd_opt(2023, month, day)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp() as f64
        };
        assert_eq!(
            first_timestamp(read_file(&fixture.0, None).unwrap()),
            expected(3, 4)
        );
        assert_eq!(
            first_timestamp(read_file_with_csv_options(&fixture.0, None, &options).unwrap()),
            expected(4, 3)
        );
        assert_eq!(
            first_timestamp(
                read_file_with_csv_options(&fixture.0, Some("CSV file"), &options).unwrap()
            ),
            expected(4, 3)
        );
    }

    #[test]
    fn genuine_fast_and_hawc_text_precede_generic_csv() {
        // Example files from pyDatView (MIT), kept in tests/fixtures.
        let fast = Fixture::new(
            "txt",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/FASTIn_arf_coords.txt"
            )),
        );
        let file = read_file(&fast.0, None).unwrap();
        assert_eq!(file.format_name(), "FAST input file");
        let frames = file.to_dataframes().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].df.nrows(), 399);

        let power = Fixture::new(
            "txt",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/HAWCStab2.pwr"
            )),
        );
        let file = read_file(&power.0, None).unwrap();
        assert_eq!(file.format_name(), "HAWCStab2 power file");
        let frames = file.to_dataframes().unwrap();
        assert_eq!(frames[0].df.ncols(), 15);
        assert!(frames[0].df.nrows() > 1);
    }

    #[test]
    fn genuine_turbsim_and_rosco_text_precede_generic_csv() {
        let turbsim = Fixture::new(
            "txt",
            b"TurbSim user time series\n3 nComp\n2 nPoints\n1 RefPtID\nPointyi Pointzi\n(m) (m)\n0 10\n2 20\n--------Time Series\nTime P1u P2u P1v P2v P1w P2w\n(s) (m/s) (m/s) (m/s) (m/s) (m/s) (m/s)\n0 1 2 3 4 5 6\n1 7 8 9 10 11 12\n",
        );
        let file = read_file(&turbsim.0, None).unwrap();
        assert_eq!(file.format_name(), "TurbSim time series");
        let frames = file.to_dataframes().unwrap();
        assert_eq!(frames[0].name, "Points");
        assert_eq!(frames[1].df.nrows(), 2);
        let options = crate::generic::csv::CsvOptions {
            separator: Some(b'|'),
            dayfirst: true,
            ..Default::default()
        };
        for format in [None, Some("TurbSim time series")] {
            assert_eq!(
                read_file_with_csv_options(&turbsim.0, format, &options)
                    .unwrap()
                    .format_name(),
                "TurbSim time series"
            );
        }
        assert_eq!(frames[1].df.ncols(), 7);

        let rosco = Fixture::new(
            "txt",
            b"# Rotor performance tables\n# Pitch angle vector\n0 5\n# TSR vector\n4 8\n# Power coefficient\n\n0.4 0.3\n0.48 0.32\n# Thrust coefficient\n\n0.8 0.7\n0.75 0.65\n",
        );
        let file = read_file(&rosco.0, None).unwrap();
        assert_eq!(file.format_name(), "ROSCO Performance file");
        let frames = file.to_dataframes().unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].name, "CP");
        assert_eq!(frames[0].df.nrows(), 2);
        assert_eq!(frames[0].df.ncols(), 3);
    }

    #[test]
    fn hawc_counted_ae_and_pc_text_are_distinguished() {
        let ae = Fixture::new(
            "txt",
            b"1 r[m] Chord[m] T/C[%] Set no.\n1 2\n0 3 100 1\n1 2 30 1\n",
        );
        let file = read_file(&ae.0, None).unwrap();
        assert_eq!(file.format_name(), "HAWC2 AE file");
        assert_eq!(file.to_dataframes().unwrap()[0].df.nrows(), 2);

        let pc = Fixture::new("txt", b"1\n1\n1 2 30.0\n-5 -0.2 0.03 0.0\n5 0.4 0.02 0.0\n");
        let file = read_file(&pc.0, None).unwrap();
        assert_eq!(file.format_name(), "HAWC2 PC file");
        assert_eq!(file.to_dataframes().unwrap()[0].df.nrows(), 2);
    }

    #[test]
    fn numbered_hawc_induction_header_is_selected() {
        let mut text = String::from("# r [m]");
        for i in 1..14 {
            text.push_str(&format!(" {i:02} q{i} [-]"));
        }
        text.push('\n');
        text.push_str(&(0..14).map(|i| i.to_string()).collect::<Vec<_>>().join(" "));
        text.push('\n');
        let fixture = Fixture::new("txt", text.as_bytes());
        let file = read_file(&fixture.0, None).unwrap();
        assert_eq!(file.format_name(), "HAWCStab2 induction file");
        let frames = file.to_dataframes().unwrap();
        assert_eq!(frames[0].df.ncols(), 14);
        assert_eq!(frames[0].df.column(0).name, "r_[m]");
        assert_eq!(frames[0].df.column(0).floats().unwrap(), [0.0]);
    }

    #[test]
    fn unrelated_telemetry_is_rejected_by_specialized_readers() {
        let samples: [&[u8]; 4] = [
            b"Time_Of_Day_Seconds Core CPU Avg_MHz Busy%\n1679142896.7 - - 159 5.42\n1679142897.7 - - 156 5.24\n",
            b"Date,GPU Incompatible Sensor [W],Pitch angle [deg]\n2023-03-18 12:34:56,1,3\n2023-03-18 12:34:57,2,4\n",
            b"Time nComp Power\n0 3 100\n1 3 101\n",
            b"0 10\n1 11\n2 12\n",
        ];
        let specialized = [
            "FAST input file",
            "TurbSim time series",
            "ROSCO Performance file",
            "HAWC2 PC file",
            "HAWC2 AE file",
            "HAWCStab2 power file",
            "HAWCStab2 induction file",
        ];
        for sample in samples {
            let fixture = Fixture::new("txt", sample);
            for name in specialized {
                assert!(
                    matches!(
                        read_file(&fixture.0, Some(name)),
                        Err(IoError::WrongFormat(_))
                    ),
                    "{name} claimed unrelated telemetry"
                );
            }
            assert_eq!(
                read_file(&fixture.0, None).unwrap().format_name(),
                "CSV file"
            );
        }
    }

    #[test]
    fn unnumbered_units_do_not_identify_hawcstab() {
        for count in [14, 15, 18, 38] {
            let header = (0..count)
                .map(|i| format!("sensor{i} [W]"))
                .collect::<Vec<_>>()
                .join(" ");
            let fixture = Fixture::new(
                "txt",
                format!("# {header}\nnot a numeric table\n").as_bytes(),
            );
            for name in ["HAWCStab2 power file", "HAWCStab2 induction file"] {
                assert!(matches!(
                    read_file(&fixture.0, Some(name)),
                    Err(IoError::WrongFormat(_))
                ));
            }
        }
    }

    #[test]
    fn identified_turbsim_corruption_is_not_treated_as_csv() {
        let fixture = Fixture::new(
            "txt",
            b"3 nComp\n2 nPoints\n1 RefPtID\nPointyi Pointzi\n(m) (m)\n0 10\nbad point\n",
        );
        assert!(matches!(
            read_file(&fixture.0, None),
            Err(IoError::BrokenFormat(_))
        ));
    }
}
