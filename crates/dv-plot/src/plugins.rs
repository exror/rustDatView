//! The non-GUI behavior of the built-in pyDatView plugins.
use dv_core::{Column, ColumnData, DataFrame, DvError, Result, Table};
use dv_tools::signal_analysis::{apply_filter, apply_sampler, Filter, Sampler};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Plugin {
    Mask { expression: String },
    Filter { name: String, param: f64 },
    Sampler { name: String, param: Vec<f64> },
    Binning { bins: usize, min: f64, max: f64 },
    Outliers { median_deviation: f64 },
    Units { flavor: String },
    RenameChannels { flavor: String },
    RadialAverage { method: String, param: f64 },
    RadialConcat,
    Concat { common_only: bool },
}
impl Plugin {
    pub fn filter() -> Self {
        Self::Filter {
            name: "Moving average".into(),
            param: 100.0,
        }
    }
    pub fn sampler() -> Self {
        Self::Sampler {
            name: "Every n".into(),
            param: vec![2.0],
        }
    }
    pub fn outliers() -> Self {
        Self::Outliers {
            median_deviation: 5.0,
        }
    }
    pub fn radial_average() -> Self {
        Self::RadialAverage {
            method: "constantwindow".into(),
            param: 2.0,
        }
    }
    pub fn is_plot(&self) -> bool {
        matches!(
            self,
            Self::Filter { .. }
                | Self::Sampler { .. }
                | Self::Binning { .. }
                | Self::Outliers { .. }
        )
    }
    pub fn is_reversible(&self) -> bool {
        matches!(self, Self::Mask { .. } | Self::RenameChannels { .. })
    }
    pub fn is_adder(&self) -> bool {
        matches!(
            self,
            Self::RadialAverage { .. } | Self::RadialConcat | Self::Concat { .. }
        )
    }
    pub fn apply_xy(&self, x: &[f64], y: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
        if x.len() != y.len() {
            return Err(DvError::msg("x and y lengths differ"));
        }
        match self {
            Self::Filter { name, param } => {
                Ok((x.to_vec(), apply_filter(x, y, &filter(name, *param)?)?))
            }
            Self::Sampler { name, param } => apply_sampler(x, y, &sampler(name, param)?),
            Self::Binning { bins, min, max } => dv_tools::stats::bin_signal(
                x,
                y,
                &bin_edges(*bins, *min, *max)?,
                &dv_tools::stats::BinStatistic::Mean,
            ),
            Self::Outliers { median_deviation } => {
                if !median_deviation.is_finite() || *median_deviation < 0.0 {
                    return Err(DvError::msg(
                        "Median deviation must be nonnegative and finite",
                    ));
                }
                Ok((
                    x.to_vec(),
                    dv_tools::signal_analysis::reject_outliers(y, *median_deviation, true),
                ))
            }
            _ => Err(DvError::msg(
                "This plugin operates on tables, not plot data",
            )),
        }
    }
    pub fn apply_table(&self, table: &mut Table) -> Result<()> {
        match self {
            Self::Mask { expression } => {
                let previous = table.mask.take();
                let previous_string = std::mem::take(&mut table.mask_string);
                if let Err(e) = table.apply_mask_string(expression, false) {
                    table.mask = previous;
                    table.mask_string = previous_string;
                    return Err(e);
                }
                Ok(())
            }
            Self::Units { flavor } => {
                let f = match flavor.as_str() {
                    "SI" => dv_tools::pandalib::UnitFlavor::SI,
                    "WE" => dv_tools::pandalib::UnitFlavor::WE,
                    _ => return Err(DvError::msg("Unit flavor must be SI or WE")),
                };
                dv_tools::pandalib::change_units(&mut table.data, f)
            }
            Self::RenameChannels { flavor } => rename_channels(&mut table.data, flavor),
            _ => Err(DvError::msg(
                "This plugin must be applied as a plot filter or table adder",
            )),
        }
    }
    /// Create tables for the full-data variants of plot actions. `x_column` is positional.
    pub fn add_tables(&self, table: &Table, x_column: usize) -> Result<Vec<Table>> {
        match self {
            Self::Mask { expression } => {
                let mask = dv_core::formula::eval_mask(&table.data, expression)?;
                if !mask.iter().any(|&v| v) {
                    return Err(DvError::msg("Mask returned no values"));
                }
                Ok(vec![Table::from_df(
                    table.data.filter_rows(&mask),
                    &format!("{}_masked", table.raw_name()),
                )])
            }
            Self::RadialAverage { method, param } => {
                use dv_io::postpro::{AverageMethod, AverageOptions};
                let mut options = AverageOptions::constant_window(*param);
                options.method = match method.as_str() {
                    "constantwindow" => AverageMethod::ConstantWindow,
                    "periods" => AverageMethod::Periods,
                    "periods_omega" => AverageMethod::PeriodsOmega,
                    _ => return Err(DvError::msg("Unknown radial averaging method")),
                };
                let frames = if table.filename.is_empty() {
                    dv_io::postpro::radial_avg(&table.data, &options, None)?
                } else {
                    dv_io::postpro::radial_avg_file(
                        std::path::Path::new(&table.filename),
                        &table.data,
                        &options,
                    )?
                };
                if frames.is_empty() {
                    return Err(DvError::msg("No OpenFAST radial data found"));
                }
                Ok(frames
                    .into_iter()
                    .map(|(name, df)| Table::from_df(df, &name))
                    .collect())
            }
            Self::RadialConcat => Ok(vec![Table::from_df(
                dv_io::postpro::spanwise_concat(&table.data)?,
                &format!("{}_concat", table.name()),
            )]),
            Self::Concat { .. } => Err(DvError::msg(
                "Concatenation requires multiple tables; use Pipeline::apply",
            )),
            _ if self.is_plot() => {
                if x_column >= table.ncols() {
                    return Err(DvError::msg("x column out of bounds"));
                }
                let mut df = match self {
                    Self::Filter { name, param } => dv_tools::signal_analysis::apply_filter_df(
                        &table.data,
                        x_column,
                        &filter(name, *param)?,
                    )?,
                    Self::Sampler { name, param } => dv_tools::signal_analysis::apply_sampler_df(
                        &table.data,
                        x_column,
                        &sampler(name, param)?,
                    )?,
                    Self::Binning { bins, min, max } => {
                        let binned = dv_tools::stats::bin_df(
                            &table.data,
                            &bin_edges(*bins, *min, *max)?,
                            x_column,
                            dv_tools::stats::BinStatistic::Mean,
                        )?;
                        let mut order = vec![x_column];
                        order.extend((0..binned.ncols()).filter(|&i| {
                            i != x_column && !binned.column(i).name.eq_ignore_ascii_case("index")
                        }));
                        binned.select_columns(&order)
                    }
                    Self::Outliers { .. } => {
                        let x = table.data.column(x_column);
                        let xv = numeric_column(x)?;
                        let mut columns = Vec::with_capacity(table.ncols());
                        for (i, col) in table.data.columns().iter().enumerate() {
                            if i == x_column {
                                columns.push(col.clone());
                                continue;
                            }
                            let (_, values) = self.apply_xy(xv, numeric_column(col)?)?;
                            columns.push(Column::float(col.name.clone(), values));
                        }
                        DataFrame::from_columns(columns)?
                    }
                    _ => unreachable!(),
                };
                if df.ncols() > 0 && df.column(0).name == "Index" {
                    let n = df.nrows();
                    df.set(
                        0,
                        Column::float("Index", (0..n).map(|i| i as f64).collect()),
                    )?;
                }
                let suffix = match self {
                    Self::Filter { .. } => "filtered",
                    Self::Sampler { .. } => "resampled",
                    Self::Binning { .. } => "binned",
                    _ => "outliers",
                };
                Ok(vec![Table::from_df(
                    df,
                    &format!("{}_{suffix}", table.raw_name()),
                )])
            }
            _ => Err(DvError::msg(
                "This table action does not generate new tables",
            )),
        }
    }
}
fn numeric_column(col: &Column) -> Result<&[f64]> {
    match &col.data {
        ColumnData::Float(v) | ColumnData::DateTime(v) => Ok(v),
        ColumnData::Str(_) => Err(DvError::msg(format!("Column {} is not numeric", col.name))),
    }
}
fn filter(name: &str, param: f64) -> Result<Filter> {
    if !param.is_finite() || param <= 0.0 {
        return Err(DvError::msg("Filter parameter must be positive and finite"));
    }
    match name {
        "Moving average" if param.round_ties_even() >= 1.0 => {
            Ok(Filter::MovingAverage(param.round_ties_even() as usize))
        }
        "Low pass 1st order" => Ok(Filter::LowPass(param)),
        "High pass 1st order" => Ok(Filter::HighPass(param)),
        _ => Err(DvError::msg(format!(
            "Unsupported filter or parameter {name}"
        ))),
    }
}
fn sampler(name: &str, param: &[f64]) -> Result<Sampler> {
    let first = || {
        param
            .first()
            .copied()
            .ok_or_else(|| DvError::msg("Sampler requires a parameter"))
    };
    match name {
        "Replace" => Ok(Sampler::Replace(param.to_vec())),
        "Insert" => Ok(Sampler::Insert(param.to_vec())),
        "Remove" => Ok(Sampler::Remove(param.to_vec())),
        "Every n" => {
            let n = first()?;
            if !n.is_finite() || n.trunc() == 0.0 {
                return Err(DvError::msg("Every n requires a nonzero stride"));
            }
            Ok(Sampler::EveryN(n as isize))
        }
        "Linspace" if param.len() == 3 => {
            if !param[2].is_finite() || param[2] < 1.0 {
                return Err(DvError::msg("Linspace count must be positive"));
            }
            Ok(Sampler::Linspace {
                min: param[0],
                max: param[1],
                count: param[2] as usize,
            })
        }
        "Time-based" => Ok(Sampler::TimeBased(first()?)),
        "Delta x" if param.len() == 1 || param.len() == 3 => Ok(Sampler::DeltaX {
            step: param[0],
            min: param.get(1).copied().filter(|v| !v.is_nan()),
            max: param.get(2).copied().filter(|v| !v.is_nan()),
        }),
        _ => Err(DvError::msg(format!(
            "Unsupported sampler or parameters {name}"
        ))),
    }
}
fn bin_edges(bins: usize, min: f64, max: f64) -> Result<Vec<f64>> {
    if bins == 0 || !min.is_finite() || !max.is_finite() || min >= max {
        return Err(DvError::msg(
            "Binning requires positive bins and xmin < xmax",
        ));
    }
    Ok((0..=bins)
        .map(|i| min + (max - min) * i as f64 / bins as f64)
        .collect())
}
pub fn rename_channels(df: &mut DataFrame, flavor: &str) -> Result<()> {
    match flavor {
        "OF2To3" | "OF3To2" => {
            let forward = flavor == "OF2To3";
            let re = regex::Regex::new(if forward { r"^B(\dN)" } else { r"^AB(\dN)" })
                .map_err(|e| DvError::msg(e.to_string()))?;
            let replacements = if forward {
                [("AOA_", "Alpha_"), ("AIn_", "AxInd_"), ("ApI_", "TnInd_")]
            } else {
                [("Alpha_", "AOA_"), ("AxInd_", "AIn_"), ("TnInd_", "ApI_")]
            };
            for c in df.columns_mut() {
                c.name = re
                    .replace(&c.name, if forward { "AB${1}" } else { "B${1}" })
                    .into_owned();
                for (old, new) in replacements {
                    c.name = c.name.replace(old, new);
                }
            }
            Ok(())
        }
        "FldToAero" | "AeroToFld" => {
            let (a, b) = if flavor == "FldToAero" {
                ("Fld", "Aero")
            } else {
                ("Aero", "Fld")
            };
            for c in df.columns_mut() {
                c.name = c.name.replace(a, b);
            }
            Ok(())
        }
        _ => Err(DvError::msg(
            "Channel rename flavor must be OF2To3, OF3To2, FldToAero or AeroToFld",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rename_round_trip() {
        let mut df = DataFrame::from_floats(["B1N001AOA_ [deg]"], vec![vec![1.]]).unwrap();
        let old = df.clone();
        rename_channels(&mut df, "OF2To3").unwrap();
        assert_eq!(df.column(0).name, "AB1N001Alpha_ [deg]");
        rename_channels(&mut df, "OF3To2").unwrap();
        assert_eq!(df, old);
    }
    #[test]
    fn unknown_plugin_options_are_errors() {
        assert!(Plugin::Filter {
            name: "invalid".into(),
            param: 1.
        }
        .apply_xy(&[0., 1.], &[1., 2.])
        .is_err());
    }
    #[test]
    fn failed_mask_preserves_existing_selection() {
        let mut t = Table::from_df(
            DataFrame::from_floats(["x"], vec![vec![0., 1., 2.]]).unwrap(),
            "t",
        );
        Plugin::Mask {
            expression: "{x}>0".into(),
        }
        .apply_table(&mut t)
        .unwrap();
        let old = t.mask.clone();
        assert!(Plugin::Mask {
            expression: "{missing}>0".into()
        }
        .apply_table(&mut t)
        .is_err());
        assert_eq!(t.mask, old);
        assert_eq!(t.mask_string, "{x}>0");
    }
    #[test]
    fn fractional_sampling_and_filter_parameters_match_upstream() {
        assert_eq!(
            filter("Moving average", 2.5).unwrap(),
            Filter::MovingAverage(2)
        );
        assert_eq!(sampler("Every n", &[2.9]).unwrap(), Sampler::EveryN(2));
        assert!(sampler("Delta x", &[1., 0.]).is_err());
    }
}
