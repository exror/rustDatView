//! The deliberately small YAML subset used by OpenFAST summary files.
use super::binary::{broken, number, text_file};
use dv_core::IoError;
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    Number(f64),
    Text(String),
    Array(Vec<Vec<f64>>),
    Strings(Vec<Vec<String>>),
    Map(BTreeMap<String, YamlValue>),
}
impl YamlValue {
    pub fn array(&self) -> Option<&[Vec<f64>]> {
        if let Self::Array(a) = self {
            Some(a)
        } else {
            None
        }
    }
    pub fn flat(&self) -> Option<Vec<f64>> {
        self.array().map(|a| a.iter().flatten().copied().collect())
    }
}
pub fn yaml_read(path: &std::path::Path) -> Result<BTreeMap<String, YamlValue>, IoError> {
    yaml_parse(&text_file(path)?)
}
fn clean(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}
fn list(line: &str) -> Vec<String> {
    let l = clean(line)
        .trim_matches(['[', ']'])
        .trim()
        .trim_end_matches(',')
        .trim();
    if l.is_empty() {
        Vec::new()
    } else {
        l.split(',').map(|s| s.trim().into()).collect()
    }
}
fn array(rows: Vec<Vec<String>>) -> Result<YamlValue, IoError> {
    let n = rows.first().map_or(0, Vec::len);
    if rows.iter().any(|r| r.len() != n) {
        return Err(broken("YAML array rows have inconsistent widths"));
    }
    if rows
        .first()
        .is_none_or(|r| r.iter().all(|s| number(s).is_ok()))
    {
        Ok(YamlValue::Array(
            rows.iter()
                .map(|r| r.iter().map(|s| number(s)).collect())
                .collect::<Result<_, _>>()?,
        ))
    } else {
        Ok(YamlValue::Strings(rows))
    }
}
fn scalar(s: &str) -> Result<YamlValue, IoError> {
    let s = s.trim();
    if s.starts_with('[') {
        array(vec![list(s)])
    } else if s.starts_with('{') {
        let mut d = BTreeMap::new();
        for pair in s.trim_matches(['{', '}']).split(',') {
            let (k, v) = pair
                .split_once(':')
                .ok_or_else(|| broken("invalid inline YAML map"))?;
            d.insert(k.trim().into(), scalar(v)?);
        }
        Ok(YamlValue::Map(d))
    } else {
        Ok(number(s)
            .map(YamlValue::Number)
            .unwrap_or_else(|_| YamlValue::Text(s.into())))
    }
}
pub fn yaml_parse(text: &str) -> Result<BTreeMap<String, YamlValue>, IoError> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut i = 0;
    let mut out = BTreeMap::new();
    while i < lines.len() {
        let line = clean(lines[i]);
        i += 1;
        if line.is_empty() {
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| broken(format!("YAML line {i} has no colon")))?;
        let value = value.trim();
        let val = if value.is_empty() {
            let mut rows = Vec::new();
            while i < lines.len() {
                let line = clean(lines[i]);
                if !line.starts_with('-') {
                    break;
                }
                rows.push(list(line[1..].trim()));
                i += 1;
            }
            array(rows)?
        } else {
            scalar(value)?
        };
        out.insert(key.trim().into(), val);
    }
    Ok(out)
}
