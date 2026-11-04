/// Numeric values are parsed from borrowed fields; uncommon localized numbers use
/// a stack buffer rather than allocating a string for every measurement.
#[derive(Debug)]
pub(super) enum Value<'a> {
    Number(f64, Option<&'a str>),
    Boolean(bool),
    Missing,
    Text,
}

pub(super) fn is_missing(s: &str) -> bool {
    let s = s.trim();
    s.is_empty()
        || s == "-"
        || s.eq_ignore_ascii_case("NA")
        || s.eq_ignore_ascii_case("N/A")
        || s.eq_ignore_ascii_case("[N/A]")
        || s.eq_ignore_ascii_case("NaN")
        || s.eq_ignore_ascii_case("NaT")
        || s.eq_ignore_ascii_case("NULL")
        || s.eq_ignore_ascii_case("None")
        || s.eq_ignore_ascii_case("<NA>")
}

fn number(s: &str, decimal_comma: bool) -> Option<f64> {
    if let Ok(value) = s.parse::<f64>() {
        return Some(value);
    }
    let (negative, hex) =
        if let Some(rest) = s.strip_prefix("-0x").or_else(|| s.strip_prefix("-0X")) {
            (true, Some(rest))
        } else {
            (false, s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")))
        };
    if let Some(hex) = hex {
        let value = u64::from_str_radix(hex, 16).ok()? as f64;
        return Some(if negative { -value } else { value });
    }
    if !s.bytes().any(|b| matches!(b, b',' | b'\'' | b' ' | b'\t')) {
        return None;
    }
    let comma = s.rfind(',');
    let dot = s.rfind('.');
    let comma_is_decimal = match (comma, dot) {
        (Some(c), Some(d)) => c > d,
        (Some(c), None) if decimal_comma => {
            let _ = c;
            true
        }
        (Some(c), None) => {
            let fractional = s[c + 1..].split(['e', 'E']).next().unwrap_or("");
            fractional.len() != 3
        }
        _ => false,
    };
    let mut buffer = [0u8; 128];
    let mut len = 0;
    for b in s.bytes() {
        let mapped = match b {
            b',' if comma_is_decimal => b'.',
            b',' | b'\'' | b' ' | b'\t' => continue,
            b'.' if comma_is_decimal && comma.is_some() => continue,
            _ => b,
        };
        if len == buffer.len() {
            return None;
        }
        buffer[len] = mapped;
        len += 1;
    }
    std::str::from_utf8(&buffer[..len]).ok()?.parse().ok()
}

fn split_unit(s: &str) -> Option<(&str, &str)> {
    let mut start = s.len();
    for (i, ch) in s.char_indices().rev() {
        if ch.is_alphabetic() || matches!(ch, '%' | '°' | 'µ' | 'μ' | '/' | '²' | '³') {
            start = i;
        } else {
            break;
        }
    }
    if start == 0 || start == s.len() {
        return None;
    }
    let numeric = s[..start].trim_end();
    let unit = s[start..].trim();
    if numeric.is_empty() || unit.is_empty() {
        None
    } else {
        Some((numeric, unit))
    }
}

pub(super) fn parse(s: &str, decimal_comma: bool) -> Value<'_> {
    let s = s.trim();
    if let Some(v) = number(s, decimal_comma) {
        return Value::Number(v, None);
    }
    if is_missing(s) {
        return Value::Missing;
    }
    if s.eq_ignore_ascii_case("yes")
        || s.eq_ignore_ascii_case("true")
        || s.eq_ignore_ascii_case("on")
    {
        return Value::Boolean(true);
    }
    if s.eq_ignore_ascii_case("no")
        || s.eq_ignore_ascii_case("false")
        || s.eq_ignore_ascii_case("off")
    {
        return Value::Boolean(false);
    }
    if let Some((body, unit)) = split_unit(s) {
        if let Some(value) = number(body, decimal_comma) {
            return Value::Number(value, Some(unit));
        }
    }
    Value::Text
}
