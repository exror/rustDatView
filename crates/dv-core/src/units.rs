//! Unit parsing, conversion, and presentation for telemetry channels.
//!
//! A parsed value is expressed in the unit's `symbol`, not necessarily its
//! family base. Convert it with [`UnitInfo::to_base_value`] before combining
//! values with different units. The bases are W, V, A, Hz, °C, %, rpm, B,
//! B/s, s, J, and fps. Bytes use decimal SI prefixes unless an `i` marks an
//! IEC binary prefix. Unknown units retain their spelling and are not assumed
//! to be mutually convertible.

use std::fmt::{self, Write};

/// A grouping of compatible telemetry units, rather than a complete SI type
/// system. In particular, rotational speed and frame rate have their own
/// families instead of being grouped with frequency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UnitFamily {
    Power,
    Voltage,
    Current,
    Frequency,
    Temperature,
    Percent,
    Rpm,
    Data,
    Throughput,
    Time,
    Energy,
    Charge,
    FrameRate,
    Boolean,
    Dimensionless,
    Other,
}

impl fmt::Display for UnitFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Power => "Power",
            Self::Voltage => "Voltage",
            Self::Current => "Current",
            Self::Frequency => "Clock",
            Self::Temperature => "Temperature",
            Self::Percent => "Load",
            Self::Rpm => "Fan speed",
            Self::Data => "Data",
            Self::Throughput => "Throughput",
            Self::Time => "Time",
            Self::Energy => "Energy",
            Self::Charge => "Charge",
            Self::FrameRate => "Frame rate",
            Self::Boolean => "Flags",
            Self::Dimensionless => "Other",
            Self::Other => "Other",
        })
    }
}

/// A normalized unit and its affine conversion to the family's base unit.
///
/// The conversion is `base = value * to_base + offset_to_base`. The offset
/// matters for absolute temperatures: K and °F are based on °C, not on zero.
/// Applying this conversion to a *temperature difference* is incorrect;
/// differences should use only the scale factor.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitInfo {
    pub family: UnitFamily,
    /// Canonical display symbol; unrecognized units retain their input text.
    pub symbol: String,
    /// Multiplicative factor from this unit to the family base.
    pub to_base: f64,
    /// Additive offset after multiplication, in the family base unit.
    pub offset_to_base: f64,
}

impl UnitInfo {
    fn new(family: UnitFamily, symbol: impl Into<String>, to_base: f64, offset: f64) -> Self {
        Self {
            family,
            symbol: symbol.into(),
            to_base,
            offset_to_base: offset,
        }
    }

    /// Converts a value in `self.symbol` into the family base unit.
    pub fn to_base_value(&self, value: f64) -> f64 {
        value * self.to_base + self.offset_to_base
    }

    /// Converts a value in the family base unit into `self.symbol`.
    pub fn from_base_value(&self, value: f64) -> f64 {
        (value - self.offset_to_base) / self.to_base
    }

    /// Converts an absolute value to another unit of the same family.
    /// Unknown units are convertible only when their retained symbols match.
    /// Returns `None` for different families or invalid target scales.
    pub fn convert_value(&self, value: f64, target: &UnitInfo) -> Option<f64> {
        if self.family != target.family
            || (self.family == UnitFamily::Other && self.symbol != target.symbol)
            || !target.to_base.is_finite()
            || target.to_base == 0.0
        {
            return None;
        }
        Some(target.from_base_value(self.to_base_value(value)))
    }
}

fn equal_any(input: &str, alternatives: &[&str]) -> bool {
    alternatives.iter().any(|s| input.eq_ignore_ascii_case(s))
}

// SI letter prefixes are case-sensitive: m is milli and M is mega. The
// uppercase K used by some exporters is accepted as the customary k.
fn si_prefix(prefix: &str) -> Option<(&'static str, f64)> {
    Some(match prefix {
        "" => ("", 1.0),
        "y" => ("y", 1e-24),
        "z" => ("z", 1e-21),
        "a" => ("a", 1e-18),
        "f" => ("f", 1e-15),
        "p" => ("p", 1e-12),
        "n" => ("n", 1e-9),
        "u" | "µ" | "μ" => ("µ", 1e-6),
        "m" => ("m", 1e-3),
        "k" | "K" => ("k", 1e3),
        "M" => ("M", 1e6),
        "G" => ("G", 1e9),
        "T" => ("T", 1e12),
        "P" => ("P", 1e15),
        "E" => ("E", 1e18),
        "Z" => ("Z", 1e21),
        "Y" => ("Y", 1e24),
        _ if prefix.eq_ignore_ascii_case("pico") => ("p", 1e-12),
        _ if prefix.eq_ignore_ascii_case("nano") => ("n", 1e-9),
        _ if prefix.eq_ignore_ascii_case("micro") => ("µ", 1e-6),
        _ if prefix.eq_ignore_ascii_case("milli") => ("m", 1e-3),
        _ if prefix.eq_ignore_ascii_case("kilo") => ("k", 1e3),
        _ if prefix.eq_ignore_ascii_case("mega") => ("M", 1e6),
        _ if prefix.eq_ignore_ascii_case("giga") => ("G", 1e9),
        _ if prefix.eq_ignore_ascii_case("tera") => ("T", 1e12),
        _ => return None,
    })
}

fn strip_suffix_ignore_case<'a>(input: &'a str, suffix: &str) -> Option<&'a str> {
    let split = input.len().checked_sub(suffix.len())?;
    let prefix = input.get(..split)?;
    let end = input.get(split..)?;
    end.eq_ignore_ascii_case(suffix).then_some(prefix)
}

fn scaled_unit(input: &str, aliases: &[&str], canonical: &str) -> Option<(String, f64)> {
    for alias in aliases {
        if let Some(prefix) = strip_suffix_ignore_case(input, alias) {
            // Case-fold long names, but do not turn e.g. Pa (pascal) into
            // PA (peta-ampere) by folding a prefixed one-letter symbol.
            if alias.len() == 1 && !prefix.is_empty() && !input.ends_with(*alias) {
                continue;
            }
            if let Some((prefix, factor)) = si_prefix(prefix) {
                return Some((format!("{prefix}{canonical}"), factor));
            }
        }
    }
    None
}

fn data_prefix(prefix: &str) -> Option<(&'static str, f64)> {
    Some(match prefix {
        "Ki" | "ki" => ("Ki", 1_024.0),
        "Mi" => ("Mi", 1_048_576.0),
        "Gi" => ("Gi", 1_073_741_824.0),
        "Ti" => ("Ti", 1_099_511_627_776.0),
        "Pi" => ("Pi", 1_125_899_906_842_624.0),
        _ => return si_prefix(prefix),
    })
}

// Returns a display symbol and its multiplier in bytes. Unlike the unit
// suffixes spelled out as words, abbreviated B and b must not be case-folded.
fn data_unit(input: &str) -> Option<(String, f64)> {
    for (suffix, symbol, byte_factor) in [
        ("bytes", "B", 1.0),
        ("byte", "B", 1.0),
        ("bits", "b", 0.125),
        ("bit", "b", 0.125),
    ] {
        if let Some(prefix) = strip_suffix_ignore_case(input, suffix) {
            if let Some((prefix, factor)) = data_prefix(prefix) {
                return Some((format!("{prefix}{symbol}"), factor * byte_factor));
            }
        }
    }
    let (prefix, symbol, byte_factor) = if let Some(prefix) = input.strip_suffix('B') {
        (prefix, "B", 1.0)
    } else {
        (input.strip_suffix('b')?, "b", 0.125)
    };
    let (prefix, factor) = data_prefix(prefix)?;
    Some((format!("{prefix}{symbol}"), factor * byte_factor))
}

fn throughput_unit(input: &str) -> Option<(String, f64)> {
    if let Some((numerator, denominator)) = input.rsplit_once('/') {
        if equal_any(denominator.trim(), &["s", "sec", "second", "seconds"]) {
            if let Some((symbol, factor)) = data_unit(numerator.trim()) {
                return Some((format!("{symbol}/s"), factor));
            }
        }
    }
    for suffix in [" per second", "ps"] {
        if let Some(numerator) = strip_suffix_ignore_case(input, suffix) {
            if let Some((symbol, factor)) = data_unit(numerator.trim()) {
                return Some((format!("{symbol}/s"), factor));
            }
        }
    }
    None
}

/// Parses a unit spelling, optionally enclosed in `[...]` or `(...)`.
///
/// Symbols are normalized while preserving scale (for example `uA` becomes
/// `µA` and `RPM` becomes `rpm`). Prefixes are case-sensitive: `mW` is a
/// milliwatt and `MW` a megawatt; `MB` is decimal megabytes and `MiB` is
/// binary mebibytes. `C` and `F` are interpreted as temperature aliases;
/// use a spelled-out name for an ambiguous physical unit such as coulombs.
/// `T` is retained as [`UnitFamily::Other`], not guessed to mean temperature
/// or dimensionless multiplication. Unknown and empty units are preserved as
/// `Other` and `Dimensionless` respectively.
pub fn parse_unit(input: &str) -> UnitInfo {
    use UnitFamily::*;

    let mut input = input.trim();
    if (input.starts_with('[') && input.ends_with(']'))
        || (input.starts_with('(') && input.ends_with(')'))
    {
        input = input[1..input.len() - 1].trim();
    }
    if input.is_empty() {
        return UnitInfo::new(Dimensionless, "", 1.0, 0.0);
    }
    if input == "%" || equal_any(input, &["percent", "percentage", "pct"]) {
        return UnitInfo::new(Percent, "%", 1.0, 0.0);
    }
    if equal_any(input, &["yes/no", "yes / no"]) {
        return UnitInfo::new(Boolean, "Yes/No", 1.0, 0.0);
    }
    if equal_any(input, &["on/off", "on / off"]) {
        return UnitInfo::new(Boolean, "On/Off", 1.0, 0.0);
    }
    if equal_any(input, &["true/false", "true / false"]) {
        return UnitInfo::new(Boolean, "True/False", 1.0, 0.0);
    }
    if input == "×" || equal_any(input, &["x", "multiplier"]) {
        return UnitInfo::new(Dimensionless, "x", 1.0, 0.0);
    }
    if input == "1" || input == "-" {
        return UnitInfo::new(Dimensionless, input, 1.0, 0.0);
    }
    if input == "℃"
        || input == "°C"
        || input == "°c"
        || input == "C"
        || equal_any(
            input,
            &[
                "degC",
                "degree C",
                "degrees C",
                "celsius",
                "degree celsius",
                "degrees celsius",
            ],
        )
    {
        return UnitInfo::new(Temperature, "°C", 1.0, 0.0);
    }
    if input == "K" || equal_any(input, &["kelvin", "degree kelvin", "degrees kelvin"]) {
        return UnitInfo::new(Temperature, "K", 1.0, -273.15);
    }
    if input == "℉"
        || input == "°F"
        || input == "°f"
        || input == "F"
        || equal_any(
            input,
            &[
                "degF",
                "degree F",
                "degrees F",
                "fahrenheit",
                "degree fahrenheit",
                "degrees fahrenheit",
            ],
        )
    {
        return UnitInfo::new(Temperature, "°F", 5.0 / 9.0, -32.0 * 5.0 / 9.0);
    }
    if equal_any(
        input,
        &["rpm", "r/min", "rev/min", "revolutions per minute"],
    ) {
        return UnitInfo::new(Rpm, "rpm", 1.0, 0.0);
    }
    if equal_any(input, &["fps", "frame/s", "frames/s", "frames per second"]) {
        return UnitInfo::new(FrameRate, "fps", 1.0, 0.0);
    }
    if let Some((symbol, factor)) = throughput_unit(input) {
        return UnitInfo::new(Throughput, symbol, factor, 0.0);
    }
    if let Some((symbol, factor)) = data_unit(input) {
        return UnitInfo::new(Data, symbol, factor, 0.0);
    }
    if equal_any(input, &["min", "minute", "minutes"]) {
        return UnitInfo::new(Time, "min", 60.0, 0.0);
    }
    if equal_any(input, &["h", "hr", "hour", "hours"]) {
        return UnitInfo::new(Time, "h", 3_600.0, 0.0);
    }
    if equal_any(input, &["d", "day", "days"]) {
        return UnitInfo::new(Time, "d", 86_400.0, 0.0);
    }
    for (family, aliases, canonical, multiplier) in [
        (Power, &["watts", "watt", "W"][..], "W", 1.0),
        (Voltage, &["volts", "volt", "V"][..], "V", 1.0),
        (
            Current,
            &["amperes", "ampere", "amps", "amp", "A"][..],
            "A",
            1.0,
        ),
        (Frequency, &["hertz", "Hz"][..], "Hz", 1.0),
        (
            Time,
            &["seconds", "second", "secs", "sec", "s"][..],
            "s",
            1.0,
        ),
        (Energy, &["joules", "joule", "J"][..], "J", 1.0),
        (
            Energy,
            &["watt-hours", "watt-hour", "watt hours", "watt hour", "Wh"][..],
            "Wh",
            3_600.0,
        ),
        // Coulombs are written A·s: a bare `C` is read as °C.
        (
            Charge,
            &["coulombs", "coulomb", "A·s", "A*s", "As"][..],
            "A·s",
            1.0,
        ),
        (Charge, &["amp-hours", "amp-hour", "Ah"][..], "Ah", 3_600.0),
    ] {
        if let Some((symbol, factor)) = scaled_unit(input, aliases, canonical) {
            return UnitInfo::new(family, symbol, factor * multiplier, 0.0);
        }
    }
    UnitInfo::new(Other, input, 1.0, 0.0)
}

// Parentheses often contain descriptions, so only split unknown expressions
// which visibly resemble physical unit notation, not arbitrary words or IDs.
fn looks_like_other_unit(input: &str) -> bool {
    if [
        "m", "mm", "cm", "km", "µm", "um", "g", "kg", "mg", "N", "kN", "Nm", "Pa", "kPa", "MPa",
        "bar", "dB", "dBA", "deg", "°", "rad", "sr", "Ω", "Ohm", "ohm", "lx", "lux", "lm", "T",
    ]
    .contains(&input)
    {
        return true;
    }
    let has_operator = input
        .chars()
        .any(|c| matches!(c, '/' | '*' | '·' | '⋅' | '^' | '²' | '³'));
    if !has_operator {
        return false;
    }
    let mut part_len = 0;
    let mut part_all_upper = true;
    for c in input.chars().chain(std::iter::once('/')) {
        if c.is_alphabetic() || matches!(c, 'µ' | 'μ' | 'Ω' | '°') {
            part_len += 1;
            part_all_upper &= c.is_ascii_uppercase();
        } else if c.is_ascii_digit()
            || c.is_whitespace()
            || matches!(c, '/' | '*' | '·' | '⋅' | '^' | '²' | '³' | '-' | '+')
        {
            if part_len > 3 || (part_len > 1 && part_all_upper) {
                return false;
            }
            part_len = 0;
            part_all_upper = true;
        } else {
            return false;
        }
    }
    true
}

/// Splits a channel label into `(name, original_unit_spelling)`.
///
/// Explicit square-bracket units may be unknown; numeric bracketed or
/// parenthesized indices remain part of the name. Descriptions are retained unless
/// they parse as units or look like a physical unit expression. Bare trailing
/// units are recognized conservatively: ambiguous one-letter tokens such as
/// `A`, `C`, `K`, `s`, and `x` are not removed. Logger-style suffixes after an
/// underscore (`Power_W`, `Time_s`, `Clock_MHz`) are units only when they parse
/// as one and cannot be read as part of the name (`Phase_A`, `Temp_min`).
/// Missing units yield `""`. Both returned strings are trimmed, and the unit
/// can be passed to [`parse_unit`] for normalization and conversion.
pub fn split_name_unit(input: &str) -> (String, String) {
    let input = input.trim();
    for (open, close) in [('[', ']'), ('(', ')')] {
        if input.ends_with(close) {
            if let Some(index) = input.rfind(open) {
                let name = input[..index].trim();
                let unit = input[index + open.len_utf8()..input.len() - close.len_utf8()].trim();
                let is_index = unit
                    .trim_start_matches('#')
                    .chars()
                    .all(|c| c.is_ascii_digit());
                let parsed = parse_unit(unit);
                if !name.is_empty()
                    && !unit.is_empty()
                    && !is_index
                    && (open == '['
                        || parsed.family != UnitFamily::Other
                        || looks_like_other_unit(unit))
                {
                    return (name.to_owned(), unit.to_owned());
                }
            }
        }
    }
    if let Some(index) = input.rfind(char::is_whitespace) {
        let name = input[..index].trim();
        let unit = input[index..].trim();
        let info = parse_unit(unit);
        let unambiguous = unit.chars().count() > 1 || unit == "W" || unit == "%";
        if !name.is_empty()
            && unambiguous
            && !matches!(info.family, UnitFamily::Other | UnitFamily::Dimensionless)
        {
            return (name.to_owned(), unit.to_owned());
        }
    }
    if let Some((name, unit)) = input.rsplit_once('_') {
        let name = name.trim();
        if !name.is_empty() && !unit.contains(char::is_whitespace) && underscore_unit(name, unit) {
            return (name.to_owned(), unit.to_owned());
        }
    }
    (input.to_owned(), String::new())
}

// Logger exports abbreviate units after an underscore. A single letter there
// is usually an identifier (Phase_A, Bank_B) and a spelled-out word usually
// part of the name (Temp_min, Time_Of_Day_Seconds); accept those only where
// the name says what they measure, or not at all. V and W are also the
// velocity components of wind and flow measurements (Wind_U, Wind_V, Wind_W).
fn underscore_unit(name: &str, unit: &str) -> bool {
    let info = parse_unit(unit);
    if matches!(info.family, UnitFamily::Other | UnitFamily::Dimensionless) {
        return false;
    }
    let name = name.to_ascii_lowercase();
    match unit {
        "J" | "s" => true,
        "V" | "W" => !["wind", "vel", "speed", "flow", "gust"]
            .iter()
            .any(|word| name.contains(word)),
        "C" | "F" | "K" => name.contains("temp"),
        "A" => name.contains("curr") || name.contains("amp"),
        _ if unit.chars().count() == 1 => false,
        _ => {
            let abbreviation = unit.chars().count() <= 4 || info.symbol.eq_ignore_ascii_case(unit);
            abbreviation
                && !["min", "day", "days"]
                    .iter()
                    .any(|word| unit.eq_ignore_ascii_case(word))
        }
    }
}

/// Whether a standalone token, such as a cell in a units row, reads as a unit:
/// a recognized unit or physical notation like `bar`, `deg`, `kPa` or
/// `L/min`. Ordinary words and column names do not.
pub fn is_unit_symbol(token: &str) -> bool {
    let token = token.trim();
    !token.is_empty()
        && (!matches!(
            parse_unit(token).family,
            UnitFamily::Other | UnitFamily::Dimensionless
        ) || looks_like_other_unit(token))
}

/// Returns the result unit and factor for integrating power over seconds.
///
/// Multiply the numerical integral in `y.symbol · s` by the returned factor
/// to obtain joules. `x_is_time_seconds` is true only when the actual x values
/// are seconds (including epoch seconds); callers must convert other time
/// scales themselves. Non-power signals return `None`, so callers can label
/// their general integral using the original y and x units instead.
pub fn integral_unit(y: &UnitInfo, x_is_time_seconds: bool) -> Option<(&'static str, f64)> {
    (x_is_time_seconds && y.family == UnitFamily::Power).then_some(("J", y.to_base))
}

/// Selects a readable energy unit for a quantity expressed in joules.
///
/// The absolute-value thresholds are 1,000 J, 3,600 J, 1,000,000 J and
/// 3,600,000 J, selecting J, kJ, Wh, MJ and kWh respectively. Sign is retained.
/// Non-finite values are returned unchanged with the label J.
pub fn scale_energy_joules(joules: f64) -> (f64, &'static str) {
    let magnitude = joules.abs();
    if !joules.is_finite() || magnitude < 1_000.0 {
        (joules, "J")
    } else if magnitude < 3_600.0 {
        (joules / 1_000.0, "kJ")
    } else if magnitude < 1_000_000.0 {
        (joules / 3_600.0, "Wh")
    } else if magnitude < 3_600_000.0 {
        (joules / 1_000_000.0, "MJ")
    } else {
        (joules / 3_600_000.0, "kWh")
    }
}

const SI_PREFIXES: [(&str, f64); 17] = [
    ("y", 1e-24),
    ("z", 1e-21),
    ("a", 1e-18),
    ("f", 1e-15),
    ("p", 1e-12),
    ("n", 1e-9),
    ("µ", 1e-6),
    ("m", 1e-3),
    ("", 1.0),
    ("k", 1e3),
    ("M", 1e6),
    ("G", 1e9),
    ("T", 1e12),
    ("P", 1e15),
    ("E", 1e18),
    ("Z", 1e21),
    ("Y", 1e24),
];

fn rounded_significant(value: f64, sig: usize) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let exponent = value.abs().log10().floor() as i32;
    let step = 10_f64.powi(exponent - sig as i32 + 1);
    if step == 0.0 || !step.is_finite() {
        value
    } else {
        let rounded = (value / step).round() * step;
        if rounded.is_finite() {
            rounded
        } else {
            value
        }
    }
}

fn format_number(value: f64, sig: usize) -> String {
    // Leave room for grouping and the unit so common readouts need one allocation.
    let mut number = String::with_capacity(32);
    if !value.is_finite() {
        write!(&mut number, "{value}").expect("writing to a String cannot fail");
        return number;
    }
    if value == 0.0 {
        number.push('0');
        return number;
    }
    let exponent = value.abs().log10().floor() as i32;
    if !(-6..16).contains(&exponent) {
        write!(&mut number, "{:.*e}", sig - 1, value).expect("writing to a String cannot fail");
        let end = number
            .find('e')
            .expect("scientific formatting contains an exponent");
        if let Some(dot) = number[..end].find('.') {
            let mut trimmed = end;
            while number.as_bytes()[trimmed - 1] == b'0' {
                trimmed -= 1;
            }
            if trimmed == dot + 1 {
                trimmed = dot;
            }
            number.replace_range(trimmed..end, "");
        }
        return number;
    }
    let decimals = (sig as i32 - exponent - 1).max(0) as usize;
    write!(
        &mut number,
        "{:.*}",
        decimals,
        rounded_significant(value, sig)
    )
    .expect("writing to a String cannot fail");
    if number.contains('.') {
        while number.ends_with('0') {
            number.pop();
        }
        if number.ends_with('.') {
            number.pop();
        }
    }
    let start = if number.starts_with('-') { 1 } else { 0 };
    let mut index = number.find('.').unwrap_or(number.len());
    while index - start > 3 {
        index -= 3;
        number.insert(index, ',');
    }
    number
}

fn with_unit(value: f64, symbol: &str, sig: usize) -> String {
    let mut output = format_number(value, sig);
    if !symbol.is_empty() {
        output.push(' ');
        output.push_str(symbol);
    }
    output
}

/// Formats a value expressed in `unit.symbol` with at most `sig` significant
/// digits (clamped to 1–17). SI prefixes are selected for power, voltage,
/// current, frequency, data, throughput, time and energy. Data in bits remains
/// in bits, and seconds may be displayed as minutes or hours. Absolute
/// temperatures, percentages, flags, dimensionless values, and unknown units
/// retain their original scale; in particular, formatting never changes °F or
/// K into °C. Large fixed-point numbers use comma grouping. Non-finite values
/// retain their original unit and Rust's `NaN`/`inf` spelling.
pub fn format_quantity(value: f64, unit: &UnitInfo, sig: usize) -> String {
    use UnitFamily::*;
    let sig = sig.clamp(1, 17);
    let base_symbol = match unit.family {
        Power => "W",
        Voltage => "V",
        Current => "A",
        Frequency => "Hz",
        Data if unit.symbol.ends_with('b') => "b",
        Data => "B",
        Throughput if unit.symbol.ends_with("b/s") => "b/s",
        Throughput => "B/s",
        Time => "s",
        Energy => "J",
        _ => return with_unit(value, &unit.symbol, sig),
    };
    if !value.is_finite() {
        return with_unit(value, &unit.symbol, sig);
    }
    let mut base = unit.to_base_value(value);
    if base_symbol == "b" || base_symbol == "b/s" {
        base *= 8.0;
    }
    // Keep the original scale when conversion would overflow or underflow.
    if !base.is_finite() || (value != 0.0 && base == 0.0) {
        return with_unit(value, &unit.symbol, sig);
    }
    if unit.family == Time && rounded_significant(base.abs(), sig) >= 60.0 {
        let minutes = base / 60.0;
        if rounded_significant(minutes.abs(), sig) >= 60.0 {
            return with_unit(base / 3_600.0, "h", sig);
        }
        return with_unit(minutes, "min", sig);
    }
    let magnitude = base.abs();
    let mut index = if magnitude == 0.0 {
        8
    } else {
        SI_PREFIXES
            .iter()
            .rposition(|(_, scale)| magnitude >= *scale)
            .unwrap_or(0)
    };
    if unit.family == Time {
        index = index.min(8);
    }
    let mut scaled = base / SI_PREFIXES[index].1;
    if index + 1 < SI_PREFIXES.len()
        && !(unit.family == Time && index == 8)
        && rounded_significant(scaled.abs(), sig) >= 1_000.0
    {
        index += 1;
        scaled = base / SI_PREFIXES[index].1;
    }
    let mut output = format_number(scaled, sig);
    output.push(' ');
    output.push_str(SI_PREFIXES[index].0);
    output.push_str(base_symbol);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9 * expected.abs().max(1.0),
            "{actual} != {expected}"
        );
    }

    #[test]
    fn parses_exporter_units_and_distinguishes_prefixes() {
        for (input, family, symbol, factor) in [
            ("[W]", UnitFamily::Power, "W", 1.0),
            ("mW", UnitFamily::Power, "mW", 0.001),
            ("MW", UnitFamily::Power, "MW", 1e6),
            ("kW", UnitFamily::Power, "kW", 1e3),
            ("(mV)", UnitFamily::Voltage, "mV", 0.001),
            ("mA", UnitFamily::Current, "mA", 0.001),
            ("uA", UnitFamily::Current, "µA", 1e-6),
            ("μA", UnitFamily::Current, "µA", 1e-6),
            ("GHz", UnitFamily::Frequency, "GHz", 1e9),
            ("[MHz]", UnitFamily::Frequency, "MHz", 1e6),
            ("mhz", UnitFamily::Frequency, "mHz", 0.001),
            ("[RPM]", UnitFamily::Rpm, "rpm", 1.0),
            ("[FPS]", UnitFamily::FrameRate, "fps", 1.0),
            ("[Yes/No]", UnitFamily::Boolean, "Yes/No", 1.0),
            ("[x]", UnitFamily::Dimensionless, "x", 1.0),
            ("ms", UnitFamily::Time, "ms", 0.001),
            ("second", UnitFamily::Time, "s", 1.0),
            ("[MB]", UnitFamily::Data, "MB", 1e6),
            ("GB", UnitFamily::Data, "GB", 1e9),
            ("MiB", UnitFamily::Data, "MiB", 1_048_576.0),
            ("MB/s", UnitFamily::Throughput, "MB/s", 1e6),
            ("MiB / sec", UnitFamily::Throughput, "MiB/s", 1_048_576.0),
            ("Mbps", UnitFamily::Throughput, "Mb/s", 125_000.0),
            ("kWh", UnitFamily::Energy, "kWh", 3_600_000.0),
            ("[A·s]", UnitFamily::Charge, "A·s", 1.0),
            ("mAh", UnitFamily::Charge, "mAh", 3.6),
        ] {
            let parsed = parse_unit(input);
            assert_eq!(
                (parsed.family, parsed.symbol.as_str()),
                (family, symbol),
                "{input}"
            );
            close(parsed.to_base, factor);
            close(parsed.offset_to_base, 0.0);
        }
        assert_eq!(parse_unit("[T]").family, UnitFamily::Other);
        assert_eq!(parse_unit("[T]").symbol, "T");
        assert_eq!(parse_unit("Pa").family, UnitFamily::Other);
        assert_eq!(parse_unit("PA").family, UnitFamily::Current);
        assert_eq!(parse_unit("[widgets]").symbol, "widgets");
        assert_eq!(parse_unit("").family, UnitFamily::Dimensionless);
    }

    #[test]
    fn converts_absolute_temperature_and_rejects_unrelated_units() {
        for spelling in ["C", "degC", "°C", "℃", "celsius"] {
            assert_eq!(parse_unit(spelling).symbol, "°C");
        }
        for spelling in ["F", "degF", "°F", "℉", "fahrenheit"] {
            assert_eq!(parse_unit(spelling).symbol, "°F");
        }
        let c = parse_unit("°C");
        let f = parse_unit("°F");
        let k = parse_unit("K");
        close(f.to_base_value(32.0), 0.0);
        close(f.to_base_value(212.0), 100.0);
        close(k.to_base_value(273.15), 0.0);
        close(c.convert_value(100.0, &f).unwrap(), 212.0);
        close(f.convert_value(-40.0, &c).unwrap(), -40.0);
        close(c.convert_value(0.0, &k).unwrap(), 273.15);
        assert_eq!(c.convert_value(1.0, &parse_unit("W")), None);
        assert_eq!(parse_unit("T").convert_value(1.0, &parse_unit("psi")), None);
        assert_eq!(
            parse_unit("T").convert_value(2.0, &parse_unit("T")),
            Some(2.0)
        );
    }

    #[test]
    fn splits_labels_without_eating_descriptions_or_indices() {
        for (input, name, unit) in [
            ("CPU Package Power [W]", "CPU Package Power", "W"),
            ("GPU Core Clock [MHz]", "GPU Core Clock", "MHz"),
            ("Time (s)", "Time", "s"),
            ("Speed (m/s)", "Speed", "m/s"),
            ("Flow [custom units]", "Flow", "custom units"),
            ("Package Power W", "Package Power", "W"),
            ("GPU Clock GHz", "GPU Clock", "GHz"),
            ("CPU A", "CPU A", ""),
            ("Core [#0]", "Core [#0]", ""),
            ("Core (1)", "Core (1)", ""),
            ("Pressure (Pa)", "Pressure", "Pa"),
            (
                "CPU Temperature (Tctl/Tdie)",
                "CPU Temperature (Tctl/Tdie)",
                "",
            ),
            ("CPU utilization (total)", "CPU utilization (total)", ""),
            ("Processor (CPU/GPU)", "Processor (CPU/GPU)", ""),
            ("Status", "Status", ""),
            ("Power_W", "Power", "W"),
            ("Time_s", "Time", "s"),
            ("Clock_MHz", "Clock", "MHz"),
            ("Fan_RPM", "Fan", "RPM"),
            ("Load_pct", "Load", "pct"),
            ("timestamp_ms", "timestamp", "ms"),
            ("CPU_Temp_C", "CPU_Temp", "C"),
            ("Core_Current_A", "Core_Current", "A"),
            ("Phase_A", "Phase_A", ""),
            ("Bank_C", "Bank_C", ""),
            ("Temp_min", "Temp_min", ""),
            ("Time_of_day", "Time_of_day", ""),
            ("Volt_max", "Volt_max", ""),
            ("B1N1_Fx", "B1N1_Fx", ""),
            ("Time_Of_Day_Seconds", "Time_Of_Day_Seconds", ""),
            ("CPU Package Power_W", "CPU Package Power", "W"),
            ("VDD_V", "VDD", "V"),
            ("CH1_V", "CH1", "V"),
            ("GPU_W", "GPU", "W"),
            ("VDD_mA", "VDD", "mA"),
            ("VDD_I", "VDD_I", ""),
            ("Wind_V", "Wind_V", ""),
            ("Wind_W", "Wind_W", ""),
            ("Vel_W", "Vel_W", ""),
        ] {
            assert_eq!(
                split_name_unit(input),
                (name.to_owned(), unit.to_owned()),
                "{input}"
            );
        }
    }

    #[test]
    fn power_integrals_and_energy_display_are_correct_at_boundaries() {
        assert_eq!(integral_unit(&parse_unit("mW"), true), Some(("J", 0.001)));
        assert_eq!(integral_unit(&parse_unit("kW"), true), Some(("J", 1_000.0)));
        assert_eq!(integral_unit(&parse_unit("W"), false), None);
        assert_eq!(integral_unit(&parse_unit("A"), true), None);
        for (joules, value, symbol) in [
            (999.0, 999.0, "J"),
            (1_000.0, 1.0, "kJ"),
            (3_599.0, 3.599, "kJ"),
            (3_600.0, 1.0, "Wh"),
            (1_000_000.0, 1.0, "MJ"),
            (3_600_000.0, 1.0, "kWh"),
            (-3_600_000.0, -1.0, "kWh"),
        ] {
            let (actual, actual_symbol) = scale_energy_joules(joules);
            close(actual, value);
            assert_eq!(actual_symbol, symbol);
        }
        assert_eq!(scale_energy_joules(f64::INFINITY), (f64::INFINITY, "J"));
    }

    #[test]
    fn formatting_scales_values_without_changing_temperature() {
        assert_eq!(format_quantity(1_250.0, &parse_unit("W"), 4), "1.25 kW");
        assert_eq!(format_quantity(250.0, &parse_unit("mW"), 4), "250 mW");
        assert_eq!(format_quantity(999.9, &parse_unit("W"), 3), "1 kW");
        assert_eq!(format_quantity(1_500.0, &parse_unit("MHz"), 4), "1.5 GHz");
        assert_eq!(format_quantity(1.0, &parse_unit("MiB"), 4), "1.049 MB");
        assert_eq!(format_quantity(1.0, &parse_unit("Mbps"), 4), "1 Mb/s");
        assert_eq!(format_quantity(1_500.0, &parse_unit("ms"), 4), "1.5 s");
        assert_eq!(format_quantity(3_600.0, &parse_unit("s"), 4), "1 h");
        assert_eq!(format_quantity(32.0, &parse_unit("°F"), 4), "32 °F");
        assert_eq!(format_quantity(273.15, &parse_unit("K"), 5), "273.15 K");
        assert_eq!(format_quantity(12_345.0, &parse_unit("T"), 3), "12,300 T");
        assert_eq!(format_quantity(f64::NAN, &parse_unit("mW"), 4), "NaN mW");
        assert_eq!(format_quantity(2.0, &parse_unit(""), 4), "2");
    }
}
