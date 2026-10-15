//! Port of `pydatview/Tables.py`: [`Table`] and [`TableList`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_TABLE_ID: AtomicU64 = AtomicU64::new(1);

use chrono::{NaiveDate, NaiveDateTime};
use regex::Regex;

use crate::common::{clean_col, ellude_common, multi_interp, no_unit, Extrap};
use crate::dataframe::{datetime_to_seconds, Column, ColumnData, DataFrame};
use crate::error::{DvError, Result};
use crate::file::{FileObject, IoError};
use crate::formula::{eval_formula, eval_mask};

/// A formula column remembered by a table so it can be re-applied on reload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Formula {
    /// Column position in the table (including the `Index` column).
    pub pos: usize,
    pub formula: String,
    pub name: String,
}

/// Parse a date/time string. `dayfirst` resolves `01/02/2020` ambiguity.
pub fn parse_datetime_str(s: &str, dayfirst: bool) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(datetime_to_seconds(&d.naive_utc()));
    }
    let (dmy, mdy) = (
        ["%d/%m/%Y", "%d-%m-%Y", "%d.%m.%Y"],
        ["%m/%d/%Y", "%m-%d-%Y", "%m.%d.%Y"],
    );
    let (first, second) = if dayfirst { (dmy, mdy) } else { (mdy, dmy) };
    let mut date_fmts: Vec<&str> = vec!["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d", "%Y%m%d"];
    date_fmts.extend(first);
    date_fmts.extend(second);
    date_fmts.extend([
        "%d %b %Y",
        "%d %B %Y",
        "%b %d %Y",
        "%B %d %Y",
        "%b %d, %Y",
        "%B %d, %Y",
    ]);
    let time_fmts = ["%H:%M:%S%.f", "%H:%M:%S", "%H:%M"];
    for df in &date_fmts {
        for sep in [" ", "T", "_"] {
            for tf in &time_fmts {
                let f = format!("{df}{sep}{tf}");
                if let Ok(d) = NaiveDateTime::parse_from_str(s, &f) {
                    return Some(datetime_to_seconds(&d));
                }
            }
        }
        if let Ok(d) = NaiveDate::parse_from_str(s, df) {
            // %Y%m%d would accept plain integers such as "20200101"; only
            // accept 8-digit strings for it.
            if *df == "%Y%m%d" && s.len() != 8 {
                continue;
            }
            return Some(datetime_to_seconds(&d.and_hms_opt(0, 0, 0)?));
        }
    }
    None
}

/// A table of data, typically coming from a file.
#[derive(Debug, Clone)]
pub struct Table {
    pub data: DataFrame,
    /// Raw name; a leading `>` marks a user-given name that is never elluded.
    raw_name: String,
    pub active_name: String,
    /// Absolute filename, empty if the table does not come from a file.
    pub filename: String,
    /// Name of the file format used to read the file, empty if unknown.
    pub fileformat_name: String,
    /// File object the table was read from.
    pub fileobject: Option<Arc<dyn FileObject>>,
    pub extension: String,
    pub mask: Option<Vec<bool>>,
    pub mask_string: String,
    pub formulas: Vec<Formula>,
    columns_pre_transpose: Option<Vec<String>>,
    source_frame_name: String,
    identity: u64,
}

impl Table {
    /// Create a table. `name` empty means "no name inside the file".
    pub fn new(
        data: DataFrame,
        name: &str,
        filename: &str,
        fileformat_name: &str,
        dayfirst: bool,
        fileobject: Option<Arc<dyn FileObject>>,
    ) -> Self {
        let mut t = Table {
            data: DataFrame::new(),
            raw_name: String::new(),
            active_name: String::new(),
            filename: filename.to_string(),
            fileformat_name: fileformat_name.to_string(),
            fileobject,
            extension: String::new(),
            mask: None,
            mask_string: String::new(),
            formulas: Vec::new(),
            columns_pre_transpose: None,
            source_frame_name: name.to_string(),
            identity: NEXT_TABLE_ID.fetch_add(1, Ordering::Relaxed),
        };
        t.set_data(data, dayfirst);
        t.setup_name(name);
        t
    }

    /// Create a table from a dataframe only (no file).
    pub fn from_df(data: DataFrame, name: &str) -> Self {
        Self::new(data, name, "", "", false, None)
    }

    /// Stable process-local identity for pipeline snapshots. Renaming a table
    /// does not change it; cloning a snapshot preserves it.
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// Original named frame inside the source file, unaffected by user renames.
    pub fn source_frame_name(&self) -> &str {
        &self.source_frame_name
    }

    /// Store `data`, inserting an `Index` column and converting date columns.
    pub fn set_data(&mut self, mut data: DataFrame, dayfirst: bool) {
        let has_index = data
            .columns()
            .first()
            .is_some_and(|c| c.name.to_lowercase().contains("index"));
        if !has_index {
            let n = data.nrows();
            data.insert(
                0,
                Column::float("Index", (0..n).map(|i| i as f64).collect()),
            )
            .expect("index length");
        }
        // Delete empty, all-NaN columns at the end (e.g. trailing csv separators).
        while data.ncols() > 1 {
            let last = data.column(data.ncols() - 1);
            let all_nan = match &last.data {
                ColumnData::Float(v) | ColumnData::DateTime(v) => v.iter().all(|x| x.is_nan()),
                ColumnData::Str(v) => v.iter().all(|x| x.trim().is_empty()),
            };
            if last.name.is_empty() && all_nan {
                data.remove(data.ncols() - 1);
            } else {
                break;
            }
        }
        self.data = data;
        self.convert_time_columns(dayfirst);
    }

    /// Convert string columns that contain dates to datetime columns, and
    /// strip the other string columns.
    pub fn convert_time_columns(&mut self, dayfirst: bool) {
        if self.data.nrows() == 0 {
            return;
        }
        for c in self.data.columns_mut() {
            let ColumnData::Str(v) = &mut c.data else {
                continue;
            };
            let first = v.iter().find(|s| !s.trim().is_empty() && s.trim() != "NaT");
            let is_date = first.is_some_and(|s| parse_datetime_str(s, dayfirst).is_some());
            if is_date {
                let parsed: Option<Vec<f64>> = v
                    .iter()
                    .map(|s| {
                        let t = s.trim();
                        if t.is_empty() || t == "NaT" {
                            Some(f64::NAN)
                        } else {
                            parse_datetime_str(t, dayfirst)
                        }
                    })
                    .collect();
                if let Some(p) = parsed {
                    c.data = ColumnData::DateTime(p);
                    continue;
                }
            }
            for s in v.iter_mut() {
                let t = s.trim();
                if t.len() != s.len() {
                    *s = t.to_string();
                }
            }
        }
    }

    /// Build the "codename" `dir|basename|name`.
    fn setup_name(&mut self, name: &str) {
        let mut splits: Vec<String> = Vec::new();
        let mut ext = String::new();
        if !self.filename.is_empty() {
            let p = PathBuf::from(&self.filename);
            let mut base_dir = p
                .parent()
                .map(|d| d.to_string_lossy().to_string())
                .unwrap_or_default();
            if base_dir.is_empty() {
                let cwd = std::env::current_dir().unwrap_or_default();
                self.filename = cwd.join(&p).to_string_lossy().to_string();
                base_dir = cwd.to_string_lossy().to_string();
            }
            splits.push(base_dir.replace(['/', '\\'], "|"));
            let p = PathBuf::from(&self.filename);
            let basename = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            ext = p
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            if !basename.is_empty() {
                splits.push(basename);
            }
        }
        if !name.is_empty() {
            splits.push(name.to_string());
        }
        self.extension = ext;
        let mut n = splits.join("|");
        if n.is_empty() {
            n = "default".into();
        }
        self.raw_name = n.clone();
        self.active_name = n;
    }

    /// Transpose the table (numeric content). Transposing twice restores the
    /// original column names when the shape allows it.
    pub fn transpose(&mut self) {
        let mut df = self.data.clone();
        if let Some(i) = df.find("Index") {
            df.remove(i);
        }
        let mut t = df.transpose();
        match self.columns_pre_transpose.take() {
            None => self.columns_pre_transpose = Some(df.names()),
            Some(prev) => {
                if prev.len() == t.ncols() {
                    for (c, n) in t.columns_mut().iter_mut().zip(prev) {
                        c.name = n;
                    }
                }
            }
        }
        // `Index` was removed above, so set_data re-inserts a fresh one.
        let keep = self.columns_pre_transpose.take();
        self.set_data(std::mem::take(&mut t), false);
        self.columns_pre_transpose = keep;
    }

    // --- Mask
    pub fn clear_mask(&mut self) {
        self.mask_string.clear();
        self.mask = None;
    }

    /// Apply a mask expression (see [`crate::formula`]).
    ///
    /// With `add`, returns the masked data and a new name and leaves the
    /// table untouched; otherwise stores the mask on the table.
    /// An empty string or `"no mask"` clears the mask.
    pub fn apply_mask_string(
        &mut self,
        s_mask: &str,
        add: bool,
    ) -> Result<Option<(DataFrame, String)>> {
        self.clear_mask();
        let s = s_mask.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("no mask") {
            return Ok(None);
        }
        let mask = eval_mask(&self.data, s).map_err(|e| {
            DvError::msg(format!(
                "Error: The mask failed to evaluate for table: {}\n{e}",
                self.nickname()
            ))
        })?;
        if !mask.iter().any(|&m| m) {
            return Err(DvError::msg(format!(
                "Error: The mask returned no value for table: {}",
                self.nickname()
            )));
        }
        if add {
            Ok(Some((
                self.data.filter_rows(&mask),
                format!("{}_masked", self.raw_name),
            )))
        } else {
            self.mask = Some(mask);
            self.mask_string = s_mask.to_string();
            Ok(None)
        }
    }

    /// Column `i`, with the mask applied if any.
    pub fn get_column(&self, i: usize) -> Column {
        let c = self.data.column(i);
        match &self.mask {
            Some(m) => Column::new(c.name.clone(), c.data.filter(m)),
            None => c.clone(),
        }
    }

    // --- Column manipulations
    pub fn rename_column(&mut self, i: usize, new_name: &str) {
        self.data.column_mut(i).name = new_name.to_string();
    }

    /// Rename all columns using plain string replacements `(new, old)`.
    pub fn rename_columns_str(&mut self, repl: &[(String, String)]) {
        for c in self.data.columns_mut() {
            for (new, old) in repl {
                c.name = c.name.replace(old.as_str(), new);
            }
        }
    }

    /// Rename all columns using regex replacements `(new, old_pattern)`.
    pub fn rename_columns_regex(&mut self, repl: &[(String, String)]) -> Result<()> {
        let res: Vec<(String, Regex)> = repl
            .iter()
            .map(|(n, o)| {
                Regex::new(o)
                    .map(|r| (n.clone(), r))
                    .map_err(|e| DvError::msg(e.to_string()))
            })
            .collect::<Result<_>>()?;
        for c in self.data.columns_mut() {
            for (new, re) in &res {
                c.name = re.replace_all(&c.name, new.as_str()).to_string();
            }
        }
        Ok(())
    }

    /// Delete columns by index (not by name: names may be duplicated).
    pub fn delete_columns(&mut self, icols: &[usize]) {
        self.data.drop_columns(icols);
        let mut sorted = icols.to_vec();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        for i in sorted {
            if let Some(k) = self.formulas.iter().position(|f| f.pos == i) {
                self.formulas.remove(k);
            }
            for f in self.formulas.iter_mut() {
                if f.pos > i {
                    f.pos -= 1;
                }
            }
        }
    }

    /// User rename: the name is protected from ellusion.
    pub fn rename(&mut self, new_name: &str) {
        self.raw_name = format!(">{new_name}");
    }

    /// Insert a column after position `i` (`None` = at the end).
    pub fn add_column(
        &mut self,
        name: &str,
        data: Vec<f64>,
        i: Option<usize>,
        formula: &str,
    ) -> Result<()> {
        let ncols = self.data.ncols();
        let i = match i {
            None => ncols.saturating_sub(1),
            Some(i) if i >= ncols => ncols.saturating_sub(1),
            Some(i) => i,
        };
        self.data.insert(i + 1, Column::float(name, data))?;
        for f in self.formulas.iter_mut() {
            if f.pos > i {
                f.pos += 1;
            }
        }
        self.formulas.push(Formula {
            pos: i + 1,
            formula: formula.to_string(),
            name: name.to_string(),
        });
        Ok(())
    }

    /// Replace the column at position `i` (must not be the `Index` column).
    pub fn set_column(
        &mut self,
        name: &str,
        data: Vec<f64>,
        i: usize,
        formula: &str,
    ) -> Result<()> {
        if i < 1 {
            return Err(DvError::msg(format!("Cannot set column at position {i}")));
        }
        self.data.set(i, Column::float(name, data))?;
        for f in self.formulas.iter_mut() {
            if f.pos == i {
                f.name = name.to_string();
                f.formula = formula.to_string();
            }
        }
        Ok(())
    }

    pub fn add_column_by_formula(
        &mut self,
        name: &str,
        formula: &str,
        i: Option<usize>,
    ) -> Result<()> {
        let col = eval_formula(&self.data, formula)?;
        self.add_column(name, col, i, formula)
    }

    pub fn set_column_by_formula(&mut self, name: &str, formula: &str, i: usize) -> Result<()> {
        let col = eval_formula(&self.data, formula)?;
        self.set_column(name, col, i, formula)
    }

    /// Data without the `Index` column (used for export).
    pub fn data_without_index(&self) -> DataFrame {
        let mut df = self.data.clone();
        if let Some(i) = df.find("Index") {
            df.remove(i);
        }
        df
    }

    // --- Properties
    pub fn basename(&self) -> String {
        Path::new(&self.filename)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    }
    pub fn shapestring(&self) -> String {
        format!("{}x{}", self.ncols(), self.nrows())
    }
    pub fn columns(&self) -> Vec<String> {
        self.data.names()
    }
    pub fn columns_clean(&self) -> Vec<String> {
        self.data
            .columns()
            .iter()
            .map(|c| no_unit(&c.name))
            .collect()
    }
    /// Name without the protection marker `>`.
    pub fn name(&self) -> &str {
        self.raw_name.strip_prefix('>').unwrap_or(&self.raw_name)
    }
    pub fn raw_name(&self) -> &str {
        &self.raw_name
    }
    pub fn set_raw_name(&mut self, n: &str) {
        self.raw_name = n.to_string();
    }
    pub fn nickname(&self) -> &str {
        self.name().rsplit('|').next().unwrap_or("")
    }
    pub fn ncols(&self) -> usize {
        self.data.ncols()
    }
    pub fn nrows(&self) -> usize {
        self.data.nrows()
    }

    /// Dummy table with columns `Time`, `cos`, `sin` (or the given names).
    pub fn create_dummy(
        n: usize,
        label: &str,
        columns: Option<Vec<String>>,
        ncols: Option<usize>,
    ) -> Self {
        let names: Vec<String> = match (columns, ncols) {
            (Some(c), _) => c,
            (None, Some(k)) => (0..k).map(|i| format!("C{i}")).collect(),
            (None, None) => vec!["Time".into(), format!("cos{label}"), format!("sin{label}")],
        };
        let t: Vec<f64> = (0..n)
            .map(|i| {
                if n > 1 {
                    i as f64 / (n - 1) as f64
                } else {
                    0.0
                }
            })
            .collect();
        let cols: Vec<Vec<f64>> = names
            .iter()
            .enumerate()
            .map(|(k, _)| match k {
                0 => t.clone(),
                1 => t.iter().map(|x| x.cos()).collect(),
                2 => t.iter().map(|x| x.sin()).collect(),
                k => t.iter().map(|x| x * k as f64).collect(),
            })
            .collect();
        let df = DataFrame::from_floats(names, cols).expect("dummy");
        Table::from_df(df, &format!("dummy{label}"))
    }
}

/// How table names are displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Naming {
    /// Common parts of the table names are removed.
    #[default]
    Ellude,
    /// File basenames are used.
    FileNames,
}

/// Reader callback used by [`TableList::load_tables_from_files`]:
/// `(path, Some(format_name) | None for auto-detection) -> file object`.
pub type Reader<'a> = dyn Fn(&Path, Option<&str>) -> Result<Arc<dyn FileObject>, IoError> + 'a;

/// List of tables (port of `TableList`).
#[derive(Debug, Clone, Default)]
pub struct TableList {
    pub tabs: Vec<Table>,
    pub naming: Naming,
    pub dayfirst: bool,
    pub hasswap: bool,
}

impl TableList {
    pub fn new(tabs: Vec<Table>) -> Self {
        Self {
            tabs,
            ..Default::default()
        }
    }
    pub fn len(&self) -> usize {
        self.tabs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, Table> {
        self.tabs.iter()
    }
    pub fn append(&mut self, t: Table) {
        self.tabs.push(t);
    }
    pub fn clean(&mut self) {
        self.tabs.clear();
    }

    /// Add dataframes as tables (`add=false` replaces the current tables).
    pub fn from_dataframes(&mut self, dfs: Vec<DataFrame>, names: &[String], add: bool) {
        if !add {
            self.clean();
        }
        for (df, name) in dfs.into_iter().zip(names) {
            self.tabs
                .push(Table::new(df, name, "", "", self.dayfirst, None));
        }
    }

    /// Load files and append their tables.
    ///
    /// `files` pairs a path with an optional format name (`None` = auto).
    /// Returns the indices of the new tables and the warnings.
    pub fn load_tables_from_files(
        &mut self,
        files: &[(PathBuf, Option<String>)],
        add: bool,
        reload: bool,
        reader: &Reader,
    ) -> (Vec<usize>, Vec<String>) {
        if !add {
            self.clean();
        }
        if reload {
            self.hasswap = false;
        }
        let mut warns = Vec::new();
        let mut new_idx = Vec::new();
        for (f, ff) in files {
            let fs = f.to_string_lossy().to_string();
            if fs.is_empty() {
                continue;
            }
            if self
                .unique_filenames()
                .iter()
                .any(|u| Path::new(u) == f.as_path() || *u == fs)
            {
                warns.push(format!("Warn: Cannot add a file already opened {fs}"));
                continue;
            }
            let (tabs, warn) = self.load_file_tabs(f, ff.as_deref(), reload, reader);
            if !warn.is_empty() {
                warns.push(warn);
            }
            for t in tabs {
                new_idx.push(self.tabs.len());
                self.tabs.push(t);
            }
        }
        (new_idx, warns)
    }

    /// Read one file into tables; returns the tables and a warning (possibly empty).
    pub fn load_file_tabs(
        &self,
        filename: &Path,
        fileformat: Option<&str>,
        reload: bool,
        reader: &Reader,
    ) -> (Vec<Table>, String) {
        let fs = filename.to_string_lossy().to_string();
        if !filename.is_file() {
            return (vec![], format!("Error: File not found: `{fs}`\n"));
        }
        let fo = match reader(filename, fileformat) {
            Ok(fo) => fo,
            Err(e) if reload && fileformat.is_some() => {
                let w = format!(
                    "Failed to read file:\n\n   {fs}\n\nwith fileformat: {}\n\nIf you see this message, the reader tried again and succeeded with \"auto\"-fileformat.\n\n{e}\n",
                    fileformat.unwrap_or("")
                );
                let (t, w2) = self.load_file_tabs(filename, None, false, reader);
                return (t, w + &w2);
            }
            Err(e) => return (vec![], io_error_message(&fs, &e)),
        };
        let frames = match fo.to_dataframes() {
            Ok(f) => f,
            Err(e) => return (vec![], io_error_message(&fs, &e)),
        };
        let fmt = fo.format_name().to_string();
        let mut tabs = Vec::new();
        for nf in frames {
            if nf.df.ncols() == 0 && nf.df.nrows() == 0 {
                continue;
            }
            tabs.push(Table::new(
                nf.df,
                &nf.name,
                &fs,
                &fmt,
                self.dayfirst,
                Some(fo.clone()),
            ));
        }
        let warn = if tabs.is_empty() {
            format!("Warn: No dataframe found in file: {fs}\n")
        } else {
            String::new()
        };
        (tabs, warn)
    }

    /// Reload the file of table `i_tab`, replacing all tables of that file.
    pub fn reload_one_tab(&mut self, i_tab: usize, reader: &Reader) -> Result<String> {
        let filename = self.tabs[i_tab].filename.clone();
        if filename.is_empty() {
            return Err(DvError::msg(
                "Cannot reload Table as it was not set from a file",
            ));
        }
        let fmt = self.tabs[i_tab].fileformat_name.clone();
        let fmt = if fmt.is_empty() { None } else { Some(fmt) };
        let itabs: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.filename == filename)
            .map(|(i, _)| i)
            .collect();
        let old_names: Vec<String> = itabs
            .iter()
            .map(|&i| self.tabs[i].raw_name.clone())
            .collect();
        let (tabs, warn) = self.load_file_tabs(Path::new(&filename), fmt.as_deref(), true, reader);
        for (i, t) in tabs.into_iter().enumerate() {
            if i >= itabs.len() {
                self.tabs.push(t);
            } else {
                let it = itabs[i];
                self.tabs[it] = t;
                if !self.hasswap {
                    self.tabs[it].raw_name = old_names[i].clone();
                }
            }
        }
        Ok(warn)
    }

    /// True if the given tables have identical (cleaned) columns.
    pub fn have_same_columns(&self, idx: Option<&[usize]>) -> bool {
        let all: Vec<usize> = (0..self.tabs.len()).collect();
        let idx = idx.unwrap_or(&all);
        let Some(&i0) = idx.first() else { return true };
        let n0 = self.tabs[i0].ncols();
        if !idx.iter().all(|&i| self.tabs[i].ncols() == n0) {
            return false;
        }
        let c0 = self.tabs[i0].columns_clean();
        idx.iter().all(|&i| self.tabs[i].columns_clean() == c0)
    }

    /// Rename table `i`; fails if the name is already used. Returns the old name.
    pub fn rename_table(&mut self, i: usize, new_name: &str) -> Result<String> {
        let old = self.tabs[i].name().to_string();
        if self.tabs.iter().any(|t| t.name() == new_name) {
            return Err(DvError::msg(
                "Error: This table already exist, choose a different name.",
            ));
        }
        self.tabs[i].rename(new_name);
        Ok(old)
    }

    pub fn swap(&mut self, i1: usize, i2: usize) {
        self.hasswap = true;
        self.tabs.swap(i1, i2);
    }

    /// Sort tables by display name.
    pub fn sort_by_name(&mut self) {
        let names = self.get_display_tab_names();
        let mut idx: Vec<usize> = (0..names.len()).collect();
        idx.sort_by(|&a, &b| names[a].cmp(&names[b]));
        let mut old: Vec<Option<Table>> = std::mem::take(&mut self.tabs)
            .into_iter()
            .map(Some)
            .collect();
        self.tabs = idx.into_iter().map(|i| old[i].take().unwrap()).collect();
    }

    /// Merge tables (port of `mergeTabs`).
    ///
    /// With `icommon` (one column index per table), all tables are linearly
    /// interpolated onto the sorted union of their common-column values;
    /// otherwise (or if interpolation fails) tables are concatenated by row
    /// position. The merged table is appended; returns its name and data.
    pub fn merge_tabs(
        &mut self,
        idx: Option<&[usize]>,
        icommon: Option<&[usize]>,
        extrap: Extrap,
    ) -> (String, DataFrame) {
        let all: Vec<usize> = (0..self.tabs.len()).collect();
        let idx = idx.unwrap_or(&all).to_vec();
        let dfs: Vec<&DataFrame> = idx.iter().map(|&i| &self.tabs[i].data).collect();
        let interp = icommon.and_then(|ic| merge_interp(&dfs, ic, extrap));
        let df = interp.unwrap_or_else(|| DataFrame::concat_horizontal(&dfs));
        let name = format!("{}_merged", self.tabs[idx[0]].name());
        self.tabs.push(Table::from_df(df.clone(), &name));
        (name, df)
    }

    /// Vertical stacking (port of `vstack`); appends the new table.
    pub fn vstack(&mut self, idx: Option<&[usize]>, common_only: bool) -> (String, DataFrame) {
        let all: Vec<usize> = (0..self.tabs.len()).collect();
        let idx = idx.unwrap_or(&all).to_vec();
        let dfs: Vec<&DataFrame> = idx.iter().map(|&i| &self.tabs[i].data).collect();
        let mut df = DataFrame::concat_vertical(&dfs, common_only);
        if let Some(i) = df.find("Index") {
            df.remove(i);
        }
        let n = df.nrows();
        df.insert(
            0,
            Column::float("Index", (0..n).map(|i| i as f64).collect()),
        )
        .expect("index");
        let name = format!("{}_concat", self.tabs[idx[0]].name());
        self.tabs.push(Table::from_df(df.clone(), &name));
        (name, df)
    }

    pub fn delete_tabs(&mut self, idx: &[usize]) {
        let mut i = 0;
        self.tabs.retain(|_| {
            let keep = !idx.contains(&i);
            i += 1;
            keep
        });
    }

    pub fn set_active_names(&mut self, names: &[String]) {
        for (t, n) in self.tabs.iter_mut().zip(names) {
            t.active_name = n.clone();
        }
    }

    /// Names displayed in the GUI, according to [`Naming`].
    pub fn get_display_tab_names(&self) -> Vec<String> {
        match self.naming {
            Naming::Ellude => {
                let names: Vec<String> = self.tabs.iter().map(|t| t.raw_name.clone()).collect();
                let last: Vec<String> = names
                    .iter()
                    .map(|n| n.rsplit('|').next().unwrap_or("").to_string())
                    .collect();
                let unique: std::collections::HashSet<&String> = last.iter().collect();
                if unique.len() == names.len() {
                    ellude_common(&last, 2)
                } else {
                    ellude_common(&names, 2)
                }
            }
            Naming::FileNames => self.tabs.iter().map(|t| t.basename()).collect(),
        }
    }

    pub fn tab_names(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.name().to_string()).collect()
    }
    pub fn filenames(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.filename.clone()).collect()
    }
    /// Unique non-empty filenames, in order of appearance.
    pub fn unique_filenames(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in &self.tabs {
            if !t.filename.is_empty() && !out.contains(&t.filename) {
                out.push(t.filename.clone());
            }
        }
        out
    }
    /// Unique file objects with their filenames.
    pub fn unique_fileobjects(&self) -> Vec<(String, Arc<dyn FileObject>)> {
        let mut out: Vec<(String, Arc<dyn FileObject>)> = Vec::new();
        for t in &self.tabs {
            if let Some(fo) = &t.fileobject {
                if !t.filename.is_empty() && !out.iter().any(|(f, _)| *f == t.filename) {
                    out.push((t.filename.clone(), fo.clone()));
                }
            }
        }
        out
    }
    /// Unique filenames with their format names (empty = unknown).
    pub fn filenames_and_formats(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for t in &self.tabs {
            if !out.iter().any(|(f, _)| *f == t.filename) {
                out.push((t.filename.clone(), t.fileformat_name.clone()));
            }
        }
        out
    }

    // --- Formulas
    /// Formulas of every table, keyed by raw name, sorted by position.
    pub fn store_formulas(&self) -> Vec<(String, Vec<Formula>)> {
        self.tabs
            .iter()
            .map(|t| {
                let mut f = t.formulas.clone();
                f.sort_by_key(|f| f.pos);
                (t.raw_name.clone(), f)
            })
            .collect()
    }

    /// Re-apply formulas stored with [`Self::store_formulas`]. Returns errors.
    pub fn apply_formulas(&mut self, formulas: &[(String, Vec<Formula>)]) -> Vec<String> {
        let mut errors = Vec::new();
        for t in self.tabs.iter_mut() {
            if let Some((_, fs)) = formulas.iter().find(|(n, _)| *n == t.raw_name) {
                for f in fs {
                    if let Err(e) =
                        t.add_column_by_formula(&f.name, &f.formula, Some(f.pos.saturating_sub(1)))
                    {
                        errors.push(format!(
                            "Formula `{}` failed for table {}: {e}",
                            f.name,
                            t.nickname()
                        ));
                    }
                }
            }
        }
        errors
    }

    pub fn create_dummy(ntabs: usize, n: usize, add_label: bool) -> Self {
        let tabs = (0..ntabs)
            .map(|i| {
                Table::create_dummy(
                    n,
                    &if add_label {
                        format!("_{i}")
                    } else {
                        String::new()
                    },
                    None,
                    None,
                )
            })
            .collect();
        Self::new(tabs)
    }
}

/// Interpolate all frames on the union of their common column values.
fn merge_interp(dfs: &[&DataFrame], icommon: &[usize], extrap: Extrap) -> Option<DataFrame> {
    if icommon.len() != dfs.len() {
        return None;
    }
    let mut x_new: Vec<f64> = Vec::new();
    for (df, &ic) in dfs.iter().zip(icommon) {
        if ic >= df.ncols() {
            return None;
        }
        x_new.extend(df.f64_col(ic).iter().copied().filter(|v| !v.is_nan()));
    }
    x_new.sort_by(|a, b| a.total_cmp(b));
    x_new.dedup();
    let mut parts: Vec<DataFrame> = Vec::new();
    for (k, (df, &ic)) in dfs.iter().zip(icommon).enumerate() {
        let xp = df.f64_col(ic);
        let mut cols = Vec::new();
        for (j, c) in df.columns().iter().enumerate() {
            if c.name == "Index" || (k > 0 && j == ic) {
                continue;
            }
            if c.is_string() {
                return None;
            }
            let v = if j == ic {
                x_new.clone()
            } else {
                multi_interp(&x_new, &xp, &c.as_f64(), extrap)
            };
            cols.push(Column::float(c.name.clone(), v));
        }
        parts.push(DataFrame::from_columns(cols).ok()?);
    }
    let refs: Vec<&DataFrame> = parts.iter().collect();
    let mut df = DataFrame::concat_horizontal(&refs);
    let n = df.nrows();
    df.insert(
        0,
        Column::float("Index", (0..n).map(|i| i as f64).collect()),
    )
    .ok()?;
    Some(df)
}

fn io_error_message(fs: &str, e: &IoError) -> String {
    match e {
        IoError::NotFound(p) => format!(
            "Error: A file was not found!\n\n While opening:\n\n {fs}\n\n the following file was not found:\n\n {}\n",
            p.display()
        ),
        IoError::Io(_) => format!("Error: IO Error thrown while opening file: {fs}\n{e}\n"),
        IoError::Empty(_) => format!("Error: File empty!\n\nFile is empty: {fs}\n\nOpen a different file.\n"),
        IoError::FormatNotDetected(_) => {
            format!("Error: File format not detected!\n\nFile: {fs}\n\nUse an explicit file-format from the list\n")
        }
        IoError::WrongFormat(m) => format!(
            "Error: Wrong file format!\n\nFile: {fs}\n\nThe file parser for the selected format failed to open the file.\n\nThe reported error was:\n{m}\n\nDouble-check your file format and report this error if you think it's a bug.\n"
        ),
        IoError::BrokenFormat(m) => format!(
            "Error: Inconsistency in the file format!\n\nFile: {fs}\n\nThe reported error was:\n\n{m}\n\nDouble-check your file format and report this error if you think it's a bug."
        ),
        IoError::Other(m) => format!("Error while reading file: {fs}\n\n{m}\n"),
    }
}

/// Clean column names helper re-exported for column matching.
pub fn clean_columns(cols: &[String]) -> Vec<String> {
    cols.iter().map(|c| clean_col(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(names: [&str; 2], a: Vec<f64>, b: Vec<f64>) -> Table {
        Table::from_df(DataFrame::from_floats(names, vec![a, b]).unwrap(), "")
    }

    #[test]
    fn default_name_and_index() {
        let t = tab(["ColA", "ColB"], vec![1.0, 2.0], vec![3.0, 4.0]);
        assert_eq!(t.raw_name(), "default");
        assert_eq!(t.columns(), vec!["Index", "ColA", "ColB"]);
    }

    #[test]
    fn merge_interpolates_on_common_column() {
        let t1 = tab(
            ["ID", "ColA"],
            vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5],
            vec![10.0, 10.5, 11.0, 11.5, 12.0, 12.5],
        );
        let t2 = tab(["ID", "ColB"], vec![1.0, 2.0, 3.0], vec![11.0, 12.0, 13.0]);
        let mut tl = TableList::new(vec![t1, t2]);
        let (_, df) = tl.merge_tabs(None, Some(&[1, 1]), Extrap::Nan);
        assert_eq!(df.names(), vec!["Index", "ID", "ColA", "ColB"]);
        assert_eq!(&*df.f64_col(1), &[0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0]);
        let a = df.f64_col(2);
        assert_eq!(&a[..6], &[10.0, 10.5, 11.0, 11.5, 12.0, 12.5]);
        assert!(a[6].is_nan());
        let b = df.f64_col(3);
        assert!(b[0].is_nan() && b[1].is_nan());
        assert_eq!(&b[2..], &[11.0, 11.5, 12.0, 12.5, 13.0]);
        assert_eq!(&*df.f64_col(0), &[0., 1., 2., 3., 4., 5., 6.]);
    }

    #[test]
    fn vstack_common_only() {
        let t1 = tab(["ID0", "ColA"], vec![0.0, 0.5], vec![10.0, 10.5]);
        let t2 = tab(["ID", "ColA"], vec![1.0], vec![11.0]);
        let mut tl = TableList::new(vec![t1, t2]);
        let (_, df) = tl.vstack(None, true);
        assert_eq!(df.names(), vec!["Index", "ColA"]);
        assert_eq!(&*df.f64_col(0), &[0., 1., 2.]);
        assert_eq!(&*df.f64_col(1), &[10.0, 10.5, 11.0]);
    }

    #[test]
    fn formula_positions_follow_deletions() {
        let mut t = tab(["A", "B"], vec![1.0, 2.0], vec![3.0, 4.0]);
        t.add_column_by_formula("C", "{A}+{B}", None).unwrap();
        assert_eq!(t.formulas[0].pos, 3);
        assert_eq!(&*t.data.f64_col(3), &[4.0, 6.0]);
        t.delete_columns(&[1]);
        assert_eq!(t.formulas[0].pos, 2);
        t.delete_columns(&[2]);
        assert!(t.formulas.is_empty());
    }

    #[test]
    fn mask_and_dates() {
        let df = DataFrame::from_columns(vec![
            Column::string(
                "Date",
                vec!["2020-01-01".into(), "2020-01-02 12:00:00".into()],
            ),
            Column::string("Label", vec![" a ".into(), "b".into()]),
        ])
        .unwrap();
        let mut t = Table::from_df(df, "x");
        assert!(t.data.column(1).is_datetime());
        assert_eq!(
            t.data.column(2).data,
            ColumnData::Str(vec!["a".into(), "b".into()])
        );
        t.apply_mask_string("{Date} > '2020-01-01'", false).unwrap();
        assert_eq!(t.mask, Some(vec![false, true]));
        assert!(t.apply_mask_string("{Index} > 5", false).is_err());
    }

    #[test]
    fn transpose_twice_restores_names() {
        let mut t = tab(["A", "B"], vec![1.0, 2.0], vec![3.0, 4.0]);
        t.transpose();
        assert_eq!(t.columns(), vec!["Index", "C0", "C1"]);
        t.transpose();
        assert_eq!(t.columns(), vec!["Index", "A", "B"]);
    }
}
