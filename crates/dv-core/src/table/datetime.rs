//! Inference and reusable parsing for calendar timestamps in imported columns.

use chrono::{NaiveDate, NaiveTime};

use crate::dataframe::datetime_to_seconds;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateStyle {
    Numeric { dayfirst: bool },
    Compact,
    Text { dayfirst: bool },
}

/// A reusable calendar-date parser inferred from a column of samples.
///
/// Naive dates and times are represented as UTC-naive epoch seconds. Explicit
/// offsets are applied, so offset-bearing timestamps represent their UTC instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeFormat {
    style: DateStyle,
}

impl DateTimeFormat {
    /// Parse a date, or a date and 12- or 24-hour time, into Unix epoch seconds.
    pub fn parse(&self, s: &str) -> Option<f64> {
        let (date, time) = split_datetime(s.trim());
        let date = self.parse_date(date)?;
        match time {
            Some(time) => combine(date, time),
            None => Some(datetime_to_seconds(&date.and_hms_opt(0, 0, 0)?)),
        }
    }

    /// Parse a date and a separate time without allocating a combined string.
    /// Empty or invalid parts return `None`.
    pub fn parse_parts(&self, date: &str, time: &str) -> Option<f64> {
        combine(self.parse_date(date.trim())?, time.trim())
    }

    fn parse_date(&self, date: &str) -> Option<NaiveDate> {
        match self.style {
            DateStyle::Numeric { dayfirst } => {
                let (a, b, c) = date_fields(date)?;
                if a.len() == 4 && b.len() <= 2 && c.len() <= 2 {
                    NaiveDate::from_ymd_opt(number(a)? as i32, number(b)?, number(c)?)
                } else {
                    ordered_date(a, b, c, dayfirst)
                }
            }
            DateStyle::Compact => {
                if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                NaiveDate::from_ymd_opt(
                    number(&date[..4])? as i32,
                    number(&date[4..6])?,
                    number(&date[6..])?,
                )
            }
            DateStyle::Text { dayfirst } => text_date(date, dayfirst),
        }
    }
}

/// Infer a reusable datetime format, inspecting every calendar-shaped sample
/// when day/month ordering needs to be decided. Empty strings and `NaT` are
/// ignored. Malformed or incompatible samples do not establish an order; the
/// returned parser will reject them. Conflicting valid day-first and month-first
/// evidence returns `None` rather than silently mixing interpretations.
///
/// The second return value is `true` when both orders remain possible and
/// `dayfirst` had to choose between them. Bare numbers, including compact
/// dates, are not inferred as calendar columns: they may be elapsed time.
/// Compact dates are recognized when accompanied by a time, such as
/// `20230318T123456.700Z`.
pub fn infer_datetime_format<'a>(
    samples: impl IntoIterator<Item = &'a str>,
    dayfirst: bool,
) -> Option<(DateTimeFormat, bool)> {
    let mut numeric = false;
    let mut saw_ordered = false;
    let mut can_dayfirst = true;
    let mut can_monthfirst = true;

    for sample in samples {
        let sample = sample.trim();
        if sample.is_empty() || sample == "NaT" {
            continue;
        }
        let (date, time) = split_datetime(sample);
        if time.is_some_and(|t| parse_time(t).is_none()) {
            continue;
        }

        if !numeric {
            let Some(style) = detect_date_style(date, time.is_some()) else {
                continue;
            };
            if !matches!(style, DateStyle::Numeric { .. }) {
                return Some((DateTimeFormat { style }, false));
            }
            numeric = true;
        }
        if let Some((dmy, mdy)) = ordered_evidence(date) {
            saw_ordered = true;
            can_dayfirst &= dmy;
            can_monthfirst &= mdy;
            if !can_dayfirst && !can_monthfirst {
                return None;
            }
        }
    }

    if !numeric {
        return None;
    }
    let ambiguous = saw_ordered && can_dayfirst && can_monthfirst;
    let use_dayfirst = if can_dayfirst && can_monthfirst {
        dayfirst
    } else {
        can_dayfirst
    };
    Some((
        DateTimeFormat {
            style: DateStyle::Numeric {
                dayfirst: use_dayfirst,
            },
        },
        ambiguous,
    ))
}

/// Parse a single date/time string. `dayfirst` resolves day/month ambiguity.
///
/// For compatibility, this explicit single-value parser accepts an eight-digit
/// `YYYYMMDD` date, whereas column inference deliberately rejects bare numbers.
pub fn parse_datetime_str(s: &str, dayfirst: bool) -> Option<f64> {
    if let Some((format, _)) = infer_datetime_format([s], dayfirst) {
        return format.parse(s);
    }
    let s = s.trim();
    if s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()) {
        return DateTimeFormat {
            style: DateStyle::Compact,
        }
        .parse(s);
    }
    None
}

fn detect_date_style(date: &str, has_time: bool) -> Option<DateStyle> {
    if has_time && date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) {
        let style = DateStyle::Compact;
        return DateTimeFormat { style }.parse_date(date).map(|_| style);
    }

    if date_fields(date).is_some() {
        let style = DateStyle::Numeric { dayfirst: true };
        if (DateTimeFormat { style }).parse_date(date).is_some() || ordered_evidence(date).is_some()
        {
            return Some(style);
        }
        return None;
    }

    for dayfirst in [true, false] {
        if text_date(date, dayfirst).is_some() {
            return Some(DateStyle::Text { dayfirst });
        }
    }
    None
}

fn date_fields(date: &str) -> Option<(&str, &str, &str)> {
    let separator = date.bytes().find(|b| !b.is_ascii_digit())?;
    if !matches!(separator, b'-' | b'/' | b'.') {
        return None;
    }
    let mut parts = date.split(separator as char);
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || a.is_empty() || b.is_empty() || c.is_empty() {
        return None;
    }
    Some((a, b, c))
}

fn number(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    s.bytes().try_fold(0_u32, |n, b| {
        if b.is_ascii_digit() {
            n.checked_mul(10)?.checked_add(u32::from(b - b'0'))
        } else {
            None
        }
    })
}

fn year(s: &str) -> Option<i32> {
    let value = number(s)? as i32;
    match s.len() {
        2 => Some(if value >= 70 {
            1900 + value
        } else {
            2000 + value
        }),
        4 => Some(value),
        _ => None,
    }
}

fn ordered_date(a: &str, b: &str, c: &str, dayfirst: bool) -> Option<NaiveDate> {
    if a.len() > 2 || b.len() > 2 {
        return None;
    }
    let (a, b) = (number(a)?, number(b)?);
    let (month, day) = if dayfirst { (b, a) } else { (a, b) };
    NaiveDate::from_ymd_opt(year(c)?, month, day)
}

fn ordered_evidence(date: &str) -> Option<(bool, bool)> {
    let (a, b, c) = date_fields(date)?;
    let dmy = ordered_date(a, b, c, true).is_some();
    let mdy = ordered_date(a, b, c, false).is_some();
    (dmy || mdy).then_some((dmy, mdy))
}

fn text_date(date: &str, dayfirst: bool) -> Option<NaiveDate> {
    let mut parts = date.split_whitespace();
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || c.len() != 4 {
        return None;
    }
    let (month, day) = if dayfirst {
        (month_name(b)?, number(a)?)
    } else {
        (month_name(a)?, number(b.strip_suffix(',').unwrap_or(b))?)
    };
    NaiveDate::from_ymd_opt(year(c)?, month, day)
}

fn month_name(s: &str) -> Option<u32> {
    const MONTHS: [(&str, &str); 12] = [
        ("Jan", "January"),
        ("Feb", "February"),
        ("Mar", "March"),
        ("Apr", "April"),
        ("May", "May"),
        ("Jun", "June"),
        ("Jul", "July"),
        ("Aug", "August"),
        ("Sep", "September"),
        ("Oct", "October"),
        ("Nov", "November"),
        ("Dec", "December"),
    ];
    MONTHS
        .iter()
        .position(|(short, long)| s.eq_ignore_ascii_case(short) || s.eq_ignore_ascii_case(long))
        .map(|i| i as u32 + 1)
}

// The first colon belongs to the clock (or, for a compact clock, its offset).
// Locate the start of that clock without assuming the date has no spaces.
fn split_datetime(s: &str) -> (&str, Option<&str>) {
    let bytes = s.as_bytes();
    if let Some(colon) = bytes.iter().position(|&b| b == b':') {
        let mut start = colon;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        if start > 0
            && start < colon
            && (bytes[start - 1].is_ascii_whitespace()
                || matches!(bytes[start - 1], b'T' | b't' | b'_'))
        {
            let mut date = s[..start].trim_end();
            if date.ends_with('T') || date.ends_with('t') || date.ends_with('_') {
                date = &date[..date.len() - 1];
            }
            if !date.is_empty() {
                return (date.trim_end(), Some(&s[start..]));
            }
        }
    }
    for i in 1..bytes.len().saturating_sub(1) {
        if matches!(bytes[i], b'T' | b't' | b'_')
            && bytes[i - 1].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
        {
            return (&s[..i], Some(&s[i + 1..]));
        }
    }
    (s, None)
}

fn combine(date: NaiveDate, time: &str) -> Option<f64> {
    let (time, offset) = parse_time(time)?;
    Some(datetime_to_seconds(&date.and_time(time)) - f64::from(offset))
}

// Parse clocks independently from dates, allowing a date-only inference to be
// reused with a separate time column. Offset seconds are east of UTC.
fn parse_time(time: &str) -> Option<(NaiveTime, i32)> {
    let time = time.trim();
    let bytes = time.as_bytes();
    let (hour, minute, second, mut pos, has_seconds) =
        if bytes.get(1) == Some(&b':') || bytes.get(2) == Some(&b':') {
            let mut pos = 0;
            let hour = clock_number(bytes, &mut pos)?;
            if bytes.get(pos) != Some(&b':') {
                return None;
            }
            pos += 1;
            let minute = clock_number(bytes, &mut pos)?;
            let has_seconds = bytes.get(pos) == Some(&b':');
            let second = if has_seconds {
                pos += 1;
                clock_number(bytes, &mut pos)?
            } else {
                0
            };
            (hour, minute, second, pos, has_seconds)
        } else {
            if bytes.len() < 6 || !bytes[..6].iter().all(u8::is_ascii_digit) {
                return None;
            }
            let pair = |i: usize| u32::from(bytes[i] - b'0') * 10 + u32::from(bytes[i + 1] - b'0');
            (pair(0), pair(2), pair(4), 6, true)
        };

    let mut nanos = 0_u32;
    if matches!(bytes.get(pos), Some(b'.' | b',')) {
        if !has_seconds {
            return None;
        }
        pos += 1;
        let start = pos;
        let mut digits = 0;
        while let Some(&b) = bytes.get(pos) {
            if !b.is_ascii_digit() {
                break;
            }
            if digits < 9 {
                nanos = nanos * 10 + u32::from(b - b'0');
                digits += 1;
            }
            pos += 1;
        }
        if pos == start {
            return None;
        }
        for _ in digits..9 {
            nanos *= 10;
        }
    }

    let suffix = time[pos..].trim_start();
    let meridiem = suffix.get(..2);
    let (hour, suffix) = match meridiem {
        Some(value) if value.eq_ignore_ascii_case("am") || value.eq_ignore_ascii_case("pm") => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            let hour = hour % 12
                + if value.eq_ignore_ascii_case("pm") {
                    12
                } else {
                    0
                };
            (hour, &suffix[2..])
        }
        _ => (hour, suffix),
    };
    let offset = parse_offset(suffix.trim())?;
    let second = if second == 60 {
        nanos += 1_000_000_000;
        59
    } else {
        second
    };
    Some((
        NaiveTime::from_hms_nano_opt(hour, minute, second, nanos)?,
        offset,
    ))
}

fn clock_number(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let start = *pos;
    let mut value = 0;
    while *pos - start < 2 {
        let Some(&b) = bytes.get(*pos) else {
            break;
        };
        if !b.is_ascii_digit() {
            break;
        }
        value = value * 10 + u32::from(b - b'0');
        *pos += 1;
    }
    (*pos > start).then_some(value)
}

fn parse_offset(offset: &str) -> Option<i32> {
    if offset.is_empty() || offset.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    let bytes = offset.as_bytes();
    let sign = match bytes.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes) = match bytes {
        [_, h1, h2, b':', m1, m2] => ([*h1, *h2], [*m1, *m2]),
        [_, h1, h2, m1, m2] => ([*h1, *h2], [*m1, *m2]),
        _ => return None,
    };
    if !hours.iter().chain(&minutes).all(u8::is_ascii_digit) {
        return None;
    }
    let pair = |b: [u8; 2]| i32::from(b[0] - b'0') * 10 + i32::from(b[1] - b'0');
    let (hours, minutes) = (pair(hours), pair(minutes));
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3600 + minutes * 60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataframe::{Column, ColumnData, DataFrame};
    use crate::table::Table;

    fn epoch(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> f64 {
        datetime_to_seconds(
            &NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .and_hms_opt(h, min, s)
                .unwrap(),
        )
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }

    #[test]
    fn column_evidence_overrides_preference_and_is_reused() {
        let samples = ["03/04/2023", "", "NaT", "13.04.2023", "04-05-2023"];
        let (fmt, ambiguous) = infer_datetime_format(samples, false).unwrap();
        assert!(!ambiguous);
        assert_eq!(fmt.parse(samples[0]), Some(epoch(2023, 4, 3, 0, 0, 0)));
        assert_eq!(fmt.parse(samples[3]), Some(epoch(2023, 4, 13, 0, 0, 0)));
        assert_eq!(fmt.parse(samples[4]), Some(epoch(2023, 5, 4, 0, 0, 0)));
        close(
            fmt.parse_parts("3/4/23", "1:2:3.456789").unwrap(),
            epoch(2023, 4, 3, 1, 2, 3) + 0.456789,
        );

        let (fmt, ambiguous) = infer_datetime_format(["03/04/23", "04/18/23"], true).unwrap();
        assert!(!ambiguous);
        assert_eq!(fmt.parse("03/04/23"), Some(epoch(2023, 3, 4, 0, 0, 0)));
        assert!(infer_datetime_format(["13/04/23", "04/18/23"], true).is_none());

        let mixed = ["2023-04-01", "03/04/2023", "13/04/2023"];
        let (fmt, ambiguous) = infer_datetime_format(mixed, false).unwrap();
        assert!(!ambiguous);
        assert_eq!(fmt.parse(mixed[0]), Some(epoch(2023, 4, 1, 0, 0, 0)));
        assert_eq!(fmt.parse(mixed[1]), Some(epoch(2023, 4, 3, 0, 0, 0)));
    }

    #[test]
    fn ambiguous_dates_honor_dayfirst() {
        let samples = ["01/02/2020", "03/04/2020"];
        let (day, ambiguous) = infer_datetime_format(samples, true).unwrap();
        assert!(ambiguous);
        let (month, ambiguous) = infer_datetime_format(samples, false).unwrap();
        assert!(ambiguous);
        assert_eq!(day.parse(samples[0]), Some(epoch(2020, 2, 1, 0, 0, 0)));
        assert_eq!(month.parse(samples[0]), Some(epoch(2020, 1, 2, 0, 0, 0)));
    }

    #[test]
    fn offsets_compact_utc_and_fractional_seconds() {
        let utc = epoch(2023, 3, 18, 12, 34, 56) + 0.7;
        let samples = [
            "2023-03-18T14:34:56.700+02:00",
            "2023-03-18T07:04:56.700-05:30",
        ];
        let (fmt, ambiguous) = infer_datetime_format(samples, false).unwrap();
        assert!(!ambiguous);
        for sample in samples {
            close(fmt.parse(sample).unwrap(), utc);
        }
        close(fmt.parse("2023-03-18T12:34:56.700Z").unwrap(), utc);
        close(
            fmt.parse_parts("2023-03-18", "14:34:56.700+0200").unwrap(),
            utc,
        );
        let (compact, _) = infer_datetime_format(["20230318T123456.700Z"], false).unwrap();
        close(compact.parse("20230318T123456.700Z").unwrap(), utc);
        close(compact.parse_parts("20230318", "123456.700Z").unwrap(), utc);
        assert!(fmt.parse("2023-03-18T12:34:56+24:00").is_none());
        assert!(fmt.parse_parts("2023-03-18", "").is_none());
    }

    #[test]
    fn twelve_hour_clocks_and_offsets() {
        let samples = ["03/18/2023 12:34:56 PM", "03/19/2023 12:00:00 AM"];
        let (fmt, ambiguous) = infer_datetime_format(samples, false).unwrap();
        assert!(!ambiguous);
        assert_eq!(fmt.parse(samples[0]), Some(epoch(2023, 3, 18, 12, 34, 56)));
        assert_eq!(fmt.parse(samples[1]), Some(epoch(2023, 3, 19, 0, 0, 0)));
        assert_eq!(
            parse_datetime_str("03/18/2023 01:02:03 pm", false),
            Some(epoch(2023, 3, 18, 13, 2, 3))
        );
        close(
            fmt.parse_parts("03/18/2023", "1:02:03.25 PM +02:00")
                .unwrap(),
            epoch(2023, 3, 18, 11, 2, 3) + 0.25,
        );
        assert_eq!(
            fmt.parse_parts("03/18/2023", "12:30am+0130"),
            Some(epoch(2023, 3, 17, 23, 0, 0))
        );
        assert_eq!(
            fmt.parse_parts("03/18/2023", "23:34:56"),
            Some(epoch(2023, 3, 18, 23, 34, 56))
        );
        assert!(fmt.parse_parts("03/18/2023", "00:30 AM").is_none());
        assert!(fmt.parse_parts("03/18/2023", "13:30 PM").is_none());
        assert!(fmt.parse_parts("03/18/2023", "01:30 PM +24:00").is_none());
    }

    #[test]
    fn date_parts_validate_years_and_offsets_cross_midnight() {
        let (fmt, _) = infer_datetime_format(["02/29/24", "03/18/24"], true).unwrap();
        assert_eq!(fmt.parse("02/29/24"), Some(epoch(2024, 2, 29, 0, 0, 0)));
        assert!(fmt.parse("02/29/23").is_none());
        assert_eq!(fmt.parse("01/01/69"), Some(epoch(2069, 1, 1, 0, 0, 0)));
        assert_eq!(fmt.parse("01/01/70"), Some(epoch(1970, 1, 1, 0, 0, 0)));
        assert_eq!(
            fmt.parse_parts("01/01/70", "00:30:00+01:00"),
            Some(epoch(1969, 12, 31, 23, 30, 0))
        );
        close(
            fmt.parse_parts("12/31/69", "23:59:59.5-00:30").unwrap(),
            epoch(2070, 1, 1, 0, 29, 59) + 0.5,
        );
        assert!(fmt.parse_parts("02/29/24", "12:00:00+01:60").is_none());
    }

    #[test]
    fn old_date_forms_and_numeric_safety() {
        let expected = epoch(2023, 3, 18, 12, 34, 56);
        for sample in [
            "2023-03-18 12:34:56",
            "2023/03/18T12:34:56",
            "2023.03.18_12:34:56",
            "20230318 12:34:56",
            "18.3.2023 12:34:56",
            "03/18/2023 12:34:56",
            "18 Mar 2023 12:34:56",
            "18 March 2023 12:34:56",
            "Mar 18 2023 12:34:56",
            "March 18, 2023 12:34:56",
        ] {
            assert_eq!(
                parse_datetime_str(sample, false),
                Some(expected),
                "{sample}"
            );
        }
        assert_eq!(
            parse_datetime_str("20230318", false),
            Some(epoch(2023, 3, 18, 0, 0, 0))
        );
        for values in [["0.000", "0.001"], ["20230318", "20230319"]] {
            assert!(infer_datetime_format(values, false).is_none());
        }
    }

    #[test]
    fn table_conversion_uses_column_order_and_preserves_non_dates() {
        let df = DataFrame::from_columns(vec![
            Column::string(
                "Date",
                vec!["03/04/2023".into(), "13/04/2023".into(), "".into()],
            ),
            Column::string(
                "Elapsed",
                vec!["20230318".into(), "20230319".into(), "1".into()],
            ),
            Column::string(
                "Invalid",
                vec![" 03/04/2023 ".into(), "bad".into(), "".into()],
            ),
            Column::float("Elapsed [s]", vec![0.0, 0.1, 0.2]),
        ])
        .unwrap();
        let table = Table::from_df(df, "");
        let ColumnData::DateTime(dates) = &table.data.column(1).data else {
            panic!("date column was not converted");
        };
        assert_eq!(dates[0], epoch(2023, 4, 3, 0, 0, 0));
        assert_eq!(dates[1], epoch(2023, 4, 13, 0, 0, 0));
        assert!(dates[2].is_nan());
        assert!(matches!(&table.data.column(2).data, ColumnData::Str(_)));
        assert_eq!(
            table.data.column(3).data,
            ColumnData::Str(vec!["03/04/2023".into(), "bad".into(), "".into()])
        );
        assert_eq!(
            table.data.column(4).data,
            ColumnData::Float(vec![0.0, 0.1, 0.2])
        );
    }
}
