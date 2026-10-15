//! BeamDyn and SubDyn summary readers and nodal mode-shape conversion.
use super::binary::{broken, text_file};
use super::mini_yaml::{yaml_parse, YamlValue};
use dv_core::{Column, DataFrame, FileObject, IoError, NamedFrame};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone)]
pub struct FastSummaryFile {
    pub filename: PathBuf,
    pub module: String,
    pub data: BTreeMap<String, YamlValue>,
}
#[derive(Debug, Clone)]
pub struct NodeDisplacements {
    pub nodes: Vec<usize>,
    pub positions: Vec<[f64; 3]>,
    pub displacement: Vec<Vec<[f64; 3]>>,
}
#[derive(Debug, Clone)]
pub struct SummaryModes {
    pub guyan: NodeDisplacements,
    pub craig_bampton: NodeDisplacements,
    pub frequencies: Vec<f64>,
}
#[derive(Debug, Clone)]
pub struct SummaryGraphNode {
    pub id: usize,
    pub position: [f64; 3],
    pub node_type: usize,
    pub dofs: Vec<usize>,
}
#[derive(Debug, Clone)]
pub struct SummaryGraphElement {
    pub id: usize,
    pub node_ids: [usize; 2],
    pub element_type: usize,
    pub area: f64,
    pub density: f64,
    pub young_modulus: f64,
    pub shear_modulus: f64,
    pub diameter: f64,
    pub color: [f64; 3],
}
#[derive(Debug, Clone)]
pub struct SummaryGraphMode {
    pub name: String,
    pub frequency: f64,
    pub displacement: Vec<[f64; 3]>,
}
#[derive(Debug, Clone)]
pub struct SummaryGraph {
    pub nodes: Vec<SummaryGraphNode>,
    pub elements: Vec<SummaryGraphElement>,
    pub modes: Vec<SummaryGraphMode>,
}
impl FastSummaryFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        let text = text_file(path)?;
        let header = text
            .lines()
            .take(5)
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let module = if header.contains("subdyn") {
            "SubDyn"
        } else if header.contains("beamdyn") {
            "BeamDyn"
        } else {
            return Err(IoError::WrongFormat(
                "not a SubDyn or BeamDyn summary".into(),
            ));
        };
        let mut data = yaml_parse(&text)?;
        if module == "SubDyn" {
            if let Some(YamlValue::Array(a)) = data.get_mut("DOF2Nodes") {
                for (i, row) in a.iter_mut().enumerate() {
                    if row.len() == 3 {
                        row.insert(0, (i + 1) as f64);
                    }
                    if row.len() != 4 {
                        return Err(broken("DOF2Nodes requires three or four columns"));
                    }
                    row[0] -= 1.0;
                }
            } else {
                return Err(broken("SubDyn summary lacks DOF2Nodes"));
            }
            for key in ["DOF___L", "DOF___B", "DOF___F"] {
                if let Some(YamlValue::Array(a)) = data.get_mut(key) {
                    for v in a.iter_mut().flatten() {
                        *v -= 1.0;
                    }
                }
            }
            if let Some(YamlValue::Array(nodes)) = data.get("Nodes") {
                if nodes.iter().any(|r| r.len() < 4) {
                    return Err(broken("Nodes requires ID,x,y,z"));
                }
                let coordinates = [("X", 1), ("Y", 2), ("Z", 3)]
                    .map(|(axis, j)| (axis, nodes.iter().map(|r| vec![r[j]]).collect::<Vec<_>>()));
                for (axis, values) in coordinates {
                    data.insert(axis.into(), YamlValue::Array(values));
                }
            }
        }
        data.insert("module".into(), YamlValue::Text(module.into()));
        Ok(Self {
            filename: path.into(),
            module: module.into(),
            data,
        })
    }
    fn matrix(&self, key: &str) -> Result<&[Vec<f64>], IoError> {
        self.data
            .get(key)
            .and_then(YamlValue::array)
            .ok_or_else(|| broken(format!("summary lacks numeric array {key}")))
    }
    fn indices(&self, key: &str) -> Result<Vec<usize>, IoError> {
        self.matrix(key)?
            .iter()
            .flatten()
            .map(|v| {
                if *v >= 0.0 && v.fract() == 0.0 {
                    Ok(*v as usize)
                } else {
                    Err(broken(format!("invalid DOF in {key}")))
                }
            })
            .collect()
    }
    pub fn nodes_disp(
        &self,
        dofs: &[usize],
        values: &[Vec<f64>],
        max_disp: Option<f64>,
        sort_dimension: Option<usize>,
    ) -> Result<NodeDisplacements, IoError> {
        let map = self.matrix("DOF2Nodes")?;
        let nodes = self.matrix("Nodes")?;
        if dofs.len() != values.len() {
            return Err(broken("mode row count differs from DOF count"));
        }
        let nshape = values.first().map_or(0, Vec::len);
        if values.iter().any(|r| r.len() != nshape) {
            return Err(broken("inconsistent mode widths"));
        }
        let ids = dofs
            .iter()
            .map(|&d| {
                map.get(d)
                    .map(|r| r[1] as usize)
                    .ok_or_else(|| broken("DOF index outside DOF2Nodes"))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let mut result = NodeDisplacements {
            nodes: ids.into_iter().collect(),
            positions: Vec::new(),
            displacement: Vec::new(),
        };
        result.positions = vec![[f64::NAN; 3]; result.nodes.len()];
        result.displacement = vec![vec![[f64::NAN; 3]; nshape]; result.nodes.len()];
        for (&d, row) in dofs.iter().zip(values) {
            let m = &map[d];
            let node = m[1] as usize;
            let component = m[3] as usize;
            if (1..=3).contains(&component) {
                let i = result
                    .nodes
                    .binary_search(&node)
                    .map_err(|_| broken("missing node"))?;
                let pos = nodes
                    .get(node.checked_sub(1).ok_or_else(|| broken("zero node ID"))?)
                    .ok_or_else(|| broken("node ID outside Nodes"))?;
                result.positions[i] = [pos[1], pos[2], pos[3]];
                for (s, &v) in row.iter().enumerate() {
                    result.displacement[i][s][component - 1] = v;
                }
            }
        }
        if let Some(max) = max_disp {
            for s in 0..nshape {
                let m = result
                    .displacement
                    .iter()
                    .flat_map(|r| r[s])
                    .filter(|v| v.is_finite())
                    .map(f64::abs)
                    .fold(0.0, f64::max);
                if m > 1e-5 {
                    for node in &mut result.displacement {
                        for v in &mut node[s] {
                            *v *= max / m;
                        }
                    }
                }
            }
        }
        if let Some(d) = sort_dimension {
            if d > 2 {
                return Err(broken("sort dimension must be 0,1,2"));
            }
            let mut order = (0..result.nodes.len()).collect::<Vec<_>>();
            order.sort_by(|&i, &j| result.positions[i][d].total_cmp(&result.positions[j][d]));
            result = NodeDisplacements {
                nodes: order.iter().map(|&i| result.nodes[i]).collect(),
                positions: order.iter().map(|&i| result.positions[i]).collect(),
                displacement: order
                    .iter()
                    .map(|&i| result.displacement[i].clone())
                    .collect(),
            };
        }
        Ok(result)
    }
    pub fn get_modes(
        &self,
        max_disp: Option<f64>,
        sort_dimension: Option<usize>,
    ) -> Result<SummaryModes, IoError> {
        let max = match max_disp {
            Some(v) => v,
            None => {
                let nodes = self.matrix("Nodes")?;
                (1..=3)
                    .map(|a| {
                        let lo = nodes.iter().map(|r| r[a]).fold(f64::INFINITY, f64::min);
                        let hi = nodes.iter().map(|r| r[a]).fold(f64::NEG_INFINITY, f64::max);
                        hi - lo
                    })
                    .fold(0.0, f64::max)
                    * 0.1
            }
        };
        let boundary = self.indices("DOF___B")?;
        let internal = self.indices("DOF___L")?;
        let fixed = self.indices("DOF___F")?;
        let dofs = boundary
            .iter()
            .chain(&internal)
            .chain(&fixed)
            .copied()
            .collect::<Vec<_>>();
        let phi_m = self.matrix("PhiM")?;
        let phi_r = self.matrix("PhiR")?;
        let cbn = phi_m.first().map_or(0, Vec::len);
        let gyn = boundary.len();
        if phi_m.len() != internal.len()
            || phi_r.len() != internal.len()
            || phi_r.iter().any(|r| r.len() != gyn)
        {
            return Err(broken("summary mode and internal DOF dimensions differ"));
        }
        let mut cb = vec![vec![0.0; cbn]; boundary.len()];
        cb.extend_from_slice(phi_m);
        cb.extend(vec![vec![0.0; cbn]; fixed.len()]);
        let mut gy = vec![vec![0.0; gyn]; boundary.len()];
        for (i, row) in gy.iter_mut().enumerate() {
            row[i] = 1.0;
        }
        gy.extend_from_slice(phi_r);
        gy.extend(vec![vec![0.0; gyn]; fixed.len()]);
        Ok(SummaryModes {
            guyan: self.nodes_disp(&dofs, &gy, Some(max), sort_dimension)?,
            craig_bampton: self.nodes_disp(&dofs, &cb, Some(max), sort_dimension)?,
            frequencies: self
                .data
                .get("CB_frequencies")
                .and_then(YamlValue::flat)
                .unwrap_or_default(),
        })
    }
    pub fn dataframe(
        &self,
        sort_dimension: Option<usize>,
        remove_zero: bool,
    ) -> Result<DataFrame, IoError> {
        if self.module == "BeamDyn" {
            let rows = self.matrix("Init_Nodes_E1")?;
            let n = rows.first().map_or(0, Vec::len);
            return DataFrame::from_rows((0..n).map(|i| i.to_string()), rows)
                .map_err(|e| broken(e.to_string()));
        }
        let modes = self.get_modes(None, sort_dimension)?;
        let gy = &modes.guyan;
        let cb = &modes.craig_bampton;
        if gy.nodes != cb.nodes {
            return Err(broken("Guyan and CB node lists differ"));
        }
        let mut cols = vec![
            Column::float("z_[m]", gy.positions.iter().map(|p| p[2]).collect()),
            Column::float("x_[m]", gy.positions.iter().map(|p| p[0]).collect()),
            Column::float("y_[m]", gy.positions.iter().map(|p| p[1]).collect()),
        ];
        let mut displacement_cols = Vec::new();
        for (prefix, mode) in [("Guyan", gy), ("CB", cb)] {
            let n = mode.displacement.first().map_or(0, Vec::len);
            for s in 0..n {
                for (a, axis) in ["x", "y", "z"].iter().enumerate() {
                    let disp = mode
                        .displacement
                        .iter()
                        .map(|r| if r[s][a].is_nan() { 0.0 } else { r[s][a] })
                        .collect::<Vec<_>>();
                    if remove_zero && disp.iter().all(|v| *v == 0.0) {
                        continue;
                    }
                    let total = disp
                        .iter()
                        .zip(&mode.positions)
                        .map(|(v, p)| v + p[a])
                        .collect();
                    cols.push(Column::float(
                        format!("{prefix}Mode{}{axis}_[m]", s + 1),
                        total,
                    ));
                    displacement_cols.push(Column::float(
                        format!("{prefix}Disp{}{axis}_[m]", s + 1),
                        disp,
                    ));
                }
            }
        }
        cols.extend(displacement_cols);
        DataFrame::from_columns(cols).map_err(|e| broken(e.to_string()))
    }
    pub fn to_graph(&self) -> Result<SummaryGraph, IoError> {
        let node_rows = self.matrix("Nodes")?;
        let element_rows = self.matrix("Elements")?;
        let dofs = self.matrix("DOF2Nodes")?;
        if node_rows.iter().any(|n| n.len() < 5) || element_rows.iter().any(|e| e.len() < 9) {
            return Err(broken(
                "graph export requires Nodes with type and Elements with area/density",
            ));
        }
        let nodes = node_rows
            .iter()
            .map(|n| SummaryGraphNode {
                id: n[0] as usize,
                position: [n[1], n[2], n[3]],
                node_type: n[4] as usize,
                dofs: dofs
                    .iter()
                    .filter(|d| d[1] == n[0])
                    .map(|d| d[0] as usize)
                    .collect(),
            })
            .collect();
        let colors = [
            [0.1, 0.1, 0.1],
            [0.753, 0.561, 0.05],
            [0.541, 0.753, 0.05],
            [0.753, 0.05, 0.204],
            [0.918, 0.702, 0.125],
        ];
        let mut elements = Vec::with_capacity(element_rows.len());
        for e in element_rows {
            let kind = e[5] as usize;
            let color = *colors
                .get(kind)
                .ok_or_else(|| broken("unknown SubDyn graph element type"))?;
            elements.push(SummaryGraphElement {
                id: e[0] as usize,
                node_ids: [e[1] as usize, e[2] as usize],
                element_type: kind,
                area: e[7],
                density: e[8],
                young_modulus: e[7],
                shear_modulus: e[8],
                diameter: (e[7] / std::f64::consts::PI).sqrt() * 4.0,
                color,
            });
        }
        let mode_shapes = self.get_modes(None, None)?;
        let mut modes = Vec::new();
        for (prefix, shapes) in [
            ("GY", &mode_shapes.guyan),
            ("CB", &mode_shapes.craig_bampton),
        ] {
            for i in 0..shapes.displacement.first().map_or(0, Vec::len) {
                let frequency = if prefix == "GY" {
                    1.0 / (2.0 * std::f64::consts::PI)
                } else {
                    *mode_shapes
                        .frequencies
                        .get(i)
                        .ok_or_else(|| broken("missing CB frequency"))?
                };
                modes.push(SummaryGraphMode {
                    name: format!("{prefix}{}", i + 1),
                    frequency,
                    displacement: shapes.displacement.iter().map(|r| r[i]).collect(),
                });
            }
        }
        Ok(SummaryGraph {
            nodes,
            elements,
            modes,
        })
    }
    /// SubDyn structural connectivity and modes, in the upstream visualization schema.
    pub fn to_json(&self, path: Option<&Path>) -> Result<serde_json::Value, IoError> {
        let modes = self.get_modes(None, None)?;
        let nodes = self.matrix("Nodes")?;
        let elements = self.matrix("Elements")?;
        if elements.iter().any(|r| r.len() < 8) {
            return Err(broken(
                "Elements requires at least eight columns for JSON export",
            ));
        }
        let positions = nodes.iter().map(|r| [r[1], r[2], r[3]]).collect::<Vec<_>>();
        let connectivity = elements
            .iter()
            .map(|r| [(r[1] - 1.0) as i64, (r[2] - 1.0) as i64])
            .collect::<Vec<_>>();
        let properties=elements.iter().map(|r|serde_json::json!({"shape":"cylinder","type":r[5] as i64,"Diam":(r[7]/std::f64::consts::PI).sqrt()*4.0})).collect::<Vec<_>>();
        let mut output_modes = Vec::new();
        for (prefix, shapes) in [("GY", &modes.guyan), ("CB", &modes.craig_bampton)] {
            let n = shapes.displacement.first().map_or(0, Vec::len);
            for i in 0..n {
                let omega = if prefix == "GY" {
                    1.0
                } else {
                    *modes
                        .frequencies
                        .get(i)
                        .ok_or_else(|| broken("missing CB frequency"))?
                        * 2.0
                        * std::f64::consts::PI
                };
                let displacement = shapes.displacement.iter().map(|r| r[i]).collect::<Vec<_>>();
                output_modes.push(serde_json::json!({"name":format!("{prefix}{}",i+1),"omega":omega,"Displ":displacement}));
            }
        }
        let value = serde_json::json!({"Connectivity":connectivity,"Nodes":positions,"ElemProps":properties,"Modes":output_modes,"groundLevel":nodes.iter().map(|r|r[3]).fold(f64::INFINITY,f64::min)});
        if let Some(path) = path {
            let w = std::io::BufWriter::new(std::fs::File::create(path)?);
            serde_json::to_writer_pretty(w, &value).map_err(|e| IoError::Other(e.to_string()))?;
        }
        Ok(value)
    }
}
impl FileObject for FastSummaryFile {
    fn format_name(&self) -> &str {
        if self.module == "SubDyn" {
            "SubDyn summary file"
        } else {
            "BeamDyn summary file"
        }
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        Ok(vec![NamedFrame::unnamed(self.dataframe(Some(2), true)?)])
    }
    fn info(&self) -> String {
        format!(
            "{} summary: {}",
            self.module,
            self.data.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    }
}
