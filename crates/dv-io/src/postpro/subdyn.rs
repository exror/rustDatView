//! SubDyn member/node geometry and output-channel postprocessing (no FEM dependency).
use crate::openfast_input::FastInputFile;
use dv_core::{Column, DataFrame, DvError, Result};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct SubDynPoint {
    pub label: String,
    pub coordinates: [f64; 3],
}
#[derive(Debug, Clone, Default)]
pub struct SubDynGeometry {
    pub member_nodes: Vec<SubDynPoint>,
    pub member_joints: Vec<SubDynPoint>,
    pub output_nodes: Vec<SubDynPoint>,
}
fn integer(v: f64, description: &str) -> Result<usize> {
    if !v.is_finite() || v < 1.0 || v.fract() != 0.0 {
        Err(DvError::msg(format!("Invalid SubDyn {description}: {v}")))
    } else {
        Ok(v as usize)
    }
}
impl SubDynGeometry {
    pub fn from_input(file: &FastInputFile) -> Result<Self> {
        let joints = super::positions::table_rows(file, "Joints")?;
        let members = super::positions::table_rows(file, "Members")?;
        let outputs = super::positions::table_rows(file, "MemberOuts")?;
        let ndiv = integer(
            file.number("NDiv")
                .ok_or_else(|| DvError::msg("SubDyn NDiv is missing"))?,
            "NDiv",
        )?;
        let mut joint_map = BTreeMap::new();
        for row in joints {
            if row.len() < 4 {
                return Err(DvError::msg("SubDyn joints require ID,x,y,z"));
            }
            joint_map.insert(integer(row[0], "joint ID")?, [row[1], row[2], row[3]]);
        }
        let mut out = Self::default();
        let mut member_map = BTreeMap::new();
        for (i, row) in members.iter().enumerate() {
            if row.len() < 5 {
                return Err(DvError::msg(
                    "SubDyn members require ID,joint1,joint2,property1,property2",
                ));
            }
            let id = integer(row[0], "member ID")?;
            let a = joint_map
                .get(&integer(row[1], "member first joint")?)
                .ok_or_else(|| DvError::msg("SubDyn member references an unknown joint"))?;
            let b = joint_map
                .get(&integer(row[2], "member last joint")?)
                .ok_or_else(|| DvError::msg("SubDyn member references an unknown joint"))?;
            // Cable and rigid members are not subdivided by the upstream graph conversion.
            let divisions = if row.get(5).is_some_and(|&t| t == 2.0 || t == 3.0) {
                1
            } else {
                ndiv
            };
            let coordinates: Vec<[f64; 3]> = (0..=divisions)
                .map(|n| {
                    let fraction = n as f64 / divisions as f64;
                    std::array::from_fn(|axis| a[axis] + fraction * (b[axis] - a[axis]))
                })
                .collect();
            for (n, &p) in coordinates.iter().enumerate() {
                out.member_nodes.push(SubDynPoint {
                    label: format!("M{}N{}", i + 1, n + 1),
                    coordinates: p,
                });
            }
            out.member_joints.push(SubDynPoint {
                label: format!("M{}J1", i + 1),
                coordinates: *a,
            });
            out.member_joints.push(SubDynPoint {
                label: format!("M{}J2", i + 1),
                coordinates: *b,
            });
            member_map.insert(id, coordinates);
        }
        for (i, row) in outputs.iter().enumerate() {
            if row.len() < 2 {
                return Err(DvError::msg(
                    "SubDyn MemberOuts requires member ID and output node count",
                ));
            }
            let member = member_map
                .get(&integer(row[0], "output member ID")?)
                .ok_or_else(|| DvError::msg("SubDyn output references an unknown member"))?;
            let count = if row[1] == 0.0 {
                0
            } else {
                integer(row[1], "output node count")?
            };
            if row.len() < 2 + count {
                return Err(DvError::msg(
                    "SubDyn MemberOuts row has fewer nodes than requested",
                ));
            }
            for (n, &node) in row[2..2 + count].iter().enumerate() {
                let index = integer(node, "output node index")?;
                let p = member.get(index - 1).ok_or_else(|| {
                    DvError::msg("SubDyn output node exceeds member discretization")
                })?;
                out.output_nodes.push(SubDynPoint {
                    label: format!("M{}N{}", i + 1, n + 1),
                    coordinates: *p,
                });
            }
        }
        Ok(out)
    }
    pub fn points_member_nodes(&self) -> Result<DataFrame> {
        point_frame(&self.member_nodes)
    }
    pub fn points_member_joints(&self) -> Result<DataFrame> {
        point_frame(&self.member_joints)
    }
    pub fn points_output_nodes(&self) -> Result<DataFrame> {
        point_frame(&self.output_nodes)
    }
    /// Input must be time-averaged (exactly one row); angular member outputs convert rad to deg.
    pub fn member_postpro(&self, average: &DataFrame) -> Result<(DataFrame, DataFrame)> {
        if average.nrows() != 1 {
            return Err(DvError::msg(
                "SubDyn member postprocessing requires one averaged row",
            ));
        }
        Ok((
            extract(average, &self.output_nodes, 'N')?,
            extract(average, &self.member_joints, 'J')?,
        ))
    }
}
fn point_frame(points: &[SubDynPoint]) -> Result<DataFrame> {
    DataFrame::from_columns(vec![
        Column::string("Point", points.iter().map(|p| p.label.clone()).collect()),
        Column::float("x_[m]", points.iter().map(|p| p.coordinates[0]).collect()),
        Column::float("y_[m]", points.iter().map(|p| p.coordinates[1]).collect()),
        Column::float("z_[m]", points.iter().map(|p| p.coordinates[2]).collect()),
    ])
}
static CHANNEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(M\d+[NJ]\d+)(TD[xyz]ss|RD[xyz]e|[FM][KM][xyz]e)_\[(m|rad|N|N\*m|N-m|Nm)\]$")
        .unwrap()
});
fn extract(average: &DataFrame, points: &[SubDynPoint], kind: char) -> Result<DataFrame> {
    let mut names: Vec<String> = Vec::new();
    let mut data: Vec<Vec<f64>> = Vec::new();
    for col in average.columns() {
        let Some(c) = CHANNEL.captures(&col.name) else {
            continue;
        };
        if !c[1].contains(kind) {
            continue;
        }
        let variable = &c[2];
        let unit = &c[3];
        let supported = if kind == 'N' {
            variable.starts_with("TD")
                || variable.starts_with("RD")
                || variable.starts_with("FK")
                || variable.starts_with("MK")
        } else {
            variable.starts_with("FK")
                || variable.starts_with("MK")
                || variable.starts_with("FM")
                || variable.starts_with("MM")
        };
        if !supported {
            continue;
        }
        let label = &c[1];
        let index = points
            .iter()
            .position(|p| p.label == label)
            .ok_or_else(|| {
                DvError::msg(format!(
                    "SubDyn output {label} has no input-deck coordinate"
                ))
            })?;
        let output_unit = match unit {
            "rad" => "deg",
            "N*m" | "N-m" => "Nm",
            s => s,
        };
        let name = format!("{variable}_[{output_unit}]");
        let ic = match names.iter().position(|n| n == &name) {
            Some(i) => i,
            None => {
                names.push(name);
                data.push(vec![f64::NAN; points.len()]);
                names.len() - 1
            }
        };
        let value = col.as_f64()[0];
        data[ic][index] = if unit == "rad" {
            value.to_degrees()
        } else {
            value
        };
    }
    if names.is_empty() {
        return Ok(DataFrame::new());
    }
    let indices: Vec<usize> = (0..points.len())
        .filter(|&i| data.iter().any(|v| !v[i].is_nan()))
        .collect();
    let selected: Vec<SubDynPoint> = indices.iter().map(|&i| points[i].clone()).collect();
    let mut result = point_frame(&selected)?;
    for (name, values) in names.into_iter().zip(data) {
        result.push(Column::float(
            name,
            indices.iter().map(|&i| values[i]).collect(),
        ))?;
    }
    Ok(result)
}
