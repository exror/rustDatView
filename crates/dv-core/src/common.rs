//! Port of `pydatview/common.py` (non-GUI helpers) plus the linear
//! interpolation primitive used by table merging.

use std::collections::{BTreeMap, HashSet};

use regex::Regex;

// ---------------------------------------------------------------------------
// Ellude
// ---------------------------------------------------------------------------

/// Longest common prefix of `strings`.
pub fn common_start(strings: &[String]) -> String {
    let Some(first) = strings.first() else {
        return String::new();
    };
    let mut out = String::new();
    let mut iters: Vec<std::str::Chars> = strings.iter().map(|s| s.chars()).collect();
    for c0 in first.chars() {
        if iters.iter_mut().all(|it| it.next() == Some(c0)) {
            out.push(c0);
        } else {
            break;
        }
    }
    out
}

/// Longest common suffix of `strings`.
pub fn common_end(strings: &[String]) -> String {
    let rev: Vec<String> = strings.iter().map(|s| s.chars().rev().collect()).collect();
    common_start(&rev).chars().rev().collect()
}

/// Truncate `s` before the last `.`, `_` or `|`; unchanged if none.
pub fn find_leftstop(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().rev().enumerate() {
        if matches!(c, '.' | '_' | '|') {
            return chars[..chars.len() - (i + 1)].iter().collect();
        }
    }
    s.to_string()
}

fn char_slice(s: &[char], start: usize, end_from_back: usize) -> String {
    let end = s.len().saturating_sub(end_from_back);
    if start >= end {
        String::new()
    } else {
        s[start..end].iter().collect()
    }
}

/// Remove the parts common to all `strings` (port of `ellude_common`).
///
/// Strings starting with `>` are "protected": they are returned without the
/// `>` and are ignored when computing the common parts.
pub fn ellude_common(strings: &[String], min_length: usize) -> Vec<String> {
    let s_list: Vec<&String> = strings
        .iter()
        .filter(|s| !s.is_empty() && !s.starts_with('>'))
        .collect();
    let (mut ns, mut ne) = (0usize, 0usize);
    if s_list.len() == 1 {
        let s = s_list[0];
        ns = match s.rfind('|') {
            Some(b) => s[..b].chars().count() + 1,
            None => 0,
        };
        ne = 0;
    } else if s_list.len() > 1 {
        let owned: Vec<String> = s_list.iter().map(|s| (*s).clone()).collect();
        let mut ss: Vec<char> = common_start(&owned).chars().collect();
        let mut se: Vec<char> = common_end(&owned).chars().collect();
        let rfind = |v: &[char], c: char| -> Option<usize> {
            let n = v.len().saturating_sub(1);
            v[..n].iter().rposition(|&x| x == c)
        };
        let find = |v: &[char], c: char| -> Option<usize> {
            let n = v.len().saturating_sub(1);
            v[..n].iter().position(|&x| x == c)
        };
        if let Some(iu) = rfind(&ss, '_') {
            if iu > 0 {
                ss.truncate(iu + 1);
            }
        }
        if let Some(iu) = find(&se, '_') {
            if iu > 0 {
                se = se[iu..].to_vec();
            }
        }
        if let Some(iu) = find(&se, '.') {
            if iu > 0 {
                se = se[iu..].to_vec();
            }
        }
        ns = ss.len();
        ne = se.len();
        // Reduce start length if some strings end up too short.
        let current_min = s_list
            .iter()
            .map(|s| {
                let c: Vec<char> = s.chars().collect();
                char_slice(&c, ns, ne)
                    .trim_start_matches('_')
                    .chars()
                    .count()
            })
            .min()
            .unwrap_or(0);
        if current_min < min_length {
            let delta = min_length - current_min;
            let keep = ss.len().saturating_sub(delta);
            ss.truncate(keep);
            ns = ss.len();
            let ss_str: String = ss.iter().collect();
            let stopped = find_leftstop(&ss_str);
            let nstop = stopped.chars().count();
            ns = if nstop == ns { 0 } else { nstop + 1 };
        }
    }
    strings
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if let Some(rest) = s.strip_prefix('>') {
                rest.to_string()
            } else {
                let c: Vec<char> = s.chars().collect();
                let r = char_slice(&c, ns, ne).trim_start_matches('_').to_string();
                if r.is_empty() {
                    format!("tab{i}")
                } else {
                    r
                }
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Key value
// ---------------------------------------------------------------------------

fn parse_py_float(s: &str) -> Option<f64> {
    match s {
        "inf" | "+inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        "nan" => Some(f64::NAN),
        _ => s.parse().ok(),
    }
}

/// Parse `all=(0.1,-2),b=(inf,0)` into a map of tuples.
pub fn extract_key_tuples(text: &str) -> BTreeMap<String, (f64, f64)> {
    let re = Regex::new(
        r"(?P<key>[\w\-]+)=\((?P<v1>[0-9+epinf.\-]*?),(?P<v2>[0-9+epinf.\-]*?)\)(?:$|,)",
    )
    .unwrap();
    let t = text.replace(' ', "");
    re.captures_iter(&t)
        .filter_map(|c| {
            Some((
                c["key"].to_string(),
                (parse_py_float(&c["v1"])?, parse_py_float(&c["v2"])?),
            ))
        })
        .collect()
}

/// Parse `all=0.1, b=inf, c=-0.3e+10` into a map.
pub fn extract_key_num(text: &str) -> BTreeMap<String, f64> {
    let re = Regex::new(r"(?P<key>[\w\-]+)=(?P<v>[0-9+epinf.\-]*?)(?:$|,)").unwrap();
    let t = text.replace(' ', "");
    re.captures_iter(&t)
        .filter_map(|c| Some((c["key"].to_string(), parse_py_float(&c["v"])?)))
        .collect()
}

/// Sampling step of `x` (first difference, falling back to the last one
/// when the first is NaN). NaN when fewer than two values.
pub fn get_dt(x: &[f64]) -> f64 {
    if x.len() <= 1 {
        return f64::NAN;
    }
    let dx = x[1] - x[0];
    if dx.is_nan() {
        let n = x.len();
        return x[n - 1] - x[n - 2];
    }
    dx
}

// ---------------------------------------------------------------------------
// Common columns
// ---------------------------------------------------------------------------

/// Result of [`get_common_col_indices`], one entry per column list.
#[derive(Debug, Clone, PartialEq)]
pub struct CommonCols {
    pub keep: Vec<Vec<usize>>,
    pub miss: Vec<Vec<usize>>,
    pub dupl: Vec<Vec<usize>>,
    pub n_cols: Vec<usize>,
}

/// Indices of columns common to all lists (compared with [`clean_col`]).
pub fn get_common_col_indices(col_lists: &[Vec<String>]) -> CommonCols {
    let cleaned: Vec<Vec<String>> = col_lists
        .iter()
        .map(|l| l.iter().map(|s| clean_col(s)).collect())
        .collect();
    let n_cols = cleaned.iter().map(|c| c.len()).collect();
    let mut common: Vec<String> = cleaned.first().cloned().unwrap_or_default();
    for other in cleaned.iter().skip(1) {
        let set: HashSet<&String> = other.iter().collect();
        common.retain(|c| set.contains(c));
    }
    let mut out = CommonCols {
        keep: vec![],
        miss: vec![],
        dupl: vec![],
        n_cols,
    };
    for cols in &cleaned {
        let mut keep = Vec::new();
        let mut dupl = Vec::new();
        for com in &common {
            let idx: Vec<usize> = cols
                .iter()
                .enumerate()
                .filter(|(_, c)| *c == com)
                .map(|(i, _)| i)
                .collect();
            if let Some(&i0) = idx.first() {
                if !keep.contains(&i0) {
                    keep.push(i0);
                    dupl.extend_from_slice(&idx[1..]);
                }
            }
        }
        let miss = (0..cols.len())
            .filter(|i| !keep.contains(i) && !dupl.contains(i))
            .collect();
        out.keep.push(keep);
        out.miss.push(miss);
        out.dupl.push(dupl);
    }
    out
}

// ---------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------

/// Normalised column name used to match columns across tables.
pub fn clean_col(s: &str) -> String {
    let s = no_unit(s);
    let s = no_unit(&s.trim().replace('(', " [").replace(')', "]"));
    s.to_lowercase().trim().replace(['_', ' ', '-'], "")
}

/// Byte index of the last `[` if it is not the first character.
fn unit_split_index(s: &str) -> Option<usize> {
    match s.rfind('[') {
        Some(i) if i > 0 => Some(i),
        _ => None,
    }
}

/// Column name without its unit: `"Speed [m/s]"` → `"Speed"`.
pub fn no_unit(s: &str) -> String {
    let s = s.replace('(', " [").replace(')', "]").replace("_[", " [");
    match unit_split_index(&s) {
        Some(iu) => s[..iu].trim().to_string(),
        None => s,
    }
}

/// Unit of a column name: `"Speed [m/s]"` → `"m/s"`.
pub fn unit(s: &str) -> String {
    let s = s.replace('(', " [").replace(')', "]");
    match unit_split_index(&s) {
        Some(iu) => s[iu + 1..].replace(']', ""),
        None => String::new(),
    }
}

/// `(name, unit)` of a column name.
pub fn splitunit(s: &str) -> (String, String) {
    let s = s.replace('(', " [").replace(')', "]");
    match unit_split_index(&s) {
        Some(iu) => (s[..iu].trim().to_string(), s[iu + 1..].replace(']', "")),
        None => (s, String::new()),
    }
}

/// Inverse of the unit of a column name (used for FFT frequency labels).
pub fn inverse_unit(s: &str) -> String {
    let u = unit(s);
    let u = u.trim();
    if u.is_empty() {
        String::new()
    } else if u == "-" {
        "-".into()
    } else if u.chars().count() == 1 {
        format!("1/{u}")
    } else if u == "m/s" {
        "s/m".into()
    } else if u == "deg" {
        "1/deg".into()
    } else {
        format!("1/({u})")
    }
}

/// Simple substring filtering; case-insensitive when `pattern` is lowercase.
/// Returns matching indices.
pub fn filter_list(list: &[String], pattern: &str) -> Vec<usize> {
    let ignore_case = pattern == pattern.to_lowercase();
    list.iter()
        .enumerate()
        .filter(|(_, s)| {
            if ignore_case {
                s.to_lowercase().contains(pattern)
            } else {
                s.contains(pattern)
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// Unique values, keeping first-appearance order.
pub fn unique<T: Clone + Eq + std::hash::Hash>(l: &[T]) -> Vec<T> {
    let mut seen = HashSet::new();
    l.iter()
        .filter(|x| seen.insert((*x).clone()))
        .cloned()
        .collect()
}

/// True if two axis-aligned rectangles (bottom-left, top-right) overlap.
#[allow(clippy::too_many_arguments)]
pub fn rectangle_overlap(
    blx1: f64,
    bly1: f64,
    trx1: f64,
    try1: f64,
    blx2: f64,
    bly2: f64,
    trx2: f64,
    try2: f64,
) -> bool {
    !(trx1 < blx2 || blx1 > trx2 || try1 < bly2 || bly1 > try2)
}

// ---------------------------------------------------------------------------
// Pretty printing
// ---------------------------------------------------------------------------

/// Python-style scientific notation (`1.234e+05`).
pub fn fmt_e(x: f64, prec: usize) -> String {
    if !x.is_finite() {
        return if x.is_nan() {
            "nan".into()
        } else if x > 0.0 {
            "inf".into()
        } else {
            "-inf".into()
        };
    }
    let s = format!("{:.*e}", prec, x);
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    format!("{mant}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

/// 6-character time representation of a duration in seconds.
pub fn pretty_time(t: f64) -> String {
    if t.is_nan() {
        return "NaT".into();
    }
    if t < 0.0 {
        "------".into()
    } else if t < 1.0 {
        format!("{:2}.{:02}s", 0, (t * 100.0).floor() as i64)
    } else if t < 60.0 {
        let s = t.floor();
        format!("{:2}.{:02}s", s as i64, ((t - s) * 100.0).floor() as i64)
    } else if t < 3600.0 {
        format!(
            "{:2}m{:02}s",
            (t / 60.0).floor() as i64,
            (t.floor() as i64) % 60
        )
    } else if t < 86400.0 {
        format!(
            "{:2}h{:02}m",
            (t / 3600.0).floor() as i64,
            ((t.floor() as i64) % 3600) / 60
        )
    } else if t < 8_553_600.0 {
        format!(
            "{:2}d{:02}h",
            (t / 86400.0).floor() as i64,
            ((t.floor() as i64) % 86400) / 3600
        )
    } else if t < 31_536_000.0 {
        format!("{:4.1}mo", t / (3600.0 * 24.0 * 30.5))
    } else {
        format!("{:.1}y", t / (3600.0 * 24.0 * 365.25))
    }
}

/// Number formatting used in the info panel.
pub fn pretty_num(x: f64) -> String {
    if x.is_nan() {
        return "NA".into();
    }
    if x.abs() < 1000.0 && x.abs() > 1e-4 {
        format!("{x:9.4}")
    } else {
        fmt_e(x, 3)
    }
}

/// Short number formatting with `digits` decimals (2, 3 or 4).
pub fn pretty_num_short(x: f64, digits: usize) -> String {
    if x.abs() < 1000.0 && x.abs() > 1e-1 {
        format!("{x:.digits$}")
    } else {
        fmt_e(x, digits)
    }
}

/// True if `s` contains CJK characters.
pub fn has_chinese_char(s: &str) -> bool {
    const RANGES: &[(u32, u32)] = &[
        (0x4E00, 0x62FF),
        (0x6300, 0x77FF),
        (0x7800, 0x8CFF),
        (0x8D00, 0x9FCC),
        (0x3400, 0x4DB5),
        (0x20000, 0x215FF),
        (0x21600, 0x230FF),
        (0x23100, 0x245FF),
        (0x24600, 0x260FF),
        (0x26100, 0x275FF),
        (0x27600, 0x290FF),
        (0x29100, 0x2A6DF),
        (0x2A700, 0x2B734),
        (0x2B740, 0x2B81D),
        (0x2B820, 0x2CEAF),
        (0x2CEB0, 0x2EBEF),
        (0x2F800, 0x2FA1F),
    ];
    s.chars().any(|c| {
        let c = c as u32;
        RANGES.iter().any(|&(lo, hi)| c >= lo && c <= hi)
    })
}

// ---------------------------------------------------------------------------
// Interpolation
// ---------------------------------------------------------------------------

/// Extrapolation behaviour of [`multi_interp`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Extrap {
    /// Use the first/last value outside of the range of `xp`.
    #[default]
    Bounded,
    /// NaN outside of the range of `xp`.
    Nan,
}

/// Linear interpolation of `fp` (values at sorted `xp`) at the points `x`
/// (port of `signal_analysis.multiInterp` for a single row).
pub fn multi_interp(x: &[f64], xp: &[f64], fp: &[f64], extrap: Extrap) -> Vec<f64> {
    assert_eq!(xp.len(), fp.len(), "xp and fp must have the same length");
    let n = xp.len();
    if n == 0 {
        return vec![f64::NAN; x.len()];
    }
    let (xmin, xmax) = xp
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    x.iter()
        .map(|&xi| {
            // numpy searchsorted(xp, xi, 'left') - 1
            let j = xp.partition_point(|&v| v < xi) as isize - 1;
            let (jb, ja, mut dd) = if j < 0 {
                (0, 0, 0.0)
            } else if j as usize >= n - 1 {
                (n - 1, n - 1, 0.0)
            } else {
                let j = j as usize;
                (j, j + 1, (xi - xp[j]) / (xp[j + 1] - xp[j]))
            };
            if extrap == Extrap::Nan && (xi < xmin || xi > xmax) {
                dd = f64::NAN;
            }
            (1.0 - dd) * fp[jb] + fp[ja] * dd
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn ellude() {
        assert_eq!(
            ellude_common(&v(&["AAA|abcd", "AAA|abce"]), 2),
            v(&["abcd", "abce"])
        );
        assert_eq!(
            ellude_common(&v(&["file_1.csv", "file_2.csv"]), 2),
            v(&["file_1", "file_2"])
        );
        assert_eq!(
            ellude_common(&v(&["Sim_A.out", "Sim_B.out"]), 2),
            v(&["Sim_A", "Sim_B"])
        );
        assert_eq!(
            ellude_common(&v(&["Sim_Long_A", "Sim_Long_BB"]), 2),
            v(&["Long_A", "Long_BB"])
        );
        assert_eq!(
            ellude_common(&v(&[">protected", "dir|x"]), 2),
            v(&["protected", "x"])
        );
    }

    #[test]
    fn units() {
        assert_eq!(no_unit("Speed [m/s]"), "Speed");
        assert_eq!(no_unit("Speed_[m/s]"), "Speed");
        assert_eq!(no_unit("Speed (m/s)"), "Speed");
        assert_eq!(unit("Speed [m/s]"), "m/s");
        assert_eq!(inverse_unit("Time [s]"), "1/s");
        assert_eq!(inverse_unit("Az [deg]"), "1/deg");
        assert_eq!(clean_col("Wind_Speed (m/s)"), "windspeed");
    }

    #[test]
    fn interp_extrap() {
        let xp = [0.0, 1.0, 2.0];
        let fp = [10.0, 20.0, 30.0];
        let y = multi_interp(&[-1.0, 0.0, 0.5, 2.0, 3.0], &xp, &fp, Extrap::Nan);
        assert!(y[0].is_nan() && y[4].is_nan());
        assert_eq!(&y[1..4], &[10.0, 15.0, 30.0]);
        let y = multi_interp(&[-1.0, 3.0], &xp, &fp, Extrap::Bounded);
        assert_eq!(y, vec![10.0, 30.0]);
    }

    #[test]
    fn pretty() {
        assert_eq!(pretty_num(12345.0), "1.234e+04");
        assert_eq!(pretty_num(1.5), "   1.5000");
        assert_eq!(pretty_time(612.0), "10m12s");
        assert_eq!(extract_key_num("all=0.1, b=inf")["b"], f64::INFINITY);
    }
}
