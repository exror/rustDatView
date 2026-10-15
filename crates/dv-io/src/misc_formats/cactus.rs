use super::common::*;
use dv_core::{DataFrame, FileObject, IoError, NamedFrame};
use std::{collections::BTreeMap, path::Path, sync::Arc};
struct Csv {
    names: Vec<String>,
    rows: Vec<Vec<f64>>,
}
impl Csv {
    fn col(&self, name: &str) -> Result<Vec<f64>, IoError> {
        let i = self
            .names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| broken(format!("Missing CACTUS channel {name}")))?;
        Ok(self.rows.iter().map(|r| r[i]).collect())
    }
    fn selected(&self, blade: f64, element: f64) -> Result<Self, IoError> {
        let b = self
            .names
            .iter()
            .position(|n| n == "Blade")
            .ok_or_else(|| broken("Missing Blade channel"))?;
        let e = self
            .names
            .iter()
            .position(|n| n == "Element")
            .ok_or_else(|| broken("Missing Element channel"))?;
        Ok(Self {
            names: self.names.clone(),
            rows: self
                .rows
                .iter()
                .filter(|r| r[b] == blade && r[e] == element)
                .cloned()
                .collect(),
        })
    }
}
fn csv(path: &Path) -> Result<Csv, IoError> {
    let s = text(path)?;
    let mut l = s.lines();
    let names = split_csv(l.next().ok_or_else(|| broken("Missing CSV header"))?);
    let rows = l
        .filter(|s| !s.trim().is_empty())
        .map(|s| split_csv(s).iter().map(|s| number(s)).collect())
        .collect::<Result<Vec<Vec<f64>>, IoError>>()?;
    if rows.iter().any(|r| r.len() != names.len()) {
        return Err(broken("CACTUS CSV width mismatch"));
    }
    Ok(Csv { names, rows })
}
fn split_csv(s: &str) -> Vec<String> {
    let mut q = false;
    let mut out = Vec::new();
    let mut word = String::new();
    for c in s.chars() {
        if c == '"' {
            q = !q;
        } else if c == ',' && !q {
            out.push(word.trim().to_string());
            word.clear();
        } else {
            word.push(c);
        }
    }
    out.push(word.trim().to_string());
    out
}
fn nml(s: &str) -> Result<BTreeMap<String, String>, IoError> {
    let mut m = BTreeMap::new();
    let cleaned = s
        .lines()
        .map(|s| s.split('!').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    if !cleaned.to_lowercase().contains("&configinputs")
        || !cleaned.to_lowercase().contains("&caseinputs")
    {
        return Err(wrong("Not a CACTUS namelist"));
    }
    let re =
        regex::Regex::new(r#"(?i)([a-z][a-z0-9_]*)\s*=\s*('[^']*'|"[^"]*"|[^,\s/]+)"#).unwrap();
    for c in re.captures_iter(&cleaned) {
        m.insert(c[1].to_lowercase(), c[2].trim_matches(['\'', '"']).into());
    }
    Ok(m)
}
pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let s = text(path)?;
    let config = nml(&s)?;
    let get = |key: &str| {
        config
            .get(key)
            .map(String::as_str)
            .ok_or_else(|| broken(format!("Missing CACTUS input {key}")))
    };
    let num = |key: &str| number(get(key)?);
    let rpm = num("rpm")?;
    let tsr = num("ut")?;
    let omega = rpm * std::f64::consts::PI / 30.0;
    let nti = num("nti")?;
    let nr = num("nr")?;
    if omega == 0.0 || tsr == 0.0 || nti <= 0.0 || nr <= 0.0 {
        return Err(broken("Invalid CACTUS rotation/time configuration"));
    }
    let dt = 2.0 * std::f64::consts::PI / omega / nti;
    let dir = path.parent().unwrap_or(Path::new("."));
    let geometry = dir.join(get("geomfilepath")?);
    let mut geom = BTreeMap::new();
    let mut geometry_found = false;
    if geometry.is_file() {
        geometry_found = true;
        for l in text(&geometry)?.lines().take(11) {
            if let Some((k, v)) = l.split('!').next().unwrap_or("").split_once(':') {
                geom.insert(k.trim().to_lowercase(), v.trim().to_string());
            }
        }
    }
    let r = if geometry_found {
        number(
            geom.get("refr")
                .ok_or_else(|| broken("Missing CACTUS geometry refr"))?,
        )? / 3.28084
    } else {
        1.0
    };
    let area = if geometry_found {
        number(
            geom.get("refar")
                .ok_or_else(|| broken("Missing CACTUS geometry refar"))?,
        )? * r
            * r
    } else {
        2.0 * r * r
    };
    let u = omega * r / tsr;
    let rho = num("rho")? * 1.2 / 0.002328;
    let f0 = 0.5 * rho * area * u * u;
    let q0 = f0 * r;
    let p0 = f0 * u;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let output = dir.join("output");
    let time_path = output.join(format!("{stem}_TimeData.csv"));
    let elem_path = output.join(format!("{stem}_ElementData.csv"));
    let ds_path = output.join(format!("{stem}_DSData.csv"));
    let mut frames = Vec::new();
    let mut cols = Vec::new();
    let mut values = Vec::new();
    let mut n = 0;
    if time_path.is_file() {
        let mut time = csv(&time_path)?;
        n = time.rows.len();
        if n > (nr * nti) as usize {
            return Err(broken("CACTUS output exceeds configured timesteps"));
        }
        let nb = time
            .names
            .iter()
            .filter(|s| s.as_str() == "Blade Fx Coeff. (-)")
            .count();
        if time.names.len() != 8 + 4 * nb {
            return Err(broken("Unexpected CACTUS TimeData columns"));
        }
        for b in 0..nb {
            for (j, key) in ["Fx", "Fy", "Fz", "Torque"].iter().enumerate() {
                time.names[8 + b * 4 + j] = format!("Blade{} {key} Coeff. (-)", b + 1);
            }
        }
        let cp = time.col("Power Coeff. (-)")?;
        let cq = time.col("Torque Coeff. (-)")?;
        let fx = time.col("Fx Coeff. (-)")?;
        let fy = time.col("Fy Coeff. (-)")?;
        let fz = time.col("Fz Coeff. (-)")?;
        let mut add = |key: &str, v: Vec<f64>| {
            cols.push(key.to_string());
            values.push(v);
        };
        let t: Vec<_> = (0..n).map(|i| i as f64 * dt).collect();
        let theta: Vec<_> = t.iter().map(|t| t * omega).collect();
        add("Time_[s]", t);
        add(
            "Azimuth_[deg]",
            theta
                .iter()
                .map(|p| p.to_degrees().rem_euclid(360.0))
                .collect(),
        );
        add("HWindSpeedX_[m/s]", vec![u; n]);
        add("RtAeroCp_[-]", cp.clone());
        add("RtAeroCq_[-]", cq.clone());
        for (key, v, scale) in [
            ("ZAeroFxg_[N]", &fx, f0),
            ("ZAeroFyg_[N]", &fz, -f0),
            ("ZAeroFzg_[N]", &fy, f0),
            ("ZAeroMzg_[N-m]", &cq, q0),
            ("RtAeroFxh_[N]", &fy, f0),
        ] {
            add(key, v.iter().map(|v| v * scale).collect());
        }
        add(
            "RtAeroFyh_[N]",
            (0..n)
                .map(|i| (theta[i].cos() * fx[i] - theta[i].sin() * fz[i]) * f0)
                .collect(),
        );
        add(
            "RtAeroFzh_[N]",
            (0..n)
                .map(|i| -(theta[i].cos() * fz[i] + theta[i].sin() * fx[i]) * f0)
                .collect(),
        );
        add("RtAeroMxh_[N-m]", cq.iter().map(|v| v * q0).collect());
        let mut power: Vec<_> = cp.iter().map(|v| v * p0).collect();
        if n > 1 {
            power[0] = power[1];
        }
        add("RtAeroPwr_[W]", power);
        add("RotSpeed_[rpm]", vec![rpm; n]);
        for b in 1..=nb {
            let bx = time.col(&format!("Blade{b} Fx Coeff. (-)"))?;
            let by = time.col(&format!("Blade{b} Fy Coeff. (-)"))?;
            let bz = time.col(&format!("Blade{b} Fz Coeff. (-)"))?;
            let torque = time.col(&format!("Blade{b} Torque Coeff. (-)"))?;
            let azi = (b - 1) as f64 * 2.0 * std::f64::consts::PI / nb as f64;
            add(
                &format!("Y{b}AeroFxb_[N]"),
                (0..n)
                    .map(|i| {
                        -((theta[i] - azi).cos() * bz[i] + (theta[i] - azi).sin() * bx[i]) * f0
                    })
                    .collect(),
            );
            add(
                &format!("Y{b}AeroFyb_[N]"),
                (0..n)
                    .map(|i| ((theta[i] - azi).cos() * bx[i] - (theta[i] - azi).sin() * bz[i]) * f0)
                    .collect(),
            );
            add(
                &format!("Y{b}AeroFzb_[N]"),
                by.iter().map(|v| -v * f0).collect(),
            );
            add(
                &format!("Y{b}AeroMxb_[N-m]"),
                torque.iter().map(|v| v * q0).collect(),
            );
            let mut p: Vec<_> = torque.iter().map(|v| v * q0 * omega).collect();
            if n > 1 {
                p[0] = p[1];
            }
            add(&format!("Y{b}AeroPwr_[W]"), p);
            for (key, v, scale) in [
                ("Fxg_[N]", &bx, f0),
                ("Fyg_[N]", &bz, -f0),
                ("Fzg_[N]", &by, f0),
                ("Mzg_[N-m]", &torque, q0),
            ] {
                add(
                    &format!("Z{b}Aero{key}"),
                    v.iter().map(|v| v * scale).collect(),
                );
            }
        }
        add("Wind1VelX_[m/s]", vec![u; n]);
        if elem_path.is_file() {
            let elem = csv(&elem_path)?;
            let ds = if ds_path.is_file() {
                Some(csv(&ds_path)?)
            } else {
                None
            };
            append_elements(
                &elem,
                ds.as_ref(),
                nb,
                n,
                dt,
                omega,
                u,
                r,
                &mut cols,
                &mut values,
            )?;
        }
        cols.extend(time.names);
        for c in 0..time.rows.first().map(|r| r.len()).unwrap_or(0) {
            values.push(time.rows.iter().map(|r| r[c]).collect());
        }
        let df = DataFrame::from_floats(cols, values).map_err(|e| broken(e.to_string()))?;
        frames.push(NamedFrame::unnamed(df));
    } else {
        if elem_path.is_file() {
            let e = csv(&elem_path)?;
            frames.push(NamedFrame::new("ElementData", table(&e.names, &e.rows)?));
        }
        if ds_path.is_file() {
            let d = csv(&ds_path)?;
            frames.push(NamedFrame::new("DSData", table(&d.names, &d.rows)?));
        }
    }
    Ok(Arc::new(ParsedFile{path:path.into(),format:"CACTUS file",frames,fields:None,description:format!("RPM: {rpm}\nTSR: {tsr}\nR: {r}\nU: {u}\ndt: {dt}\nTime rows: {n}\nGeometry loaded: {geometry_found}")}))
}
#[allow(clippy::too_many_arguments)]
fn append_elements(
    e: &Csv,
    ds: Option<&Csv>,
    nb: usize,
    n: usize,
    dt: f64,
    omega: f64,
    u: f64,
    r: f64,
    cols: &mut Vec<String>,
    values: &mut Vec<Vec<f64>>,
) -> Result<(), IoError> {
    let blades = e.col("Blade")?;
    let elements = e.col("Element")?;
    let mut pairs: Vec<_> = blades.into_iter().zip(elements).collect();
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    pairs.dedup();
    for (b, el) in pairs {
        if b < 1.0 || b > nb as f64 {
            return Err(broken("Invalid CACTUS blade index"));
        }
        let sec = e.selected(b, el)?;
        if sec.rows.len() != n {
            return Err(broken("CACTUS element time length mismatch"));
        }
        let prefix = format!("AB{}N{:03}", b as usize, el as usize);
        let mut add = |key: &str, v: Vec<f64>| {
            cols.push(format!("{prefix}{key}"));
            values.push(v);
        };
        for (output, input, scale) in [
            ("Alpha_[deg]", "AOA25 (deg)", -1.0),
            ("Alpha50_[deg]", "AOA50 (deg)", -1.0),
            ("Alpha75_[deg]", "AOA75 (deg)", -1.0),
            ("Cl_[-]", "CL (-)", -1.0),
            ("Cd_[-]", "CD (-)", 1.0),
            ("Cm_[-]", "CM25 (-)", -1.0),
            ("Cn_[-]", "CN (-)", -1.0),
            ("Ct_[-]", "CT (-)", -1.0),
            ("Cxg_[-]", "Fx (-)", 1.0),
            ("Cyg_[-]", "Fz (-)", -1.0),
            ("Czg_[-]", "Fy (-)", 1.0),
            ("ClC_[-]", "CLCirc (-)", -1.0),
            ("Re_[-]", "Re (-)", 1e-6),
            ("Gam_[m^2/s]", "GB (?)", -u * r),
            ("Vrel_[m/s]", "Ur (-)", u),
        ] {
            add(output, sec.col(input)?.iter().map(|v| v * scale).collect());
        }
        let cl = sec.col("CL (-)")?;
        let cd = sec.col("CD (-)")?;
        let alpha = sec.col("AOA25 (deg)")?;
        add(
            "Ct2_[-]",
            (0..n)
                .map(|i| cl[i] * alpha[i].to_radians().sin() - cd[i] * alpha[i].to_radians().cos())
                .collect(),
        );
        let ix = sec.col("IndU (-)")?;
        let iz = sec.col("IndW (-)")?;
        let azi = (b - 1.0) * 2.0 * std::f64::consts::PI / nb as f64;
        add(
            "Vindx_[m/s]",
            (0..n)
                .map(|i| {
                    let p = i as f64 * dt * omega - azi;
                    -(p.cos() * iz[i] + p.sin() * ix[i]) * u
                })
                .collect(),
        );
        add(
            "Vindy_[m/s]",
            (0..n)
                .map(|i| {
                    let p = i as f64 * dt * omega - azi;
                    (p.cos() * ix[i] - p.sin() * iz[i]) * u
                })
                .collect(),
        );
        if let Some(ds) = ds {
            let d = ds.selected(b, el)?;
            if d.rows.len() != n {
                return Err(broken("CACTUS DS time length mismatch"));
            }
            for (out, input, scale) in [
                ("alpha_34_[deg]", "alpha (deg)", -1.0),
                ("alphaE_[deg]", "alrefL (deg)", -1.0),
                ("alphaED_[deg]", "alrefD (deg)", -1.0),
                ("adotnorm_[-]", "adotnorm (-)", -1.0),
            ] {
                add(out, d.col(input)?.iter().map(|v| v * scale).collect());
            }
            for (out, input, scale) in [
                ("activeL_[-]", "DynamicFlagL", 1.0),
                ("activeD_[-]", "DynamicFlagD", 1.0),
                ("alphaLagD_[deg]", "alphaLagD (deg)", -1.0),
                ("delP_[-]", "delN", 1.0),
                ("delN_[-]", "delP", 1.0),
                ("transA_[-]", "transA", 1.0),
                ("gammaL_[-]", "gammaL", 1.0),
                ("gammaD_[-]", "gammaD", 1.0),
                ("dalphaL_[deg]", "dalphaL (deg)", -1.0),
                ("Tu_[s]", "Tu", 1.0),
                ("alphaDot_[rad/s]", "alphadot", -1.0),
            ] {
                if d.names.iter().any(|s| s == input) {
                    add(out, d.col(input)?.iter().map(|v| v * scale).collect());
                }
            }
            if d.names.iter().any(|s| s == "DynamicFlagL") {
                let a = d.col("alpha (deg)")?;
                let refd = d.col("alrefD (deg)")?;
                add(
                    "dalphaD_[deg]",
                    a.iter().zip(refd).map(|(a, b)| b - a).collect(),
                );
                add(
                    "alphaDot2_[rad/s]",
                    (0..n)
                        .map(|i| if i == 0 { 0.0 } else { -(a[i] - a[i - 1]) })
                        .collect(),
                );
            }
        }
    }
    Ok(())
}
