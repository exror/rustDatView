//! Station coordinates derived from OpenFAST input decks.
use super::{AverageOptions, FarmPositions, SubDynGeometry};
use crate::openfast_input::{FastInputDeck, FastInputFile, InputValue};
use dv_core::{Column, DataFrame, DvError, Result};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct StationPositions {
    pub radii: Vec<f64>,
    pub node_indices: Option<Vec<usize>>,
}
#[derive(Debug, Clone)]
pub struct SpanwisePositions {
    pub ad: Option<StationPositions>,
    pub ed: Option<StationPositions>,
    pub ed_tower: Option<StationPositions>,
    pub bd: Option<StationPositions>,
    pub radius: Option<f64>,
    pub tower_length: Option<f64>,
    pub hub_radius: f64,
    pub density: f64,
    pub blades: usize,
    pub chord: Vec<f64>,
    pub farm: Option<FarmPositions>,
    pub subdyn: Option<SubDynGeometry>,
}
impl Default for SpanwisePositions {
    fn default() -> Self {
        Self {
            ad: None,
            ed: None,
            ed_tower: None,
            bd: None,
            radius: None,
            tower_length: None,
            hub_radius: 0.0,
            density: 1.225,
            blades: 3,
            chord: Vec::new(),
            farm: None,
            subdyn: None,
        }
    }
}
fn required_number(file: &FastInputFile, key: &str) -> Result<f64> {
    file.number(key)
        .ok_or_else(|| DvError::msg(format!("Input parameter `{key}` is missing or not numeric")))
}
fn count(file: &FastInputFile, key: &str) -> Result<usize> {
    let n = required_number(file, key)?;
    if !n.is_finite() || n < 0.0 || n.fract() != 0.0 {
        return Err(DvError::msg(format!("Invalid count `{key}`")));
    }
    Ok(n as usize)
}
pub(super) fn table_rows(file: &FastInputFile, key: &str) -> Result<Vec<Vec<f64>>> {
    file.table(key)
        .ok_or_else(|| DvError::msg(format!("Input table `{key}` is missing")))?
        .numeric_rows()
        .map_err(DvError::from)
}
fn stations(n: usize, length: f64, offset: f64) -> Result<(Vec<f64>, Vec<f64>)> {
    if n == 0 {
        return Err(DvError::msg("Station count must be positive"));
    }
    let fraction: Vec<f64> = (0..n).map(|i| (i as f64 + 0.5) / n as f64).collect();
    let positions = fraction.iter().map(|f| f * length + offset).collect();
    Ok((fraction, positions))
}
pub fn ed_blade_stations(file: &FastInputFile) -> Result<(Vec<f64>, Vec<f64>)> {
    stations(
        count(file, "BldNodes")?,
        required_number(file, "TipRad")? - required_number(file, "HubRad")?,
        required_number(file, "HubRad")?,
    )
}
pub fn ed_tower_stations(file: &FastInputFile, add_base: bool) -> Result<(Vec<f64>, Vec<f64>)> {
    let base = required_number(file, "TowerBsHt")?;
    stations(
        count(file, "TwrNodes")?,
        required_number(file, "TowerHt")? - base,
        if add_base { base } else { 0.0 },
    )
}
fn select_gages(
    file: &FastInputFile,
    positions: &[f64],
    count_key: &str,
    index_key: &str,
) -> Result<StationPositions> {
    let n = count(file, count_key)?;
    if n == 0 {
        return Ok(StationPositions {
            radii: vec![],
            node_indices: Some(vec![]),
        });
    }
    let indices = file
        .numbers(index_key)
        .ok_or_else(|| DvError::msg(format!("Gage indices `{index_key}` are missing")))?;
    if indices.len() < n {
        return Err(DvError::msg(format!("Not enough indices in `{index_key}`")));
    }
    let indices: Vec<usize> = indices[..n]
        .iter()
        .map(|&i| {
            if !i.is_finite() || i < 1.0 || i.fract() != 0.0 || i > positions.len() as f64 {
                Err(DvError::msg(format!(
                    "Gage index {i} is outside the input stations"
                )))
            } else {
                Ok(i as usize)
            }
        })
        .collect::<Result<_>>()?;
    Ok(StationPositions {
        radii: indices.iter().map(|&i| positions[i - 1]).collect(),
        node_indices: Some(indices),
    })
}
pub fn ed_blade_gages(file: &FastInputFile) -> Result<StationPositions> {
    select_gages(file, &ed_blade_stations(file)?.1, "NBlGages", "BldGagNd")
}
pub fn ed_tower_gages(file: &FastInputFile, add_base: bool) -> Result<StationPositions> {
    select_gages(
        file,
        &ed_tower_stations(file, add_base)?.1,
        "NTwGages",
        "TwrGagNd",
    )
}
pub fn ad_blade_gages(ad: &FastInputFile, blade: &FastInputFile) -> Result<StationPositions> {
    let rows = table_rows(blade, "BldAeroNodes")?;
    let radii = rows
        .iter()
        .map(|r| {
            r.first()
                .copied()
                .ok_or_else(|| DvError::msg("Empty AeroDyn blade row"))
        })
        .collect::<Result<Vec<_>>>()?;
    select_gages(ad, &radii, "NBlOuts", "BlOutNd")
}
pub fn ad14_blade_gages(ad: &FastInputFile) -> Result<StationPositions> {
    let table = ad
        .table("BldAeroNodes")
        .ok_or_else(|| DvError::msg("AeroDyn14 blade nodes are missing"))?;
    let mut radii = Vec::new();
    let mut indices = Vec::new();
    for (i, row) in table.rows.iter().enumerate() {
        let print = if row.len() == 6 {
            match &row[5] {
                InputValue::Bool(v) => *v,
                InputValue::Text(v) => v.to_lowercase().starts_with('p'),
                _ => false,
            }
        } else {
            true
        };
        if !print {
            continue;
        }
        let r = match row.first() {
            Some(InputValue::Number(v)) => *v,
            Some(InputValue::Integer(v)) => *v as f64,
            Some(InputValue::Text(v)) => v
                .parse()
                .map_err(|_| DvError::msg("Invalid AeroDyn14 radius"))?,
            _ => return Err(DvError::msg("Invalid AeroDyn14 radius")),
        };
        radii.push(r);
        indices.push(i + 1);
    }
    Ok(StationPositions {
        radii,
        node_indices: Some(indices),
    })
}
/// Gauss-Legendre roots, mapped onto a straight BeamDyn member's z extent.
fn gauss_stations(n: usize, length: f64) -> Result<Vec<f64>> {
    if n == 0 {
        return Err(DvError::msg("BeamDyn quadrature needs at least one point"));
    }
    let mut roots = Vec::with_capacity(n);
    for i in 0..n {
        let mut x = (std::f64::consts::PI * (i as f64 + 0.75) / (n as f64 + 0.5)).cos();
        let mut converged = false;
        for _ in 0..100 {
            let mut p0 = 1.0;
            let mut p1 = x;
            for k in 2..=n {
                let p = ((2 * k - 1) as f64 * x * p1 - (k - 1) as f64 * p0) / k as f64;
                p0 = p1;
                p1 = p;
            }
            let (pn, prev) = if n == 1 { (x, 1.0) } else { (p1, p0) };
            let derivative = n as f64 * (x * pn - prev) / (x * x - 1.0);
            let next = x - pn / derivative;
            if (next - x).abs() < 1e-14 {
                x = next;
                converged = true;
                break;
            }
            x = next;
        }
        if !converged {
            return Err(DvError::msg("BeamDyn Gauss quadrature did not converge"));
        }
        roots.push(length * (1.0 + x) / 2.0);
    }
    roots.sort_by(f64::total_cmp);
    Ok(roots)
}
pub fn bd_blade_stations(bd: &FastInputFile, blade: &FastInputFile) -> Result<Vec<f64>> {
    let geometry = table_rows(bd, "MemberGeom")?;
    let z: Vec<f64> = geometry
        .iter()
        .map(|r| {
            r.get(2)
                .copied()
                .ok_or_else(|| DvError::msg("BeamDyn member geometry requires x,y,z"))
        })
        .collect::<Result<_>>()?;
    let length = z
        .last()
        .zip(z.first())
        .map(|(a, b)| a - b)
        .ok_or_else(|| DvError::msg("BeamDyn member geometry is empty"))?;
    match count(bd, "quadrature")? {
        1 => gauss_stations(count(bd, "order_elem")? + 1, length),
        2 => {
            let refine = if bd
                .string("refine")
                .is_some_and(|v| v.to_lowercase().contains("default"))
            {
                1
            } else {
                count(bd, "refine")?
            };
            if refine == 0 {
                return Err(DvError::msg("BeamDyn refinement must be positive"));
            }
            let table = blade
                .table("BeamProperties")
                .ok_or_else(|| DvError::msg("BeamDyn blade properties are missing"))?;
            let spans = table
                .column("span")
                .or_else(|| table.column("Span"))
                .ok_or_else(|| DvError::msg("BeamDyn blade property spans are missing"))?;
            if spans.is_empty() || spans.windows(2).any(|w| w[1] <= w[0]) {
                return Err(DvError::msg("BeamDyn property spans must be increasing"));
            }
            let mut radii = Vec::with_capacity((spans.len() - 1) * refine + 1);
            for w in spans.windows(2) {
                for i in 0..refine {
                    radii.push(length * (w[0] + (w[1] - w[0]) * i as f64 / refine as f64));
                }
            }
            radii.push(length * spans[spans.len() - 1]);
            Ok(radii)
        }
        q => Err(DvError::msg(format!("Unsupported BeamDyn quadrature {q}"))),
    }
}
pub fn bd_blade_gages(bd: &FastInputFile) -> Result<StationPositions> {
    let rows = table_rows(bd, "MemberGeom")?;
    let z = rows
        .iter()
        .map(|r| {
            r.get(2)
                .copied()
                .ok_or_else(|| DvError::msg("Invalid BeamDyn geometry"))
        })
        .collect::<Result<Vec<_>>>()?;
    select_gages(bd, &z, "NNodeOuts", "OutNd")
}
fn nodal(file: &FastInputFile, columns: &[String], module: super::SpanwiseModule) -> bool {
    // AeroDyn's all-node output detection is channel-based in upstream; ED/BD use the input section.
    if module == super::SpanwiseModule::AeroDyn {
        return columns
            .iter()
            .any(|c| c.starts_with("AB1N") || c == "B1N001Cl_[-]");
    }
    file.has_nodal || file.get("OutList_Nodal").is_some() || file.get("BldNd_BlOutNd").is_some()
}
pub fn fast_spanwise_outputs(
    filename: &Path,
    output_columns: &[String],
) -> Result<SpanwisePositions> {
    let deck = FastInputDeck::read_selected(
        filename,
        &[
            "ElastoDyn",
            "AeroDyn14",
            "AeroDyn15",
            "AeroDynBlade",
            "BeamDyn",
            "BeamDynBlade",
            "SubDyn",
        ],
    )?;
    if !deck.missing_files.is_empty() {
        return Err(DvError::msg(format!(
            "Referenced geometry file not found: {}",
            deck.missing_files[0].display()
        )));
    }
    spanwise_positions_from_deck(&deck, output_columns)
}
pub fn spanwise_positions_from_deck(
    deck: &FastInputDeck,
    output_columns: &[String],
) -> Result<SpanwisePositions> {
    let mut out = SpanwisePositions::default();
    let main = deck.file("FAST").or_else(|| deck.file("FASTFarm"));
    if let Some(farm) = deck
        .file("FASTFarm")
        .or_else(|| main.filter(|f| f.get("OutRadii").is_some()))
    {
        out.farm = Some(FarmPositions::from_input(farm)?);
        return Ok(out);
    }
    if let Some(main) = main {
        out.radius = main.number("TipRad");
        out.hub_radius = main
            .number("HubRad")
            .or_else(|| main.number("HubRad(1)"))
            .or_else(|| main.number("BldHubRad_bl(1_1)"))
            .unwrap_or(0.0);
        out.density = main.number("AirDens").unwrap_or(out.density);
    }
    if let Some(ed) = deck.file("ED") {
        out.radius = Some(required_number(ed, "TipRad")?);
        out.hub_radius = required_number(ed, "HubRad")?;
        out.blades = ed.integer("NumBl").unwrap_or(3).max(1) as usize;
        out.ed = Some(
            if nodal(ed, output_columns, super::SpanwiseModule::ElastoDyn) {
                StationPositions {
                    radii: ed_blade_stations(ed)?.1,
                    node_indices: None,
                }
            } else {
                ed_blade_gages(ed)?
            },
        );
        out.tower_length =
            Some(required_number(ed, "TowerHt")? - required_number(ed, "TowerBsHt")?);
        out.ed_tower = Some(ed_tower_gages(ed, true)?);
    }
    if let Some(ad) = deck.file("AD").or_else(|| deck.file("AeroDyn14")) {
        out.density = ad
            .number("Rho")
            .or_else(|| ad.number("AirDens"))
            .unwrap_or(out.density);
        if let Some(blade) = deck.file("AeroDynBlade") {
            let rows = table_rows(blade, "BldAeroNodes")?;
            let all_r = rows
                .iter()
                .map(|r| {
                    r.first()
                        .copied()
                        .ok_or_else(|| DvError::msg("Invalid AeroDyn blade nodes"))
                })
                .collect::<Result<Vec<_>>>()?;
            if out.radius.is_none() {
                out.radius = all_r.last().copied();
            }
            let full = nodal(ad, output_columns, super::SpanwiseModule::AeroDyn);
            let mut stations = if full {
                StationPositions {
                    radii: all_r,
                    node_indices: None,
                }
            } else {
                ad_blade_gages(ad, blade)?
            };
            out.chord = if full {
                rows.iter()
                    .map(|r| {
                        r.get(5)
                            .copied()
                            .ok_or_else(|| DvError::msg("AeroDyn blade chord is missing"))
                    })
                    .collect::<Result<_>>()?
            } else {
                stations
                    .node_indices
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|&i| {
                        rows[i - 1]
                            .get(5)
                            .copied()
                            .ok_or_else(|| DvError::msg("AeroDyn blade chord is missing"))
                    })
                    .collect::<Result<_>>()?
            };
            for r in &mut stations.radii {
                *r += out.hub_radius;
            }
            out.ad = Some(stations);
        } else if ad.table("BldAeroNodes").is_some() {
            out.ad = Some(ad14_blade_gages(ad)?);
        } else {
            return Err(DvError::msg("AeroDyn blade file could not be loaded"));
        }
    }
    if let Some(bd) = deck.file("BD") {
        let mut stations = if nodal(bd, output_columns, super::SpanwiseModule::BeamDyn) {
            StationPositions {
                radii: bd_blade_stations(
                    bd,
                    deck.file("BeamDynBlade")
                        .ok_or_else(|| DvError::msg("BeamDyn blade file could not be loaded"))?,
                )?,
                node_indices: None,
            }
        } else {
            bd_blade_gages(bd)?
        };
        for r in &mut stations.radii {
            *r += out.hub_radius;
        }
        if out.radius.is_none() {
            out.radius = stations.radii.last().copied();
        }
        out.bd = Some(stations);
    }
    if let Some(sd) = deck.file("SD") {
        out.subdyn = Some(SubDynGeometry::from_input(sd)?);
    }
    Ok(out)
}
pub(super) fn station_coordinates(
    nodes: usize,
    positions: Option<&StationPositions>,
) -> Result<Vec<f64>> {
    if let Some(p) = positions {
        if p.radii.len() < nodes {
            return Err(DvError::msg(format!(
                "{} radial positions for {nodes} output stations",
                p.radii.len()
            )));
        }
        Ok(p.radii[..nodes].to_vec())
    } else {
        Ok((0..nodes)
            .map(|i| i as f64 / (nodes.saturating_sub(1)) as f64)
            .collect())
    }
}
pub fn insert_spanwise_columns(
    df: &mut DataFrame,
    positions: Option<&StationPositions>,
    scale: Option<f64>,
    axis: &str,
) -> Result<()> {
    let n = df.nrows();
    if n == 0 {
        return Ok(());
    }
    let r = station_coordinates(n, positions)?;
    let (name, normalized) = if positions.is_some() && scale.is_some() {
        (
            if axis == "H" {
                "H/L_[-]".into()
            } else {
                format!("{axis}/R_[-]")
            },
            r.iter().map(|v| v / scale.unwrap()).collect(),
        )
    } else {
        (
            "i/n_[-]".into(),
            (0..n)
                .map(|i| i as f64 / n.saturating_sub(1) as f64)
                .collect(),
        )
    };
    df.insert(0, Column::float(name, normalized))?;
    if let Some(indices) = positions.and_then(|p| p.node_indices.as_ref()) {
        if indices.len() < n {
            return Err(DvError::msg("Not enough node indices for spanwise outputs"));
        }
        df.push(Column::float(
            "Node_[#]",
            indices[..n].iter().map(|&v| v as f64).collect(),
        ))?;
    }
    df.push(Column::float("i_[#]", (1..=n).map(|i| i as f64).collect()))?;
    if positions.is_some() {
        df.push(Column::float(format!("{axis}_[m]"), r))?;
    }
    Ok(())
}
/// Discover the companion driver without suppressing input parsing failures.
pub fn radial_avg_file(
    output: &Path,
    df: &DataFrame,
    options: &AverageOptions,
) -> Result<Vec<(String, DataFrame)>> {
    let mut stem = output.with_extension("");
    if let Some(name) = stem.file_name().and_then(|s| s.to_str()).map(str::to_owned) {
        if let Some(i) = name.to_lowercase().find(".ad") {
            stem.set_file_name(&name[..i]);
        }
    }
    let extensions: &[&str] = if super::is_fast_farm(df) {
        &["fstf", "FSTF", "Fstf", "fmas", "FMAS", "Fmas"]
    } else {
        &["fst", "FST", "Fst", "dvr", "DVR", "Dvr"]
    };
    let input = extensions
        .iter()
        .map(|ext| {
            let mut name = stem.as_os_str().to_os_string();
            name.push(".");
            name.push(ext);
            std::path::PathBuf::from(name)
        })
        .find(|p| p.is_file());
    let positions = input
        .as_deref()
        .map(|p| fast_spanwise_outputs(p, &df.names()))
        .transpose()?;
    super::radial_avg(df, options, positions.as_ref())
}
