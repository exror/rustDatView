//! OpenFAST linearization operating points, Jacobians and eigenanalysis.
use super::binary::{broken, number, text_file};
use super::short_descr::short_description;
use dv_core::{DataFrame, FileObject, IoError, NamedFrame};
use nalgebra::{Complex, DMatrix};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[derive(Debug, Clone)]
pub struct OperatingPoint {
    pub values: Vec<f64>,
    pub rotating_frame: Vec<String>,
    pub derivative_order: Vec<usize>,
    pub description: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct LinearMatrix {
    pub rows: usize,
    pub columns: usize,
    pub values: Vec<f64>,
}
#[derive(Debug, Clone, Default)]
pub struct LinearizationOptions {
    pub star_substitute: Option<f64>,
    pub remove_states_pattern: Option<String>,
}
#[derive(Debug, Clone)]
pub struct FastLinearizationFile {
    pub filename: PathBuf,
    pub header: Vec<String>,
    pub operating_points: BTreeMap<String, OperatingPoint>,
    pub matrices: BTreeMap<String, LinearMatrix>,
    pub ed_dofs: Vec<String>,
    pub azimuth: f64,
    pub rotor_speed: f64,
    pub wind_speed: f64,
    pub time: f64,
}
#[derive(Debug, Clone)]
pub struct EigenAnalysis {
    pub damped_frequency: Vec<f64>,
    pub damping_ratio: Vec<f64>,
    pub natural_frequency: Vec<f64>,
    pub eigenvalues: Vec<Complex<f64>>,
    pub modes: DMatrix<Complex<f64>>,
}
impl FastLinearizationFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        Self::read_with_options(path, &Default::default())
    }
    pub fn read_with_options(path: &Path, options: &LinearizationOptions) -> Result<Self, IoError> {
        let text = text_file(path)?;
        let lines = text.lines().collect::<Vec<_>>();
        let end = lines
            .iter()
            .take(32)
            .position(|l| l.contains("Jacobians included"))
            .ok_or_else(|| {
                IoError::WrongFormat("linearization header lacks Jacobians included".into())
            })?;
        let header = lines[..=end]
            .iter()
            .map(|l| l.trim().to_owned())
            .collect::<Vec<_>>();
        let value = |key: &str| {
            header
                .iter()
                .find_map(|l| {
                    l.split_once(key)
                        .and_then(|(_, v)| v.split_whitespace().next())
                        .and_then(|v| number(v).ok())
                })
                .unwrap_or(f64::NAN)
        };
        let count = |key: &str| -> Result<usize, IoError> {
            let v = value(key);
            if v >= 0.0 && v.is_finite() && v.fract() == 0.0 {
                Ok(v as usize)
            } else {
                Err(broken(format!("missing or invalid {key}")))
            }
        };
        let nx = count("Number of continuous states:")?;
        let nxd = count("Number of discrete states:")?;
        let nz = count("Number of constraint states:")?;
        let nu = count("Number of inputs:")?;
        let ny = count("Number of outputs:")?;
        let mut result = Self {
            filename: path.into(),
            header: header.clone(),
            operating_points: BTreeMap::new(),
            matrices: BTreeMap::new(),
            ed_dofs: Vec::new(),
            azimuth: value("Azimuth:"),
            rotor_speed: value("Rotor Speed:"),
            wind_speed: value("Wind Speed:"),
            time: value("Simulation time:"),
        };
        let star = Regex::new(r"\*+").map_err(|e| broken(e.to_string()))?;
        let parse = |s: &str| -> Result<f64, IoError> {
            let s = star.replace_all(s, "inf");
            let v = number(&s)?;
            if v.is_infinite() {
                options.star_substitute.ok_or_else(||broken("ill-formatted or infinite value in linearization; supply star_substitute explicitly"))
            } else {
                Ok(v)
            }
        };
        let mut i = end + 1;
        while i < lines.len() {
            let line = lines[i].trim();
            i += 1;
            let op = if line.contains("Order of continuous state derivatives:") {
                Some(("xdot", nx, 2))
            } else if line.contains("Order of continuous states:") {
                Some(("x", nx, 1))
            } else if line.contains("Order of discrete states:") {
                Some(("xd", nxd, 2))
            } else if line.contains("Order of inputs") {
                Some(("u", nu, 0))
            } else if line.contains("Order of outputs") {
                Some(("y", ny, 0))
            } else if line.contains("Order of constraint states:") {
                Some(("z", nz, 0))
            } else {
                None
            };
            if let Some((name, n, default_order)) = op {
                let deriv = lines
                    .get(i)
                    .ok_or_else(|| broken("missing operating point columns"))?
                    .contains("Derivative Order");
                i += 2;
                let mut point = OperatingPoint {
                    values: Vec::with_capacity(n),
                    rotating_frame: Vec::with_capacity(n),
                    derivative_order: Vec::with_capacity(n),
                    description: Vec::with_capacity(n),
                };
                for _ in 0..n {
                    let l = lines
                        .get(i)
                        .ok_or_else(|| broken("truncated operating point"))?;
                    i += 1;
                    let sp = l.split_whitespace().collect::<Vec<_>>();
                    if sp.len() < 3 {
                        return Err(broken("invalid operating point row"));
                    }
                    let rot = if sp[1].contains(',') { 4 } else { 2 };
                    let first = sp[1].trim_end_matches(',');
                    point.values.push(parse(first)?);
                    point.rotating_frame.push(
                        sp.get(rot)
                            .ok_or_else(|| broken("missing rotating frame"))?
                            .to_string(),
                    );
                    let desc = if deriv {
                        point.derivative_order.push(
                            sp.get(rot + 1)
                                .ok_or_else(|| broken("missing derivative order"))?
                                .parse()
                                .map_err(|_| broken("invalid derivative order"))?,
                        );
                        rot + 2
                    } else {
                        point.derivative_order.push(default_order);
                        rot + 1
                    };
                    point.description.push(
                        sp.get(desc..)
                            .ok_or_else(|| broken("missing variable description"))?
                            .join(" "),
                    );
                }
                result.operating_points.insert(name.into(), point);
                continue;
            }
            let mat = match line {
                "A:" => Some(("A", nx, nx)),
                "B:" => Some(("B", nx, nu)),
                "C:" => Some(("C", ny, nx)),
                "D:" => Some(("D", ny, nu)),
                "dUdu:" => Some(("dUdu", nu, nu)),
                "dUdy:" => Some(("dUdy", nu, ny)),
                "StateRotation:" => Some(("StateRotation", nx, nx)),
                _ => None,
            };
            let mat = if let Some(dofs) = line.strip_prefix("ED M:") {
                result.ed_dofs = dofs.split_whitespace().map(str::to_owned).collect();
                Some(("M", 24, 24))
            } else {
                mat
            };
            if let Some((name, n, m)) = mat {
                let mut values = Vec::with_capacity(n * m);
                for _ in 0..n {
                    let row = lines
                        .get(i)
                        .ok_or_else(|| broken(format!("truncated matrix {name}")))?;
                    i += 1;
                    let sp = row.split_whitespace().collect::<Vec<_>>();
                    if sp.len() != m {
                        return Err(broken(format!(
                            "matrix {name} row width {} instead of {m}",
                            sp.len()
                        )));
                    }
                    for v in sp {
                        let v = parse(v)?;
                        if v.is_nan() {
                            return Err(broken(format!("NaN in matrix {name}")));
                        }
                        values.push(v);
                    }
                }
                result.matrices.insert(
                    name.into(),
                    LinearMatrix {
                        rows: n,
                        columns: m,
                        values,
                    },
                );
            }
        }
        if let Some(pattern) = &options.remove_states_pattern {
            result.remove_states_pattern(pattern)?;
        }
        Ok(result)
    }
    pub fn remove_states_pattern(&mut self, pattern: &str) -> Result<(), IoError> {
        let regex = Regex::new(pattern).map_err(|e| broken(e.to_string()))?;
        let indices = self
            .operating_points
            .get("x")
            .map(|p| {
                p.description
                    .iter()
                    .enumerate()
                    .filter_map(|(i, d)| regex.is_match(d).then_some(i))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        self.remove_states(&indices)
    }
    pub fn remove_states(&mut self, remove: &[usize]) -> Result<(), IoError> {
        let n = self.operating_points.get("x").map_or(0, |p| p.values.len());
        if n == 0 {
            return Ok(());
        }
        if remove.iter().any(|i| *i >= n) {
            return Err(broken("state index out of bounds"));
        }
        let keep = (0..n).filter(|i| !remove.contains(i)).collect::<Vec<_>>();
        if keep.is_empty() {
            return Err(broken("all states would be removed"));
        }
        for key in ["x", "xdot"] {
            if let Some(p) = self.operating_points.get_mut(key) {
                p.values = keep.iter().map(|&i| p.values[i]).collect();
                p.rotating_frame = keep.iter().map(|&i| p.rotating_frame[i].clone()).collect();
                p.derivative_order = keep.iter().map(|&i| p.derivative_order[i]).collect();
                p.description = keep.iter().map(|&i| p.description[i].clone()).collect();
            }
        }
        for (key, rows, cols) in [
            ("A", true, true),
            ("B", true, false),
            ("C", false, true),
            ("StateRotation", true, true),
        ] {
            if let Some(m) = self.matrices.get_mut(key) {
                let rr = if rows {
                    keep.clone()
                } else {
                    (0..m.rows).collect()
                };
                let cc = if cols {
                    keep.clone()
                } else {
                    (0..m.columns).collect()
                };
                let mut data = Vec::with_capacity(rr.len() * cc.len());
                for &r in &rr {
                    for &c in &cc {
                        data.push(m.values[r * m.columns + c]);
                    }
                }
                m.rows = rr.len();
                m.columns = cc.len();
                m.values = data;
            }
        }
        Ok(())
    }
    fn labels(&self, key: &str) -> Vec<String> {
        self.operating_points
            .get(key)
            .map(|p| p.description.iter().map(|d| short_description(d)).collect())
            .unwrap_or_default()
    }
    /// pandas matrix index labels (the core DataFrame itself stores columns only).
    pub fn matrix_row_labels(&self, key: &str) -> Option<Vec<String>> {
        if !self.matrices.contains_key(key) {
            return None;
        }
        match key {
            "A" | "B" | "StateRotation" => Some(self.labels("x")),
            "C" | "D" => Some(self.labels("y")),
            "dUdu" | "dUdy" => Some(self.labels("u")),
            "M" => Some(self.ed_dofs.clone()),
            _ => None,
        }
    }
    pub fn eigenanalysis(
        &self,
        normalize_by_max: bool,
        sort: bool,
        discard_negative_imaginary: bool,
    ) -> Result<EigenAnalysis, IoError> {
        let a = self
            .matrices
            .get("A")
            .ok_or_else(|| broken("eigenanalysis requires A matrix"))?;
        if a.rows != a.columns || a.rows == 0 {
            return Err(broken("A matrix must be nonempty and square"));
        }
        let matrix = DMatrix::from_row_slice(a.rows, a.columns, &a.values);
        let eig = matrix.complex_eigenvalues();
        let mut eig = eig
            .iter()
            .copied()
            .filter(|v| !discard_negative_imaginary || v.im > 0.0)
            .collect::<Vec<_>>();
        if sort {
            eig.sort_by(|a, b| a.norm().total_cmp(&b.norm()));
        }
        let mut modes = DMatrix::zeros(a.rows, eig.len());
        for (j, &lambda) in eig.iter().enumerate() {
            let mut shifted = matrix.map(|v| Complex::new(v, 0.0));
            for i in 0..a.rows {
                shifted[(i, i)] -= lambda;
            }
            let svd = shifted.svd(false, true);
            let vt = svd.v_t.ok_or_else(|| broken("eigenvector SVD failed"))?;
            let k = svd
                .singular_values
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.total_cmp(b.1))
                .map(|(i, _)| i)
                .ok_or_else(|| broken("empty eigenvector SVD"))?;
            let max = (0..a.rows).map(|i| vt[(k, i)].norm()).fold(0.0, f64::max);
            for i in 0..a.rows {
                modes[(i, j)] = vt[(k, i)].conj() / if normalize_by_max { max } else { 1.0 };
            }
        }
        let tau = 2.0 * std::f64::consts::PI;
        Ok(EigenAnalysis {
            damped_frequency: eig.iter().map(|v| v.im / tau).collect(),
            damping_ratio: eig.iter().map(|v| -v.re / v.norm()).collect(),
            natural_frequency: eig.iter().map(|v| v.norm() / tau).collect(),
            eigenvalues: eig,
            modes,
        })
    }
}
impl FileObject for FastLinearizationFile {
    fn format_name(&self) -> &str {
        "FAST linearization output"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        let mut frames = Vec::new();
        for (key, colkey) in [
            ("A", "x"),
            ("B", "u"),
            ("C", "x"),
            ("D", "u"),
            ("x", "x"),
            ("xd", "xd"),
            ("xdot", "xdot"),
            ("u", "u"),
            ("y", "y"),
            ("z", "z"),
            ("M", "M"),
            ("dUdu", "u"),
            ("dUdy", "y"),
        ] {
            if let Some(m) = self.matrices.get(key) {
                let labels = if colkey == "M" {
                    self.ed_dofs.clone()
                } else {
                    self.labels(colkey)
                };
                let labels = if labels.is_empty() {
                    (0..m.columns).map(|i| i.to_string()).collect()
                } else {
                    labels
                };
                if labels.len() != m.columns {
                    return Err(broken(format!(
                        "matrix {key} labels differ from dimensions"
                    )));
                }
                let cols = (0..m.columns)
                    .map(|c| (0..m.rows).map(|r| m.values[r * m.columns + c]).collect())
                    .collect();
                frames.push(NamedFrame::new(key, super::binary::frame(labels, cols)?));
            } else if let Some(p) = self.operating_points.get(key) {
                let names = if key == "xd" {
                    (0..p.values.len()).map(|i| i.to_string()).collect()
                } else {
                    self.labels(key)
                };
                let df = DataFrame::from_floats(names, p.values.iter().map(|v| vec![*v]).collect())
                    .map_err(|e| broken(e.to_string()))?;
                frames.push(NamedFrame::new(key, df));
            }
        }
        Ok(frames)
    }
    fn info(&self) -> String {
        format!("FAST linearization at t={} s, azimuth={} rad, rotor speed={} rad/s, wind={} m/s\nMatrices: {}",self.time,self.azimuth,self.rotor_speed,self.wind_speed,self.matrices.keys().cloned().collect::<Vec<_>>().join(", "))
    }
}
