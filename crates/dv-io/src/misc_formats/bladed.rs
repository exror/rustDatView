use super::common::*;
use dv_core::{DataFrame, FileObject, IoError, NamedFrame};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
struct Output {
    name: String,
    cols: Vec<String>,
    values: Vec<Vec<f64>>,
    time: Option<(f64, f64)>,
    n: usize,
}
fn unit(s: &str) -> String {
    s.replace("[[", "[")
        .replace("]]", "]")
        .replace("TT", "s^2")
        .replace('T', "s")
        .replace('A', "rad")
        .replace('P', "W")
        .replace('L', "m")
        .replace('F', "N")
        .replace('M', "kg")
}
fn one(path: &Path) -> Result<Output, IoError> {
    let s = text(path)?;
    let mut metadata = BTreeMap::<String, String>::new();
    let mut active = String::new();
    for line in s.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        let sp = l.split_once(char::is_whitespace).unwrap_or((l, ""));
        if [
            "NDIMENS", "DIMENS", "FORMAT", "GENLAB", "AXIVAL", "AXITICK", "VARIAB", "VARUNIT",
            "AXISLAB", "NVARS", "MIN", "STEP",
        ]
        .contains(&sp.0)
        {
            active = sp.0.into();
            metadata.insert(active.clone(), sp.1.trim().into());
        } else if ["VARIAB", "VARUNIT", "AXITICK"].contains(&active.as_str()) {
            if let Some(v) = metadata.get_mut(&active) {
                v.push(' ');
                v.push_str(l);
            }
        }
    }
    let get = |k: &str| {
        metadata
            .get(k)
            .map(String::as_str)
            .ok_or_else(|| broken(format!("Missing Bladed metadata {k}")))
    };
    let ndim = count(get("NDIMENS")?)?;
    if ![2, 3].contains(&ndim) {
        return Err(broken("Bladed needs 2 or 3 dimensions"));
    }
    let dims: Vec<usize> = get("DIMENS")?
        .split_whitespace()
        .map(count)
        .collect::<Result<_, _>>()?;
    if dims.len() != ndim {
        return Err(broken("Invalid Bladed dimensions"));
    }
    let nc = dims[0];
    let ns = if ndim == 3 { dims[1] } else { 1 };
    let mut nr = dims[ndim - 1];
    if nc == 0 || ns == 0 {
        return Err(broken("Invalid Bladed channel/section count"));
    }
    let names = quoted(get("VARIAB")?);
    let units = quoted(get("VARUNIT")?);
    if names.len() != nc || units.len() != nc {
        return Err(broken("Bladed sensor count mismatch"));
    }
    let sections = if ndim == 3 {
        if let Some(v) = metadata.get("AXIVAL") {
            numbers(v)?
                .into_iter()
                .map(|v| format!("{}m", (v * 100.0).round() / 100.0))
                .collect()
        } else {
            quoted(get("AXITICK")?)
        }
    } else {
        vec![String::new()]
    };
    if sections.len() != ns {
        return Err(broken("Bladed section count mismatch"));
    }
    let p = PathBuf::from(path.to_string_lossy().replace('%', "$"));
    let b = std::fs::read(p)?;
    let numeric = std::str::from_utf8(&b).ok().and_then(|s| {
        s.split_whitespace()
            .map(number)
            .collect::<Result<Vec<_>, _>>()
            .ok()
    });
    let flat = if let Some(v) = numeric {
        v
    } else {
        let f = get("FORMAT")?;
        let width = if f.trim().ends_with('8') { 8 } else { 4 };
        if b.len() % width != 0 {
            return Err(broken("Truncated Bladed binary data"));
        }
        b.chunks_exact(width)
            .map(|v| {
                if width == 8 {
                    f64::from_le_bytes(v.try_into().unwrap())
                } else {
                    f32::from_le_bytes(v.try_into().unwrap()) as f64
                }
            })
            .collect()
    };
    if nr == 0 {
        nr = flat.len() / (nc * ns);
    }
    if flat.len() != nr * nc * ns {
        return Err(broken("Bladed data size mismatch"));
    }
    let mut cols = Vec::new();
    let mut values = Vec::new();
    for (j, section) in sections.iter().enumerate() {
        for c in 0..nc {
            let n = if ndim == 3 {
                format!("{section}-{}", names[c])
            } else {
                names[c].clone()
            };
            cols.push(format!("{n} [{}]", unit(&units[c])));
            values.push((0..nr).map(|t| flat[(t * ns + j) * nc + c]).collect());
        }
    }
    let time = match (metadata.get("MIN"), metadata.get("STEP")) {
        (Some(a), Some(b)) => Some((number(a)?, number(b)?)),
        _ => None,
    };
    Ok(Output {
        name: metadata
            .get("GENLAB")
            .map(|s| s.replace('\'', ""))
            .unwrap_or_else(|| "Misc".into()),
        cols,
        values,
        time,
        n: nr,
    })
}
pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let ext = path.extension().unwrap_or_default().to_string_lossy();
    let project = ext.eq_ignore_ascii_case("$pj");
    let mut paths = Vec::new();
    if project {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let prefix = format!("{stem}.%");
        for entry in std::fs::read_dir(path.parent().unwrap_or(Path::new(".")))? {
            let p = entry?.path();
            let file = p.file_name().unwrap_or_default().to_string_lossy();
            if let Some(suffix) = file.strip_prefix(&prefix) {
                if suffix.len() >= 2
                    && suffix.as_bytes()[0].is_ascii_digit()
                    && suffix.as_bytes()[1].is_ascii_digit()
                {
                    paths.push(p);
                }
            }
        }
        paths.sort();
    } else {
        paths.push(PathBuf::from(path.to_string_lossy().replace('$', "%")));
    }
    if paths.is_empty() {
        return Err(broken("No Bladed sensor files found"));
    }
    let multiple = paths.len() > 1;
    let mut groups = BTreeMap::<usize, Output>::new();
    for p in paths {
        let mut out = match one(&p) {
            Ok(o) => o,
            Err(IoError::Io(e)) if multiple && e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if project && multiple && out.time.is_none() {
            continue;
        }
        if let Some(old) = groups.get_mut(&out.n) {
            old.cols.append(&mut out.cols);
            old.values.append(&mut out.values);
            old.name = format!("Misc_{}", out.n);
        } else {
            if let Some((t0, dt)) = out.time {
                out.cols.insert(0, "Time [s]".into());
                out.values
                    .insert(0, (0..out.n).map(|i| t0 + i as f64 * dt).collect());
            }
            groups.insert(out.n, out);
        }
    }
    let misc = groups
        .values()
        .filter(|o| o.name.starts_with("Misc_"))
        .count();
    let len = groups.len();
    let mut frames = Vec::new();
    for (_, mut out) in groups {
        if misc == 1 && out.name.starts_with("Misc_") {
            out.name = "Misc".into();
        }
        let mut cols = Vec::new();
        let mut values = Vec::new();
        for (c, v) in out.cols.into_iter().zip(out.values) {
            if !cols.contains(&c) {
                cols.push(c);
                values.push(v);
            }
        }
        let df = DataFrame::from_floats(cols, values).map_err(|e| broken(e.to_string()))?;
        frames.push(NamedFrame::new(
            if len == 1 { String::new() } else { out.name },
            df,
        ));
    }
    Ok(object(path, "Bladed output file", frames))
}
