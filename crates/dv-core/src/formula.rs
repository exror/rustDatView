//! Expression evaluator for formula columns and masks.
//!
//! pyDatView evaluates Python/numpy expressions where `{Column}` is replaced
//! by the column data (`formulae.py`, `plugins/data_mask.py`). This module
//! implements the numpy subset those expressions use:
//!
//! - column references `{Name}` (name without unit, or full name),
//! - numbers and strings (including datetime comparisons and string masks),
//!   constants `pi`, `e`, `nan`, `inf`, `True`, `False`,
//! - `+ - * / // % **`, unary `- + ~ not`,
//! - comparisons `< <= > >= == !=`, boolean `& && | || and or`,
//! - numpy functions, with optional `np.`/`numpy.`/`math.` prefix
//!   (see [`call_function`]).
//!
//! Booleans are represented as `1.0`/`0.0`.

use chrono::NaiveDate;

use crate::common::no_unit;
use crate::dataframe::{datetime_to_seconds, ColumnData, DataFrame};
use crate::error::{DvError, Result};

mod string_mask;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Col(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
}

fn tokenize(s: &str) -> Result<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let err = |m: String| DvError::Formula(m);
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '{' {
            let start = i + 1;
            let end = c[start..]
                .iter()
                .position(|&x| x == '}')
                .ok_or_else(|| err("unclosed `{`".into()))?
                + start;
            out.push(Tok::Col(
                c[start..end].iter().collect::<String>().trim().to_string(),
            ));
            i = end + 1;
        } else if ch == '\'' || ch == '"' {
            i += 1;
            let mut value = String::new();
            let mut closed = false;
            while i < c.len() {
                let current = c[i];
                i += 1;
                if current == ch {
                    closed = true;
                    break;
                }
                if current == '\\' {
                    let escaped = *c
                        .get(i)
                        .ok_or_else(|| err("unfinished string escape".into()))?;
                    i += 1;
                    value.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '\\' => '\\',
                        '\'' => '\'',
                        '"' => '"',
                        _ => return Err(err(format!("unsupported string escape `\\{escaped}`"))),
                    });
                } else {
                    value.push(current);
                }
            }
            if !closed {
                return Err(err("unclosed string".into()));
            }
            out.push(Tok::Str(value));
        } else if ch.is_ascii_digit()
            || (ch == '.' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.') {
                i += 1;
            }
            if i < c.len() && (c[i] == 'e' || c[i] == 'E') {
                let save = i;
                i += 1;
                if i < c.len() && (c[i] == '+' || c[i] == '-') {
                    i += 1;
                }
                if i < c.len() && c[i].is_ascii_digit() {
                    while i < c.len() && c[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let t: String = c[start..i].iter().collect();
            out.push(Tok::Num(
                t.parse()
                    .map_err(|_| err(format!("invalid number `{t}`")))?,
            ));
        } else if ch.is_alphabetic() || ch == '_' {
            let start = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.') {
                i += 1;
            }
            out.push(Tok::Ident(c[start..i].iter().collect()));
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            let op2 = ["**", "//", "<=", ">=", "==", "!=", "&&", "||"]
                .into_iter()
                .find(|o| *o == two);
            if let Some(o) = op2 {
                out.push(Tok::Op(o));
                i += 2;
                continue;
            }
            let t = match ch {
                '(' => Tok::LParen,
                ')' => Tok::RParen,
                '[' => Tok::LBracket,
                ']' => Tok::RBracket,
                ',' => Tok::Comma,
                '+' => Tok::Op("+"),
                '-' => Tok::Op("-"),
                '*' => Tok::Op("*"),
                '/' => Tok::Op("/"),
                '%' => Tok::Op("%"),
                '<' => Tok::Op("<"),
                '>' => Tok::Op(">"),
                '&' => Tok::Op("&"),
                '|' => Tok::Op("|"),
                '~' => Tok::Op("~"),
                _ => return Err(err(format!("unexpected character `{ch}`"))),
            };
            out.push(t);
            i += 1;
        }
    }
    Ok(out)
}

/// Evaluated value.
#[derive(Debug, Clone)]
enum Val {
    Scalar(f64),
    /// Array, flagged when it comes from a datetime column.
    Arr(Vec<f64>, bool),
    Str(String),
    /// Borrow text through a column index rather than cloning its strings.
    TextColumn(usize),
}

impl Val {
    fn is_date(&self) -> bool {
        matches!(self, Val::Arr(_, true))
    }
}

struct Ctx<'a> {
    df: &'a DataFrame,
    toks: Vec<Tok>,
    pos: usize,
}

fn parse_date(s: &str) -> Option<f64> {
    let s = s.trim();
    for f in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M:%S",
    ] {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(s, f) {
            return Some(datetime_to_seconds(&d));
        }
    }
    for f in ["%Y-%m-%d", "%Y/%m/%d", "%d/%m/%Y"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, f) {
            return Some(datetime_to_seconds(&d.and_hms_opt(0, 0, 0)?));
        }
    }
    None
}

fn to_num(v: Val, other_is_date: bool) -> Result<Val> {
    match v {
        Val::Str(s) if other_is_date => parse_date(&s)
            .map(Val::Scalar)
            .ok_or_else(|| DvError::Formula(format!("cannot parse date `{s}`"))),
        Val::Str(s) => Err(DvError::Formula(format!(
            "string `{s}` used in a numeric expression"
        ))),
        Val::TextColumn(_) => Err(DvError::Formula(
            "text column used in a numeric expression".into(),
        )),
        v => Ok(v),
    }
}

fn binop(a: Val, b: Val, f: impl Fn(f64, f64) -> f64) -> Result<Val> {
    let (ad, bd) = (a.is_date(), b.is_date());
    let a = to_num(a, bd)?;
    let b = to_num(b, ad)?;
    Ok(match (a, b) {
        (Val::Scalar(x), Val::Scalar(y)) => Val::Scalar(f(x, y)),
        (Val::Arr(x, _), Val::Scalar(y)) => {
            Val::Arr(x.into_iter().map(|v| f(v, y)).collect(), false)
        }
        (Val::Scalar(x), Val::Arr(y, _)) => {
            Val::Arr(y.into_iter().map(|v| f(x, v)).collect(), false)
        }
        (Val::Arr(x, _), Val::Arr(y, _)) => {
            if x.len() != y.len() {
                return Err(DvError::Formula(format!(
                    "length mismatch: {} vs {}",
                    x.len(),
                    y.len()
                )));
            }
            Val::Arr(x.into_iter().zip(y).map(|(u, v)| f(u, v)).collect(), false)
        }
        _ => unreachable!(),
    })
}

fn text_at<'a>(df: &'a DataFrame, value: &'a Val, row: usize) -> Result<&'a str> {
    match value {
        Val::Str(s) => Ok(s.as_str()),
        Val::TextColumn(i) => match &df.column(*i).data {
            ColumnData::Str(values) => Ok(&values[row]),
            _ => unreachable!(),
        },
        _ => Err(DvError::Formula(
            "cannot compare text and numeric values".into(),
        )),
    }
}

fn comparison(df: &DataFrame, a: Val, b: Val, op: &str) -> Result<Val> {
    if !a.is_date()
        && !b.is_date()
        && (matches!(a, Val::Str(_) | Val::TextColumn(_))
            || matches!(b, Val::Str(_) | Val::TextColumn(_)))
    {
        let array = matches!(a, Val::TextColumn(_)) || matches!(b, Val::TextColumn(_));
        let compare = |row| -> Result<f64> {
            let x = text_at(df, &a, row)?;
            let y = text_at(df, &b, row)?;
            Ok(b2f(match op {
                "<" => x < y,
                "<=" => x <= y,
                ">" => x > y,
                ">=" => x >= y,
                "==" => x == y,
                _ => x != y,
            }))
        };
        return if array {
            Ok(Val::Arr(
                (0..df.nrows()).map(compare).collect::<Result<_>>()?,
                false,
            ))
        } else {
            Ok(Val::Scalar(compare(0)?))
        };
    }
    binop(a, b, |x, y| {
        b2f(match op {
            "<" => x < y,
            "<=" => x <= y,
            ">" => x > y,
            ">=" => x >= y,
            "==" => x == y,
            _ => x != y,
        })
    })
}

fn unop(a: Val, f: impl Fn(f64) -> f64) -> Result<Val> {
    Ok(match to_num(a, false)? {
        Val::Scalar(x) => Val::Scalar(f(x)),
        Val::Arr(x, _) => Val::Arr(x.into_iter().map(f).collect(), false),
        Val::Str(_) | Val::TextColumn(_) => unreachable!(),
    })
}

fn b2f(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}
fn truthy(x: f64) -> bool {
    x != 0.0 && !x.is_nan()
}

fn arr_of(v: Val) -> Result<Vec<f64>> {
    match to_num(v, false)? {
        Val::Arr(x, _) => Ok(x),
        Val::Scalar(x) => Ok(vec![x]),
        Val::Str(_) | Val::TextColumn(_) => unreachable!(),
    }
}

fn reduce(v: Val, f: impl Fn(&[f64]) -> f64) -> Result<Val> {
    Ok(Val::Scalar(f(&arr_of(v)?)))
}

fn nan_filtered(x: &[f64]) -> Vec<f64> {
    x.iter().copied().filter(|v| !v.is_nan()).collect()
}

fn mean(x: &[f64]) -> f64 {
    x.iter().sum::<f64>() / x.len() as f64
}
fn std(x: &[f64]) -> f64 {
    let m = mean(x);
    (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}
fn median(x: &[f64]) -> f64 {
    let mut s = x.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let n = s.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 1 {
        s[n / 2]
    } else {
        0.5 * (s[n / 2 - 1] + s[n / 2])
    }
}

/// Apply a named function. Names may be prefixed by `np.`, `numpy.` or `math.`.
fn call_function(name: &str, mut args: Vec<Val>) -> Result<Val> {
    let short = name.rsplit('.').next().unwrap_or(name);
    let nargs = args.len();
    let need = |n: usize| -> Result<()> {
        if nargs == n {
            Ok(())
        } else {
            Err(DvError::Formula(format!(
                "`{name}` expects {n} argument(s), got {nargs}"
            )))
        }
    };
    macro_rules! u {
        ($f:expr) => {{
            need(1)?;
            unop(args.pop().unwrap(), $f)
        }};
    }
    macro_rules! b {
        ($f:expr) => {{
            need(2)?;
            let y = args.pop().unwrap();
            let x = args.pop().unwrap();
            binop(x, y, $f)
        }};
    }
    match short {
        "sin" => u!(f64::sin),
        "cos" => u!(f64::cos),
        "tan" => u!(f64::tan),
        "arcsin" | "asin" => u!(f64::asin),
        "arccos" | "acos" => u!(f64::acos),
        "arctan" | "atan" => u!(f64::atan),
        "sinh" => u!(f64::sinh),
        "cosh" => u!(f64::cosh),
        "tanh" => u!(f64::tanh),
        "exp" => u!(f64::exp),
        "log" => u!(f64::ln),
        "log10" => u!(f64::log10),
        "log2" => u!(f64::log2),
        "sqrt" => u!(f64::sqrt),
        "abs" | "absolute" | "fabs" => u!(f64::abs),
        "floor" => u!(f64::floor),
        "ceil" => u!(f64::ceil),
        "round" | "around" | "rint" => u!(f64::round_ties_even),
        "sign" => u!(|x: f64| if x.is_nan() {
            f64::NAN
        } else if x > 0.0 {
            1.0
        } else if x < 0.0 {
            -1.0
        } else {
            0.0
        }),
        "deg2rad" | "radians" => u!(f64::to_radians),
        "rad2deg" | "degrees" => u!(f64::to_degrees),
        "isnan" => u!(|x: f64| b2f(x.is_nan())),
        "arctan2" | "atan2" => b!(f64::atan2),
        "hypot" => b!(f64::hypot),
        "power" | "pow" => b!(f64::powf),
        "mod" | "fmod" => b!(|a: f64, b: f64| a - b * (a / b).floor()),
        "minimum" | "fmin" => b!(|a: f64, b: f64| if a.is_nan() || b.is_nan() {
            f64::NAN
        } else {
            a.min(b)
        }),
        "maximum" | "fmax" => b!(|a: f64, b: f64| if a.is_nan() || b.is_nan() {
            f64::NAN
        } else {
            a.max(b)
        }),
        "mean" => {
            need(1)?;
            reduce(args.pop().unwrap(), mean)
        }
        "nanmean" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| mean(&nan_filtered(x)))
        }
        "std" => {
            need(1)?;
            reduce(args.pop().unwrap(), std)
        }
        "nanstd" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| std(&nan_filtered(x)))
        }
        "median" => {
            need(1)?;
            reduce(args.pop().unwrap(), median)
        }
        "nanmedian" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| median(&nan_filtered(x)))
        }
        "sum" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| x.iter().sum())
        }
        "nansum" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| nan_filtered(x).iter().sum())
        }
        "min" | "amin" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| {
                if x.iter().any(|v| v.is_nan()) {
                    f64::NAN
                } else {
                    x.iter().copied().fold(f64::INFINITY, f64::min)
                }
            })
        }
        "max" | "amax" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| {
                if x.iter().any(|v| v.is_nan()) {
                    f64::NAN
                } else {
                    x.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                }
            })
        }
        "nanmin" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| {
                nan_filtered(x).into_iter().fold(f64::INFINITY, f64::min)
            })
        }
        "nanmax" => {
            need(1)?;
            reduce(args.pop().unwrap(), |x| {
                nan_filtered(x)
                    .into_iter()
                    .fold(f64::NEG_INFINITY, f64::max)
            })
        }
        "cumsum" => {
            need(1)?;
            let x = arr_of(args.pop().unwrap())?;
            let mut acc = 0.0;
            Ok(Val::Arr(
                x.into_iter()
                    .map(|v| {
                        acc += v;
                        acc
                    })
                    .collect(),
                false,
            ))
        }
        "gradient" => {
            // numpy.gradient with unit spacing (or spacing array as 2nd arg).
            if nargs != 1 && nargs != 2 {
                return Err(DvError::Formula(
                    "`gradient` expects 1 or 2 arguments".into(),
                ));
            }
            let xs = if nargs == 2 {
                Some(arr_of(args.pop().unwrap())?)
            } else {
                None
            };
            let y = arr_of(args.pop().unwrap())?;
            Ok(Val::Arr(gradient(&y, xs.as_deref())?, false))
        }
        "where" => {
            need(3)?;
            let b = arr_of(args.pop().unwrap())?;
            let a = arr_of(args.pop().unwrap())?;
            let c = arr_of(args.pop().unwrap())?;
            let n = c.len().max(a.len()).max(b.len());
            let at = |v: &[f64], i: usize| -> Result<f64> {
                match v.len() {
                    1 => Ok(v[0]),
                    l if l == n => Ok(v[i]),
                    l => Err(DvError::Formula(format!(
                        "where: length mismatch ({l} vs {n})"
                    ))),
                }
            };
            let out = (0..n)
                .map(|i| {
                    Ok(if truthy(at(&c, i)?) {
                        at(&a, i)?
                    } else {
                        at(&b, i)?
                    })
                })
                .collect::<Result<Vec<f64>>>()?;
            Ok(if n == 1 {
                Val::Scalar(out[0])
            } else {
                Val::Arr(out, false)
            })
        }
        "clip" => {
            need(3)?;
            let hi = args.pop().unwrap();
            let lo = args.pop().unwrap();
            let x = args.pop().unwrap();
            let x = binop(x, lo, f64::max)?;
            binop(x, hi, f64::min)
        }
        _ => Err(DvError::Formula(format!("unknown function `{name}`"))),
    }
}

fn gradient(y: &[f64], x: Option<&[f64]>) -> Result<Vec<f64>> {
    let n = y.len();
    if n < 2 {
        return Err(DvError::Formula(
            "gradient requires at least 2 values".into(),
        ));
    }
    let xv = |i: usize| x.map_or(i as f64, |x| x[i]);
    if let Some(x) = x {
        if x.len() != n {
            return Err(DvError::Formula("gradient: length mismatch".into()));
        }
    }
    let mut g = vec![0.0; n];
    g[0] = (y[1] - y[0]) / (xv(1) - xv(0));
    g[n - 1] = (y[n - 1] - y[n - 2]) / (xv(n - 1) - xv(n - 2));
    for i in 1..n - 1 {
        let hs = xv(i) - xv(i - 1);
        let hd = xv(i + 1) - xv(i);
        g[i] = (hs * hs * y[i + 1] + (hd * hd - hs * hs) * y[i] - hd * hd * y[i - 1])
            / (hs * hd * (hd + hs));
    }
    Ok(g)
}

impl<'a> Ctx<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn is_op(&self, ops: &[&str]) -> Option<&'static str> {
        match self.peek() {
            Some(Tok::Op(o)) if ops.contains(o) => Some(o),
            Some(Tok::Ident(w))
                if (w == "and" || w == "or" || w == "not") && ops.contains(&w.as_str()) =>
            {
                Some(match w.as_str() {
                    "and" => "and",
                    "or" => "or",
                    _ => "not",
                })
            }
            _ => None,
        }
    }

    fn or(&mut self) -> Result<Val> {
        let mut a = self.and()?;
        while self.is_op(&["|", "||", "or"]).is_some() {
            self.pos += 1;
            let b = self.and()?;
            a = binop(a, b, |x, y| b2f(truthy(x) || truthy(y)))?;
        }
        Ok(a)
    }
    fn and(&mut self) -> Result<Val> {
        let mut a = self.not()?;
        while self.is_op(&["&", "&&", "and"]).is_some() {
            self.pos += 1;
            let b = self.not()?;
            a = binop(a, b, |x, y| b2f(truthy(x) && truthy(y)))?;
        }
        Ok(a)
    }
    fn not(&mut self) -> Result<Val> {
        if self.is_op(&["~", "not"]).is_some() {
            self.pos += 1;
            let a = self.not()?;
            return unop(a, |x| b2f(!truthy(x)));
        }
        self.cmp()
    }
    fn cmp(&mut self) -> Result<Val> {
        let mut a = self.add()?;
        while let Some(op) = self.is_op(&["<", "<=", ">", ">=", "==", "!="]) {
            self.pos += 1;
            let b = self.add()?;
            a = comparison(self.df, a, b, op)?;
        }
        Ok(a)
    }
    fn add(&mut self) -> Result<Val> {
        let mut a = self.mul()?;
        while let Some(op) = self.is_op(&["+", "-"]) {
            self.pos += 1;
            let b = self.mul()?;
            a = if op == "+" {
                binop(a, b, |x, y| x + y)?
            } else {
                binop(a, b, |x, y| x - y)?
            };
        }
        Ok(a)
    }
    fn mul(&mut self) -> Result<Val> {
        let mut a = self.unary()?;
        while let Some(op) = self.is_op(&["*", "/", "//", "%"]) {
            self.pos += 1;
            let b = self.unary()?;
            a = match op {
                "*" => binop(a, b, |x, y| x * y),
                "/" => binop(a, b, |x, y| x / y),
                "//" => binop(a, b, |x, y| (x / y).floor()),
                _ => binop(a, b, |x, y| x - y * (x / y).floor()),
            }?;
        }
        Ok(a)
    }
    fn unary(&mut self) -> Result<Val> {
        if let Some(op) = self.is_op(&["-", "+"]) {
            self.pos += 1;
            let a = self.unary()?;
            return if op == "-" { unop(a, |x| -x) } else { Ok(a) };
        }
        self.power()
    }
    fn power(&mut self) -> Result<Val> {
        let a = self.atom()?;
        if self.is_op(&["**"]).is_some() {
            self.pos += 1;
            let b = self.unary()?;
            return binop(a, b, f64::powf);
        }
        Ok(a)
    }
    fn column(&self, name: &str) -> Result<Val> {
        let cols = self.df.columns();
        let idx = cols
            .iter()
            .position(|c| no_unit(&c.name).trim() == name)
            .or_else(|| cols.iter().position(|c| c.name.trim() == name))
            .ok_or_else(|| DvError::Formula(format!("unknown column `{{{name}}}`")))?;
        let c = &cols[idx];
        Ok(match &c.data {
            ColumnData::DateTime(v) => Val::Arr(v.clone(), true),
            ColumnData::Str(_) => Val::TextColumn(idx),
            _ => Val::Arr(c.as_f64().into_owned(), false),
        })
    }
    fn atom(&mut self) -> Result<Val> {
        match self.next() {
            Some(Tok::Num(x)) => Ok(Val::Scalar(x)),
            Some(Tok::Str(s)) => Ok(Val::Str(s)),
            Some(Tok::Col(name)) => self.column(&name),
            Some(Tok::LParen) => {
                let v = self.or()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err(DvError::Formula("expected `)`".into())),
                }
            }
            Some(Tok::Ident(id)) => {
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.or()?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => {
                                    return Err(DvError::Formula(format!(
                                        "expected `,` or `)` in call to `{id}`"
                                    )))
                                }
                            }
                        }
                    } else {
                        self.pos += 1;
                    }
                    return call_function(&id, args);
                }
                let short = id.rsplit('.').next().unwrap_or(&id);
                match short {
                    "pi" => Ok(Val::Scalar(std::f64::consts::PI)),
                    "e" => Ok(Val::Scalar(std::f64::consts::E)),
                    "nan" | "NaN" | "NAN" => Ok(Val::Scalar(f64::NAN)),
                    "inf" | "Inf" => Ok(Val::Scalar(f64::INFINITY)),
                    "True" => Ok(Val::Scalar(1.0)),
                    "False" => Ok(Val::Scalar(0.0)),
                    _ => Err(DvError::Formula(format!("unknown name `{id}`"))),
                }
            }
            Some(t) => Err(DvError::Formula(format!("unexpected token {t:?}"))),
            None => Err(DvError::Formula("unexpected end of expression".into())),
        }
    }
}

fn eval_val(df: &DataFrame, expr: &str) -> Result<Val> {
    let toks = tokenize(expr)?;
    if toks.first() == Some(&Tok::LBracket) {
        return string_mask::evaluate(df, &toks).map(|values| Val::Arr(values, false));
    }
    if toks.is_empty() {
        return Err(DvError::Formula("empty expression".into()));
    }
    let mut ctx = Ctx { df, toks, pos: 0 };
    let v = ctx.or()?;
    if ctx.pos < ctx.toks.len() {
        return Err(DvError::Formula(format!(
            "unexpected trailing token {:?}",
            ctx.toks[ctx.pos]
        )));
    }
    Ok(v)
}

/// Evaluate a formula to a column of `df.nrows()` values (scalars are broadcast).
pub fn eval_formula(df: &DataFrame, expr: &str) -> Result<Vec<f64>> {
    let n = df.nrows();
    match eval_val(df, expr)? {
        Val::Scalar(x) => Ok(vec![x; n]),
        Val::Arr(v, _) if v.len() == n => Ok(v),
        Val::Arr(v, _) => Err(DvError::Formula(format!(
            "result has {} values, table has {n} rows",
            v.len()
        ))),
        Val::Str(s) => Err(DvError::Formula(format!(
            "formula evaluates to a string `{s}`"
        ))),
        Val::TextColumn(_) => Err(DvError::Formula(
            "formula evaluates to text rather than numeric values".into(),
        )),
    }
}

/// Evaluate a boolean mask expression (NaN and 0 are false).
pub fn eval_mask(df: &DataFrame, expr: &str) -> Result<Vec<bool>> {
    Ok(eval_formula(df, expr)?.into_iter().map(truthy).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataframe::Column;

    fn df() -> DataFrame {
        DataFrame::from_columns(vec![
            Column::float("Time [s]", vec![0.0, 1.0, 2.0, 3.0]),
            Column::float("Speed_[m/s]", vec![1.0, 2.0, 3.0, 4.0]),
            Column::datetime("Date", vec![0.0, 86400.0, 2.0 * 86400.0, 3.0 * 86400.0]),
        ])
        .unwrap()
    }

    #[test]
    fn arithmetic_and_functions() {
        let d = df();
        assert_eq!(
            eval_formula(&d, "{Time}*2 + 1").unwrap(),
            vec![1.0, 3.0, 5.0, 7.0]
        );
        assert_eq!(
            eval_formula(&d, "-{Speed}**2").unwrap(),
            vec![-1.0, -4.0, -9.0, -16.0]
        );
        assert_eq!(
            eval_formula(&d, "np.sqrt({Speed}*{Speed}) - mean({Speed})").unwrap(),
            vec![-1.5, -0.5, 0.5, 1.5]
        );
        assert_eq!(eval_formula(&d, "2**3**2").unwrap()[0], 512.0);
        assert!((eval_formula(&d, "cos(pi)").unwrap()[0] + 1.0).abs() < 1e-15);
        assert!(eval_formula(&d, "{Nope}+1").is_err());
    }

    #[test]
    fn masks() {
        let d = df();
        assert_eq!(
            eval_mask(&d, "({Time}>0) & ({Time}<3)").unwrap(),
            vec![false, true, true, false]
        );
        assert_eq!(
            eval_mask(&d, "{Time} > 1 or {Speed} == 1").unwrap(),
            vec![true, false, true, true]
        );
        assert_eq!(
            eval_mask(&d, "{Date} >= '1970-01-03'").unwrap(),
            vec![false, false, true, true]
        );
        assert_eq!(
            eval_mask(&d, "~({Time}>=2)").unwrap(),
            vec![true, true, false, false]
        );
    }
    #[test]
    fn string_masks_and_comprehensions() {
        let d = DataFrame::from_columns(vec![
            Column::string(
                "Name",
                vec![
                    "Rotor".into(),
                    "rotor".into(),
                    "tower".into(),
                    "Rotor's".into(),
                ],
            ),
            Column::float("Time [s]", vec![0., 1., 2., 3.]),
        ])
        .unwrap();
        assert_eq!(
            eval_mask(
                &d,
                "({Name} == 'Rotor') || ({Time} > 2 && {Name} != 'tower')"
            )
            .unwrap(),
            [true, false, false, true]
        );
        assert_eq!(
            eval_mask(&d, "['Rotor' in str(x) for x in {Name}]").unwrap(),
            [true, false, false, true]
        );
        assert_eq!(
            eval_mask(&d, "[x != 'tower' and 'rot' in str(x) for x in {Name}]").unwrap(),
            [false, true, false, false]
        );
        assert_eq!(
            eval_mask(&d, "{Name} == 'Rotor\\'s'").unwrap(),
            [false, false, false, true]
        );
        assert!(eval_mask(&d, "[__import__('os') for x in {Name}]").is_err());
    }
}
