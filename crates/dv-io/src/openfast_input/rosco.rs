use super::input::{atom, broken, float, read_text, tokens, InputEntry, InputValue};
use dv_core::{DataFrame, Field2D, FileObject, IoError, NamedFrame};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct RoscoDisconFile {
    pub filename: PathBuf,
    pub entries: Vec<InputEntry>,
}
impl RoscoDisconFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        Self::parse(path, &read_text(path)?)
    }
    pub fn parse(path: &Path, text: &str) -> Result<Self, IoError> {
        let mut entries = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('!') {
                entries.push(InputEntry {
                    label: String::new(),
                    value: InputValue::Text(line.into()),
                    description: String::new(),
                    is_comment: true,
                    dimension: None,
                    table_type: 0,
                });
                continue;
            }
            let (values, comment) = line.split_once('!').ok_or_else(|| {
                IoError::WrongFormat(format!("DISCON missing parameter comment at {}", i + 1))
            })?;
            let t = tokens(values);
            if t.is_empty() {
                return Err(broken("DISCON empty value"));
            }
            let (key, description) = comment
                .trim()
                .split_once(char::is_whitespace)
                .unwrap_or((comment.trim(), ""));
            if key.is_empty() {
                return Err(broken("DISCON missing key"));
            }
            let value = if t.len() == 1 {
                let v = atom(&t[0]);
                if let InputValue::Text(s) = v {
                    InputValue::Text(s.trim_matches(['"', '\'']).into())
                } else {
                    v
                }
            } else {
                let v: Vec<_> = t.iter().map(|s| atom(s)).collect();
                if v.iter().any(|v| v.as_f64().is_none()) {
                    return Err(broken("non-numeric DISCON array"));
                }
                InputValue::List(v)
            };
            entries.push(InputEntry {
                label: key.into(),
                value,
                description: description.trim().into(),
                is_comment: false,
                dimension: None,
                table_type: 0,
            });
        }
        if !entries.iter().any(|e| !e.is_comment) {
            return Err(IoError::WrongFormat("no DISCON parameters".into()));
        }
        Ok(Self {
            filename: path.into(),
            entries,
        })
    }
    pub fn get(&self, key: &str) -> Option<&InputValue> {
        self.entries
            .iter()
            .find(|e| !e.is_comment && e.label.eq_ignore_ascii_case(key))
            .map(|e| &e.value)
    }
    pub fn numbers(&self, key: &str) -> Option<Vec<f64>> {
        match self.get(key)? {
            InputValue::List(v) => v.iter().map(InputValue::as_f64).collect(),
            v => Some(vec![v.as_f64()?]),
        }
    }
    pub fn set(&mut self, key: &str, value: InputValue) -> Result<(), IoError> {
        let e = self
            .entries
            .iter_mut()
            .find(|e| !e.is_comment && e.label.eq_ignore_ascii_case(key))
            .ok_or_else(|| broken(format!("missing DISCON key {key}")))?;
        e.value = value;
        Ok(())
    }
    pub fn to_text(&self) -> String {
        let width = self
            .entries
            .iter()
            .map(|e| e.label.len())
            .max()
            .unwrap_or(0)
            .max(18);
        let mut s = String::new();
        for e in &self.entries {
            if e.is_comment {
                s.push_str(&e.value.to_string());
            } else {
                let value = match &e.value {
                    InputValue::Text(t) => format!("\"{t}\""),
                    InputValue::List(v) => v
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" "),
                    v => v.to_string(),
                };
                s.push_str(&format!(
                    "{value}    ! {:width$} {}",
                    e.label, e.description
                ));
            }
            s.push('\n');
        }
        s
    }
    pub fn write(&self, path: &Path) -> Result<(), IoError> {
        fs::write(path, self.to_text())?;
        Ok(())
    }
    fn schedule(
        &self,
        name: &str,
        keys: &[&str],
        columns: &[&str],
        scale: &[f64],
    ) -> Result<NamedFrame, IoError> {
        let values: Vec<Vec<f64>> = keys
            .iter()
            .zip(scale)
            .map(|(key, scale)| {
                self.numbers(key)
                    .ok_or_else(|| broken(format!("missing DISCON schedule {key}")))
                    .map(|mut v| {
                        for x in &mut v {
                            *x *= scale;
                        }
                        v
                    })
            })
            .collect::<Result<_, _>>()?;
        Ok(NamedFrame::new(
            name,
            DataFrame::from_floats(columns.iter().copied(), values)
                .map_err(|e| broken(e.to_string()))?,
        ))
    }
}
impl FileObject for RoscoDisconFile {
    fn format_name(&self) -> &str {
        "ROSCO DISCON file"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        let mut v = Vec::new();
        if self.get("PC_GS_n").is_some() {
            v.push(self.schedule(
                "PitchSchedule",
                &[
                    "PC_GS_angles",
                    "PC_GS_KP",
                    "PC_GS_KI",
                    "PC_GS_KD",
                    "PC_GS_TF",
                ],
                &["Pitch_[deg]", "KP_[-]", "KI_[s]", "KD_[1/s]", "TF_[-]"],
                &[180.0 / std::f64::consts::PI, 1.0, 1.0, 1.0, 1.0],
            )?);
        }
        if self.get("PS_BldPitchMin_n").is_some() {
            v.push(self.schedule(
                "PitchSaturation",
                &["PS_WindSpeeds", "PS_BldPitchMin"],
                &["WindSpeed_[m/s]", "Pitch_[deg]"],
                &[1.0, 1.0],
            )?);
        }
        if self.get("PRC_n").is_some() {
            v.push(self.schedule(
                "PowerTracking",
                &["PRC_WindSpeeds", "PRC_RotorSpeeds"],
                &["WindSpeed_[m/s]", "RotorSpeed_[rpm]"],
                &[1.0, 30.0 / std::f64::consts::PI],
            )?);
        }
        Ok(v)
    }
}

#[derive(Debug, Clone)]
pub struct RoscoPerformanceFile {
    pub filename: PathBuf,
    pub name: String,
    pub pitch: Vec<f64>,
    pub tsr: Vec<f64>,
    pub wind_speeds: Option<Vec<f64>>,
    /// Coefficient matrices are row-major, shape `(tsr.len(), pitch.len())`.
    pub cp: Vec<f64>,
    pub ct: Vec<f64>,
    pub cq: Vec<f64>,
}
impl RoscoPerformanceFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        Self::parse(path, &read_text(path)?)
    }
    pub fn parse(path: &Path, text: &str) -> Result<Self, IoError> {
        let lines: Vec<_> = text.lines().collect();
        if !lines.iter().take(12).any(|l| l.contains("Pitch angle")) {
            return Err(IoError::WrongFormat("ROSCO pitch vector not found".into()));
        }
        let mut f = Self {
            filename: path.into(),
            name: String::new(),
            pitch: Vec::new(),
            tsr: Vec::new(),
            wind_speeds: None,
            cp: Vec::new(),
            ct: Vec::new(),
            cq: Vec::new(),
        };
        let mut i = 0;
        while i < lines.len() {
            let l = lines[i];
            if l.contains("Pitch angle") || l.contains("TSR vector") || l.contains("Wind speed") {
                let v = parse_numbers(
                    lines
                        .get(i + 1)
                        .ok_or_else(|| broken("missing ROSCO vector"))?,
                )?;
                if l.contains("Pitch angle") {
                    f.pitch = v;
                } else if l.contains("TSR") {
                    f.tsr = v;
                } else {
                    f.wind_speeds = Some(v);
                }
                i += 2;
                continue;
            }
            let target = if l.contains("Power coefficient") {
                Some(&mut f.cp)
            } else if l.contains("Thrust coefficient") {
                Some(&mut f.ct)
            } else if l.contains("Torque coefficient") {
                Some(&mut f.cq)
            } else {
                None
            };
            if let Some(target) = target {
                if f.pitch.is_empty() || f.tsr.is_empty() {
                    return Err(broken("ROSCO axes must precede coefficient tables"));
                }
                i += 1;
                while lines.get(i).is_some_and(|l| l.trim().is_empty()) {
                    i += 1;
                }
                for _ in 0..f.tsr.len() {
                    let row = parse_numbers(
                        lines
                            .get(i)
                            .ok_or_else(|| broken("truncated ROSCO table"))?,
                    )?;
                    if row.len() != f.pitch.len() {
                        return Err(broken("ROSCO matrix width differs from pitch vector"));
                    }
                    target.extend(row);
                    i += 1;
                }
                continue;
            }
            if let Some((_, name)) = l.split_once("wind turbine:") {
                f.name = name.trim().trim_end_matches('-').trim().into();
            }
            i += 1;
        }
        f.check_consistency()?;
        Ok(f)
    }
    pub fn new(
        pitch: Vec<f64>,
        tsr: Vec<f64>,
        cp: Option<Vec<f64>>,
        ct: Vec<f64>,
        cq: Option<Vec<f64>>,
        wind_speeds: Option<Vec<f64>>,
        name: String,
    ) -> Result<Self, IoError> {
        let mut f = Self {
            filename: PathBuf::new(),
            name,
            pitch,
            tsr,
            wind_speeds,
            cp: cp.unwrap_or_default(),
            ct,
            cq: cq.unwrap_or_default(),
        };
        f.check_consistency()?;
        Ok(f)
    }
    pub fn check_consistency(&mut self) -> Result<(), IoError> {
        let n = self.pitch.len() * self.tsr.len();
        if n == 0 || self.ct.len() != n {
            return Err(broken("invalid ROSCO axes/thrust matrix"));
        }
        if self.cp.is_empty() && self.cq.is_empty() {
            return Err(broken("ROSCO CP and CQ cannot both be absent"));
        }
        if self.cq.is_empty() {
            if self.cp.len() != n {
                return Err(broken("invalid ROSCO CP dimensions"));
            }
            self.cq = self
                .cp
                .iter()
                .enumerate()
                .map(|(i, c)| c / self.tsr[i / self.pitch.len()])
                .collect();
        }
        if self.cp.is_empty() {
            if self.cq.len() != n {
                return Err(broken("invalid ROSCO CQ dimensions"));
            }
            self.cp = self
                .cq
                .iter()
                .enumerate()
                .map(|(i, c)| c * self.tsr[i / self.pitch.len()])
                .collect();
        }
        if self.cp.len() != n || self.cq.len() != n {
            return Err(broken("invalid ROSCO coefficient dimensions"));
        }
        if self
            .cp
            .iter()
            .zip(&self.cq)
            .enumerate()
            .any(|(i, (p, q))| (p - q * self.tsr[i / self.pitch.len()]).abs() / 0.5 * 100.0 > 5.0)
        {
            return Err(broken(
                "inconsistency between ROSCO power and torque coefficients",
            ));
        }
        Ok(())
    }
    pub fn cp_max(&self) -> Result<(f64, f64, f64), IoError> {
        let (i, v) = self
            .cp
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .ok_or_else(|| broken("empty CP matrix"))?;
        Ok((
            *v,
            self.tsr[i / self.pitch.len()],
            self.pitch[i % self.pitch.len()],
        ))
    }
    pub fn to_text(&self) -> Result<String, IoError> {
        let mut f = self.clone();
        f.check_consistency()?;
        let mut s=format!("# ----- Rotor performance tables for the wind turbine: {} -----\n# ------------ Written using rustDatView\n\n# Pitch angle vector, {} entries - x axis (matrix columns) (deg)\n{}\n# TSR vector, {} entries - y axis (matrix rows) (-)\n{}\n",f.name,f.pitch.len(),numbers_text(&f.pitch,4),f.tsr.len(),numbers_text(&f.tsr,4));
        if let Some(ws) = &f.wind_speeds {
            s.push_str(&format!(
                "# Wind speed vector - z axis (m/s)\n{}\n",
                numbers_text(ws, 4)
            ));
        }
        for (name, m) in [("Power", &f.cp), ("Thrust", &f.ct), ("Torque", &f.cq)] {
            s.push_str(&format!("\n# {name} coefficient\n\n"));
            for row in m.chunks(f.pitch.len()) {
                s.push_str(&numbers_text(row, 6));
                s.push('\n');
            }
        }
        Ok(s)
    }
    pub fn write(&self, path: &Path) -> Result<(), IoError> {
        fs::write(path, self.to_text()?)?;
        Ok(())
    }
    pub fn f_cp(&self, pitch: f64, tsr: f64) -> Result<f64, IoError> {
        self.interpolate(&self.cp, pitch, tsr)
    }
    pub fn f_ct(&self, pitch: f64, tsr: f64) -> Result<f64, IoError> {
        self.interpolate(&self.ct, pitch, tsr)
    }
    fn interpolate(&self, m: &[f64], pitch: f64, tsr: f64) -> Result<f64, IoError> {
        let rows = m
            .chunks(self.pitch.len())
            .map(|r| spline(&self.pitch, r, pitch))
            .collect::<Result<Vec<_>, _>>()?;
        spline(&self.tsr, &rows, tsr)
    }
    pub fn to_aerodisc(
        &self,
        path: &Path,
        radius: f64,
        csv: bool,
        wind_speed: Option<f64>,
        omega_rpm: Option<f64>,
    ) -> Result<(), IoError> {
        let ws = wind_speed.or_else(|| {
            if omega_rpm.is_none() {
                self.wind_speeds
                    .as_ref()
                    .filter(|w| w.len() == 1)
                    .map(|w| w[0])
            } else {
                None
            }
        });
        if ws.is_none() && omega_rpm.is_none() {
            return Err(broken("provide wind speed or rotational speed"));
        }
        let mut s: String = if csv {
            "TSR_(-),RtSpd_(rpm),VRel_(m/s),Skew_(deg),Pitch_(deg),C_Fx_(-),C_Fy_(-),C_Fz_(-),C_Mx_(-),C_My_(-),C_Mz_(-)\n".into()
        } else {
            "TSR RtSpd VRel Skew Pitch C_Fx C_Fy C_Fz C_Mx C_My C_Mz\n(-) (rpm) (m/s) (deg) (deg) (-) (-) (-) (-) (-) (-)\n".into()
        };
        let sep = if csv { "," } else { " " };
        for (j, tsr) in self.tsr.iter().enumerate() {
            let (u, rpm) = if let Some(u) = ws {
                (u, tsr * u / radius * 30.0 / std::f64::consts::PI)
            } else {
                let rpm = omega_rpm.unwrap();
                (rpm * std::f64::consts::PI / 30.0 * radius / tsr, rpm)
            };
            for (i, pitch) in self.pitch.iter().enumerate() {
                let k = j * self.pitch.len() + i;
                s.push_str(
                    &[
                        *tsr, rpm, u, 0.0, *pitch, self.ct[k], 0.0, 0.0, self.cq[k], 0.0, 0.0,
                    ]
                    .iter()
                    .map(|v| format!("{v:.4}"))
                    .collect::<Vec<_>>()
                    .join(sep),
                );
                s.push('\n');
            }
        }
        fs::write(path, s)?;
        Ok(())
    }
}
fn parse_numbers(s: &str) -> Result<Vec<f64>, IoError> {
    s.split_whitespace()
        .map(|s| float(s).ok_or_else(|| broken(format!("invalid ROSCO number {s}"))))
        .collect()
}
fn numbers_text(v: &[f64], precision: usize) -> String {
    v.iter()
        .map(|v| format!("{v:.precision$}"))
        .collect::<Vec<_>>()
        .join("   ")
}
/// Tensor-product cubic interpolation with not-a-knot endpoint conditions,
/// matching scipy's regular-grid cubic interpolator on strictly increasing axes.
fn spline(x: &[f64], y: &[f64], q: f64) -> Result<f64, IoError> {
    let n = x.len();
    if n < 4 || y.len() != n || x.windows(2).any(|w| w[1] <= w[0]) {
        return Err(broken(
            "cubic interpolation requires four increasing axis points",
        ));
    }
    if q < x[0] || q > x[n - 1] || !q.is_finite() {
        return Err(broken("interpolation point outside ROSCO axes"));
    }
    let mut a = nalgebra::DMatrix::<f64>::zeros(n, n);
    let mut b = nalgebra::DVector::<f64>::zeros(n);
    let h0 = x[1] - x[0];
    let h1 = x[2] - x[1];
    a[(0, 0)] = -h1;
    a[(0, 1)] = h0 + h1;
    a[(0, 2)] = -h0;
    for i in 1..n - 1 {
        let l = x[i] - x[i - 1];
        let r = x[i + 1] - x[i];
        a[(i, i - 1)] = l;
        a[(i, i)] = 2.0 * (l + r);
        a[(i, i + 1)] = r;
        b[i] = 6.0 * ((y[i + 1] - y[i]) / r - (y[i] - y[i - 1]) / l);
    }
    let l = x[n - 2] - x[n - 3];
    let r = x[n - 1] - x[n - 2];
    a[(n - 1, n - 3)] = -r;
    a[(n - 1, n - 2)] = l + r;
    a[(n - 1, n - 1)] = -l;
    let m = a
        .lu()
        .solve(&b)
        .ok_or_else(|| broken("singular cubic interpolation system"))?;
    let i = x.partition_point(|v| *v <= q).saturating_sub(1).min(n - 2);
    let h = x[i + 1] - x[i];
    let l = (x[i + 1] - q) / h;
    let r = (q - x[i]) / h;
    Ok(l * y[i]
        + r * y[i + 1]
        + ((l * l * l - l) * m[i] + (r * r * r - r) * m[i + 1]) * h * h / 6.0)
}
impl FileObject for RoscoPerformanceFile {
    fn format_name(&self) -> &str {
        "ROSCO Performance file"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        let names: Vec<_> = std::iter::once("TSR_[-]".into())
            .chain(self.pitch.iter().map(|p| format!("Pitch_{p:.2}_[deg]")))
            .collect();
        let mut result = Vec::new();
        for (name, m) in [("CP", &self.cp), ("CT", &self.ct), ("CQ", &self.cq)] {
            let mut columns = vec![self.tsr.clone()];
            for i in 0..self.pitch.len() {
                columns.push(
                    (0..self.tsr.len())
                        .map(|j| m[j * self.pitch.len() + i])
                        .collect(),
                );
            }
            result.push(NamedFrame::new(
                name,
                DataFrame::from_floats(names.clone(), columns)
                    .map_err(|e| broken(e.to_string()))?,
            ));
        }
        Ok(result)
    }
    fn to_2d_fields(&self) -> Option<Result<Vec<Field2D>, IoError>> {
        Some(Ok([("CP", &self.cp), ("CT", &self.ct), ("CQ", &self.cq)]
            .iter()
            .map(|(name, m)| Field2D {
                name: format!("{name} [-]"),
                x: self.tsr.clone(),
                y: self.pitch.clone(),
                sx: "TSR [-]".into(),
                sy: "pitch [deg]".into(),
                m: m.iter().map(|v| if *v < 0.0 { 0.0 } else { *v }).collect(),
            })
            .collect()))
    }
}
