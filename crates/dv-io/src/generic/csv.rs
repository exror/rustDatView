//! Delimited text import. Option line numbers are zero-based physical lines.
mod tokenize;
mod value;

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use dv_core::{
    table::infer_datetime_format, Column, ColumnData, DataFrame, FileObject, IoError, NamedFrame,
};
use tokenize::{owned_field, records, unquote, without_comment, Fields, Record};
use value::{is_missing, parse, Value};

#[derive(Debug, Clone)]
pub struct CsvOptions {
    /// None detects comma, semicolon, tab, pipe, or whitespace. Space means whitespace.
    pub separator: Option<u8>,
    pub column_names: Vec<String>,
    pub comment_char: Option<char>,
    pub comment_lines: Vec<usize>,
    /// Negative values select a line relative to the end of the leading comments.
    pub column_names_line: Option<isize>,
    pub detect_column_names: bool,
    /// Used only when a date column remains ambiguous after examining all rows.
    pub dayfirst: bool,
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
            dayfirst: false,
        }
    }
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    read_with_options(path, &CsvOptions::default())
}

/// Decode the supported text encodings once, before interpreting delimiters.
pub(crate) fn decode_text(bytes: &[u8]) -> Result<(Cow<'_, str>, &'static str), IoError> {
    if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return std::str::from_utf8(body)
            .map(|s| (Cow::Borrowed(s), "UTF-8 BOM"))
            .map_err(super::broken);
    }
    let utf16 = if bytes.starts_with(&[0xff, 0xfe]) {
        Some((true, "UTF-16LE BOM"))
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        Some((false, "UTF-16BE BOM"))
    } else {
        None
    };
    if let Some((little, label)) = utf16 {
        let body = &bytes[2..];
        if !body.len().is_multiple_of(2) {
            return Err(IoError::BrokenFormat("odd-length UTF-16 text".into()));
        }
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        return String::from_utf16(&units)
            .map(|s| (Cow::Owned(s), label))
            .map_err(super::broken);
    }
    if bytes.contains(&0) {
        return Err(IoError::WrongFormat("NUL in non-Unicode text".into()));
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok((Cow::Borrowed(s), "UTF-8"));
    }
    // Undefined Windows-1252 byte values retain their corresponding C1 controls.
    const C1: [char; 32] = [
        '\u{20ac}', '\u{0081}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008d}',
        '\u{017d}', '\u{008f}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}',
        '\u{0153}', '\u{009d}', '\u{017e}', '\u{0178}',
    ];
    let mut decoded = String::with_capacity(bytes.len().saturating_add(bytes.len() / 100));
    let mut start = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if byte >= 0x80 {
            decoded.push_str(std::str::from_utf8(&bytes[start..i]).map_err(super::broken)?);
            decoded.push(if byte <= 0x9f {
                C1[(byte - 0x80) as usize]
            } else {
                char::from(byte)
            });
            start = i + 1;
        }
    }
    decoded.push_str(std::str::from_utf8(&bytes[start..]).map_err(super::broken)?);
    Ok((Cow::Owned(decoded), "Windows-1252"))
}

fn temporal_shape(s: &str) -> bool {
    let s = unquote(s);
    let bytes = s.as_bytes();
    let digits = bytes.iter().filter(|b| b.is_ascii_digit()).count();
    digits >= 4
        && (s.matches(['/', '-', '.']).count() >= 2
            || s.matches(':').count() >= 2
            || (s.contains('T') && s.ends_with('Z')))
}

fn row_quality(fields: &[&str], decimal_comma: bool) -> (usize, usize) {
    let mut recognized = 0;
    let mut nonempty = 0;
    for field in fields {
        let s = unquote(field);
        if is_missing(s) {
            continue;
        }
        nonempty += 1;
        if !matches!(parse(s, decimal_comma), Value::Text) || temporal_shape(s) {
            recognized += 1;
        }
    }
    (recognized, nonempty)
}

fn looks_like_data(fields: &[&str], decimal_comma: bool) -> bool {
    let (good, nonempty) = row_quality(fields, decimal_comma);
    good > 0 && good * 2 >= nonempty && (good >= 2 || fields.len() == 1)
}

fn longest_data_run(sample: &[Vec<&str>], decimal_comma: bool) -> (Option<usize>, usize) {
    let (mut best, mut best_len, mut run_start, mut run_len, mut run_width) = (None, 0, 0, 0, 0);
    for (i, row) in sample.iter().enumerate() {
        if !looks_like_data(row, decimal_comma) {
            run_len = 0;
            continue;
        }
        if run_len == 0 || row.len() != run_width {
            run_start = i;
            run_width = row.len();
            run_len = 0;
        }
        run_len += 1;
        if run_len > best_len {
            best = Some(run_start);
            best_len = run_len;
        }
    }
    (best, best_len)
}

fn sniff_separator(records: &[Record<'_>]) -> u8 {
    fn score(records: &[Record<'_>], limit: usize) -> (usize, bool, usize, usize, u8) {
        let mut best = (0usize, false, 0usize, 0usize, b' ');
        for separator in *b",;\t| " {
            let mut counts: HashMap<usize, (usize, usize)> = HashMap::new();
            for record in records.iter().take(limit) {
                let fields: Vec<_> = Fields::new(record.text, separator).collect();
                if fields.len() < 2 {
                    continue;
                }
                let (good, total) = row_quality(&fields, separator == b';');
                if good >= 2 && good * 2 >= total {
                    let count = counts.entry(fields.len()).or_default();
                    count.0 += 1;
                    count.1 += good;
                }
            }
            for (width, (count, good)) in counts {
                // An explicit delimiter beats whitespace when both split rows
                // consistently: padded values such as "135.0," otherwise look
                // numeric to the whitespace split. More recognized fields
                // break remaining ties, e.g. comma decimals in a semicolon table.
                let candidate = (count, separator != b' ', good, width, separator);
                if candidate > best {
                    best = candidate;
                }
            }
        }
        best
    }
    let first = score(records, 160);
    if first.0 < 2 && records.len() > 160 {
        score(records, 4096).4
    } else {
        first.4
    }
}

fn unit_token(s: &str) -> Option<&str> {
    let s = unquote(s)
        .trim()
        .trim_matches(|c| matches!(c, '[' | ']' | '(' | ')'));
    if s.is_empty() || s == "-" {
        return Some("");
    }
    match s.to_ascii_lowercase().as_str() {
        "s" | "sec" | "second" | "seconds" => Some("s"),
        "v" | "volt" | "volts" => Some("V"),
        "sequence" | "index" => Some(""),
        "a" | "w" | "kw" | "mw" | "ma" | "µa" | "μa" | "hz" | "khz" | "mhz" | "ghz" | "ms"
        | "us" | "µs" | "μs" | "°c" | "celsius" | "°f" | "k" | "%" | "rpm" | "fps" | "j" | "wh"
        | "var" | "mb" | "mib" => Some(s),
        _ => None,
    }
}

fn is_units_row(fields: &[&str], offset: usize, width: usize) -> bool {
    if fields.len() < offset + width || width == 0 {
        return false;
    }
    let selected = &fields[offset..offset + width];
    let recognized = selected.iter().filter(|s| unit_token(s).is_some()).count();
    let nonempty = selected.iter().filter(|s| !unquote(s).is_empty()).count();
    nonempty > 0
        && recognized == width
        && selected
            .iter()
            .any(|s| unit_token(s).is_some_and(|u| !u.is_empty()))
}

fn append_unit(name: &str, unit: &str) -> String {
    if unit.is_empty() || has_unit(name) {
        name.to_owned()
    } else {
        format!("{name} [{unit}]")
    }
}

fn has_unit(name: &str) -> bool {
    let name = name.trim_end();
    (name.ends_with(']') && name.contains('[')) || (name.ends_with(')') && name.contains('('))
}

// Some established logging schemas define units in sensor IDs or field names
// rather than in the displayed column header. Only apply these mappings when
// enough identifying fields are present to distinguish the schema.
fn infer_schema_units(names: &mut [String], header: &[String], sensor_ids: Option<&[&str]>) {
    let has = |name: &str| header.iter().any(|field| field.eq_ignore_ascii_case(name));
    let lhm = sensor_ids.is_some_and(|ids| {
        ids.len() >= names.len()
            && ids.iter().filter(|id| unquote(id).starts_with('/')).count() >= 2
    });
    let ptat = has("Timestamp") && has("Device") && has("CFreq") && has("UFreq") && has("TStat");
    let turbo = has("Time_Of_Day_Seconds") && has("Bzy_MHz") && has("PkgWatt");
    let presentmon = has("Application") && has("TimeInSeconds") && has("msBetweenPresents");
    let joulescope = ["#time", "current", "voltage", "power", "charge", "energy"]
        .iter()
        .all(|name| has(name));
    let nvidia = has("timestamp") && has("temperature.gpu") && has("power.draw [W]");
    for (index, (name, raw)) in names.iter_mut().zip(header).enumerate() {
        let unit = if lhm {
            sensor_ids
                .and_then(|ids| ids.get(index))
                .and_then(|id| unquote(id).rsplit('/').nth(1))
                .and_then(|kind| match kind.to_ascii_lowercase().as_str() {
                    "power" => Some("W"),
                    "temperature" => Some("°C"),
                    "load" => Some("%"),
                    "clock" => Some("MHz"),
                    "voltage" => Some("V"),
                    "current" => Some("A"),
                    "fan" => Some("RPM"),
                    _ => None,
                })
        } else if ptat {
            match raw.as_str() {
                "CFreq" | "UFreq" => Some("MHz"),
                "Util" | "C0" | "C1" | "C6" => Some("%"),
                "Temp" => Some("°C"),
                "Power" => Some("W"),
                "Volt" => Some("V"),
                _ => None,
            }
        } else if turbo {
            match raw.as_str() {
                "Avg_MHz" | "Bzy_MHz" | "TSC_MHz" => Some("MHz"),
                "Busy%" | "CPU%c1" | "CPU%c6" => Some("%"),
                "CoreTmp" | "PkgTmp" => Some("°C"),
                "PkgWatt" => Some("W"),
                _ => None,
            }
        } else if presentmon {
            if raw == "TimeInSeconds" {
                Some("s")
            } else if raw.starts_with("ms") {
                Some("ms")
            } else if matches!(raw.as_str(), "AllowsTearing" | "Dropped") {
                Some("Yes/No")
            } else {
                None
            }
        } else if joulescope {
            match raw.as_str() {
                "#time" => Some("s"),
                "current" => Some("A"),
                "voltage" => Some("V"),
                "power" => Some("W"),
                "charge" => Some("A·s"),
                "energy" => Some("J"),
                _ => None,
            }
        } else if nvidia && raw.eq_ignore_ascii_case("temperature.gpu") {
            Some("°C")
        } else {
            None
        };
        if let Some(unit) = unit {
            *name = append_unit(name, unit);
        }
    }
}

fn sequence_timebase(
    sample: &[Vec<&str>],
    header_index: Option<usize>,
    units_index: Option<usize>,
    offset: usize,
    data_width: usize,
) -> Option<(usize, f64, f64)> {
    let header = sample.get(header_index?)?;
    let units = sample.get(units_index?)?;
    let x = (offset..data_width).find(|&i| {
        header
            .get(i)
            .is_some_and(|s| unquote(s).eq_ignore_ascii_case("X"))
            && units
                .get(i)
                .is_some_and(|s| unquote(s).eq_ignore_ascii_case("Sequence"))
    })?;
    let value = |key: &str| {
        let i = header
            .iter()
            .position(|s| unquote(s).eq_ignore_ascii_case(key))?;
        match parse(unquote(units.get(i)?), false) {
            Value::Number(value, _) if value.is_finite() => Some(value),
            _ => None,
        }
    };
    let start = value("Start")?;
    let increment = value("Increment")?;
    (increment != 0.0).then_some((x - offset, start, increment))
}

fn unique_names(names: &mut [String]) -> usize {
    let mut used = HashSet::new();
    let mut changed = 0;
    for name in names {
        if used.insert(name.clone()) {
            continue;
        }
        let original = name.clone();
        let split = original
            .rfind(" [")
            .filter(|_| original.ends_with(']'))
            .unwrap_or(original.len());
        let mut number = 2;
        loop {
            let candidate = format!("{} #{number}{}", &original[..split], &original[split..]);
            if used.insert(candidate.clone()) {
                *name = candidate;
                changed += 1;
                break;
            }
            number += 1;
        }
    }
    changed
}

fn field_vec<'a>(record: Record<'a>, separator: u8, marker: Option<char>) -> Vec<&'a str> {
    Fields::new(without_comment(record.text, marker), separator).collect()
}

fn same_header(fields: &[&str], header: &[String], offset: usize, trailing: bool) -> bool {
    let len = fields.len().saturating_sub(usize::from(
        trailing && fields.last().is_some_and(|f| unquote(f).is_empty()),
    ));
    len >= offset
        && len - offset == header.len()
        && fields
            .get(offset)
            .is_some_and(|first| unquote(first) == header[0])
        && fields[offset..len]
            .iter()
            .zip(header)
            .all(|(a, b)| unquote(a) == b)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Data,
    Empty,
    RepeatedHeader,
    SensorGroup,
}

fn row_kind(
    fields: &[&str],
    header: &[String],
    offset: usize,
    trailing: bool,
    has_date_time_pair: bool,
) -> RowKind {
    if fields.is_empty() {
        RowKind::Empty
    } else if !header.is_empty() && same_header(fields, header, offset, trailing) {
        RowKind::RepeatedHeader
    } else if has_date_time_pair && unquote(fields[0]).starts_with("Sensor Group:") {
        RowKind::SensorGroup
    } else {
        RowKind::Data
    }
}

struct Numeric {
    values: Vec<f64>,
    invalid: bool,
    boolean: bool,
    unit: Option<String>,
}

enum Builder<'a> {
    Numeric(Numeric),
    Date(Vec<&'a str>),
    Text(Vec<String>),
}

impl<'a> Builder<'a> {
    fn new(
        sample: &str,
        is_pair_time: bool,
        decimal_comma: bool,
        capacity: usize,
        previous: usize,
    ) -> Self {
        if is_pair_time
            || (!is_missing(sample)
                && matches!(parse(sample, decimal_comma), Value::Text)
                && temporal_shape(sample)
                && infer_datetime_format([sample], false).is_some())
        {
            let mut values = Vec::with_capacity(capacity);
            values.resize(previous, "");
            Self::Date(values)
        } else if matches!(parse(sample, decimal_comma), Value::Text) && !is_missing(sample) {
            let mut values = Vec::with_capacity(capacity);
            values.resize(previous, String::new());
            Self::Text(values)
        } else {
            let mut values = Vec::with_capacity(capacity);
            values.resize(previous, f64::NAN);
            Self::Numeric(Numeric {
                values,
                invalid: false,
                boolean: false,
                unit: None,
            })
        }
    }

    fn push(&mut self, raw: &'a str, decimal_comma: bool) {
        let s = unquote(raw);
        match self {
            Self::Date(values) => values.push(s),
            Self::Text(values) => values.push(owned_field(raw)),
            Self::Numeric(column) => match parse(s, decimal_comma) {
                Value::Number(value, unit) => {
                    if let Some(unit) = unit {
                        if column.unit.as_deref().is_some_and(|old| old != unit) {
                            column.invalid = true;
                        }
                        if column.unit.is_none() {
                            column.unit = Some(unit.to_owned());
                        }
                    }
                    column.values.push(value);
                }
                Value::Boolean(value) => {
                    column.boolean = true;
                    column.values.push(f64::from(u8::from(value)));
                }
                Value::Missing => column.values.push(f64::NAN),
                Value::Text => {
                    column.invalid = true;
                    column.values.push(f64::NAN);
                }
            },
        }
    }
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
    let (text, encoding) = decode_text(&bytes)?;
    let all_records = records(&text)?;
    if all_records.is_empty() {
        return Err(IoError::WrongFormat("Delimited file has no records".into()));
    }
    let separator = options
        .separator
        .unwrap_or_else(|| sniff_separator(&all_records));
    if separator == b'\n' || separator == b'\r' || separator == b'"' {
        return Err(IoError::Other("Invalid field separator".into()));
    }
    let decimal_comma = separator == b';';
    let skipped: HashSet<usize> = options.comment_lines.iter().copied().collect();
    let records: Cow<'_, [Record<'_>]> = if skipped.is_empty() && options.comment_char.is_none() {
        Cow::Borrowed(&all_records)
    } else {
        Cow::Owned(
            all_records
                .iter()
                .copied()
                .filter(|record| {
                    !skipped.contains(&record.line)
                        && !without_comment(record.text, options.comment_char)
                            .trim()
                            .is_empty()
                })
                .collect(),
        )
    };
    if records.is_empty() {
        return Err(IoError::WrongFormat(
            "Delimited file has no data records".into(),
        ));
    }

    let mut sample: Vec<Vec<&str>> = records
        .iter()
        .take(256)
        .map(|r| field_vec(*r, separator, options.comment_char))
        .collect();
    let (mut best_start, best_len) = longest_data_run(&sample, decimal_comma);
    if best_len < 2 && records.len() > sample.len() {
        sample.extend(
            records
                .iter()
                .skip(256)
                .take(3840)
                .map(|r| field_vec(*r, separator, options.comment_char)),
        );
        best_start = longest_data_run(&sample, decimal_comma).0;
    }
    let mut data_start =
        best_start.unwrap_or_else(|| usize::from(options.detect_column_names && sample.len() > 1));
    while data_start > 0 && looks_like_data(&sample[data_start - 1], decimal_comma) {
        let prior = sample[data_start - 1].first().map_or("", |s| unquote(s));
        let current = sample[data_start].first().map_or("", |s| unquote(s));
        if matches!(parse(prior, decimal_comma), Value::Text)
            && !temporal_shape(prior)
            && (!matches!(parse(current, decimal_comma), Value::Text) || temporal_shape(current))
        {
            break;
        }
        data_start -= 1;
    }
    let explicit_names = !options.column_names.is_empty() || options.column_names_line.is_some();
    let mut header_index = None;
    let mut units_index = None;
    let mut offset = 0usize;
    let mut names = options.column_names.clone();
    let mut raw_header = Vec::new();
    let data_fields = sample
        .get(data_start)
        .ok_or_else(|| IoError::WrongFormat("Delimited file has no data records".into()))?;
    let named_final_field = data_start > 0
        && sample[data_start - 1].len() == data_fields.len()
        && sample[data_start - 1]
            .last()
            .is_some_and(|v| !unquote(v).is_empty());
    let trailing = separator != b' '
        && !named_final_field
        && data_fields.last().is_some_and(|v| unquote(v).is_empty())
        && sample[data_start..]
            .iter()
            .take(16)
            .filter(|r| looks_like_data(r, decimal_comma))
            .all(|r| r.last().is_some_and(|v| unquote(v).is_empty()));
    let data_width = data_fields.len().saturating_sub(usize::from(trailing));
    if data_width == 0 {
        return Err(IoError::WrongFormat("Delimited file has no columns".into()));
    }

    // Tek-style exports interleave metadata in leading fields with waveform data
    // in later fields. Only discard the empty prefix when the metadata proves it.
    if data_start >= 2 && data_fields.len() >= 4 {
        let empty_prefix = data_fields
            .iter()
            .take_while(|v| unquote(v).is_empty())
            .count();
        if empty_prefix >= 2
            && empty_prefix + 2 <= data_width
            && sample[data_start..]
                .iter()
                .take(16)
                .all(|row| row.iter().take(empty_prefix).all(|v| unquote(v).is_empty()))
            && sample[..data_start]
                .iter()
                .filter(|row| {
                    row.iter()
                        .take(empty_prefix)
                        .any(|v| !unquote(v).is_empty())
                })
                .count()
                >= 2
        {
            for h in 0..data_start.saturating_sub(1) {
                if sample[h].len() >= data_width
                    && sample[h + 1].len() >= data_width
                    && is_units_row(&sample[h + 1], empty_prefix, data_width - empty_prefix)
                    && sample[h][empty_prefix..data_width]
                        .iter()
                        .all(|v| !unquote(v).is_empty())
                {
                    offset = empty_prefix;
                    header_index = Some(h);
                    units_index = Some(h + 1);
                    break;
                }
            }
        }
    }

    if let Some(line) = options.column_names_line {
        let marker = options.comment_char.or_else(|| {
            all_records
                .first()
                .and_then(|r| r.text.trim_start().chars().next())
                .filter(|c| matches!(c, '#' | '!'))
        });
        let leading = all_records
            .iter()
            .take_while(|r| marker.is_some_and(|m| r.text.trim_start().starts_with(m)))
            .count();
        let physical = if line < 0 {
            leading as isize + line
        } else {
            line
        };
        let record = all_records
            .iter()
            .find(|r| r.line as isize == physical)
            .ok_or_else(|| super::broken("column_names_line is outside the file"))?;
        let line_text = record.text.trim();
        let line_text = marker
            .and_then(|m| line_text.strip_prefix(m))
            .unwrap_or(line_text)
            .trim();
        names = Fields::new(line_text, separator).map(owned_field).collect();
        if line >= 0 {
            data_start = records
                .iter()
                .position(|r| r.line > physical as usize)
                .unwrap_or(records.len());
        }
    } else if !options.column_names.is_empty() || !options.detect_column_names {
        data_start = 0;
    } else if header_index.is_none() && data_start > 0 {
        let previous = data_start - 1;
        let meaningful = data_width.saturating_sub(offset);
        if is_units_row(&sample[previous], offset, meaningful) && previous > 0 {
            units_index = Some(previous);
            header_index = Some(previous - 1);
        } else {
            header_index = Some(previous);
        }
    }

    if names.is_empty() && options.detect_column_names {
        if let Some(h) = header_index {
            let row = &sample[h];
            let mut end = row.len();
            if end > offset
                && row.last().is_some_and(|v| unquote(v).is_empty())
                && (trailing || end > data_width)
            {
                end -= 1;
            }
            // Rigol's Start/Increment fields are metadata, not waveform columns.
            if units_index.is_some()
                && row
                    .iter()
                    .skip(data_width)
                    .any(|s| unquote(s).eq_ignore_ascii_case("increment"))
                && row
                    .iter()
                    .skip(data_width)
                    .any(|s| unquote(s).eq_ignore_ascii_case("start"))
            {
                end = end.min(data_width);
            }
            if end > offset {
                raw_header = row[offset..end].iter().map(|s| owned_field(s)).collect();
                names = raw_header.clone();
            }
        }
    }
    if names.is_empty() {
        names = (0..data_width.saturating_sub(offset))
            .map(|i| format!("C{i}"))
            .collect();
    }
    if let Some(u) = units_index {
        let row = &sample[u];
        for (i, name) in names.iter_mut().enumerate() {
            if let Some(unit) = row.get(offset + i).and_then(|field| unit_token(field)) {
                *name = append_unit(name, unit);
            }
        }
    }
    if !explicit_names {
        let sensor_ids = header_index
            .and_then(|h| h.checked_sub(1))
            .and_then(|h| sample.get(h))
            .map(Vec::as_slice);
        infer_schema_units(&mut names, &raw_header, sensor_ids);
    }
    let sequence_timebase =
        sequence_timebase(&sample, header_index, units_index, offset, data_width);
    for (i, name) in names.iter_mut().enumerate() {
        if name.trim().is_empty() {
            *name = format!("C{i}");
        }
    }
    let date_index = names
        .iter()
        .position(|n| n.trim().eq_ignore_ascii_case("date"));
    let time_index = names
        .iter()
        .position(|n| n.trim().eq_ignore_ascii_case("time"));
    let date_time_pair = date_index.zip(time_index).filter(|(date, time)| {
        date != time
            && sample
                .iter()
                .skip(data_start)
                .take(64)
                .filter_map(|row| row.get(offset + time))
                .map(|s| unquote(s))
                .find(|s| !is_missing(s))
                .is_some_and(|s| s.contains(':'))
    });
    let capacity = records.len().saturating_sub(data_start);
    let mut builders = Vec::with_capacity(names.len());
    for (i, name) in names.iter().enumerate() {
        let sample_value = sample
            .iter()
            .skip(data_start)
            .take(64)
            .filter_map(|row| row.get(offset + i))
            .map(|s| unquote(s))
            .find(|s| !is_missing(s))
            .unwrap_or("");
        if name.to_ascii_lowercase().contains("address")
            && (sample_value.starts_with("0x") || sample_value.starts_with("0X"))
        {
            builders.push(Builder::Text(Vec::with_capacity(capacity)));
        } else {
            builders.push(Builder::new(
                sample_value,
                date_time_pair.is_some_and(|(_, t)| t == i),
                decimal_comma,
                capacity,
                0,
            ));
        }
    }
    let expected_width = names.len();
    let mut short_rows = 0usize;
    let mut long_rows = 0usize;
    let mut repeated = 0usize;
    let mut groups = Vec::new();
    let mut kept = 0usize;
    let mut fields = Vec::with_capacity(names.len() + offset + 1);
    for record in records.iter().skip(data_start) {
        fields.clear();
        fields.extend(Fields::new(
            without_comment(record.text, options.comment_char),
            separator,
        ));
        match row_kind(
            &fields,
            &raw_header,
            offset,
            trailing,
            date_time_pair.is_some(),
        ) {
            RowKind::Empty => continue,
            RowKind::RepeatedHeader => {
                repeated += 1;
                continue;
            }
            RowKind::SensorGroup => {
                groups.push(record.text.to_owned());
                continue;
            }
            RowKind::Data => {}
        }
        if trailing && fields.last().is_some_and(|v| unquote(v).is_empty()) {
            fields.pop();
        }
        let width = fields.len().saturating_sub(offset);
        if width < expected_width {
            short_rows += 1;
        }
        if width > expected_width {
            long_rows += 1;
        }
        if width > names.len() {
            while builders.len() < width {
                let i = builders.len();
                names.push(format!("C{i}"));
                let sample = fields.get(offset + i).map_or("", |s| unquote(s));
                builders.push(Builder::new(sample, false, decimal_comma, capacity, kept));
            }
        }
        for (i, builder) in builders.iter_mut().enumerate() {
            builder.push(fields.get(offset + i).copied().unwrap_or(""), decimal_comma);
        }
        kept += 1;
    }
    if kept == 0 {
        return Err(IoError::WrongFormat(
            "Delimited file has no data rows".into(),
        ));
    }

    // An initially numeric column that later contains text is reconstructed from
    // source slices, preserving the original spelling instead of stringifying floats.
    let invalid: Vec<usize> = builders
        .iter()
        .enumerate()
        .filter_map(|(i, b)| match b {
            Builder::Numeric(n) if n.invalid => Some(i),
            _ => None,
        })
        .collect();
    let mixed_columns = invalid.len();
    if !invalid.is_empty() {
        let mut values: Vec<Vec<String>> =
            invalid.iter().map(|_| Vec::with_capacity(kept)).collect();
        for record in records.iter().skip(data_start) {
            fields.clear();
            fields.extend(Fields::new(
                without_comment(record.text, options.comment_char),
                separator,
            ));
            if row_kind(
                &fields,
                &raw_header,
                offset,
                trailing,
                date_time_pair.is_some(),
            ) != RowKind::Data
            {
                continue;
            }
            for (column, rows) in invalid.iter().zip(&mut values) {
                rows.push(
                    fields
                        .get(offset + column)
                        .map_or_else(String::new, |s| owned_field(s)),
                );
            }
        }
        for (column, rows) in invalid.into_iter().zip(values) {
            builders[column] = Builder::Text(rows);
        }
    }
    let mut notes = Vec::new();
    let mut merged = None;
    if let Some((date, time)) = date_time_pair {
        if let (Some(Builder::Date(dates)), Some(Builder::Date(times))) =
            (builders.get(date), builders.get(time))
        {
            if let Some((format, ambiguous)) =
                infer_datetime_format(dates.iter().copied(), options.dayfirst)
            {
                let parsed: Option<Vec<f64>> = dates
                    .iter()
                    .zip(times)
                    .map(|(date, time)| {
                        if is_missing(date) || is_missing(time) {
                            Some(f64::NAN)
                        } else {
                            format.parse_parts(date, time)
                        }
                    })
                    .collect();
                if let Some(parsed) = parsed {
                    merged = Some((date, time, parsed));
                    notes.push("Merged Date + Time into Time".to_owned());
                    if ambiguous {
                        notes.push(format!(
                            "Ambiguous day/month dates; used {}",
                            if options.dayfirst {
                                "day-first"
                            } else {
                                "month-first"
                            }
                        ));
                    }
                }
            }
        }
    }
    let mut columns = Vec::with_capacity(builders.len());
    let merged_indices = merged.map(|(date, time, values)| {
        columns.push(Column::datetime("Time", values));
        (date, time)
    });
    let (mut boolean_columns, mut suffix_columns) = (0, 0);
    for (i, (mut name, builder)) in names.into_iter().zip(builders).enumerate() {
        if merged_indices.is_some_and(|(date, time)| date == i || time == i) {
            continue;
        }
        let data = match builder {
            Builder::Numeric(column) => {
                if column.boolean {
                    boolean_columns += 1;
                }
                if column.unit.is_some() {
                    suffix_columns += 1;
                }
                if column.boolean && !has_unit(&name) {
                    name = append_unit(&name, "Yes/No");
                } else if let Some(unit) = &column.unit {
                    name = append_unit(&name, unit);
                }
                if name.eq_ignore_ascii_case("Time_Of_Day_Seconds") {
                    notes.push("Interpreted Time_Of_Day_Seconds as Unix epoch seconds".to_owned());
                    ColumnData::DateTime(column.values)
                } else {
                    ColumnData::Float(column.values)
                }
            }
            Builder::Text(values) => ColumnData::Str(values),
            Builder::Date(values) => {
                let parsed = infer_datetime_format(values.iter().copied(), options.dayfirst)
                    .and_then(|(format, ambiguous)| {
                        let parsed: Option<Vec<f64>> = values
                            .iter()
                            .map(|s| {
                                if is_missing(s) {
                                    Some(f64::NAN)
                                } else {
                                    format.parse(s)
                                }
                            })
                            .collect();
                        if parsed.is_some() && ambiguous {
                            notes.push(format!(
                                "Ambiguous day/month dates in {name}; used {}",
                                if options.dayfirst {
                                    "day-first"
                                } else {
                                    "month-first"
                                }
                            ));
                        }
                        parsed
                    });
                match parsed {
                    Some(values) => ColumnData::DateTime(values),
                    None => ColumnData::Str(values.into_iter().map(str::to_owned).collect()),
                }
            }
        };
        columns.push(Column::new(name, data));
    }
    if merged_indices.is_none() {
        if let Some((index, start, increment)) = sequence_timebase {
            if let Some(ColumnData::Float(values)) = columns.get(index).map(|column| &column.data) {
                let time = values
                    .iter()
                    .map(|value| start + value * increment)
                    .collect();
                columns.insert(0, Column::float("Time [s]", time));
                notes.push("Derived time from sequence, start and increment".to_owned());
            }
        }
    }
    let mut final_names: Vec<_> = columns.iter().map(|c| c.name.clone()).collect();
    let unique = unique_names(&mut final_names);
    for (column, name) in columns.iter_mut().zip(final_names) {
        column.name = name;
    }
    let df = super::frame(columns)?;
    let mut detail = format!(
        "Encoding: {encoding}\nSeparator: {}",
        if separator == b' ' {
            "whitespace".to_owned()
        } else if separator == b'\t' {
            "tab".to_owned()
        } else {
            char::from(separator).to_string()
        }
    );
    if trailing {
        detail.push_str("\nDropped empty terminal column");
    }
    if decimal_comma {
        detail.push_str("\nDecimal comma enabled");
    }
    if boolean_columns > 0 {
        detail.push_str(&format!(
            "\nConverted boolean values to 0/1 in {boolean_columns} columns"
        ));
    }
    if suffix_columns > 0 {
        detail.push_str(&format!(
            "\nParsed value-unit suffixes in {suffix_columns} columns"
        ));
    }
    if mixed_columns > 0 {
        detail.push_str(&format!(
            "\nPreserved {mixed_columns} mixed-format columns as text"
        ));
    }
    if let Some(h) = header_index {
        let preamble: Vec<_> = records
            .iter()
            .take(data_start)
            .enumerate()
            .filter(|(i, _)| Some(*i) != header_index && Some(*i) != units_index)
            .map(|(_, r)| r.text)
            .collect();
        if !preamble.is_empty() {
            detail.push_str(&format!(
                "\nSkipped {} preamble lines:\n{}",
                preamble.len(),
                preamble.join("\n")
            ));
        }
        if offset > 0 {
            for index in [Some(h), units_index].into_iter().flatten() {
                let prefix = sample[index]
                    .iter()
                    .take(offset)
                    .map(|s| unquote(s))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(": ");
                if !prefix.is_empty() {
                    detail.push_str(&format!("\n{prefix}"));
                }
            }
        }
        if h > 0
            && sample[h - 1]
                .iter()
                .skip(1)
                .any(|v| unquote(v).starts_with('/'))
        {
            detail.push_str("\nLibreHardwareMonitor sensor IDs are listed in the preamble");
        }
        if let Some(u) = units_index {
            let data_end = offset + expected_width;
            for (key, value) in sample[h]
                .iter()
                .skip(data_end)
                .zip(sample[u].iter().skip(data_end))
            {
                let key = unquote(key);
                if !key.is_empty() {
                    detail.push_str(&format!("\n{key}: {}", unquote(value)));
                }
            }
        }
    }
    if !groups.is_empty() {
        detail.push_str(&format!(
            "\nSkipped {} sensor-group rows: {}",
            groups.len(),
            groups.join("; ")
        ));
    }
    if repeated > 0 {
        detail.push_str(&format!("\nSkipped {repeated} repeated header rows"));
    }
    if short_rows > 0 || long_rows > 0 {
        detail.push_str(&format!(
            "\n{short_rows} short rows padded; {long_rows} long rows preserved"
        ));
    }
    if unique > 0 {
        detail.push_str(&format!("\nMade {unique} duplicate names unique"));
    }
    if explicit_names {
        detail.push_str("\nColumn names supplied by import options");
    }
    for note in notes {
        detail.push('\n');
        detail.push_str(&note);
    }
    Ok(super::object(
        path,
        "CSV file",
        vec![NamedFrame::unnamed(df)],
        None,
        detail,
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
