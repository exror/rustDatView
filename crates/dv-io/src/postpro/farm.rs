//! FAST.Farm radial and downstream wake output channels.
use super::{AverageOptions, SpanwiseChannel};
use crate::openfast_input::FastInputFile;
use dv_core::{Column, DataFrame, DvError, Result};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default)]
pub struct FarmPositions {
    pub radial: Vec<f64>,
    pub downstream: Vec<f64>,
}
impl FarmPositions {
    pub fn from_input(file: &FastInputFile) -> Result<Self> {
        let dr = file
            .number("dr")
            .ok_or_else(|| DvError::msg("FAST.Farm radial increment dr is missing"))?;
        let indices = file
            .numbers("OutRadii")
            .ok_or_else(|| DvError::msg("FAST.Farm OutRadii is missing"))?;
        let downstream = file
            .numbers("OutDist")
            .ok_or_else(|| DvError::msg("FAST.Farm OutDist is missing"))?;
        Ok(Self {
            radial: indices.iter().map(|v| v * dr).collect(),
            downstream,
        })
    }
}
static RADIAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(CtT\d+|WkDfV[rx]T\d+)N(\d+)(D\d+)?_(\[[^\]]+\])$").unwrap());
static DIAMETER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(Wk(?:Axs|Pos|Vel|Diam)[XYZ]T\d+)D(\d+)_(\[[^\]]+\])$").unwrap()
});
pub fn is_fast_farm(df: &DataFrame) -> bool {
    df.columns()
        .iter()
        .any(|c| RADIAL.is_match(&c.name) || DIAMETER.is_match(&c.name))
}
fn group(df: &DataFrame, diameter: bool) -> Vec<SpanwiseChannel> {
    let re = if diameter { &*DIAMETER } else { &*RADIAL };
    let mut groups: Vec<SpanwiseChannel> = Vec::new();
    for (i, col) in df.columns().iter().enumerate() {
        if let Some(cap) = re.captures(&col.name) {
            let Ok(node) = cap[2].parse::<usize>() else {
                continue;
            };
            if node == 0 {
                continue;
            }
            let name = if diameter {
                format!("{}_{}", &cap[1], &cap[3])
            } else {
                format!(
                    "{}{}_{}",
                    &cap[1],
                    cap.get(3).map_or("", |c| c.as_str()),
                    &cap[4]
                )
            };
            let idx = match groups.iter().position(|g| g.name == name) {
                Some(i) => i,
                None => {
                    groups.push(SpanwiseChannel {
                        name,
                        indices: Vec::new(),
                        columns: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            let g = &mut groups[idx];
            if g.indices.contains(&node) {
                continue;
            }
            let at = g.indices.partition_point(|&n| n < node);
            g.indices.insert(at, node);
            g.columns.insert(at, i);
        }
    }
    groups.sort_by(|a, b| a.name.cmp(&b.name));
    groups
}
pub fn spanwise_columns_fast_farm(df: &DataFrame) -> Vec<SpanwiseChannel> {
    group(df, false)
}
pub fn diameterwise_columns_fast_farm(df: &DataFrame) -> Vec<SpanwiseChannel> {
    group(df, true)
}
#[derive(Debug, Clone)]
pub struct FarmPostPro {
    pub radial: DataFrame,
    pub radial_time: DataFrame,
    pub diameter: DataFrame,
}
fn insert_coordinates(df: &mut DataFrame, positions: Option<&[f64]>, axis: &str) -> Result<()> {
    if df.is_empty() {
        return Ok(());
    }
    let n = df.nrows();
    df.insert(
        0,
        Column::float("i_[#]", (1..=n).map(|i| i as f64).collect()),
    )?;
    if let Some(p) = positions {
        if p.len() < n {
            return Err(DvError::msg(format!(
                "Not enough FAST.Farm {axis} coordinates for {n} stations"
            )));
        }
        df.insert(0, Column::float(format!("{axis}_[m]"), p[..n].to_vec()))?;
    }
    // FAST.Farm deliberately uses n, not n-1, for its normalized station coordinate.
    df.push(Column::float(
        "i/n_[-]",
        (0..n).map(|i| i as f64 / n as f64).collect(),
    ))?;
    Ok(())
}
pub fn spanwise_postpro_ff(
    df: &DataFrame,
    options: &AverageOptions,
    positions: Option<&FarmPositions>,
) -> Result<FarmPostPro> {
    let radial_channels = spanwise_columns_fast_farm(df);
    let diameter_channels = diameterwise_columns_fast_farm(df);
    if radial_channels.is_empty() && diameter_channels.is_empty() {
        return Err(DvError::msg(
            "No FAST.Farm spanwise output channels detected",
        ));
    }
    let average = super::average_df(df, options)?;
    // Recompute column indices because average_df can remap or narrow columns.
    let mut radial = if radial_channels.is_empty() {
        DataFrame::new()
    } else {
        super::extract_spanwise_data(&average, &spanwise_columns_fast_farm(&average), 0)?
    };
    let mut diameter = if diameter_channels.is_empty() {
        DataFrame::new()
    } else {
        super::extract_spanwise_data(&average, &diameterwise_columns_fast_farm(&average), 0)?
    };
    insert_coordinates(&mut radial, positions.map(|p| p.radial.as_slice()), "r")?;
    insert_coordinates(
        &mut diameter,
        positions.map(|p| p.downstream.as_slice()),
        "x",
    )?;
    let time = df
        .find("Time_[s]")
        .ok_or_else(|| DvError::msg("FAST.Farm output requires Time_[s]"))?;
    let mut columns = vec![time];
    for c in &radial_channels {
        for &i in &c.columns {
            if !columns.contains(&i) {
                columns.push(i);
            }
        }
    }
    Ok(FarmPostPro {
        radial,
        radial_time: df.select_columns(&columns),
        diameter,
    })
}
pub fn sensors_farm_radial(
    turbines: usize,
    diameters: usize,
    nodes: usize,
    signals: Option<&[String]>,
) -> Vec<String> {
    let mut channels = Vec::new();
    for t in 1..=turbines {
        for n in 1..=nodes {
            channels.push(format!("CtT{t}N{n:02}_[-]"));
        }
    }
    for v in ["x", "r"] {
        for t in 1..=turbines {
            for n in 1..=nodes {
                for d in 1..=diameters {
                    channels.push(format!("WkDfV{v}T{t}N{n:02}D{d}_[m/s]"));
                }
            }
        }
    }
    if let Some(signals) = signals {
        channels.retain(|s| signals.contains(s));
    }
    channels
}
pub fn sensors_farm_diameter(
    turbines: usize,
    diameters: usize,
    signals: Option<&[String]>,
) -> Vec<String> {
    let mut channels = Vec::new();
    for (variable, unit) in [("Axs", "-"), ("Pos", "m"), ("Vel", "m/s"), ("Diam", "m")] {
        for axis in ["X", "Y", "Z"] {
            for t in 1..=turbines {
                for d in 1..=diameters {
                    channels.push(format!("Wk{variable}{axis}T{t}D{d}_[{unit}]"));
                }
            }
        }
    }
    if let Some(signals) = signals {
        channels.retain(|s| signals.contains(s));
    }
    channels
}
