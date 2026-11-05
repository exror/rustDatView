//! Human-readable values and UTC-naive time labels used by the chart.

use std::fmt::Write as _;

use chrono::{DateTime, NaiveDate, NaiveDateTime};

const MAX_SIGNIFICANT_DIGITS: usize = 17;
const MAX_TIME_DECIMALS: usize = 18;

/// Format a value with significant digits, thousands separators, and an optional unit.
///
/// Recognized, unprefixed SI units use engineering prefixes. Existing prefixes and
/// non-SI units are kept as supplied; watts also stay in watts below one megawatt.
/// Extreme magnitudes use scientific notation instead of unwieldy digit strings.
pub fn format_value(value: f64, unit: &str, sig: usize) -> String {
    let unit = unit.trim();
    let sig = sig.clamp(1, MAX_SIGNIFICANT_DIGITS);
    if !value.is_finite() {
        let text = if value.is_nan() {
            "NaN"
        } else if value.is_sign_negative() {
            "-∞"
        } else {
            "∞"
        };
        return with_unit(text.to_owned(), unit);
    }
    if value == 0.0 {
        return with_unit("0".to_owned(), unit);
    }

    let magnitude = value.abs();
    let prefixable = match unit {
        "W" => magnitude >= 1e6,
        "V" | "A" | "Hz" | "s" | "J" | "Wh" | "g" | "m" | "Pa" | "N" | "F" | "H" | "Ω" | "B" => {
            true
        }
        _ => false,
    };
    let mut prefix_exponent = 0;
    let use_prefixes = prefixable && (1e-30..1e33).contains(&magnitude);
    if use_prefixes {
        let exponent = magnitude.log10().floor() as i32;
        prefix_exponent = (exponent.div_euclid(3) * 3).clamp(-30, 30);
    }

    let mut scaled = value / 10_f64.powi(prefix_exponent);
    if use_prefixes && prefix_exponent != 30 {
        // Rounding 999.95 mV to four digits should read 1 V, not 1,000 mV.
        let rollover = 1000.0 - 0.5 * 10_f64.powi(3 - sig as i32);
        if scaled.abs() >= rollover {
            prefix_exponent += 3;
            scaled /= 1000.0;
        }
    }

    let prefix = match prefix_exponent {
        -30 => "q",
        -27 => "r",
        -24 => "y",
        -21 => "z",
        -18 => "a",
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "µ",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "M",
        9 => "G",
        12 => "T",
        15 => "P",
        18 => "E",
        21 => "Z",
        24 => "Y",
        27 => "R",
        30 => "Q",
        _ => unreachable!("engineering exponents are multiples of three"),
    };
    let mut number = significant_number(scaled, sig);
    if !unit.is_empty() {
        number.reserve(1 + prefix.len() + unit.len());
        number.push(' ');
        number.push_str(prefix);
        number.push_str(unit);
    }
    number
}

fn with_unit(mut number: String, unit: &str) -> String {
    if !unit.is_empty() {
        number.reserve(1 + unit.len());
        number.push(' ');
        number.push_str(unit);
    }
    number
}

fn significant_number(value: f64, sig: usize) -> String {
    let magnitude = value.abs();
    if magnitude == 0.0 {
        return "0".to_owned();
    }
    if !(1e-6..1e15).contains(&magnitude) {
        let mut text = format!("{value:.precision$e}", precision = sig - 1);
        let exponent = text.find('e').expect("scientific format contains e");
        let end = trim_decimal(&text[..exponent]).len();
        text.replace_range(end..exponent, "");
        return text;
    }

    let exponent = magnitude.log10().floor() as i32;
    let decimal_places = sig as i32 - 1 - exponent;
    let rounded = if decimal_places < 0 {
        let place = 10_f64.powi(-decimal_places);
        (value / place).round() * place
    } else {
        value
    };
    let mut text = format!(
        "{rounded:.precision$}",
        precision = decimal_places.max(0) as usize
    );
    let end = trim_decimal(&text).len();
    text.truncate(end);
    group_thousands(text)
}

fn trim_decimal(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

fn group_thousands(text: String) -> String {
    let (sign, unsigned) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.as_str()),
    };
    let integer_len = unsigned.find('.').unwrap_or(unsigned.len());
    if integer_len <= 3 {
        return text;
    }
    let mut grouped = String::with_capacity(text.len() + integer_len / 3);
    grouped.push_str(sign);
    for (index, byte) in unsigned.bytes().enumerate() {
        if index > 0 && index < integer_len && (integer_len - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(char::from(byte));
    }
    grouped
}

/// Format epoch seconds as UTC-naive time, choosing fields from the tick step.
/// Subsecond labels retain enough fractional digits to represent the step.
pub fn format_time(epoch: f64, step: f64) -> String {
    if step.is_finite() && step > 0.0 && step < 1e-18 {
        return format!("{} s", format_value(epoch, "", MAX_SIGNIFICANT_DIGITS));
    }
    let decimals = time_decimals(step);
    let Some((date_time, fraction)) = timestamp_parts(epoch, decimals) else {
        return format!("{} s", format_value(epoch, "", MAX_SIGNIFICANT_DIGITS));
    };
    if step >= 365.0 * 86_400.0 {
        date_time.format("%Y").to_string()
    } else if step >= 28.0 * 86_400.0 {
        date_time.format("%Y-%m").to_string()
    } else if step >= 86_400.0 {
        date_time.format("%Y-%m-%d").to_string()
    } else if step >= 60.0 && step % 60.0 == 0.0 {
        date_time.format("%H:%M").to_string()
    } else {
        let mut base = date_time.format("%H:%M:%S").to_string();
        if decimals > 0 {
            write!(base, ".{fraction:0decimals$}").expect("writing to a String cannot fail");
        }
        base
    }
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DurationStyle {
    Seconds,
    Minutes,
    Hours,
}

pub(crate) fn duration_style(seconds: f64, step: f64) -> DurationStyle {
    let scale = 10_u64.pow(time_decimals(step) as u32) as f64;
    let rounded = (seconds.abs() * scale).round() / scale;
    if rounded >= 3600.0 {
        DurationStyle::Hours
    } else if rounded >= 60.0 {
        DurationStyle::Minutes
    } else {
        DurationStyle::Seconds
    }
}

/// Format signed elapsed time as seconds, minutes:seconds, or hours:minutes:seconds.
pub fn format_duration(seconds: f64, step: f64) -> String {
    format_duration_with_style(seconds, step, DurationStyle::Seconds)
}

/// A single elapsed-time readout (cursor, selection edges): under a minute in
/// SI seconds with `sig` significant digits (`12.35 s`, `480 µs`), longer
/// spans as minutes:seconds to the millisecond.
pub fn format_elapsed(seconds: f64, sig: usize) -> String {
    if seconds.abs() < 60.0 {
        format_value(seconds, "s", sig.max(4))
    } else {
        format_duration(seconds, 0.001)
    }
}

pub(crate) fn format_duration_with_style(
    seconds: f64,
    step: f64,
    minimum_style: DurationStyle,
) -> String {
    if !seconds.is_finite()
        || seconds.abs() >= u64::MAX as f64
        || (step.is_finite() && step > 0.0 && step < 1e-18)
    {
        return format!("{} s", format_value(seconds, "", MAX_SIGNIFICANT_DIGITS));
    }
    let decimals = time_decimals(step);
    let scale = 10_u64.pow(decimals as u32);
    let abs = seconds.abs();
    let mut whole = abs.floor() as u64;
    let mut fraction = ((abs - whole as f64) * scale as f64).round() as u64;
    if fraction >= scale {
        let Some(next) = whole.checked_add(1) else {
            return format!("{} s", format_value(seconds, "", MAX_SIGNIFICANT_DIGITS));
        };
        whole = next;
        fraction = 0;
    }
    let sign = if seconds.is_sign_negative() && (whole != 0 || fraction != 0) {
        "-"
    } else {
        ""
    };
    let hours = whole / 3600;
    let minutes = (whole / 60) % 60;
    let seconds = whole % 60;
    let style = if whole >= 3600 {
        DurationStyle::Hours
    } else if whole >= 60 {
        DurationStyle::Minutes
    } else {
        DurationStyle::Seconds
    }
    .max(minimum_style);
    let mut base = if style == DurationStyle::Hours {
        format!("{sign}{hours}:{minutes:02}:{seconds:02}")
    } else if style == DurationStyle::Minutes {
        format!("{sign}{minutes}:{seconds:02}")
    } else {
        format!("{sign}{seconds}")
    };
    if decimals > 0 {
        write!(base, ".{fraction:0decimals$}").expect("writing to a String cannot fail");
    }
    if style == DurationStyle::Seconds {
        base.push_str(" s");
    }
    base
}

// More than a logarithm is needed here: a quarter-second step needs two
// fractional digits even though its order of magnitude is only 10^-1.
fn time_decimals(step: f64) -> usize {
    if !step.is_finite() || step <= 0.0 {
        return 0;
    }
    let mut scaled = step;
    for decimals in 0..=MAX_TIME_DECIMALS {
        let tolerance = (scaled.abs() * 1e-12).clamp(1e-9, 1e-6);
        if scaled >= 1.0 && (scaled - scaled.round()).abs() <= tolerance {
            return decimals;
        }
        scaled *= 10.0;
    }
    MAX_TIME_DECIMALS
}

fn timestamp_parts(epoch: f64, decimals: usize) -> Option<(NaiveDateTime, u64)> {
    if !epoch.is_finite() {
        return None;
    }
    let whole = epoch.floor();
    if whole < i64::MIN as f64 || whole >= i64::MAX as f64 {
        return None;
    }
    let mut whole = whole as i64;
    let scale = 10_u64.pow(decimals as u32);
    let mut fraction = ((epoch - whole as f64) * scale as f64).round() as u64;
    if fraction >= scale {
        whole = whole.checked_add(1)?;
        fraction = 0;
    }
    let date_time = DateTime::from_timestamp(whole, 0)?.naive_utc();
    Some((date_time, fraction))
}

pub(crate) fn date_at(epoch: f64, step: f64) -> Option<NaiveDate> {
    timestamp_parts(epoch, time_decimals(step)).map(|(date_time, _)| date_time.date())
}

pub(crate) fn format_time_with_date(epoch: f64, step: f64) -> String {
    match date_at(epoch, step) {
        Some(date) => format!("{} {}", date.format("%Y-%m-%d"), format_time(epoch, step)),
        None => format_time(epoch, step),
    }
}

/// Read back what `format_time`, `format_duration` and `format_value(…, "s", …)`
/// write: `h:mm:ss.f`, `m:ss.f`, or a number of seconds with an optional
/// `s`, `ms`, `µs`/`us` or `ns` unit. Returns seconds.
pub fn parse_seconds(text: &str) -> Option<f64> {
    let text = text.trim().replace(',', "");
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, text.as_str()),
    };
    let (number, scale) = [("ns", 1e-9), ("µs", 1e-6), ("us", 1e-6), ("ms", 1e-3), ("s", 1.0)]
        .into_iter()
        .find_map(|(suffix, scale)| text.strip_suffix(suffix).map(|rest| (rest.trim_end(), scale)))
        .unwrap_or((text, 1.0));
    let parts: Vec<&str> = number.split(':').collect();
    if parts.len() > 3 || (parts.len() > 1 && scale != 1.0) {
        return None;
    }
    let mut total = 0.0;
    for (index, part) in parts.iter().enumerate() {
        let value: f64 = part.trim().parse().ok()?;
        // Minutes and seconds after a colon stay below 60.
        if !value.is_finite() || value < 0.0 || (index > 0 && value >= 60.0) {
            return None;
        }
        total = total * 60.0 + value;
    }
    let seconds = total * scale;
    Some(if negative { -seconds } else { seconds })
}

#[cfg(test)]
mod tests {
    use super::{format_duration, format_time, format_value, parse_seconds};

    #[test]
    fn formats_significant_values_and_preserves_existing_units() {
        assert_eq!(format_value(1234.56, "", 4), "1,235");
        assert_eq!(format_value(4812.0, "MHz", 4), "4,812 MHz");
        assert_eq!(format_value(-1234.5, "°C", 5), "-1,234.5 °C");
        assert_eq!(format_value(12_345.0, "W", 4), "12,350 W");
        assert_eq!(format_value(1_000_000.0, "W", 4), "1 MW");
        assert_eq!(format_value(0.0025, "V", 3), "2.5 mV");
        assert_eq!(format_value(0.99999, "V", 4), "1 V");
        assert_eq!(format_value(1e-300, "", 4), "1e-300");
        assert_eq!(format_value(-0.0, "W", 4), "0 W");
        assert_eq!(format_value(f64::INFINITY, "V", 4), "∞ V");
    }

    #[test]
    fn time_is_utc_naive_and_adapts_to_step() {
        assert_eq!(format_time(1_704_067_199.875, 0.125), "23:59:59.875");
        assert_eq!(format_time(1_704_067_200.0, 60.0), "00:00");
        assert_eq!(format_time(1_704_067_290.0, 90.0), "00:01:30");
        assert_eq!(format_time(1_704_067_200.0, 86_400.0), "2024-01-01");
        assert_eq!(format_time(1_704_067_200.0, 31.0 * 86_400.0), "2024-01");
        assert_eq!(format_time(-0.25, 0.25), "23:59:59.75");
    }

    #[test]
    fn duration_rounds_carries_and_preserves_negative_values() {
        assert_eq!(format_duration(-3723.456, 0.001), "-1:02:03.456");
        assert_eq!(format_duration(3599.6, 1.0), "1:00:00");
        assert_eq!(format_duration(0.25, 0.25), "0.25 s");
        assert_eq!(format_duration(-0.01, 1.0), "0 s");
        assert_eq!(format_duration(65.5, 0.1), "1:05.5");
    }

    #[test]
    fn parsed_seconds_round_trip_formatted_times() {
        assert_eq!(parse_seconds(&format_duration(-3723.456, 0.001)), Some(-3723.456));
        assert_eq!(parse_seconds(&format_duration(65.5, 0.1)), Some(65.5));
        let close = |text: &str, expected: f64| {
            parse_seconds(text).is_some_and(|value| (value - expected).abs() <= expected.abs() * 1e-12)
        };
        assert!(close(&format_value(0.0012, "s", 6), 0.0012));
        assert!(close("12 µs", 12e-6));
        assert_eq!(parse_seconds("1,234.5 s"), Some(1234.5));
        assert_eq!(parse_seconds("23:59:59.875"), Some(86_399.875));
        assert_eq!(parse_seconds("1:75"), None);
        assert_eq!(parse_seconds("1:05 ms"), None);
        assert_eq!(parse_seconds("soon"), None);
    }
}
