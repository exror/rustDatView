use super::common::*;
use dv_core::{FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};
const BLADE: &str="r_[m]|EIFlap_[Nm2]|EIEdge_[Nm2]|GKt_[Nm2]|Mass_[kg/m]|Jxx_[kg.m]|PreBendFlap_[m]|PreBendEdge_[m]|Str.Twist_[deg]|PhiOut_[deg]|Ycog_[m]|Yshc_[m]|CalcOutput_[0/1]|Chord_[m]|AeroTwist_[deg]|RelThickness_[%]|AeroCenter_[m]|AeroTorsion_[0/1]|ProfileSet_[#]";
pub fn blade(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut l = Lines::new(&s);
    let first = l.next()?;
    let n = count(l.next()?).map_err(|_| wrong("Not a FLEX blade file"))?;
    let version = first
        .get(1..4)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let indices: &[usize] = match version {
        0 | 1 => &[0, 1, 2, 4, 8, 12, 6, 7],
        2 => &[0, 1, 2, 4, 8, 12, 6, 7, 3],
        3 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        _ => return Err(broken("Unsupported FLEX blade version")),
    };
    let structure = l
        .rows(n, indices.len())
        .map_err(|_| wrong("Not FLEX blade structural rows"))?;
    let beta = number(
        l.next()?
            .split_whitespace()
            .next()
            .ok_or_else(|| broken("Missing BetaC"))?,
    )?;
    let mut description = format!("Version: {version}\nBetaC: {beta}\n");
    if version == 3 {
        l.next()?;
        for key in ["FlapDamping", "EdgeDamping", "TorsDamping"] {
            description.push_str(&format!("{key}: {}\n", l.next()?));
        }
        l.next()?;
        l.next()?;
    } else {
        description.push_str(&format!("Damping: {}\n", l.next()?));
    }
    let na = match version {
        0 => 5,
        1 => 6,
        _ => 7,
    };
    let aero = l.rows(n, na)?;
    let profile = l.next()?;
    description.push_str(&format!("ProfileFile: {profile}"));
    let rows: Vec<_> = structure
        .iter()
        .zip(aero)
        .map(|(r, a)| {
            let mut out = vec![f64::NAN; 19];
            for (&i, &v) in indices.iter().zip(r) {
                out[i] = v;
            }
            for (i, &v) in a.iter().skip(1).enumerate() {
                out[13 + i] = v;
            }
            out
        })
        .collect();
    Ok(Arc::new(ParsedFile {
        path: path.into(),
        format: "FLEX blade file",
        frames: vec![NamedFrame::unnamed(table(&names(BLADE), &rows)?)],
        fields: None,
        description,
    }))
}
pub fn profile(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut l = Lines::new(&s);
    let mut frames = Vec::new();
    let mut set = 1;
    while !l.done() {
        let header = l.next()?;
        if header.trim().is_empty() && l.done() {
            break;
        }
        let n = count(l.next()?).map_err(|_| wrong("Not a FLEX profile file"))?;
        let thickness = numbers(l.next()?)?;
        if thickness.len() != n {
            return Err(wrong("FLEX profile thickness count mismatch"));
        }
        let na = count(l.next()?)?;
        for t in thickness {
            l.next()?;
            let rows = l.rows(na, 4)?;
            frames.push(NamedFrame::new(
                format!("pc_set_{set}_t_{t:.1}"),
                table(&names(POLAR), &rows)?,
            ));
        }
        set += 1;
    }
    Ok(object(path, "FLEX profile file", frames))
}
pub fn output(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let b = std::fs::read(path)?;
    let nc = i32_at(&b, 76)?;
    if nc <= 0 {
        return Err(wrong("Invalid FLEX sensor count"));
    }
    let nc = nc as usize;
    let vp = 84usize
        .checked_add(
            nc.checked_mul(4)
                .ok_or_else(|| broken("Invalid sensor count"))?,
        )
        .ok_or_else(|| broken("Invalid header size"))?;
    let version = i32_at(&b, vp)?;
    if ![0, 2, 3].contains(&version) {
        return Err(broken(format!("Unsupported FLEX binary version {version}")));
    }
    let t0 = f32_at(&b, vp + 4)?;
    let dt = f32_at(&b, vp + 8)?;
    let data_start = vp + 12 + nc * 4;
    if data_start > b.len() || (b.len() - data_start) % (nc * 2) != 0 {
        return Err(broken("FLEX binary size mismatch"));
    }
    let nt = (b.len() - data_start) / (nc * 2);
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut sensor = None;
    for p in [
        dir.join(format!("{stem}.Sensor")),
        dir.join(format!("Sensor_{stem}")),
        dir.join("sensor"),
    ] {
        if p.is_file() {
            let s = text(&p)?;
            let rows: Vec<_> = s
                .lines()
                .skip(2)
                .filter(|s| !s.trim().is_empty())
                .map(|l| l.split_whitespace().map(str::to_string).collect::<Vec<_>>())
                .collect();
            if rows.len() == nc && rows.iter().all(|r| r.len() >= 7) {
                sensor = Some(rows);
                break;
            }
        }
    }
    let sensor = sensor.ok_or_else(|| broken("FLEX result requires matching sensor metadata"))?;
    let mut cols = vec!["Time_[s]".into()];
    let mut values = vec![(0..nt)
        .map(|i| ((t0 + i as f64 * dt) as f32) as f64)
        .collect()];
    for c in 0..nc {
        cols.push(format!(
            "{}_[{}]",
            sensor[c][6],
            sensor[c][5].replace(['(', ')', '[', ']'], "")
        ));
        let scale = f32_at(&b, vp + 12 + c * 4)?;
        values.push(
            (0..nt)
                .map(|t| {
                    let idx = if version == 3 { c * nt + t } else { t * nc + c };
                    let p = data_start + idx * 2;
                    ((i16::from_le_bytes([b[p], b[p + 1]]) as f32) * (scale as f32)) as f64
                })
                .collect(),
        );
    }
    let df = dv_core::DataFrame::from_floats(cols, values).map_err(|e| broken(e.to_string()))?;
    Ok(object(
        path,
        "FLEX output file",
        vec![NamedFrame::unnamed(df)],
    ))
}
pub fn wavekin(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let ls: Vec<_> = s.lines().collect();
    if ls.len() < 12 {
        return Err(wrong("FLEX WaveKin needs eleven header lines"));
    }
    let water = float_matches(ls[5]);
    if water.len() < 4 {
        return Err(broken("Invalid WaveKin water properties"));
    }
    let displ = float_matches(ls[7]);
    let depth = float_matches(ls[9]);
    let nd = float_matches(ls[6]);
    let nz = float_matches(ls[8]);
    if nd.first().copied() != Some(displ.len() as f64)
        || nz.first().copied() != Some(depth.len() as f64)
    {
        return Err(broken("WaveKin displacement/depth count mismatch"));
    }
    let mut cols = names("Time_[s]|WaveElev_[m]");
    for x in displ {
        for z in &depth {
            cols.push(format!("u_z={:.1}_x={x:.1}_[m/s]", -z * water[0]));
        }
        for z in &depth {
            cols.push(format!("a_z={:.1}_x={x:.1}_[m/s^2]", z * water[0]));
        }
    }
    let rows = numeric_lines(ls.iter().skip(11).copied())?;
    Ok(object(
        path,
        "FLEX WaveKin file",
        vec![NamedFrame::unnamed(table(&cols, &rows)?)],
    ))
}
pub fn doc(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let ls: Vec<_> = s.lines().collect();
    if !ls.first().is_some_and(|s| s.starts_with("#Program")) {
        return Err(wrong("Not a FLEX document"));
    }
    let mut frames = Vec::new();
    let mut i = 1;
    while i < ls.len() {
        let line = ls[i].trim();
        i += 1;
        if !line.starts_with('#') || line.starts_with("###") {
            continue;
        }
        let key = line
            .split_whitespace()
            .next()
            .unwrap()
            .trim_start_matches('#');
        let array = key.contains("ShapeFunction_") && key.ends_with("_Shape")
            || [
                "Tower_SectionData",
                "Foundation_SectionData",
                "Global_Mode1_Shape",
            ]
            .contains(&key);
        if !array {
            continue;
        }
        let h = ls
            .get(i)
            .ok_or_else(|| broken("Missing FLEX array header"))?;
        i += 1;
        let mut rows = Vec::new();
        while i < ls.len() && !ls[i].trim().is_empty() && !ls[i].starts_with('#') {
            rows.push(
                ls[i]
                    .split_whitespace()
                    .map(|v| match v {
                        "Fnd" => Ok(0.0),
                        "Twr" => Ok(1.0),
                        "COG" => Ok(2.0),
                        _ => number(v),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
            i += 1;
        }
        if rows.is_empty() {
            continue;
        }
        let width = rows[0].len();
        let h = h.trim();
        let h = if h
            .split_whitespace()
            .next()
            .is_some_and(|v| v.parse::<usize>().is_ok())
        {
            h.split_once(char::is_whitespace)
                .map(|(_, v)| v)
                .unwrap_or(h)
        } else {
            h
        };
        let mut cols = if h.contains('[') {
            h.replace("rough", "rough_[-]")
                .replace("n ", "n_[-] ")
                .split(']')
                .filter(|s| s.contains('['))
                .map(|s| {
                    format!(
                        "{}]",
                        s.trim()
                            .replace(' ', "_")
                            .replace('[', "_[")
                            .replace("__", "_")
                    )
                })
                .collect::<Vec<_>>()
        } else {
            h.split_whitespace().map(str::to_string).collect()
        };
        if cols.len() != width {
            cols = (0..width).map(|j| format!("C{j}")).collect();
        }
        if key == "Global_Mode1_Shape" && width == 4 {
            for r in &mut rows {
                r.remove(0);
            }
            cols = names("H_[m]|U_FA_[-]|U_SS_[]");
        }
        frames.push(NamedFrame::new(key, table(&cols, &rows)?));
    }
    Ok(object(path, "FLEX documentation file", frames))
}
