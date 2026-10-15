use super::common::*;
use dv_core::{Field2D, FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};
pub fn gnuplot(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut blocks = Vec::new();
    let mut rows = Vec::new();
    let mut headers = Vec::new();
    for l in s.lines() {
        let l = l.trim();
        if let Some(h) = l.strip_prefix('#') {
            headers.push(h.trim());
        } else if l.is_empty() {
            if !rows.is_empty() {
                blocks.push(std::mem::take(&mut rows));
            }
        } else {
            rows.push(numbers(l).map_err(|_| wrong("Not a GNUPlot numeric file"))?);
        }
    }
    if !rows.is_empty() {
        blocks.push(rows);
    }
    if blocks.is_empty() {
        return Err(wrong("No GNUPlot data"));
    }
    let nr = blocks[0].len();
    let nc = blocks[0][0].len();
    if blocks
        .iter()
        .any(|b| b.iter().any(|r| r.len() != b[0].len()))
    {
        return Err(broken("Inconsistent GNUPlot rows"));
    }
    let same = blocks.iter().all(|b| b[0].len() == nc);
    let mesh = nc >= 2
        && blocks.iter().all(|b| {
            b.len() == nr
                && b[0].len() == nc
                && b.iter()
                    .zip(&blocks[0])
                    .all(|(r, a)| r[0] == a[0] && r[1] == b[0][1])
        });
    let defaults = |n: usize| {
        let mut c: Vec<_> = (0..n).map(|i| format!("C{i}")).collect();
        if n > 0 {
            c[0] = "x".into();
        }
        if n > 1 {
            c[1] = "y".into();
        }
        c
    };
    let cols = headers
        .iter()
        .find_map(|h| {
            let c: Vec<_> = h.split_whitespace().map(str::to_string).collect();
            (c.len() == nc).then_some(c)
        })
        .unwrap_or_else(|| defaults(nc));
    let mut frames = Vec::new();
    let mut fields = None;
    if mesh || blocks.len() == 1 {
        let all: Vec<_> = blocks.iter().flatten().cloned().collect();
        frames.push(NamedFrame::unnamed(table(&cols, &all)?));
    } else {
        for (i, b) in blocks.iter().enumerate() {
            let c = if same {
                cols.clone()
            } else {
                defaults(b[0].len())
            };
            frames.push(NamedFrame::new(format!("set{i}"), table(&c, b)?));
        }
    }
    if mesh {
        let x: Vec<_> = blocks[0].iter().map(|r| r[0]).collect();
        let y: Vec<_> = blocks.iter().map(|b| b[0][1]).collect();
        let mut out = Vec::new();
        for c in 2..nc {
            let mut m = Vec::with_capacity(nr * blocks.len());
            for ix in 0..nr {
                for b in &blocks {
                    m.push(b[ix][c]);
                }
            }
            out.push(Field2D {
                name: cols[c].clone(),
                x: x.clone(),
                y: y.clone(),
                sx: cols[0].clone(),
                sy: cols[1].clone(),
                m,
            });
        }
        fields = Some(out);
    } else if blocks.len() == 1 && nc >= 2 {
        fields = Some(vec![Field2D {
            name: "data".into(),
            x: (0..nr).map(|i| i as f64).collect(),
            y: (0..nc).map(|i| i as f64).collect(),
            sx: "rows".into(),
            sy: "columns".into(),
            m: blocks[0].iter().flatten().copied().collect(),
        }]);
    }
    Ok(Arc::new(ParsedFile {
        path: path.into(),
        format: "GNUPlot file",
        frames,
        fields,
        description: headers.join("\n"),
    }))
}
pub fn tecplot(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut vars = String::new();
    let mut in_vars = false;
    let mut rows = Vec::new();
    let mut description = String::new();
    for line in s.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let lower = l.to_lowercase();
        if lower.starts_with("variables") {
            in_vars = true;
            vars.push_str(l.get(9..).unwrap_or("").trim_start_matches('='));
            continue;
        }
        if l.split_whitespace()
            .next()
            .is_some_and(|s| number(s).is_ok())
        {
            in_vars = false;
            rows.push(numbers(l)?);
            continue;
        }
        if [
            "zone",
            "title",
            "datasetauxdata",
            "text",
            "geometry",
            "customlabels",
        ]
        .iter()
        .any(|k| lower.starts_with(k))
        {
            in_vars = false;
            description.push_str(l);
            description.push('\n');
            if lower.contains("block") {
                return Err(broken(
                    "Tecplot BLOCK data is not supported by the upstream ASCII reader",
                ));
            }
        } else if in_vars {
            vars.push(' ');
            vars.push_str(l);
        } else if vars.is_empty() {
            return Err(wrong("Not a Tecplot ASCII file"));
        }
    }
    let cols: Vec<_> = quoted(vars.trim().trim_start_matches('='))
        .into_iter()
        .map(|s| s.replace('[', "_[").replace('(', "_(").replace("__", "_"))
        .collect();
    if cols.is_empty() {
        return Err(wrong("Missing Tecplot variables"));
    }
    if rows.is_empty() {
        return Err(broken("No Tecplot numeric data"));
    }
    Ok(Arc::new(ParsedFile {
        path: path.into(),
        format: "Tecplot ASCII file",
        frames: vec![NamedFrame::unnamed(table(&cols, &rows)?)],
        fields: None,
        description,
    }))
}
#[derive(Debug, Clone)]
pub struct MannOptions {
    pub dimensions: Option<[usize; 3]>,
    pub dy: f64,
    pub dz: f64,
    pub y0: Option<f64>,
    pub z0: f64,
    pub z_mid: Option<f64>,
}
impl Default for MannOptions {
    fn default() -> Self {
        Self {
            dimensions: None,
            dy: 1.0,
            dz: 1.0,
            y0: None,
            z0: 0.0,
            z_mid: None,
        }
    }
}
pub fn mann(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    mann_with_options(path, &MannOptions::default())
}
pub fn mann_with_options(
    path: &Path,
    options: &MannOptions,
) -> Result<Arc<dyn FileObject>, IoError> {
    let dims = if let Some(n) = options.dimensions {
        n
    } else {
        let re = regex::Regex::new(r"\d+").unwrap();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let ns: Vec<usize> = re
            .find_iter(&stem)
            .filter_map(|m| m.as_str().parse().ok())
            .collect();
        if ns.len() < 3 {
            return Err(broken("Mann box needs dimensions, e.g. box_1024x32x32.u"));
        }
        [ns[ns.len() - 3], ns[ns.len() - 2], ns[ns.len() - 1]]
    };
    let [nx, ny, nz] = dims;
    if dims.contains(&0) {
        return Err(broken("Mann box dimensions must be positive"));
    }
    let n = nx
        .checked_mul(ny)
        .and_then(|n| n.checked_mul(nz))
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| broken("Mann box dimensions overflow"))?;
    let b = std::fs::read(path)?;
    if b.len() != n {
        return Err(broken("Mann box byte count does not match dimensions"));
    }
    let value = |ix: usize, iy: usize, iz: usize| {
        let p = ((ix * ny + (ny - 1 - iy)) * nz + iz) * 4;
        f32::from_le_bytes(b[p..p + 4].try_into().unwrap()) as f64
    };
    let y: Vec<_> = (0..ny)
        .map(|i| {
            options.y0.unwrap_or(-((ny - 1) as f64) * options.dy / 2.0) + i as f64 * options.dy
        })
        .collect();
    let z: Vec<_> = (0..nz)
        .map(|i| {
            options.z0
                + i as f64 * options.dz
                + options
                    .z_mid
                    .map(|v| v - options.z0 - (nz - 1) as f64 * options.dz / 2.0)
                    .unwrap_or(0.0)
        })
        .collect();
    let iy = ny / 2;
    let iz = nz / 2;
    let mut profile = Vec::new();
    for (j, &z) in z.iter().enumerate() {
        let mean = (0..nx).map(|i| value(i, iy, j)).sum::<f64>() / nx as f64;
        let std = ((0..nx)
            .map(|i| (value(i, iy, j) - mean).powi(2))
            .sum::<f64>()
            / nx as f64)
            .sqrt();
        profile.push(vec![z, mean, std, 100.0 * std / mean]);
    }
    let mut frames = vec![NamedFrame::new(
        "VertProfile",
        table(&names("z_[m]|vel_[m/s]|sigma_[m/s]|TI_[%]"), &profile)?,
    )];
    for (label, iy) in [
        ("ZMidLine", iy),
        ("ZMidYEndLine", ny - 1),
        ("ZMidYStartLine", 0),
    ] {
        let rows: Vec<_> = (0..nx)
            .map(|i| {
                vec![
                    if nx == 1 {
                        0.0
                    } else {
                        i as f64 / (nx - 1) as f64
                    },
                    value(i, iy, iz),
                ]
            })
            .collect();
        frames.push(NamedFrame::new(
            label,
            table(&names("t/T_[-]|vel_[m/s]"), &rows)?,
        ));
    }
    Ok(Arc::new(ParsedFile {
        path: path.into(),
        format: "HAWC2 Turbulence box",
        frames,
        fields: None,
        description: format!("Dimensions: {nx} x {ny} x {nz}\ny: {:?}\nz: {:?}", y, z),
    }))
}
pub fn bmodes(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let ls: Vec<_> = s.lines().collect();
    if !ls.first().is_some_and(|s| s.contains("BModes")) {
        return Err(wrong("Not BModes output"));
    }
    let mut frames = Vec::new();
    let mut frequencies = Vec::new();
    let mut i = 0;
    while i < ls.len() {
        if let Some(pos) = ls[i].find("freq = ") {
            let f = ls[i][pos + 7..]
                .split_whitespace()
                .next()
                .ok_or_else(|| broken("Missing mode frequency"))?;
            frequencies.push(number(f)?);
            i += 4;
            let mut rows = Vec::new();
            while i < ls.len() && !ls[i].trim().is_empty() && !ls[i].starts_with("===") {
                rows.push(numbers(ls[i])?);
                i += 1;
            }
            frames.push(NamedFrame::new(
                format!("Mode_{}", frames.len() + 1),
                table(
                    &names("span_loc|s-s disp|s-s slope|f-a disp|f-a slope|twist"),
                    &rows,
                )?,
            ));
        } else {
            i += 1;
        }
    }
    Ok(Arc::new(ParsedFile {
        path: path.into(),
        format: "BModes output file",
        frames,
        fields: None,
        description: format!("Frequencies [Hz]: {frequencies:?}"),
    }))
}
