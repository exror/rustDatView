use super::common::*;
use dv_core::{FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};

pub fn ae(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    ae_read(path).map_err(|e| match e {
        IoError::BrokenFormat(s) => wrong(s),
        e => e,
    })
}
fn ae_read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut l = Lines::new(&s);
    let n = count(l.next()?).map_err(|_| wrong("Not an AE file"))?;
    let mut frames = Vec::new();
    for _ in 0..n {
        let h: Vec<_> = l.next()?.split_whitespace().collect();
        if h.len() < 2 {
            return Err(wrong("AE set header needs label and row count"));
        }
        let id = count(h[0])?;
        let nr = count(h[1])?;
        let rows = l.rows(nr, 4)?;
        frames.push(NamedFrame::new(
            if n == 1 {
                String::new()
            } else {
                format!("ae_set_{id}")
            },
            table(
                &names("radius_[m]|chord_[m]|thickness_[%]|pc_set_[#]"),
                &rows,
            )?,
        ));
    }
    if frames.is_empty() {
        return Err(wrong("Empty AE sets"));
    }
    Ok(object(path, "HAWC2 AE file", frames))
}
pub fn pc(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    pc_read(path).map_err(|e| match e {
        IoError::BrokenFormat(s) => wrong(s),
        e => e,
    })
}
fn pc_read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut l = Lines::new(&s);
    let n = count(l.next()?).map_err(|_| wrong("Not a PC file"))?;
    let mut frames = Vec::new();
    for set in 1..=n {
        let np = count(l.next()?)?;
        for _ in 0..np {
            let h: Vec<_> = l.next()?.split_whitespace().collect();
            if h.len() < 3 {
                return Err(wrong("PC polar header needs index, count and thickness"));
            }
            let nr = count(h[1])?;
            let t = number(h[2])?;
            let rows = l.rows(nr, 4)?;
            frames.push(NamedFrame::new(
                format!("pc_set_{set}_t_{t:.1}"),
                table(&names(POLAR), &rows)?,
            ));
        }
    }
    if frames.is_empty() {
        return Err(wrong("Empty PC sets"));
    }
    Ok(object(path, "HAWC2 PC file", frames))
}
const REG: &str="r_[m]|m_[kg/m]|x_cg_[m]|y_cg_[m]|ri_x_[m]|ri_y_[m]|x_sh_[m]|y_sh_[m]|E_[N/m^2]|G_[N/m^2]|I_x_[m^4]|I_y_[m^4]|I_p_[m^4]|k_x_[-]|k_y_[-]|A_[m^2]|pitch_[deg]|x_e_[m]|y_e_[m]";
const FPM: &str="r_[m]|m_[kg/m]|x_cg_[m]|y_cg_[m]|ri_x_[m]|ri_y_[m]|pitch_[deg]|x_e_[m]|y_e_[m]|K11|K12|K13|K14|K15|K16|K22|K23|K24|K25|K26|K33|K34|K35|K36|K44|K45|K46|K55|K56|K66";
pub fn st_frames(path: &Path, extra: bool) -> Result<Vec<NamedFrame>, IoError> {
    let s = text(path)?;
    let ls: Vec<_> = s.lines().collect();
    if !ls.iter().take(13).any(|s| s.starts_with('#'))
        || !ls.iter().take(13).any(|s| s.starts_with('$'))
    {
        return Err(wrong("Not a HAWC2 structural file"));
    }
    let mut main = 0;
    let mut frames = Vec::new();
    let mut i = 0;
    while i < ls.len() {
        let line = ls[i].trim();
        i += 1;
        if let Some(h) = line.strip_prefix('#') {
            main = count(h)?;
        } else if let Some(h) = line.strip_prefix('$') {
            let h: Vec<_> = h.split_whitespace().collect();
            if h.len() < 2 {
                return Err(broken("Invalid structural subset"));
            }
            let sub = count(h[0])?;
            let n = count(h[1])?;
            let end = i
                .checked_add(n)
                .ok_or_else(|| broken("Invalid row count"))?;
            if end > ls.len() {
                return Err(broken("Truncated structural subset"));
            }
            let mut rows: Vec<Vec<f64>> = ls[i..end]
                .iter()
                .map(|s| numbers(s.split(';').next().unwrap_or("")))
                .collect::<Result<_, _>>()?;
            i = end;
            let width = rows
                .first()
                .ok_or_else(|| broken("Empty structural subset"))?
                .len();
            if width != 19 && width != 30 {
                return Err(broken("Structural file needs 19 or 30 columns"));
            }
            if rows.iter().any(|r| r.len() != width) {
                return Err(broken("Inconsistent structural columns"));
            }
            let mut cols = names(if width == 19 { REG } else { FPM });
            if extra {
                cols.extend(names("Ixi_[kg.m]|Iyi_[kg.m]|StrcTwst_[deg]"));
                if width == 19 {
                    cols.extend(names("EIx_[Nm^2]|EIy_[Nm^2]|GKt_[Nm^2]|EA_[N]|GA_[N]"));
                }
                cols.push("r_bar_[-]".into());
                let rmax = rows.last().unwrap()[0];
                for r in &mut rows {
                    if r.len() != width {
                        return Err(broken("Inconsistent structural columns"));
                    }
                    let ix = r[4] * r[4] * r[1];
                    let iy = r[5] * r[5] * r[1];
                    let twist = -r[if width == 19 { 16 } else { 6 }];
                    let derived = if width == 19 {
                        vec![
                            r[8] * r[10],
                            r[8] * r[11],
                            r[9] * r[12],
                            r[8] * r[15],
                            r[9] * r[15],
                        ]
                    } else {
                        Vec::new()
                    };
                    let rb = r[0] / rmax;
                    r.extend([ix, iy, twist]);
                    r.extend(derived);
                    r.push(rb);
                }
            }
            frames.push(NamedFrame::new(
                format!("{main}_{sub}"),
                table(&cols, &rows)?,
            ));
        }
    }
    Ok(frames)
}
pub fn st(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    Ok(object(path, "HAWC2 st file", st_frames(path, true)?))
}
fn slice(s: &str, a: usize, b: usize) -> &str {
    s.get(a..b.min(s.len())).unwrap_or("").trim()
}
fn channel_name(name: &str, unit: &str, desc: &str) -> String {
    let mut n = name
        .replace(' ', "")
        .replace("coo:global", "g")
        .replace("Statepos", "")
        .replace("axisangle", "rot_");
    let get = |pat: &str| {
        regex::Regex::new(pat)
            .unwrap()
            .captures(desc)
            .map(|c| c[1].to_string())
    };
    let mut pref = String::new();
    if let Some(body) = get(r"Mbdy:([-a-zA-Z0-9_.]*) ") {
        n = n.replace(&format!("coo:{body}"), "b");
        pref.push_str(&body);
    }
    if let (Some(elem), Some(z)) = (get(r"E-nr:\s*(\d+)"), get(r"Z-rel:\s*(\d+.\d+)")) {
        if let (Ok(e), Ok(z)) = (elem.parse::<f64>(), z.parse::<f64>()) {
            pref.push_str(&format!("N{}", e + z));
        }
    }
    if get(r"s=\s*(\d+.\d+)\[m\]").is_some() {
        if let Some(s) = get(r"s/S=\s*(\d+.\d+)") {
            pref.push_str(&format!("r{s}"));
        }
    }
    if let Some(node) = get(r"nodenr:\s*(\d+)") {
        pref.push_str(&format!("N{node}"));
    }
    format!("{pref}{n}_[{}]", unit.replace(['(', ')', '[', ']'], ""))
}
pub fn dat(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let sel = path.with_extension("sel");
    if !sel.is_file() {
        return Err(wrong("No HAWC2 selection file"));
    }
    let s = text(&sel)?;
    let ls: Vec<_> = s.lines().collect();
    let bh = ls
        .first()
        .is_some_and(|s| s.to_lowercase().contains("bhawc"));
    let hi = if bh {
        ls.iter()
            .position(|s| s.to_lowercase().contains("scans"))
            .map(|i| i + 1)
            .ok_or_else(|| wrong("Missing scans"))?
    } else {
        8
    };
    let h: Vec<_> = ls
        .get(hi)
        .ok_or_else(|| wrong("Missing selection header"))?
        .split_whitespace()
        .collect();
    if h.len() < 3 {
        return Err(wrong("Invalid HAWC2 selection header"));
    }
    let nr = count(h[0])?;
    let nc = count(h[1])?;
    let binary = !bh && h.get(3).is_some_and(|s| s.eq_ignore_ascii_case("binary"));
    let ci = if bh {
        ls.iter()
            .position(|s| s.to_lowercase().contains("channel"))
            .map(|i| i + 1)
            .ok_or_else(|| broken("Missing channels"))?
    } else {
        12
    };
    let mut cols = Vec::new();
    for line in ls.iter().skip(ci).filter(|s| !s.trim().is_empty()).take(nc) {
        if bh {
            let sp: Vec<_> = line.split_whitespace().collect();
            if sp.len() < 4 {
                return Err(broken("Invalid BHAWC channel"));
            }
            let unit = sp.last().unwrap();
            let end = line.rfind(unit).unwrap();
            let rest = line[line.find(sp[0]).unwrap() + sp[0].len()..end].trim();
            let halves: Vec<_> = regex::Regex::new(r"\t+| {2,}")
                .unwrap()
                .split(rest)
                .filter(|s| !s.trim().is_empty())
                .collect();
            if halves.len() != 2 {
                return Err(broken("Invalid BHAWC channel description/name"));
            }
            cols.push(channel_name(halves[1].trim(), unit, halves[0].trim()));
        } else {
            cols.push(channel_name(
                slice(line, 12, 43),
                slice(line, 43, 54),
                slice(line, 54, line.len()),
            ));
        }
    }
    if cols.len() != nc {
        return Err(broken("Truncated channel metadata"));
    }
    let data = path.with_extension("dat");
    let df = if binary {
        let b = std::fs::read(data)?;
        if b.len()
            != nr
                .checked_mul(nc)
                .and_then(|n| n.checked_mul(2))
                .ok_or_else(|| broken("Invalid result dimensions"))?
        {
            return Err(broken("Binary HAWC2 size mismatch"));
        }
        let mut values = Vec::new();
        for c in 0..nc {
            let scale = number(
                ls.get(12 + nc + 2 + c)
                    .ok_or_else(|| broken("Missing scale factor"))?
                    .trim(),
            )?;
            values.push(
                b[c * nr * 2..(c + 1) * nr * 2]
                    .chunks_exact(2)
                    .map(|v| i16::from_le_bytes(v.try_into().unwrap()) as f64 * scale)
                    .collect(),
            );
        }
        dv_core::DataFrame::from_floats(cols, values).map_err(|e| broken(e.to_string()))?
    } else {
        let t = text(&data)?;
        let rows = numeric_lines(t.lines())?;
        if rows.len() != nr {
            return Err(broken("ASCII HAWC2 scan count mismatch"));
        }
        table(&cols, &rows)?
    };
    Ok(object(
        path,
        "HAWC2 dat file",
        vec![NamedFrame::unnamed(df)],
    ))
}
pub fn pwr(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    stab(path, false)
}
pub fn ind(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    stab(path, true)
}
fn stab(path: &Path, ind: bool) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    stab_text(path, &s, ind)
}
fn stab_text(path: &Path, s: &str, ind: bool) -> Result<Arc<dyn FileObject>, IoError> {
    let mut ls = s.lines();
    let h = ls.next().unwrap_or("").trim_start();
    if !h.starts_with('#') {
        return Err(wrong("Missing HAWCStab2 header"));
    }
    let h = if ind {
        format!("00{}", h[1..].trim())
    } else {
        format!("0 {}", h[1..].trim())
    };
    let mut cols = Vec::new();
    for part in h.split(']').filter(|p| p.contains('[')) {
        let c = if ind {
            part.trim().get(2..).unwrap_or("").trim().to_string()
        } else {
            part.trim()
                .split_whitespace()
                .skip(1)
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut c = format!("{}]", c.replace(' ', "_"));
        if ind {
            c = c.replace('[', "_[").replace("__", "_");
        }
        cols.push(c);
    }
    if (!ind && cols.len() != 15) || (ind && ![38, 14, 18].contains(&cols.len())) {
        return Err(wrong("Not the requested HAWCStab2 table"));
    }
    let rows = numeric_lines(ls)?;
    Ok(object(
        path,
        if ind {
            "HAWCStab2 induction file"
        } else {
            "HAWCStab2 power file"
        },
        vec![NamedFrame::unnamed(table(&cols, &rows)?)],
    ))
}
pub fn cmb(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let mut l = s.lines();
    let h = l.next().ok_or_else(|| wrong("No Campbell header"))?;
    let n = h
        .split_whitespace()
        .last()
        .ok_or_else(|| wrong("Missing mode count"))?
        .parse::<usize>()
        .map_err(|_| wrong("Invalid mode count"))?;
    let rows = numeric_lines(l)?;
    let width = rows
        .first()
        .ok_or_else(|| broken("Empty Campbell data"))?
        .len();
    if n == 0 || width <= 1 || (width - 1) % n != 0 || (width - 1) / n > 3 {
        return Err(broken("Campbell column count mismatch"));
    }
    let mut cols = vec!["Wind_[m/s]".into()];
    for group in ["F", "D", "R"].iter().take((width - 1) / n) {
        for i in 1..=n {
            cols.push(format!("{group}{i}"));
        }
    }
    Ok(object(
        path,
        "HAWCStab2 Campbell file",
        vec![NamedFrame::unnamed(table(&cols, &rows)?)],
    ))
}

#[derive(Default)]
struct Body {
    name: String,
    sections: Vec<Vec<f64>>,
    structural: Option<String>,
    set: String,
}
fn expanded_htc(path: &Path, stack: &mut Vec<std::path::PathBuf>) -> Result<String, IoError> {
    let canonical = std::fs::canonicalize(path)?;
    if stack.contains(&canonical) {
        return Err(broken("Recursive HTC include"));
    }
    stack.push(canonical);
    let s = text(path)?;
    let mut result = String::new();
    for raw in s.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        let sp = quoted(line);
        if sp
            .first()
            .is_some_and(|s| s.eq_ignore_ascii_case("continue_in_file"))
        {
            let file = sp
                .get(1)
                .ok_or_else(|| broken("Missing HTC include filename"))?;
            result.push_str(&expanded_htc(
                &path.parent().unwrap_or(Path::new(".")).join(file),
                stack,
            )?);
        } else {
            result.push_str(raw);
            result.push('\n');
        }
    }
    stack.pop();
    Ok(result)
}
pub fn htc(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = expanded_htc(path, &mut Vec::new())?;
    let mut sections = Vec::<String>::new();
    let mut bodies = Vec::<Body>::new();
    let mut body = None;
    for raw in s.lines() {
        let code = raw.split(';').next().unwrap_or("");
        let sp = quoted(code);
        if sp.is_empty() {
            continue;
        }
        match sp[0].to_lowercase().as_str() {
            "begin" => {
                let key = sp
                    .get(1)
                    .ok_or_else(|| broken("Missing HTC section name"))?
                    .to_lowercase();
                if key == "main_body" {
                    body = Some(Body {
                        set: "1_1".into(),
                        ..Body::default()
                    });
                }
                sections.push(key);
            }
            "end" => {
                let key = sections.pop().ok_or_else(|| broken("Unbalanced HTC end"))?;
                if key == "main_body" {
                    if let Some(b) = body.take() {
                        bodies.push(b);
                    }
                }
            }
            "name" => {
                if sections.last().is_some_and(|s| s == "main_body") {
                    if let Some(b) = body.as_mut() {
                        b.name = sp.get(1).cloned().unwrap_or_default();
                    }
                }
            }
            "sec" => {
                if sections.last().is_some_and(|s| s == "c2_def") {
                    if let Some(b) = body.as_mut() {
                        b.sections.push(
                            sp[1..]
                                .iter()
                                .map(|s| number(s))
                                .collect::<Result<_, _>>()?,
                        );
                    }
                }
            }
            "filename" => {
                if sections.last().is_some_and(|s| s == "timoschenko_input") {
                    if let Some(b) = body.as_mut() {
                        b.structural = sp.get(1).cloned();
                    }
                }
            }
            "set" => {
                if sections.last().is_some_and(|s| s == "timoschenko_input") {
                    if let Some(b) = body.as_mut() {
                        if sp.len() < 3 {
                            return Err(broken("HTC set requires two indices"));
                        }
                        b.set = format!("{}_{}", sp[1], sp[2]);
                    }
                }
            }
            _ => {}
        }
    }
    if !sections.is_empty() {
        return Err(broken("Unclosed HTC sections"));
    }
    let mut frames = Vec::new();
    for mut b in bodies {
        if b.sections.is_empty() {
            continue;
        }
        if b.sections.iter().any(|r| r.len() != 5) {
            return Err(broken("HTC c2 sec needs five values"));
        }
        b.sections.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let rows: Vec<_> = b
            .sections
            .iter()
            .map(|r| vec![r[3], r[1], r[2], r[4]])
            .collect();
        frames.push(NamedFrame::new(
            format!("{}_c2", b.name),
            table(&names("z_[m]|x_[m]|y_[m]|twist_[deg]"), &rows)?,
        ));
        if let Some(file) = b.structural {
            let dir = path.parent().unwrap_or(Path::new("."));
            let mut p = dir.join(&file);
            if !p.exists() {
                p = dir.join("..").join(&file);
            }
            if p.exists() {
                let mut sets = st_frames(&p, false)?;
                let pos = sets
                    .iter()
                    .position(|f| f.name == b.set)
                    .ok_or_else(|| broken(format!("Missing structural set {}", b.set)))?;
                let frame = sets.swap_remove(pos);
                frames.push(NamedFrame::new(format!("{}_st", b.name), frame.df));
            }
        }
    }
    Ok(object(path, "HAWC2 htc file", frames))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn power_header_allows_leading_whitespace_and_preserves_units() {
        let text=" # V [m/s] 1 P [kW] 2 T [kN] 3 Cp [-] 4 Ct [-] 5 Pitch Q [Nm] 6 Flap M [kNm] 7 Edge M [kNm] 8 Pitch [deg] 9 Speed [rpm] 10 Tip x [m] 11 Tip y [m] 12 Tip z [m] 13 J_rot [kg*m^2] 14 J_DT [kg*m^2] 15\n3 54 222 0.07 0.90 -76738 -6556 44 3.67 5 -0.28 -12 119 310109414 318118065\n";
        let frames = stab_text(Path::new("power.pwr"), text, false)
            .unwrap()
            .to_dataframes()
            .unwrap();
        let frame = &frames[0].df;
        assert_eq!(
            frame
                .columns()
                .iter()
                .find(|c| c.name == "Pitch_[deg]")
                .unwrap()
                .floats()
                .unwrap(),
            [3.67]
        );
        assert_eq!(
            frame
                .columns()
                .iter()
                .find(|c| c.name == "Pitch_Q_[Nm]")
                .unwrap()
                .floats()
                .unwrap(),
            [-76738.]
        );
        assert_eq!(
            frame
                .columns()
                .iter()
                .find(|c| c.name == "J_DT_[kg*m^2]")
                .unwrap()
                .floats()
                .unwrap(),
            [318118065.]
        );
    }
}
