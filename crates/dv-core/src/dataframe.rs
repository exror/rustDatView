//! Minimal column-oriented DataFrame, standing in for pandas in pyDatView.
//!
//! Column names may be duplicated (as in pandas); all positional operations
//! therefore use indices, not names.

use std::borrow::Cow;

use chrono::{DateTime, NaiveDateTime};

use crate::error::{DvError, Result};

/// Column storage.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// Numeric data; integers are stored as floats, missing values are NaN.
    Float(Vec<f64>),
    /// Text data.
    Str(Vec<String>),
    /// Date-times as seconds since the Unix epoch (UTC, naive); NaT is NaN.
    DateTime(Vec<f64>),
}

impl ColumnData {
    pub fn len(&self) -> usize {
        match self {
            ColumnData::Float(v) | ColumnData::DateTime(v) => v.len(),
            ColumnData::Str(v) => v.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Same kind of column holding only the rows `idx`.
    pub fn take(&self, idx: &[usize]) -> ColumnData {
        match self {
            ColumnData::Float(v) => ColumnData::Float(idx.iter().map(|&i| v[i]).collect()),
            ColumnData::DateTime(v) => ColumnData::DateTime(idx.iter().map(|&i| v[i]).collect()),
            ColumnData::Str(v) => ColumnData::Str(idx.iter().map(|&i| v[i].clone()).collect()),
        }
    }
    /// Same kind of column holding only the rows where `mask` is true.
    pub fn filter(&self, mask: &[bool]) -> ColumnData {
        fn f<T: Clone>(v: &[T], mask: &[bool]) -> Vec<T> {
            v.iter()
                .zip(mask)
                .filter(|(_, &m)| m)
                .map(|(x, _)| x.clone())
                .collect()
        }
        match self {
            ColumnData::Float(v) => ColumnData::Float(f(v, mask)),
            ColumnData::DateTime(v) => ColumnData::DateTime(f(v, mask)),
            ColumnData::Str(v) => ColumnData::Str(f(v, mask)),
        }
    }
    /// Column of the same kind filled with missing values.
    pub fn missing_like(&self, n: usize) -> ColumnData {
        match self {
            ColumnData::Float(_) => ColumnData::Float(vec![f64::NAN; n]),
            ColumnData::DateTime(_) => ColumnData::DateTime(vec![f64::NAN; n]),
            ColumnData::Str(_) => ColumnData::Str(vec![String::new(); n]),
        }
    }
    /// Append `other` to `self`; mismatched kinds degrade to strings, except
    /// float/datetime which degrade to float.
    pub fn extend_from(&mut self, other: &ColumnData) {
        match (&mut *self, other) {
            (ColumnData::Float(a), ColumnData::Float(b))
            | (ColumnData::DateTime(a), ColumnData::DateTime(b))
            | (ColumnData::Float(a), ColumnData::DateTime(b)) => a.extend_from_slice(b),
            (ColumnData::DateTime(a), ColumnData::Float(b)) => {
                let mut v = std::mem::take(a);
                v.extend_from_slice(b);
                *self = ColumnData::Float(v);
            }
            (ColumnData::Str(a), b) => a.extend((0..b.len()).map(|i| b.display(i))),
            (a, ColumnData::Str(b)) => {
                let mut v: Vec<String> = (0..a.len()).map(|i| a.display(i)).collect();
                v.extend(b.iter().cloned());
                *self = ColumnData::Str(v);
            }
        }
    }
    /// Text representation of row `i` (used for display and CSV export).
    pub fn display(&self, i: usize) -> String {
        match self {
            ColumnData::Float(v) => format_float(v[i]),
            ColumnData::DateTime(v) => format_datetime(v[i]),
            ColumnData::Str(v) => v[i].clone(),
        }
    }
}

/// Shortest round-trip representation, `nan` for NaN.
pub fn format_float(x: f64) -> String {
    if x.is_nan() {
        "nan".to_string()
    } else if x.is_infinite() {
        if x > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        }
    } else {
        format!("{x}")
    }
}

/// ISO-8601 representation of a datetime in seconds since epoch, `NaT` for NaN.
pub fn format_datetime(t: f64) -> String {
    if !t.is_finite() {
        return "NaT".to_string();
    }
    let secs = t.floor();
    let nanos = ((t - secs) * 1e9).round() as u32;
    match DateTime::from_timestamp(secs as i64, nanos.min(999_999_999)) {
        Some(dt) => {
            let n = dt.naive_utc();
            if nanos == 0 {
                n.format("%Y-%m-%d %H:%M:%S").to_string()
            } else {
                n.format("%Y-%m-%d %H:%M:%S%.f").to_string()
            }
        }
        None => "NaT".to_string(),
    }
}

/// Convert a naive datetime to seconds since epoch.
pub fn datetime_to_seconds(dt: &NaiveDateTime) -> f64 {
    let u = dt.and_utc();
    u.timestamp() as f64 + u.timestamp_subsec_nanos() as f64 * 1e-9
}

/// A named column.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub data: ColumnData,
}

impl Column {
    pub fn new(name: impl Into<String>, data: ColumnData) -> Self {
        Self {
            name: name.into(),
            data,
        }
    }
    pub fn float(name: impl Into<String>, v: Vec<f64>) -> Self {
        Self::new(name, ColumnData::Float(v))
    }
    pub fn string(name: impl Into<String>, v: Vec<String>) -> Self {
        Self::new(name, ColumnData::Str(v))
    }
    pub fn datetime(name: impl Into<String>, v: Vec<f64>) -> Self {
        Self::new(name, ColumnData::DateTime(v))
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn is_float(&self) -> bool {
        matches!(self.data, ColumnData::Float(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self.data, ColumnData::Str(_))
    }
    pub fn is_datetime(&self) -> bool {
        matches!(self.data, ColumnData::DateTime(_))
    }
    /// Numeric view: floats as-is, datetimes as seconds since epoch,
    /// strings parsed as numbers (NaN when not parseable).
    pub fn as_f64(&self) -> Cow<'_, [f64]> {
        match &self.data {
            ColumnData::Float(v) | ColumnData::DateTime(v) => Cow::Borrowed(v),
            ColumnData::Str(v) => Cow::Owned(
                v.iter()
                    .map(|s| s.trim().parse::<f64>().unwrap_or(f64::NAN))
                    .collect(),
            ),
        }
    }
    /// Float slice when the column is numeric.
    pub fn floats(&self) -> Option<&[f64]> {
        match &self.data {
            ColumnData::Float(v) => Some(v),
            _ => None,
        }
    }
}

/// Ordered list of equal-length columns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataFrame {
    columns: Vec<Column>,
}

impl DataFrame {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from columns; all columns must have the same length.
    pub fn from_columns(columns: Vec<Column>) -> Result<Self> {
        if let Some(first) = columns.first() {
            let n = first.len();
            if let Some(bad) = columns.iter().find(|c| c.len() != n) {
                return Err(DvError::msg(format!(
                    "column `{}` has length {} but `{}` has length {}",
                    bad.name,
                    bad.len(),
                    first.name,
                    n
                )));
            }
        }
        Ok(Self { columns })
    }

    /// Build from float columns, `names.len()` must equal `cols.len()`.
    pub fn from_floats<S: Into<String>>(
        names: impl IntoIterator<Item = S>,
        cols: Vec<Vec<f64>>,
    ) -> Result<Self> {
        let names: Vec<String> = names.into_iter().map(Into::into).collect();
        if names.len() != cols.len() {
            return Err(DvError::msg(format!(
                "{} column names supplied for {} columns",
                names.len(),
                cols.len()
            )));
        }
        let columns: Vec<Column> = names
            .into_iter()
            .zip(cols)
            .map(|(n, c)| Column::float(n, c))
            .collect();
        Self::from_columns(columns)
    }

    /// Build from a row-major matrix of floats (`rows[i][j]` = row i, column j).
    pub fn from_rows<S: Into<String>>(
        names: impl IntoIterator<Item = S>,
        rows: &[Vec<f64>],
    ) -> Result<Self> {
        let names: Vec<String> = names.into_iter().map(Into::into).collect();
        let mut cols = vec![Vec::with_capacity(rows.len()); names.len()];
        for (i, r) in rows.iter().enumerate() {
            if r.len() != names.len() {
                return Err(DvError::msg(format!(
                    "row {i} has {} values, expected {}",
                    r.len(),
                    names.len()
                )));
            }
            for (c, v) in cols.iter_mut().zip(r) {
                c.push(*v);
            }
        }
        Self::from_floats(names, cols)
    }

    pub fn nrows(&self) -> usize {
        self.columns.first().map_or(0, |c| c.len())
    }
    pub fn ncols(&self) -> usize {
        self.columns.len()
    }
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty() || self.nrows() == 0
    }
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }
    pub fn columns_mut(&mut self) -> &mut [Column] {
        &mut self.columns
    }
    pub fn into_columns(self) -> Vec<Column> {
        self.columns
    }
    pub fn column(&self, i: usize) -> &Column {
        &self.columns[i]
    }
    pub fn column_mut(&mut self, i: usize) -> &mut Column {
        &mut self.columns[i]
    }
    pub fn names(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }
    /// Index of the first column named `name`.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }
    /// First column named `name`.
    pub fn get(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    fn check_len(&self, c: &Column) -> Result<()> {
        if !self.columns.is_empty() && c.len() != self.nrows() {
            return Err(DvError::msg(format!(
                "cannot add column `{}` of length {} to a table of {} rows",
                c.name,
                c.len(),
                self.nrows()
            )));
        }
        Ok(())
    }

    /// Insert a column at position `pos` (clamped to `ncols`).
    pub fn insert(&mut self, pos: usize, c: Column) -> Result<()> {
        self.check_len(&c)?;
        let pos = pos.min(self.columns.len());
        self.columns.insert(pos, c);
        Ok(())
    }
    pub fn push(&mut self, c: Column) -> Result<()> {
        self.check_len(&c)?;
        self.columns.push(c);
        Ok(())
    }
    /// Replace column `i`.
    pub fn set(&mut self, i: usize, c: Column) -> Result<()> {
        if c.len() != self.nrows() {
            return Err(DvError::msg(format!(
                "column `{}` has wrong length",
                c.name
            )));
        }
        self.columns[i] = c;
        Ok(())
    }
    pub fn remove(&mut self, i: usize) -> Column {
        self.columns.remove(i)
    }
    /// Keep only the columns at the given indices, in that order.
    pub fn select_columns(&self, idx: &[usize]) -> DataFrame {
        DataFrame {
            columns: idx.iter().map(|&i| self.columns[i].clone()).collect(),
        }
    }
    /// Remove the columns at the given indices.
    pub fn drop_columns(&mut self, idx: &[usize]) {
        let mut i = 0;
        self.columns.retain(|_| {
            let keep = !idx.contains(&i);
            i += 1;
            keep
        });
    }
    /// Rows at the given indices.
    pub fn take_rows(&self, idx: &[usize]) -> DataFrame {
        DataFrame {
            columns: self
                .columns
                .iter()
                .map(|c| Column::new(c.name.clone(), c.data.take(idx)))
                .collect(),
        }
    }
    /// Rows where `mask` is true.
    pub fn filter_rows(&self, mask: &[bool]) -> DataFrame {
        DataFrame {
            columns: self
                .columns
                .iter()
                .map(|c| Column::new(c.name.clone(), c.data.filter(mask)))
                .collect(),
        }
    }
    /// Rows `start..end`.
    pub fn slice_rows(&self, start: usize, end: usize) -> DataFrame {
        let idx: Vec<usize> = (start..end.min(self.nrows())).collect();
        self.take_rows(&idx)
    }
    /// Numeric view of column `i`.
    pub fn f64_col(&self, i: usize) -> Cow<'_, [f64]> {
        self.columns[i].as_f64()
    }
    /// Row-major numeric matrix of all columns.
    pub fn to_rows(&self) -> Vec<Vec<f64>> {
        let cols: Vec<Cow<[f64]>> = self.columns.iter().map(|c| c.as_f64()).collect();
        (0..self.nrows())
            .map(|i| cols.iter().map(|c| c[i]).collect())
            .collect()
    }

    /// Horizontal concatenation by row position (pandas `concat(axis=1)`):
    /// shorter frames are padded with missing values.
    pub fn concat_horizontal(dfs: &[&DataFrame]) -> DataFrame {
        let n = dfs.iter().map(|d| d.nrows()).max().unwrap_or(0);
        let mut columns = Vec::new();
        for d in dfs {
            for c in &d.columns {
                let mut data = c.data.clone();
                if data.len() < n {
                    data.extend_from(&c.data.missing_like(n - data.len()));
                }
                columns.push(Column::new(c.name.clone(), data));
            }
        }
        DataFrame { columns }
    }

    /// Vertical concatenation by column name (pandas `concat(axis=0)`).
    ///
    /// With `common_only`, only the columns present in every frame are kept
    /// (`join='inner'`); otherwise the union of columns is kept, in order of
    /// first appearance, and missing values are filled.
    pub fn concat_vertical(dfs: &[&DataFrame], common_only: bool) -> DataFrame {
        let mut names: Vec<String> = Vec::new();
        for d in dfs {
            for c in &d.columns {
                if !names.contains(&c.name) {
                    names.push(c.name.clone());
                }
            }
        }
        if common_only {
            names.retain(|n| dfs.iter().all(|d| d.find(n).is_some()));
        }
        let mut columns: Vec<Column> = Vec::with_capacity(names.len());
        for name in names {
            let mut data: Option<ColumnData> = None;
            for d in dfs {
                let n = d.nrows();
                let part = match d.get(&name) {
                    Some(c) => c.data.clone(),
                    None => match &data {
                        Some(acc) => acc.missing_like(n),
                        None => ColumnData::Float(vec![f64::NAN; n]),
                    },
                };
                match &mut data {
                    None => data = Some(part),
                    Some(acc) => acc.extend_from(&part),
                }
            }
            columns.push(Column::new(
                name,
                data.unwrap_or(ColumnData::Float(Vec::new())),
            ));
        }
        DataFrame { columns }
    }

    /// Transpose the numeric content; new columns are named `C0..Cn`.
    pub fn transpose(&self) -> DataFrame {
        let rows = self.to_rows();
        let names: Vec<String> = (0..rows.len()).map(|i| format!("C{i}")).collect();
        // Each original row becomes a column.
        DataFrame {
            columns: names
                .into_iter()
                .zip(rows)
                .map(|(n, c)| Column::float(n, c))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mismatched_names_do_not_silently_discard_data() {
        assert!(DataFrame::from_floats(["a"], vec![vec![1.], vec![2.]]).is_err());
        assert!(DataFrame::from_floats(["a", "b"], vec![vec![1.]]).is_err());
    }

    #[test]
    fn vertical_concat_inner_and_outer() {
        let a =
            DataFrame::from_floats(["ID0", "ColA"], vec![vec![0., 1.], vec![10., 11.]]).unwrap();
        let b = DataFrame::from_floats(["ID", "ColA"], vec![vec![1.], vec![12.]]).unwrap();
        let inner = DataFrame::concat_vertical(&[&a, &b], true);
        assert_eq!(inner.names(), vec!["ColA"]);
        assert_eq!(&*inner.f64_col(0), &[10., 11., 12.]);
        let outer = DataFrame::concat_vertical(&[&a, &b], false);
        assert_eq!(outer.names(), vec!["ID0", "ColA", "ID"]);
        let id0 = outer.f64_col(0);
        assert!(id0[2].is_nan());
        let id = outer.f64_col(2);
        assert!(id[0].is_nan() && id[1].is_nan() && id[2] == 1.0);
    }

    #[test]
    fn transpose_roundtrip_shape() {
        let a = DataFrame::from_floats(
            ["a", "b", "c"],
            vec![vec![1., 2.], vec![3., 4.], vec![5., 6.]],
        )
        .unwrap();
        let t = a.transpose();
        assert_eq!((t.nrows(), t.ncols()), (3, 2));
        assert_eq!(&*t.f64_col(1), &[2., 4., 6.]);
    }
}
