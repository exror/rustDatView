use super::input::{atom, broken, float, FastInputFile, InputEntry, InputTable, InputValue};
use dv_core::{Column, DataFrame, IoError, NamedFrame};

fn item(label: &str, value: InputValue) -> InputEntry {
    InputEntry {
        label: label.into(),
        value,
        description: String::new(),
        is_comment: false,
        dimension: None,
        table_type: 0,
    }
}
fn numeric(lines: &[&str], start: usize, n: usize, width: usize) -> Result<InputTable, IoError> {
    if start + n > lines.len() {
        return Err(broken("truncated matrix"));
    }
    let mut rows = Vec::with_capacity(n);
    for line in &lines[start..start + n] {
        let r: Vec<_> = line.split_whitespace().map(atom).collect();
        if r.len() != width || r.iter().any(|v| v.as_f64().is_none()) {
            return Err(broken("invalid matrix row"));
        }
        rows.push(r);
    }
    Ok(InputTable {
        columns: (0..width).map(|i| format!("C{i}")).collect(),
        units: Vec::new(),
        rows,
    })
}
impl FastInputFile {
    pub(super) fn read_ext_platform(&mut self, lines: &[&str]) -> Result<bool, IoError> {
        if lines.len() < 10
            || !lines[0].starts_with('!')
            || !lines[1].to_lowercase().contains("flex")
            || !lines[2].to_lowercase().contains("!dimension")
        {
            return Ok(false);
        }
        self.module = Some("ExtPtfm".into());
        let mut i = 2;
        while i < lines.len() {
            let l = lines[i].to_lowercase();
            if l.starts_with("!dimension:")
                || l.starts_with("!time increment:")
                || l.starts_with("!total simulation time:")
            {
                let key = if l.starts_with("!dimension") {
                    "nDOF"
                } else if l.starts_with("!time") {
                    "dt"
                } else {
                    "T"
                };
                let v = l
                    .split_once(':')
                    .and_then(|(_, s)| float(s.trim()))
                    .ok_or_else(|| broken("invalid ExtPtfm dimension/time"))?;
                self.entries.push(item(key, InputValue::Number(v)));
            } else if let Some(key) = if l.starts_with("!mass") {
                Some("MassMatrix")
            } else if l.starts_with("!stiffness") {
                Some("StiffnessMatrix")
            } else if l.starts_with("!damping") {
                Some("DampingMatrix")
            } else if l.starts_with("!loading") {
                Some("Loading")
            } else {
                None
            } {
                let dof = self
                    .integer("nDOF")
                    .filter(|n| *n > 0)
                    .ok_or_else(|| broken("invalid ExtPtfm nDOF"))?
                    as usize;
                let (n, w) = if key == "Loading" {
                    let dt = self
                        .number("dt")
                        .filter(|d| *d > 0.0)
                        .ok_or_else(|| broken("invalid ExtPtfm dt"))?;
                    let t = self
                        .number("T")
                        .ok_or_else(|| broken("missing ExtPtfm T"))?;
                    ((t / dt) as usize + 1, dof + 1)
                } else {
                    let dimension = lines
                        .get(i + 1)
                        .and_then(|l| l.split_once(':'))
                        .and_then(|(_, s)| s.trim().parse::<usize>().ok())
                        .ok_or_else(|| broken("missing matrix dimension"))?;
                    if dimension != dof {
                        return Err(broken("inconsistent ExtPtfm matrix dimensions"));
                    }
                    (dof, dof)
                };
                self.entries
                    .push(item(key, InputValue::Table(numeric(lines, i + 2, n, w)?)));
                i += n + 1;
            } else if !l.trim().is_empty() && !l.starts_with('!') {
                return Err(broken(format!("unexpected ExtPtfm content at {}", i + 1)));
            }
            i += 1;
        }
        Ok(true)
    }
    pub(super) fn ext_text(&self) -> Result<String, IoError> {
        let n = self.integer("nDOF").ok_or_else(|| broken("missing nDOF"))?;
        let mut s=format!("!Comment\n!Flex 5 Format\n!Dimension: {n}\n!Time increment: {}\n!Total simulation time: {}\n",self.number("dt").ok_or_else(||broken("missing dt"))?,self.number("T").ok_or_else(||broken("missing T"))?);
        for (key, title) in [
            ("MassMatrix", "Mass"),
            ("StiffnessMatrix", "Stiffness"),
            ("DampingMatrix", "Damping"),
            ("Loading", "Loading and Wave Elevation"),
        ] {
            let t = self
                .table(key)
                .ok_or_else(|| broken(format!("missing {key}")))?;
            s.push_str(&format!("\n!{title}\n!Dimension: {n}\n"));
            for r in &t.rows {
                s.push_str(
                    &r.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" "),
                );
                s.push('\n');
            }
        }
        Ok(s)
    }
    pub(super) fn read_ad14(&mut self, lines: &[&str]) -> Result<bool, IoError> {
        if lines.len() < 14 {
            return Ok(false);
        }
        let n = lines[2]
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<usize>().ok());
        let Some(n) = n else {
            return Ok(false);
        };
        if lines[12]
            .split_whitespace()
            .nth(n)
            .is_none_or(|s| !s.eq_ignore_ascii_case("angle"))
            || lines[13]
                .split_whitespace()
                .nth(n)
                .is_none_or(|s| !s.eq_ignore_ascii_case("minimum"))
        {
            return Ok(false);
        }
        for line in &lines[..2] {
            let mut e = item("", InputValue::Text((*line).into()));
            e.is_comment = true;
            self.entries.push(e);
        }
        for line in &lines[2..14] {
            let t: Vec<_> = line.split_whitespace().collect();
            self.entries.push(item(&t[1..].join(" "), atom(t[0])));
        }
        let count = lines[14..]
            .iter()
            .take_while(|l| !l.trim().is_empty())
            .count();
        let width = lines[14].split_whitespace().count();
        let mut t = numeric(lines, 14, count, width)?;
        if width == 1 + n * 3 || width == 1 + n * 2 {
            let ids: Vec<_> = if n == 1 {
                vec![""]
            } else {
                lines[3].split_whitespace().take(n).collect()
            };
            t.columns = vec!["Alpha".into()];
            t.units = vec!["(deg)".into()];
            for id in ids {
                for name in if width == 1 + n * 3 {
                    &["Cl", "Cd", "Cm"][..]
                } else {
                    &["Cl", "Cd"][..]
                } {
                    t.columns.push(format!("{name}{id}"));
                    t.units.push("(-)".into());
                }
            }
        }
        let mut e = item("Polar", InputValue::Table(t));
        e.table_type = 4;
        self.entries.push(e);
        self.module = Some("AeroDyn14Polar".into());
        Ok(true)
    }
    pub(super) fn read_beam(&mut self, lines: &[&str], start: usize) -> Result<(), IoError> {
        let n = self
            .integer("station_total")
            .filter(|n| *n >= 0)
            .ok_or_else(|| broken("missing BeamDyn station_total"))? as usize;
        let mut columns = vec!["Span".into()];
        for p in ["K", "M"] {
            for a in 1..=6 {
                for b in 1..=6 {
                    columns.push(format!("{p}{a}{b}"));
                }
            }
        }
        let main: Vec<_> = std::iter::once(0)
            .chain((0..6).map(|a| a * 6 + a + 1))
            .chain((0..6).map(|a| a * 6 + a + 37))
            .collect();
        let order: Vec<_> = main
            .iter()
            .copied()
            .chain((0..73).filter(|i| !main.contains(i)))
            .collect();
        let mut rows = Vec::with_capacity(n);
        let mut i = start;
        for _ in 0..n {
            while lines.get(i).is_some_and(|l| l.trim().is_empty()) {
                i += 1;
            }
            let span = lines
                .get(i)
                .and_then(|s| float(s.trim()))
                .ok_or_else(|| broken("invalid BeamDyn span"))?;
            i += 1;
            let mut row = vec![InputValue::Number(span)];
            for _ in 0..2 {
                while lines.get(i).is_some_and(|l| l.trim().is_empty()) {
                    i += 1;
                }
                let t = numeric(lines, i, 6, 6)?;
                row.extend(t.rows.into_iter().flatten());
                i += 6;
            }
            rows.push(order.iter().map(|j| row[*j].clone()).collect());
        }
        let t = InputTable {
            columns: order.iter().map(|j| columns[*j].clone()).collect(),
            units: Vec::new(),
            rows,
        };
        let mut e = item("BeamProperties", InputValue::Table(t));
        e.table_type = 5;
        self.entries.push(e);
        Ok(())
    }
    pub(super) fn frames(&self) -> Result<Vec<NamedFrame>, IoError> {
        if self.module.as_deref() == Some("ExtPtfm") {
            let t = self
                .table("Loading")
                .ok_or_else(|| broken("missing ExtPtfm loading"))?;
            let mut names: Vec<String> = [
                "Time_[s]",
                "InpF_Fx_[N]",
                "InpF_Fy_[N]",
                "InpF_Fz_[N]",
                "InpF_Mx_[Nm]",
                "InpF_My_[Nm]",
                "InpF_Mz_[Nm]",
            ]
            .iter()
            .map(|s| (*s).into())
            .collect();
            names.extend((1..=t.columns.len()).map(|i| format!("CBF_{i:03}_[-]")));
            names.truncate(t.columns.len());
            return Ok(vec![NamedFrame::unnamed(
                DataFrame::from_rows(names, &t.numeric_rows()?)
                    .map_err(|e| broken(e.to_string()))?,
            )]);
        }
        let mut result = Vec::new();
        for e in &self.entries {
            let InputValue::Table(t) = &e.value else {
                continue;
            };
            if e.label == "DampingCoeffs" || e.table_type == 7 {
                continue;
            }
            let mut columns = t.dataframe()?.columns().to_vec();
            if e.label == "BldProp" && self.get("BldFl1Sh(2)").is_some() {
                self.shapes(
                    t,
                    &mut columns,
                    &[
                        ("BldFl1Sh", "ShapeFlap1_[-]"),
                        ("BldFl2Sh", "ShapeFlap2_[-]"),
                        ("BldEdgSh", "ShapeEdge1_[-]"),
                    ],
                )?;
            }
            if e.label == "TowProp" && self.get("TwFAM1Sh(2)").is_some() {
                self.shapes(
                    t,
                    &mut columns,
                    &[
                        ("TwFAM1Sh", "ShapeForeAft1_[-]"),
                        ("TwFAM2Sh", "ShapeForeAft2_[-]"),
                        ("TwSSM1Sh", "ShapeSideSide1_[-]"),
                        ("TwSSM2Sh", "ShapeSideSide2_[-]"),
                    ],
                )?;
            }
            if e.label == "BldAeroNodes" && self.get("NumBlNds").is_some() {
                aero_geometry(t, &mut columns)?;
            }
            if e.label.starts_with("AFCoeff") {
                self.polar(t, &e.label, &mut columns)?;
            }
            result.push(NamedFrame::new(
                &e.label,
                DataFrame::from_columns(columns).map_err(|e| broken(e.to_string()))?,
            ));
        }
        if result.len() == 1 {
            result[0].name.clear();
        }
        Ok(result)
    }
    fn shapes(
        &self,
        t: &InputTable,
        cols: &mut Vec<Column>,
        modes: &[(&str, &str)],
    ) -> Result<(), IoError> {
        for (key, name) in modes {
            let mut coeffs = [0.0; 5];
            for (i, c) in coeffs.iter_mut().enumerate() {
                *c = self
                    .number(&format!("{key}({})", i + 2))
                    .ok_or_else(|| broken(format!("missing {key}({})", i + 2)))?;
            }
            let values = t
                .rows
                .iter()
                .map(|r| {
                    let x = r[0].as_f64().unwrap();
                    x * x
                        * (coeffs[0]
                            + x * (coeffs[1] + x * (coeffs[2] + x * (coeffs[3] + x * coeffs[4]))))
                })
                .collect();
            cols.push(Column::float(*name, values));
        }
        Ok(())
    }
    fn polar(&self, t: &InputTable, key: &str, cols: &mut Vec<Column>) -> Result<(), IoError> {
        let a = t
            .column("Alpha")
            .ok_or_else(|| broken("polar missing Alpha"))?;
        let cl = t.column("Cl").ok_or_else(|| broken("polar missing Cl"))?;
        let cd = t.column("Cd").ok_or_else(|| broken("polar missing Cd"))?;
        let suffix = key.strip_prefix("AFCoeff").unwrap_or("");
        let cn: Vec<_> = a
            .iter()
            .zip(&cl)
            .zip(&cd)
            .map(|((a, l), d)| l * a.to_radians().cos() + d * a.to_radians().sin())
            .collect();
        if let Some(d0) = self.number(&format!("Cd0{suffix}")) {
            cols.push(Column::float(
                "Cn_Cd0off_[-]",
                a.iter()
                    .zip(&cl)
                    .zip(&cd)
                    .map(|((a, l), d)| l * a.to_radians().cos() + (d - d0) * a.to_radians().sin())
                    .collect(),
            ));
        }
        cols.push(Column::float("Cn_[-]", cn.clone()));
        if let (Some(slope), Some(zero)) = (
            self.number(&format!("C_nalpha{suffix}")),
            self.number(&format!("alpha0{suffix}")),
        ) {
            cols.push(Column::float(
                "Cn_pot_[-]",
                a.iter()
                    .map(|a| {
                        if *a < -20.0 || *a > 30.0 {
                            f64::NAN
                        } else {
                            slope * (a - zero).to_radians()
                        }
                    })
                    .collect(),
            ));
            let marks: Option<Vec<_>> = ["alpha2", "alpha1", "alpha0"]
                .iter()
                .map(|k| {
                    self.number(&format!("{k}{suffix}"))
                        .and_then(|point| a.iter().rposition(|x| *x < point))
                })
                .collect();
            if let Some(marks) = marks {
                let mut v = vec![f64::NAN; a.len()];
                for i in marks {
                    v[i] = cn[i];
                    if i + 1 < v.len() {
                        v[i + 1] = cn[i + 1];
                    }
                }
                cols.push(Column::float("Cn_012_[-]", v));
            }
            cols.push(Column::float(
                "Cn_f_[-]",
                a.iter()
                    .map(|a| {
                        slope * (a - zero).to_radians() * ((1.0 + 0.7_f64.sqrt()) / 2.0).powi(2)
                    })
                    .collect(),
            ));
        }
        Ok(())
    }
}
fn aero_geometry(t: &InputTable, cols: &mut Vec<Column>) -> Result<(), IoError> {
    let r = t.numeric_rows()?;
    if r.iter().any(|r| r.len() < 6) {
        return Err(broken("invalid AeroDyn blade table"));
    }
    let n = r.len();
    let root = (n as f64 * 0.15) as usize;
    let mut crv = Vec::with_capacity(n);
    let mut swp = Vec::with_capacity(n);
    let mut ac = Vec::with_capacity(n);
    let mut angle = Vec::with_capacity(n);
    for (i, row) in r.iter().enumerate() {
        let a = if i < root {
            if root == 1 {
                0.5
            } else {
                0.5 - 0.25 * i as f64 / (root - 1) as f64
            }
        } else {
            0.25
        };
        let twist = row[4].to_radians();
        crv.push(row[1] + row[5] * (0.5 - a) * twist.sin());
        swp.push(row[2] + row[5] * (0.5 - a) * twist.cos());
        ac.push(a);
        let (left, right) = if i == 0 {
            (0, 1.min(n - 1))
        } else if i + 1 == n {
            (i - 1, i)
        } else {
            (i - 1, i + 1)
        };
        angle.push(
            (r[right][1] - r[left][1])
                .atan2(r[right][0] - r[left][0])
                .to_degrees(),
        );
    }
    cols.extend([
        Column::float("c2_Crv_Approx_[m]", crv),
        Column::float("c2_Swp_Approx_[m]", swp),
        Column::float("AC_Approx_[-]", ac),
        Column::float("CrvAng_Calc_[-]", angle),
    ]);
    Ok(())
}
