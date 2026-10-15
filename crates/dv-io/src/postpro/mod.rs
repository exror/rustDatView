//! OpenFAST / FAST.Farm post-processing, preserving pyDatView channel and window semantics.
mod farm;
mod maps;
mod positions;
mod subdyn;
pub use farm::*;
pub use positions::*;
pub use subdyn::*;

use dv_core::{Column, ColumnData, DataFrame, DvError, Result};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AverageMethod {
    ConstantWindow,
    Periods,
    PeriodsOmega,
}
#[derive(Debug, Clone)]
pub struct AverageOptions {
    pub method: AverageMethod,
    /// Seconds for ConstantWindow; revolutions for Periods / PeriodsOmega; None means all.
    pub parameter: Option<f64>,
    /// (new name, existing name), as in upstream ColMap.
    pub column_map: Vec<(String, String)>,
    pub keep_columns: Option<Vec<String>>,
}
impl Default for AverageOptions {
    fn default() -> Self {
        Self {
            method: AverageMethod::Periods,
            parameter: None,
            column_map: vec![],
            keep_columns: None,
        }
    }
}
impl AverageOptions {
    pub fn constant_window(seconds: f64) -> Self {
        Self {
            method: AverageMethod::ConstantWindow,
            parameter: Some(seconds),
            ..Self::default()
        }
    }
}
fn numeric<'a>(df: &'a DataFrame, name: &str) -> Result<&'a [f64]> {
    df.get(name)
        .and_then(Column::floats)
        .ok_or_else(|| DvError::msg(format!("Missing numeric channel `{name}`")))
}
fn time_name(df: &DataFrame) -> Result<&str> {
    ["Time_[s]", "Time [s]", "Time"]
        .into_iter()
        .find(|s| df.find(s).is_some())
        .ok_or_else(|| DvError::msg("A time channel is required"))
}
fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values
        .filter(|v| !v.is_nan())
        .fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 {
        f64::NAN
    } else {
        sum / n as f64
    }
}
/// Up-crossings use the index preceding a sign change, including isolated exact zeroes.
fn crossing_indices(psi: &[f64], level: f64) -> Vec<usize> {
    let mut indices = Vec::new();
    for i in 0..psi.len().saturating_sub(1) {
        let a = psi[i] - level;
        let b = psi[i + 1] - level;
        if a < 0.0 && b > 0.0 {
            indices.push(i);
        }
        if i > 0 && a == 0.0 && psi[i - 1] < level && b > 0.0 {
            indices.push(i);
        }
    }
    indices
}
pub fn find_period_time_range(
    df: &DataFrame,
    n_periods: Option<usize>,
    azimuth_column: Option<&str>,
    time_column: Option<&str>,
) -> Result<(f64, f64)> {
    let t = numeric(df, time_column.unwrap_or(time_name(df)?))?;
    let azi = match azimuth_column {
        Some(s) => s,
        None => ["Azimuth_[deg]", "Azimuth [deg]", "Azimuth", "Psi"]
            .into_iter()
            .find(|s| df.find(s).is_some())
            .ok_or_else(|| DvError::msg("Period averaging requires an azimuth channel"))?,
    };
    let psi = numeric(df, azi)?;
    if t.len() < 2 {
        return Err(DvError::msg(
            "Period averaging requires at least two samples",
        ));
    }
    let mut crossings = crossing_indices(psi, psi[psi.len() - 2]);
    if crossings.is_empty() {
        crossings = crossing_indices(psi, 180.0);
    }
    if crossings.len() < 2 {
        return Ok((t[0], t[t.len() - 1]));
    }
    let periods = n_periods
        .unwrap_or(crossings.len() - 1)
        .min(crossings.len() - 1);
    if periods == 0 {
        return Ok((t[0], t[t.len() - 1]));
    }
    Ok((
        t[crossings[crossings.len() - 1 - periods]],
        t[*crossings.last().unwrap()],
    ))
}
pub fn average_df(df: &DataFrame, options: &AverageOptions) -> Result<DataFrame> {
    let time = numeric(df, time_name(df)?)?;
    let first = time
        .iter()
        .copied()
        .find(|v| !v.is_nan())
        .ok_or_else(|| DvError::msg("Time channel contains no samples"))?;
    let last = time.iter().copied().rev().find(|v| !v.is_nan()).unwrap();
    if options.parameter.is_some_and(|v| !v.is_finite() || v < 0.0) {
        return Err(DvError::msg(
            "Averaging parameter must be finite and nonnegative",
        ));
    }
    let (start, end) = match options.method {
        AverageMethod::ConstantWindow => (options.parameter.map_or(first, |v| last - v), last),
        AverageMethod::Periods => {
            find_period_time_range(df, options.parameter.map(|v| v as usize), None, None)?
        }
        AverageMethod::PeriodsOmega => {
            let rpm = mean(numeric(df, "RotSpeed_[rpm]")?.iter().copied());
            if !rpm.is_finite() || rpm <= 0.0 {
                return Err(DvError::msg(
                    "Period averaging requires positive rotor speed",
                ));
            }
            let period = 60.0 / rpm;
            (
                last - period * options.parameter.unwrap_or((last / period).floor()),
                last,
            )
        }
    };
    let rows: Vec<usize> = time
        .iter()
        .enumerate()
        .filter(|(_, t)| **t >= start && **t <= end && !t.is_nan())
        .map(|(i, _)| i)
        .collect();
    if rows.is_empty() {
        return Err(DvError::msg("Averaging window contains no samples"));
    }
    let mut columns = Vec::new();
    for col in df.columns() {
        if !matches!(col.data, ColumnData::Float(_)) {
            continue;
        }
        let name = options
            .column_map
            .iter()
            .find(|(_, old)| old == &col.name)
            .map_or(&col.name, |(new, _)| new);
        if options
            .keep_columns
            .as_ref()
            .is_some_and(|keep| !keep.contains(name))
        {
            continue;
        }
        let v = col.floats().unwrap();
        columns.push(Column::float(name, vec![mean(rows.iter().map(|&i| v[i]))]));
    }
    if let Some(keep) = &options.keep_columns {
        columns.sort_by_key(|c| keep.iter().position(|n| n == &c.name).unwrap_or(usize::MAX));
    }
    DataFrame::from_columns(columns)
}

#[derive(Debug, Clone)]
pub struct AzimuthOptions {
    pub bins: Vec<f64>,
    pub azimuth_column: String,
    pub time_column: String,
    pub start_time: Option<f64>,
    pub periodic: bool,
    pub n_periods: Option<usize>,
}
impl Default for AzimuthOptions {
    fn default() -> Self {
        Self {
            bins: (0..=36).map(|i| i as f64 * 10.0).collect(),
            azimuth_column: "Azimuth_[deg]".into(),
            time_column: "Time_[s]".into(),
            start_time: None,
            periodic: false,
            n_periods: None,
        }
    }
}
/// Right-closed bins (lower edge excluded), matching pandas.cut. Empty bins remain NaN.
pub fn bin_mean_df(df: &DataFrame, bins: &[f64], bin_column: &str) -> Result<DataFrame> {
    if bins.len() < 2
        || bins.iter().any(|v| !v.is_finite())
        || bins.windows(2).any(|w| w[1] <= w[0])
    {
        return Err(DvError::msg(
            "Bin edges must be finite and strictly increasing",
        ));
    }
    let x = numeric(df, bin_column)?;
    let n = bins.len() - 1;
    let mut members = vec![Vec::new(); n];
    for (i, &v) in x.iter().enumerate() {
        if !v.is_finite() || v <= bins[0] || v > bins[n] {
            continue;
        }
        let bin = bins.partition_point(|&b| b < v).saturating_sub(1);
        members[bin].push(i);
    }
    let mut columns = Vec::new();
    for c in df.columns() {
        if let Some(v) = c.floats() {
            columns.push(Column::float(
                &c.name,
                members
                    .iter()
                    .map(|rows| mean(rows.iter().map(|&i| v[i])))
                    .collect(),
            ));
        }
    }
    columns.push(Column::float(
        "Counts",
        members.iter().map(|r| r.len() as f64).collect(),
    ));
    DataFrame::from_columns(columns)
}
pub fn azimuthal_average_df(df: &DataFrame, options: &AzimuthOptions) -> Result<DataFrame> {
    let range = if let Some(n) = options.n_periods {
        Some(find_period_time_range(
            df,
            Some(n),
            Some(&options.azimuth_column),
            Some(&options.time_column),
        )?)
    } else {
        options.start_time.map(|t| (t, f64::INFINITY))
    };
    let filtered;
    let df = if let Some((start, end)) = range {
        let time = numeric(df, &options.time_column)?;
        filtered = df.filter_rows(
            &time
                .iter()
                .map(|&t| t > start && t <= end)
                .collect::<Vec<_>>(),
        );
        &filtered
    } else {
        df
    };
    let mut result = bin_mean_df(df, &options.bins, &options.azimuth_column)?;
    if options.periodic {
        for c in result.columns_mut() {
            if let ColumnData::Float(v) = &mut c.data {
                v.push(v[0]);
            }
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanwiseModule {
    AeroDyn,
    ElastoDyn,
    ElastoDynTower,
    BeamDyn,
}
#[derive(Debug, Clone)]
pub struct SpanwiseChannel {
    pub name: String,
    pub indices: Vec<usize>,
    pub columns: Vec<usize>,
}
static PATTERNS: LazyLock<Vec<Vec<(Regex, &'static str)>>> = LazyLock::new(|| {
    maps::MAPS
        .iter()
        .map(|m| {
            m.iter()
                .map(|(p, n)| (Regex::new(p).expect("upstream channel regex"), *n))
                .collect()
        })
        .collect()
});
/// Index values are one-based output station numbers; source columns are zero-based.
pub fn spanwise_columns(df: &DataFrame, module: SpanwiseModule) -> Vec<SpanwiseChannel> {
    let maps = &*PATTERNS;
    let index = match module {
        SpanwiseModule::AeroDyn => 0,
        SpanwiseModule::ElastoDyn => 1,
        SpanwiseModule::ElastoDynTower => 2,
        SpanwiseModule::BeamDyn => 3,
    };
    matching_columns(df, &maps[index])
}
fn matching_columns(df: &DataFrame, patterns: &[(Regex, &str)]) -> Vec<SpanwiseChannel> {
    patterns
        .iter()
        .filter_map(|(pattern, name)| {
            let mut matches: Vec<(usize, usize)> = df
                .columns()
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let captures = pattern.captures(&c.name)?;
                    let node = captures.get(1)?.as_str().parse::<usize>().ok()?;
                    if node == 0 {
                        None
                    } else {
                        Some((node, i))
                    }
                })
                .collect();
            matches.sort_by_key(|&(node, _)| node);
            // Output postprocessing follows upstream's first occurrence of duplicate input names.
            matches.dedup_by(|a, b| df.column(a.1).name == df.column(b.1).name);
            if matches.is_empty() {
                None
            } else {
                Some(SpanwiseChannel {
                    name: (*name).into(),
                    indices: matches.iter().map(|m| m.0).collect(),
                    columns: matches.iter().map(|m| m.1).collect(),
                })
            }
        })
        .collect()
}
pub fn extract_spanwise_data(
    df: &DataFrame,
    channels: &[SpanwiseChannel],
    row: usize,
) -> Result<DataFrame> {
    if row >= df.nrows() {
        return Err(DvError::msg("Spanwise sample is outside the dataframe"));
    }
    let nodes = channels
        .iter()
        .flat_map(|c| &c.indices)
        .copied()
        .max()
        .unwrap_or(0);
    let mut columns = Vec::new();
    for c in channels {
        let mut values = vec![f64::NAN; nodes];
        for (&node, &col) in c.indices.iter().zip(&c.columns) {
            values[node - 1] = df.f64_col(col)[row];
        }
        columns.push(Column::float(&c.name, values));
    }
    columns.sort_by(|a, b| a.name.cmp(&b.name));
    DataFrame::from_columns(columns)
}
#[derive(Debug, Clone)]
pub struct SpanwiseField {
    pub name: String,
    pub values: Vec<f64>,
    pub nrows: usize,
    pub nodes: usize,
    pub radii: Vec<f64>,
}
#[derive(Debug, Clone, Default)]
pub struct SpanwiseFields {
    pub ad: Vec<SpanwiseField>,
    pub ed: Vec<SpanwiseField>,
    pub ed_tower: Vec<SpanwiseField>,
    pub bd: Vec<SpanwiseField>,
}
fn extract_fields(
    df: &DataFrame,
    module: SpanwiseModule,
    positions: Option<&StationPositions>,
) -> Result<Vec<SpanwiseField>> {
    let channels = spanwise_columns(df, module);
    let nodes = channels
        .iter()
        .flat_map(|c| &c.indices)
        .copied()
        .max()
        .unwrap_or(0);
    let radii = station_coordinates(nodes, positions)?;
    let mut fields = Vec::new();
    for c in channels {
        let mut values = vec![f64::NAN; df.nrows() * nodes];
        for (&node, &col) in c.indices.iter().zip(&c.columns) {
            let source = df.f64_col(col);
            for (r, &v) in source.iter().enumerate() {
                values[r * nodes + node - 1] = v;
            }
        }
        fields.push(SpanwiseField {
            name: c.name,
            values,
            nrows: df.nrows(),
            nodes,
            radii: radii.clone(),
        });
    }
    Ok(fields)
}
pub fn spanwise_postpro_rows(
    df: &DataFrame,
    positions: Option<&SpanwisePositions>,
) -> Result<SpanwiseFields> {
    Ok(SpanwiseFields {
        ad: extract_fields(
            df,
            SpanwiseModule::AeroDyn,
            positions.and_then(|p| p.ad.as_ref()),
        )?,
        ed: extract_fields(
            df,
            SpanwiseModule::ElastoDyn,
            positions.and_then(|p| p.ed.as_ref()),
        )?,
        ed_tower: extract_fields(
            df,
            SpanwiseModule::ElastoDynTower,
            positions.and_then(|p| p.ed_tower.as_ref()),
        )?,
        bd: extract_fields(
            df,
            SpanwiseModule::BeamDyn,
            positions.and_then(|p| p.bd.as_ref()),
        )?,
    })
}
pub fn spanwise_concat(df: &DataFrame) -> Result<DataFrame> {
    let channels = spanwise_columns(df, SpanwiseModule::AeroDyn);
    if channels.is_empty() {
        return Err(DvError::msg(
            "Cannot concatenate: no AeroDyn spanwise channels detected",
        ));
    }
    let time = numeric(df, "Time_[s]")?;
    let nodes = channels
        .iter()
        .flat_map(|c| &c.indices)
        .copied()
        .max()
        .unwrap();
    let mut columns = vec![
        Column::float(
            "Time_[s]",
            (0..nodes).flat_map(|_| time.iter().copied()).collect(),
        ),
        Column::float(
            "i_[-]",
            (1..=nodes)
                .flat_map(|i| std::iter::repeat(i as f64).take(time.len()))
                .collect(),
        ),
    ];
    for c in channels {
        let mut values = vec![f64::NAN; time.len() * nodes];
        for (&node, &col) in c.indices.iter().zip(&c.columns) {
            values[(node - 1) * time.len()..node * time.len()].copy_from_slice(&df.f64_col(col));
        }
        columns.push(Column::float(c.name, values));
    }
    DataFrame::from_columns(columns)
}
/// Radial default is the final five seconds; returns only detected modules.
pub fn radial_avg(
    df: &DataFrame,
    options: &AverageOptions,
    positions: Option<&SpanwisePositions>,
) -> Result<Vec<(String, DataFrame)>> {
    if is_fast_farm(df) {
        let farm = spanwise_postpro_ff(df, options, positions.and_then(|p| p.farm.as_ref()))?;
        let mut out = Vec::new();
        if !farm.radial.is_empty() {
            out.push(("rad".into(), farm.radial));
        }
        if !farm.diameter.is_empty() {
            out.push(("diam".into(), farm.diameter));
        }
        return Ok(out);
    }
    let avg = average_df(df, options)?;
    let mut result = Vec::new();
    for (module, name, pos, scale, axis) in [
        (
            SpanwiseModule::AeroDyn,
            "AD",
            positions.and_then(|p| p.ad.as_ref()),
            positions.and_then(|p| p.radius),
            "r",
        ),
        (
            SpanwiseModule::ElastoDyn,
            "ED",
            positions.and_then(|p| p.ed.as_ref()),
            positions.and_then(|p| p.radius),
            "r",
        ),
        (
            SpanwiseModule::ElastoDynTower,
            "ED_twr",
            positions.and_then(|p| p.ed_tower.as_ref()),
            positions.and_then(|p| p.tower_length),
            "H",
        ),
        (
            SpanwiseModule::BeamDyn,
            "BD",
            positions.and_then(|p| p.bd.as_ref()),
            positions.and_then(|p| p.radius),
            "r",
        ),
    ] {
        let channels = spanwise_columns(&avg, module);
        if channels.is_empty() {
            continue;
        }
        let mut radial = extract_spanwise_data(&avg, &channels, 0)?;
        if module == SpanwiseModule::AeroDyn {
            insert_extra_columns_ad(&mut radial, &avg, positions)?;
        }
        insert_spanwise_columns(&mut radial, pos, scale, axis)?;
        result.push((name.into(), radial));
    }
    if let Some(sd) = positions.and_then(|p| p.subdyn.as_ref()) {
        let (members, joints) = sd.member_postpro(&avg)?;
        if !members.is_empty() {
            result.push(("SD_MembersOut".into(), members));
        }
        if !joints.is_empty() {
            result.push(("SD_JointsOut".into(), joints));
        }
    }
    Ok(result)
}
fn insert_extra_columns_ad(
    radial: &mut DataFrame,
    avg: &DataFrame,
    geometry: Option<&SpanwisePositions>,
) -> Result<()> {
    for blade in 1..=3 {
        let prefix = format!("B{blade}");
        for frame in ["i", "p", "h"] {
            for axis in ["x", "y", "z"] {
                let suffix = format!("{axis}{frame}_[m/s]");
                if let (Some(a), Some(b)) = (
                    radial.get(&format!("{prefix}VDis{suffix}")),
                    radial.get(&format!("{prefix}STV{suffix}")),
                ) {
                    let flow: Vec<_> = a
                        .as_f64()
                        .iter()
                        .zip(b.as_f64().iter())
                        .map(|(x, y)| x - y)
                        .collect();
                    let relative = radial.get(&format!("{prefix}Vind{suffix}")).map(|c| {
                        flow.iter()
                            .zip(c.as_f64().iter())
                            .map(|(x, y)| x + y)
                            .collect()
                    });
                    radial.push(Column::float(format!("{prefix}Vflw{suffix}"), flow))?;
                    if let Some(v) = relative {
                        radial.push(Column::float(format!("{prefix}Vrel{suffix}"), v))?;
                    }
                }
            }
        }
        if let (Some(x), Some(y)) = (
            radial.get(&format!("{prefix}Vrelxp_[m/s]")),
            radial.get(&format!("{prefix}Vrelyp_[m/s]")),
        ) {
            let v = x
                .as_f64()
                .iter()
                .zip(y.as_f64().iter())
                .map(|(x, y)| x.atan2(*y).to_degrees())
                .collect();
            radial.push(Column::float(format!("{prefix}phi_p_[def]"), v))?;
        }
        if let Some(g) = geometry {
            if let (Some(pos), Some(radius), Some(fx), Some(wind)) = (
                g.ad.as_ref(),
                g.radius,
                radial.get(&format!("{prefix}Fx_[N/m]")),
                avg.get("Wind1VelX_[m/s]"),
            ) {
                if pos.radii.len() >= radial.nrows() {
                    let u = wind.as_f64()[0];
                    let ct: Vec<f64> = fx
                        .as_f64()
                        .iter()
                        .zip(&pos.radii)
                        .map(|(f, r)| {
                            if *r < 0.01 * radius {
                                0.0
                            } else {
                                g.blades as f64 * f / (g.density * u * u * std::f64::consts::PI * r)
                            }
                        })
                        .collect();
                    let integral: f64 = ct
                        .windows(2)
                        .zip(pos.radii.windows(2))
                        .map(|(c, r)| {
                            (r[1] - r[0]) * (r[0] * c[0] + r[1] * c[1]) / (radius * radius)
                        })
                        .sum();
                    radial.push(Column::float(format!("{prefix}Ctloc_[-]"), ct))?;
                    radial.push(Column::float(
                        format!("{prefix}CtAvg_[-]"),
                        vec![integral; radial.nrows()],
                    ))?;
                }
            }
            if let (Some(v), Some(cl)) = (
                radial.get(&format!("{prefix}Vrel_[m/s]")),
                radial.get(&format!("{prefix}Cl_[-]")),
            ) {
                if g.chord.len() >= radial.nrows() {
                    let gamma = v
                        .as_f64()
                        .iter()
                        .zip(cl.as_f64().iter())
                        .zip(&g.chord)
                        .map(|((v, cl), c)| 0.5 * c * v * cl)
                        .collect();
                    radial.push(Column::float(format!("{prefix}Gamma_[m^2/s]"), gamma))?;
                }
            }
        }
        for (induction, velocity, out, sign) in [
            ("AxInd", "Vx", "Vindx", -1.0),
            ("TnInd", "Vy", "Vindy", 1.0),
        ] {
            if radial.find(&format!("{prefix}{out}_[m/s]")).is_none() {
                if let (Some(a), Some(v)) = (
                    radial.get(&format!("{prefix}{induction}_[-]")),
                    radial.get(&format!("{prefix}{velocity}_[m/s]")),
                ) {
                    let values = a
                        .as_f64()
                        .iter()
                        .zip(v.as_f64().iter())
                        .map(|(a, v)| sign * a * v)
                        .collect();
                    radial.push(Column::float(format!("{prefix}{out}_[m/s]"), values))?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
