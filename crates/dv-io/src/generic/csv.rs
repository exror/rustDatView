//! CSV options use zero-based physical line numbers, like pyDatView.
use dv_core::{Column, ColumnData, DataFrame, Field2D, FileObject, IoError, NamedFrame};
use std::{collections::HashSet, path::Path, sync::Arc};

#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// None sniffs comma, semicolon, tab, then whitespace. Space means whitespace.
    pub separator: Option<u8>,
    pub column_names: Vec<String>,
    pub comment_char: Option<char>,
    pub comment_lines: Vec<usize>,
    /// Negative values select a line relative to the end of the leading comments.
    pub column_names_line: Option<isize>,
    pub detect_column_names: bool,
}
impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            separator: None,
            column_names: Vec::new(),
            comment_char: None,
            comment_lines: Vec::new(),
            column_names_line: None,
            detect_column_names: true,
        }
    }
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    read_with_options(path, &CsvOptions::default())
}

fn numeric(s: &str) -> Option<f64> {
    let s = s.trim();
    match s {
        "" | "NA" | "N/A" | "NaN" | "nan" | "NULL" | "null" | "None" | "<NA>" => Some(f64::NAN),
        _ => s.parse().ok(),
    }
}
fn float_count(row: &[String]) -> usize {
    row.iter()
        .filter(|s| !s.is_empty() && s.parse::<f64>().is_ok())
        .count()
}

/// Infer pandas-like numeric columns without converting quoted text indiscriminately.
pub(super) fn strings_column(name: String, values: Vec<String>) -> Column {
    if values.iter().all(|v| numeric(v).is_some()) {
        Column::float(name, values.iter().map(|v| numeric(v).unwrap()).collect())
    } else {
        Column::string(name, values)
    }
}

fn parse_records(text: &str, separator: u8) -> Result<Vec<Vec<String>>, IoError> {
    if separator != b' ' {
        let mut reader = ::csv::ReaderBuilder::new()
            .delimiter(separator)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        return reader
            .records()
            .map(|r| {
                r.map(|r| r.iter().map(|s| s.trim().to_owned()).collect())
                    .map_err(|e| IoError::WrongFormat(e.to_string()))
            })
            .collect();
    }
    // Regex-style whitespace delimiters, preserving quoted fields and newlines.
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut value = String::new();
    let mut quoted = false;
    let mut active = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            active = true;
            if quoted && chars.peek() == Some(&'"') {
                value.push('"');
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if !quoted && c.is_whitespace() {
            if active {
                row.push(std::mem::take(&mut value));
                active = false;
            }
            if c == '\n' && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
        } else {
            value.push(c);
            active = true;
        }
    }
    if quoted {
        return Err(IoError::WrongFormat("unterminated quoted CSV field".into()));
    }
    if active {
        row.push(value);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    Ok(rows)
}

fn strip_comment(line: &str, marker: Option<char>, quoted: &mut bool) -> String {
    let mut chars = line.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == '"' {
            out.push(c);
            if *quoted && chars.peek() == Some(&'"') {
                out.push(chars.next().unwrap());
            } else {
                *quoted = !*quoted;
            }
        } else if !*quoted && Some(c) == marker {
            break;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn read_with_options(
    path: &Path,
    options: &CsvOptions,
) -> Result<Arc<dyn FileObject>, IoError> {
    if options.comment_char.is_some() && !options.comment_lines.is_empty() {
        return Err(IoError::Other(
            "Provide either comment_char or comment_lines".into(),
        ));
    }
    if !options.column_names.is_empty() && options.column_names_line.is_some() {
        return Err(IoError::Other(
            "Provide either column_names or column_names_line".into(),
        ));
    }
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Err(IoError::Empty(path.into()));
    }
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().into_iter().map(char::from).collect(), // lossless Latin-1 fallback
    };
    let text = text.trim_start_matches('\u{feff}');
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.first().copied().unwrap_or("");
    let words: Vec<&str> = first.split_whitespace().collect();
    if words.len() > 1
        && words[0].parse::<i64>().is_ok()
        && words[1].chars().all(char::is_alphabetic)
    {
        return Err(IoError::WrongFormat(
            "integer followed by keyword is not likely CSV".into(),
        ));
    }
    let marker = options.comment_char.or_else(|| {
        first
            .trim()
            .chars()
            .next()
            .filter(|c| ['#', '!', ';'].contains(c))
    });
    let leading = if options.comment_lines.is_empty() {
        lines
            .iter()
            .take_while(|l| marker.is_some() && l.trim().starts_with(marker.unwrap()))
            .count()
    } else {
        0
    };
    let mut skipped: HashSet<usize> = options.comment_lines.iter().copied().collect();
    skipped.extend(0..leading);
    let header: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| skipped.contains(i))
        .map(|(_, l)| *l)
        .collect();
    let probe: Vec<&str> = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| !skipped.contains(i) && !l.trim().is_empty())
        .map(|(_, l)| *l)
        .take(2)
        .collect();
    let separator = options.separator.unwrap_or_else(|| {
        let p = probe
            .get(1)
            .or_else(|| probe.first())
            .copied()
            .unwrap_or("");
        [b',', b';', b'\t']
            .into_iter()
            .find(|&d| {
                let mut quote = false;
                p.bytes().any(|c| {
                    if c == b'"' {
                        quote = !quote;
                    }
                    c == d && !quote
                })
            })
            .unwrap_or(b' ')
    });
    let mut names = options.column_names.clone();
    if let Some(line) = options.column_names_line {
        let index = if line < 0 {
            leading as isize + line
        } else {
            line
        };
        if index < 0 || index as usize >= lines.len() {
            return Err(super::broken("column_names_line is outside the file"));
        }
        let mut l = lines[index as usize].trim();
        if let Some(c) = marker {
            l = l.strip_prefix(c).unwrap_or(l).trim();
        }
        names = parse_records(l, separator)?
            .into_iter()
            .next()
            .unwrap_or_default();
        if line >= 0 {
            skipped.extend(0..=index as usize);
        }
    }
    let mut cleaned = String::new();
    let mut quoted = false;
    for (i, line) in lines.iter().enumerate() {
        if skipped.contains(&i) {
            continue;
        }
        let line = strip_comment(line, marker, &mut quoted);
        cleaned.push_str(&line);
        cleaned.push('\n');
    }
    let mut rows = parse_records(&cleaned, separator)?;
    if rows.is_empty() {
        return Err(IoError::WrongFormat("CSV has no data records".into()));
    }
    let mut start = 0;
    if names.is_empty() && options.column_names_line.is_none() && options.detect_column_names {
        let n = float_count(&rows[0]);
        if n == 0 || (rows[0].len() > 2 && n * 2 <= rows[0].len()) {
            names = rows[0].clone();
            start = 1;
            if let Some(units) = rows.get(1) {
                let brackets = units
                    .iter()
                    .map(|s| s.matches('(').count() + s.matches('[').count())
                    .sum::<usize>();
                if brackets * 2 > names.len() {
                    if units.len() == names.len() {
                        names = names
                            .iter()
                            .zip(units)
                            .map(|(n, u)| format!("{}_{}", n.trim(), u.trim()))
                            .collect();
                    }
                    start = 2;
                }
            }
        } else {
            for h in &header {
                let h = h.trim().trim_start_matches(marker.unwrap_or('\0')).trim();
                if let Some(r) = parse_records(h, separator)?.first() {
                    if r.len() == rows[0].len() && float_count(r) < r.len() {
                        names = r.clone();
                        break;
                    }
                }
            }
        }
    }
    rows.drain(0..start);
    let width = rows.first().map_or(names.len(), Vec::len);
    if width == 0 {
        return Err(IoError::WrongFormat("CSV has no columns".into()));
    }
    if names.len() != width {
        names = (0..width).map(|i| format!("C{i}")).collect();
    }
    let mut columns: Vec<Vec<String>> =
        (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
    for (i, row) in rows.into_iter().enumerate() {
        if row.len() > width {
            return Err(IoError::WrongFormat(format!(
                "CSV row {} has {} fields; expected {}",
                i + 1,
                row.len(),
                width
            )));
        }
        let mut row = row.into_iter();
        for col in &mut columns {
            col.push(row.next().unwrap_or_default());
        }
    }
    let df = super::frame(
        names
            .into_iter()
            .zip(columns)
            .map(|(n, v)| strings_column(n.trim().into(), v))
            .collect(),
    )?;
    let fields = if df.is_empty() {
        None
    } else {
        let skip = usize::from(df.column(0).name.eq_ignore_ascii_case("index"));
        if df.columns()[skip..]
            .iter()
            .all(|c| matches!(c.data, ColumnData::Float(_) | ColumnData::DateTime(_)))
        {
            let mut m = Vec::with_capacity(df.nrows() * (df.ncols() - skip));
            for i in 0..df.nrows() {
                for c in &df.columns()[skip..] {
                    match &c.data {
                        ColumnData::Float(v) | ColumnData::DateTime(v) => m.push(v[i]),
                        _ => unreachable!(),
                    }
                }
            }
            Some(vec![Field2D {
                name: "data".into(),
                x: (0..df.nrows()).map(|i| i as f64).collect(),
                y: (0..df.ncols() - skip).map(|i| i as f64).collect(),
                sx: "rows".into(),
                sy: "columns".into(),
                m,
            }])
        } else {
            None
        }
    };
    Ok(super::object(
        path,
        "CSV file",
        vec![NamedFrame::unnamed(df)],
        fields,
        format!(
            "separator: {:?}\n{}",
            char::from(separator),
            header.join("\n")
        ),
    ))
}

pub fn write(df: &DataFrame, path: &Path) -> Result<(), IoError> {
    let mut writer = ::csv::WriterBuilder::new()
        .terminator(::csv::Terminator::Any(b'\n'))
        .from_path(path)
        .map_err(super::broken)?;
    writer
        .write_record(df.columns().iter().map(|c| c.name.as_str()))
        .map_err(super::broken)?;
    for i in 0..df.nrows() {
        writer
            .write_record(df.columns().iter().map(|c| match &c.data {
                ColumnData::Float(v) if v[i].is_nan() => String::new(),
                _ => c.data.display(i),
            }))
            .map_err(super::broken)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_whitespace() {
        assert_eq!(
            parse_records("a \"b c\"\n1 2\n", b' ').unwrap(),
            vec![vec!["a", "b c"], vec!["1", "2"]]
        );
    }
    #[test]
    fn quoted_delimiter() {
        assert_eq!(
            parse_records("\"a,b\",x\n\"line\nnext\",2\n", b',').unwrap()[1][0],
            "line\nnext"
        );
    }
    #[test]
    fn quoted_comments() {
        let mut q = false;
        assert_eq!(
            strip_comment("\"a#b\",2 #comment", Some('#'), &mut q),
            "\"a#b\",2 "
        );
    }
}
