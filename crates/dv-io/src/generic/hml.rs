//! MSI Afterburner hardware-monitoring logs use typed records, not a CSV header.
use std::{borrow::Cow, path::Path, sync::Arc};

use dv_core::{
    table::{infer_datetime_format, DateTimeFormat},
    Column, FileObject, IoError, NamedFrame,
};

const FORMAT: &str = "MSI Afterburner HML";

/// CSV's quote must be the first byte of a field, but HML pads fields before
/// quoted names too. Remove only that padding, leaving the usual case borrowed.
fn unpad_quoted_fields(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut output: Option<String> = None;
    let mut copied_from = 0;
    let mut i = 0;
    let mut at_field_start = true;
    let mut quoted = false;
    while i < bytes.len() {
        if at_field_start {
            let start = i;
            while matches!(bytes.get(i), Some(b' ' | b'\t')) {
                i += 1;
            }
            if bytes.get(i) == Some(&b'"') {
                if i > start {
                    let out = output.get_or_insert_with(|| String::with_capacity(text.len()));
                    out.push_str(&text[copied_from..start]);
                    copied_from = i;
                }
                quoted = true;
                at_field_start = false;
                i += 1;
                continue;
            }
            i = start;
            at_field_start = false;
        }
        match bytes[i] {
            b'"' if quoted && bytes.get(i + 1) == Some(&b'"') => {
                i += 2;
                continue;
            }
            b'"' if quoted => quoted = false,
            b',' | b'\r' | b'\n' if !quoted => at_field_start = true,
            _ => {}
        }
        i += 1;
    }
    match output {
        Some(mut out) => {
            out.push_str(&text[copied_from..]);
            Cow::Owned(out)
        }
        None => Cow::Borrowed(text),
    }
}

#[derive(Default)]
struct Block {
    version: String,
    devices: Vec<String>,
    metadata: Vec<String>,
    other_records: Vec<String>,
    has_schema: bool,
    // Keep the physical width as well: trailing empty header cells may just be
    // delimiters, or can be real unnamed channels identified by a 03 record.
    declared_width: usize,
    names: Vec<String>,
    units: Vec<Option<String>>,
    timestamps: Vec<f64>,
    values: Vec<Vec<f64>>,
    short_rows: usize,
    missing_timestamps: usize,
}

impl Block {
    fn context(&self) -> Self {
        Self {
            version: self.version.clone(),
            devices: self.devices.clone(),
            ..Self::default()
        }
    }

    fn same_schema(&self) -> Self {
        Self {
            version: self.version.clone(),
            devices: self.devices.clone(),
            metadata: self.metadata.clone(),
            has_schema: self.has_schema,
            declared_width: self.declared_width,
            names: self.names.clone(),
            units: self.units.clone(),
            values: (0..self.values.len()).map(|_| Vec::new()).collect(),
            ..Self::default()
        }
    }

    fn set_schema(&mut self, record: &csv::StringRecord) {
        self.has_schema = true;
        self.declared_width = record.len().saturating_sub(2);
        self.names = record.iter().skip(2).map(str::to_owned).collect();
        while self.names.last().is_some_and(String::is_empty) {
            self.names.pop();
        }
        self.units = vec![None; self.names.len()];
        self.values = (0..self.names.len()).map(|_| Vec::new()).collect();
    }

    fn expand_to(&mut self, width: usize) {
        while self.names.len() < width {
            self.names.push(String::new());
            self.units.push(None);
            self.values.push(vec![f64::NAN; self.timestamps.len()]);
        }
    }
}

fn record_description(record: &csv::StringRecord) -> String {
    record.iter().skip(1).collect::<Vec<_>>().join(", ")
}

fn broken_at(line: u64, message: impl std::fmt::Display) -> IoError {
    IoError::BrokenFormat(format!("HML record at line {line}: {message}"))
}

fn missing(value: &str) -> bool {
    let value = value.trim();
    value.is_empty()
        || value == "-"
        || value.eq_ignore_ascii_case("na")
        || value.eq_ignore_ascii_case("n/a")
        || value.eq_ignore_ascii_case("[n/a]")
        || value.eq_ignore_ascii_case("null")
        || value.eq_ignore_ascii_case("none")
        || value.eq_ignore_ascii_case("nat")
        || value.eq_ignore_ascii_case("nan")
}

fn timestamp(value: &str, format: &mut Option<DateTimeFormat>, line: u64) -> Result<f64, IoError> {
    if missing(value) {
        return Ok(f64::NAN);
    }
    if format.is_none() {
        *format = infer_datetime_format(std::iter::once(value), true).map(|(format, _)| format);
    }
    format
        .as_ref()
        .and_then(|format| format.parse(value))
        .ok_or_else(|| broken_at(line, format!("invalid timestamp “{value}”")))
}

/// Read a typed MSI Afterburner hardware-monitoring log.
pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Err(IoError::Empty(path.to_owned()));
    }
    parse(path, &bytes)
}

fn parse(path: &Path, bytes: &[u8]) -> Result<Arc<dyn FileObject>, IoError> {
    let (text, encoding) = super::csv::decode_text(bytes)
        .map_err(|err| IoError::WrongFormat(format!("cannot decode HML text: {err}")))?;
    let text = unpad_quoted_fields(&text);
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut record = csv::StringRecord::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut time_format: Option<DateTimeFormat> = None;
    let mut record_number = 0u64;

    loop {
        let present = reader.read_record(&mut record).map_err(|err| {
            if blocks.is_empty() {
                IoError::WrongFormat(format!("not an HML log: {err}"))
            } else {
                IoError::BrokenFormat(format!("invalid HML record: {err}"))
            }
        })?;
        if !present {
            break;
        }
        record_number += 1;
        let line = record.position().map_or(record_number, |pos| pos.line());
        let code = record.get(0).unwrap_or("");
        if blocks.is_empty()
            && (code != "00"
                || !record
                    .get(2)
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .starts_with("hardware monitoring log"))
        {
            return Err(IoError::WrongFormat(
                "missing MSI Afterburner hardware monitoring log signature".into(),
            ));
        }

        match code {
            "00" => {
                blocks.push(Block {
                    version: record_description(&record),
                    ..Block::default()
                });
                time_format = None;
            }
            "01" => {
                let block = blocks
                    .last_mut()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                block.devices.push(record_description(&record));
            }
            "02" => {
                let previous = blocks
                    .last()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                if previous.has_schema {
                    blocks.push(previous.context());
                    time_format = None;
                }
                let block = blocks
                    .last_mut()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                block.set_schema(&record);
            }
            "03" => {
                let block = blocks
                    .last_mut()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                if !block.has_schema {
                    return Err(broken_at(line, "channel metadata precedes channel names"));
                }
                let index = record
                    .get(2)
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(|| broken_at(line, "invalid channel metadata index"))?;
                if index >= block.declared_width {
                    return Err(broken_at(
                        line,
                        format!("channel metadata index {index} exceeds the 02 header width"),
                    ));
                }
                block.expand_to(index + 1);
                let unit = record.get(3).unwrap_or("").to_owned();
                if !block.timestamps.is_empty()
                    && block.units[index].as_ref().is_some_and(|old| old != &unit)
                {
                    let next = block.same_schema();
                    blocks.push(next);
                }
                let block = blocks
                    .last_mut()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                block.units[index] = Some(unit);
                block.metadata.push(record_description(&record));
            }
            "80" => {
                let block = blocks
                    .last_mut()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                if !block.has_schema {
                    return Err(broken_at(line, "sample precedes channel names"));
                }
                if record
                    .iter()
                    .skip(2 + block.declared_width)
                    .any(|v| !v.is_empty())
                {
                    return Err(broken_at(
                        line,
                        "sample has values beyond the 02 header width",
                    ));
                }
                // Promote a trailing unnamed header cell only when a value
                // establishes that it is a channel, rather than a delimiter.
                if let Some(last) = (0..block.declared_width.min(record.len().saturating_sub(2)))
                    .rfind(|&index| record.get(index + 2).is_some_and(|v| !v.is_empty()))
                {
                    block.expand_to(last + 1);
                }
                if record.len().saturating_sub(2) < block.names.len() {
                    block.short_rows += 1;
                }
                let time = timestamp(record.get(1).unwrap_or(""), &mut time_format, line)?;
                if time.is_nan() {
                    block.missing_timestamps += 1;
                }
                for (index, column) in block.values.iter_mut().enumerate() {
                    let value = record.get(index + 2).unwrap_or("");
                    let parsed = if missing(value) {
                        f64::NAN
                    } else {
                        value.parse::<f64>().map_err(|_| {
                            broken_at(line, format!("invalid value “{value}” for channel {index}"))
                        })?
                    };
                    column.push(parsed);
                }
                block.timestamps.push(time);
            }
            _ => {
                let block = blocks
                    .last_mut()
                    .ok_or_else(|| broken_at(line, "missing session"))?;
                block
                    .other_records
                    .push(format!("{code}, {}", record_description(&record)));
            }
        }
    }

    if blocks.is_empty() {
        return Err(IoError::WrongFormat("HML has no records".into()));
    }
    if !blocks.iter().any(|block| block.has_schema) {
        return Err(IoError::BrokenFormat(
            "HML has no channel-name record (02)".into(),
        ));
    }

    let frame_count = blocks.iter().filter(|block| block.has_schema).count();
    let mut frames = Vec::with_capacity(frame_count);
    let mut detail = format!("encoding: {encoding}\n{} session/block(s)", blocks.len());
    for (index, block) in blocks.into_iter().enumerate() {
        detail.push_str(&format!(
            "\nSession {}: {} samples, {} channels\n  Version: {}",
            index + 1,
            block.timestamps.len(),
            block.names.len(),
            block.version
        ));
        for device in &block.devices {
            detail.push_str(&format!("\n  Device: {device}"));
        }
        for metadata in &block.metadata {
            detail.push_str(&format!("\n  Channel metadata: {metadata}"));
        }
        for other in &block.other_records {
            detail.push_str(&format!("\n  Unrecognized record: {other}"));
        }
        if block.short_rows > 0 {
            detail.push_str(&format!("\n  Short sample records: {}", block.short_rows));
        }
        if block.missing_timestamps > 0 {
            detail.push_str(&format!(
                "\n  Missing timestamps: {}",
                block.missing_timestamps
            ));
        }
        if !block.has_schema {
            continue;
        }
        let mut columns = Vec::with_capacity(block.names.len() + 1);
        columns.push(Column::datetime("Timestamp", block.timestamps));
        for (channel, ((name, unit), values)) in block
            .names
            .into_iter()
            .zip(block.units)
            .zip(block.values)
            .enumerate()
        {
            let mut name = if name.is_empty() {
                format!("Channel {channel}")
            } else {
                name
            };
            if let Some(unit) = unit.filter(|unit| !unit.is_empty()) {
                let suffix = format!("[{unit}]");
                if !name.ends_with(&suffix) {
                    name.push(' ');
                    name.push_str(&suffix);
                }
            }
            columns.push(Column::float(name, values));
        }
        let df = super::frame(columns)?;
        let name = if frame_count == 1 {
            String::new()
        } else {
            format!("Session {}", index + 1)
        };
        frames.push(NamedFrame::new(name, df));
    }
    Ok(super::object(path, FORMAT, frames, None, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::{ColumnData, Field2D};

    fn parse_log(bytes: &[u8]) -> Result<Arc<dyn FileObject>, IoError> {
        parse(Path::new("synthetic.hml"), bytes)
    }

    #[test]
    fn indexed_units_duplicates_missing_and_legacy_encoding() {
        let mut bytes = b"00, 04-03-2023 12:34:56, Hardware monitoring log v1.6\n\
01, 04-03-2023 12:34:56, Synthetic GPU\n\
02, 04-03-2023 12:34:56, \"GPU, sensor\", Fan,Fan\n\
03, 04-03-2023 12:34:56, 2, RPM,0,1000,10\n\
03, 04-03-2023 12:34:56, 0, "
            .to_vec();
        bytes.extend_from_slice(
            b"\xb0C,0,100,10\n\
03, 04-03-2023 12:34:56, 1, RPM,0,1000,10\n\
80, 04-03-2023 12:34:56, 42,100,200,\n\
80, 04-03-2023 12:34:57, ,300\n",
        );
        let object = parse_log(&bytes).unwrap();
        let frames = object.to_dataframes().unwrap();
        let df = &frames[0].df;
        assert_eq!((df.nrows(), df.ncols()), (2, 4));
        assert_eq!(
            df.names(),
            ["Timestamp", "GPU, sensor [°C]", "Fan [RPM]", "Fan [RPM]"]
        );
        let ColumnData::DateTime(times) = &df.column(0).data else {
            panic!("timestamp is not datetime");
        };
        let expected = chrono::NaiveDate::from_ymd_opt(2023, 3, 4)
            .unwrap()
            .and_hms_opt(12, 34, 56)
            .unwrap()
            .and_utc()
            .timestamp() as f64;
        assert_eq!(times, &[expected, expected + 1.0]);
        assert!(df.column(1).floats().unwrap()[1].is_nan());
        assert_eq!(df.column(2).floats().unwrap(), &[100.0, 300.0]);
        assert!(df.column(3).floats().unwrap()[1].is_nan());
        let info = object.info();
        assert!(info.contains("Windows-1252"));
        assert!(info.contains("Hardware monitoring log v1.6"));
        assert!(info.contains("Synthetic GPU"));
        assert!(info.contains("0, °C, 0, 100, 10"));
        assert!(info.contains("Short sample records: 1"));
    }

    #[test]
    fn full_width_one_second_recording_preserves_every_channel() {
        use std::fmt::Write;

        let names = [
            "GPU temperature",
            "GPU usage",
            "Core clock",
            "GPU power",
            "CPU temperature",
            "CPU usage",
            "CPU clock",
            "Framerate",
            "Frametime",
        ];
        let units = ["°C", "%", "MHz", "W", "°C", "%", "MHz", "FPS", "ms"];
        let mut text = String::from(
            "00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n\
             01, 18-03-2023 12:00:00, Synthetic GPU\n\
             02, 18-03-2023 12:00:00, ",
        );
        text.push_str(&names.join(","));
        text.push('\n');
        for (index, unit) in units.iter().enumerate() {
            writeln!(
                text,
                "03, 18-03-2023 12:00:00, {index}, {unit}, 0, 1000, 10"
            )
            .unwrap();
        }
        for sample in 0..600 {
            write!(
                text,
                "80, 18-03-2023 12:{:02}:{:02}",
                sample / 60,
                sample % 60
            )
            .unwrap();
            for channel in 0..names.len() {
                write!(text, ",{}", sample + channel).unwrap();
            }
            text.push('\n');
        }

        let object = parse_log(text.as_bytes()).unwrap();
        let frames = object.to_dataframes().unwrap();
        let df = &frames[0].df;
        assert_eq!((df.nrows(), df.ncols()), (600, 10));
        let ColumnData::DateTime(times) = &df.column(0).data else {
            panic!("timestamp is not datetime");
        };
        assert_eq!(times[599] - times[0], 599.0);
        for (index, (name, unit)) in names.iter().zip(units).enumerate() {
            assert_eq!(df.column(index + 1).name, format!("{name} [{unit}]"));
            assert_eq!(
                df.column(index + 1).floats().unwrap()[599],
                (599 + index) as f64
            );
        }
    }

    #[test]
    fn repeated_sessions_and_changed_schemas_remain_separate() {
        let object = parse_log(
            b"\
00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n\
01, 18-03-2023 12:00:00, Device A\n\
02, 18-03-2023 12:00:00, Power\n\
03, 18-03-2023 12:00:00, 0, W\n\
80, 18-03-2023 12:00:01, 10\n\
02, 18-03-2023 12:00:02, Temperature,Power\n\
03, 18-03-2023 12:00:02, 1, W\n\
80, 18-03-2023 12:00:03, 40,20\n\
00, 18-03-2023 12:00:04, Hardware monitoring log v1.7\n\
01, 18-03-2023 12:00:04, Device B\n\
02, 18-03-2023 12:00:04, Clock\n\
03, 18-03-2023 12:00:04, 0, MHz\n\
80, 18-03-2023 12:00:05, 1000\n",
        )
        .unwrap();
        let frames = object.to_dataframes().unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].df.names(), ["Timestamp", "Power [W]"]);
        assert_eq!(
            frames[1].df.names(),
            ["Timestamp", "Temperature", "Power [W]"]
        );
        assert_eq!(frames[2].df.names(), ["Timestamp", "Clock [MHz]"]);
        assert_eq!(frames[1].df.column(2).floats().unwrap(), &[20.0]);
        let info = object.info();
        for expected in [
            "Session 1",
            "Session 2",
            "Session 3",
            "v1.6",
            "v1.7",
            "Device A",
            "Device B",
        ] {
            assert!(info.contains(expected), "missing {expected}");
        }
    }

    #[test]
    fn unnamed_channels_and_unit_changes_do_not_shift_values() {
        let object = parse_log(
            b"\
00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n\
02, 18-03-2023 12:00:00, A,,\n\
03, 18-03-2023 12:00:00, 1, W\n\
80, 18-03-2023 12:00:01, 1,2,\n\
03, 18-03-2023 12:00:02, 1, kW\n\
80, 18-03-2023 12:00:03, 3,4,\n",
        )
        .unwrap();
        let frames = object.to_dataframes().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].df.names(), ["Timestamp", "A", "Channel 1 [W]"]);
        assert_eq!(frames[1].df.names(), ["Timestamp", "A", "Channel 1 [kW]"]);
        assert_eq!(frames[0].df.column(2).floats().unwrap(), &[2.0]);
        assert_eq!(frames[1].df.column(2).floats().unwrap(), &[4.0]);
    }

    #[test]
    fn utf16_hml_uses_the_shared_decoder() {
        let text = "\
00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n\
02, 18-03-2023 12:00:00, Temperature\n\
03, 18-03-2023 12:00:00, 0, °C\n\
80, 18-03-2023 12:00:01, 42\n";
        let mut bytes = vec![0xff, 0xfe];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let object = parse_log(&bytes).unwrap();
        let frames = object.to_dataframes().unwrap();
        assert_eq!(frames[0].df.names(), ["Timestamp", "Temperature [°C]"]);
        assert_eq!(frames[0].df.column(1).floats().unwrap(), &[42.0]);
        assert!(object.info().contains("UTF-16LE"));
    }

    #[test]
    fn non_hml_is_wrong_format_and_inconsistent_hml_is_broken() {
        assert!(matches!(
            parse_log(b"time,power\n0,1\n"),
            Err(IoError::WrongFormat(_))
        ));
        assert!(matches!(
            parse_log(b"00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n02, t, Power\n80, 18-03-2023 12:00:00, 1,2\n"),
            Err(IoError::BrokenFormat(_))
        ));
        assert!(matches!(
            parse_log(b"00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n02, t, Power\n80, invalid, 1\n"),
            Err(IoError::BrokenFormat(_))
        ));
        assert!(matches!(
            parse_log(b"00, 18-03-2023 12:00:00, Hardware monitoring log v1.6\n02, t, Power\n80, 18-03-2023 12:00:00, unexpected\n"),
            Err(IoError::BrokenFormat(_))
        ));
    }

    #[test]
    fn csv_fields_are_created_only_for_matrices_and_remain_format_specific() {
        // A matrix export: every column holds the same quantity.
        let matrix = super::super::frame(vec![
            Column::float("Index", vec![0.0, 1.0]),
            Column::float("Bin 1 [dB]", vec![10.0, 11.0]),
            Column::float("Bin 2 [dB]", vec![20.0, 21.0]),
        ])
        .unwrap();
        let file = super::super::object(
            Path::new("matrix.csv"),
            "CSV file",
            vec![NamedFrame::unnamed(matrix)],
            None,
            String::new(),
        );
        assert_eq!(
            file.to_2d_fields().unwrap().unwrap(),
            vec![Field2D {
                name: "data".into(),
                x: vec![0.0, 1.0],
                y: vec![0.0, 1.0],
                sx: "rows".into(),
                sy: "columns".into(),
                m: vec![10.0, 20.0, 11.0, 21.0],
            }]
        );
        // A log of different quantities over time is not a field.
        let df = super::super::frame(vec![
            Column::float("Index", vec![0.0, 1.0]),
            Column::datetime("Timestamp", vec![10.0, 11.0]),
            Column::float("Power", vec![20.0, 21.0]),
        ])
        .unwrap();
        let log = super::super::object(
            Path::new("test.csv"),
            "CSV file",
            vec![NamedFrame::unnamed(df.clone())],
            None,
            String::new(),
        );
        assert!(log.to_2d_fields().is_none());
        let other = super::super::object(
            Path::new("other"),
            "other format",
            vec![NamedFrame::unnamed(df)],
            None,
            String::new(),
        );
        assert!(other.to_2d_fields().is_none());
        let existing = Field2D {
            name: "existing".into(),
            x: vec![0.0],
            y: vec![0.0],
            sx: "x".into(),
            sy: "y".into(),
            m: vec![7.0],
        };
        let other_with_fields = super::super::object(
            Path::new("other"),
            "other format",
            vec![],
            Some(vec![existing.clone()]),
            String::new(),
        );
        assert_eq!(
            other_with_fields.to_2d_fields().unwrap().unwrap(),
            vec![existing]
        );
        let strings =
            super::super::frame(vec![Column::string("state", vec!["on".into()])]).unwrap();
        let nonnumeric = super::super::object(
            Path::new("text.csv"),
            "CSV file",
            vec![NamedFrame::unnamed(strings)],
            None,
            String::new(),
        );
        assert!(nonnumeric.to_2d_fields().is_none());
    }
}
