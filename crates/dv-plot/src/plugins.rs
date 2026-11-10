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

/// A built-in pipeline step and a valid, editable initial configuration.
///
/// IDs are the serialized [`Plugin`] variant names. Some steps still require
/// suitable input data, such as OpenFAST channels for nodal processing.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub default: Plugin,
}

impl Plugin {
    /// The built-in processing steps, in menu order.
    pub fn catalog() -> Vec<PluginInfo> {
        vec![
            PluginInfo {
                id: "Mask",
                label: "Mask rows",
                description: "Keep rows matching an expression.",
                default: Self::Mask {
                    expression: "True".into(),
                },
            },
            PluginInfo {
                id: "Filter",
                label: "Filter",
                description: "Smooth or frequency-filter signals.",
                default: Self::filter(),
            },
            PluginInfo {
                id: "Sampler",
                label: "Resample",
                description: "Change the signal's sample coordinates.",
                default: Self::sampler(),
            },
            PluginInfo {
                id: "Binning",
                label: "Bin values",
                description: "Average samples in equally spaced x bins.",
                default: Self::Binning {
                    bins: 50,
                    min: 0.0,
                    max: 1.0,
                },
            },
            PluginInfo {
                id: "Outliers",
                label: "Remove outliers",
                description: "Replace values far from the median with gaps.",
                default: Self::outliers(),
            },
            PluginInfo {
                id: "Units",
                label: "Standardize units",
                description: "Convert recognized units to a standard convention.",
                default: Self::Units {
                    flavor: "SI".into(),
                },
            },
            PluginInfo {
                id: "RenameChannels",
                label: "Rename channels",
                description: "Convert channel names between OpenFAST conventions.",
                default: Self::RenameChannels {
                    flavor: "OF2To3".into(),
                },
            },
            PluginInfo {
                id: "RadialAverage",
                label: "Nodal average",
                description: "Average OpenFAST channels across a time window or revolutions.",
                default: Self::radial_average(),
            },
            PluginInfo {
                id: "RadialConcat",
                label: "Nodal concatenation",
                description: "Combine OpenFAST spanwise channels into a new table.",
                default: Self::RadialConcat,
            },
            PluginInfo {
                id: "Concat",
                label: "Concatenate tables",
                description: "Combine rows from the active tables.",
                default: Self::Concat { common_only: false },
            },
        ]
    }

    /// A concise, human-readable description of this configured step.
    pub fn summary(&self) -> String {
        match self {
            Self::Mask { expression } => {
                let expression = expression.trim();
                if expression.is_empty() || expression.eq_ignore_ascii_case("no mask") {
                    "Mask · no mask".into()
                } else {
                    format!("Mask · {expression}")
                }
            }
            Self::Filter { name, param } => match name.as_str() {
                "Moving average" => format!(
                    "Moving average · {param} {}",
                    if *param == 1.0 { "sample" } else { "samples" }
                ),
                "Low pass 1st order" => format!("Low-pass 1st order · {param} Hz"),
                "High pass 1st order" => format!("High-pass 1st order · {param} Hz"),
                _ => format!("Filter · {name} · {param}"),
            },
            Self::Sampler { name, param } => match (name.as_str(), param.as_slice()) {
                ("Replace", values) => {
                    format!("Replace coordinates · {}", coordinates_summary(values))
                }
                ("Insert", values) => {
                    format!("Insert coordinates · {}", coordinates_summary(values))
                }
                ("Remove", values) => {
                    format!("Remove coordinates · {}", coordinates_summary(values))
                }
                ("Every n", [stride, ..]) => format!("Every n · stride {stride}"),
                ("Linspace", [start, end, count]) => format!(
                    "Evenly spaced · {start}–{end} · {count} {}",
                    if *count == 1.0 { "point" } else { "points" }
                ),
                ("Time-based", [interval, ..]) => format!("Time-based · {interval} x units"),
                ("Delta x", [step]) => format!("Delta x · {step}"),
                ("Delta x", [step, min, max]) => {
                    let min = if min.is_nan() {
                        "auto".into()
                    } else {
                        min.to_string()
                    };
                    let max = if max.is_nan() {
                        "auto".into()
                    } else {
                        max.to_string()
                    };
                    format!("Delta x · {step} · {min}–{max}")
                }
                _ => format!("Resample · {name} · {}", coordinates_summary(param)),
            },
            Self::Binning { bins, min, max } => format!(
                "Binning · {bins} {} · {min}–{max}",
                if *bins == 1 { "bin" } else { "bins" }
            ),
            Self::Outliers { median_deviation } if *median_deviation == 0.0 => {
                "Remove outliers · disabled".into()
            }
            Self::Outliers { median_deviation } => {
                format!("Remove outliers · {median_deviation} × median deviation")
            }
            Self::Units { flavor } => format!("Standardize units · {flavor}"),
            Self::RenameChannels { flavor } => {
                let label = match flavor.as_str() {
                    "OF2To3" => "OpenFAST 2 → 3",
                    "OF3To2" => "OpenFAST 3 → 2",
                    "FldToAero" => "Fld → Aero",
                    "AeroToFld" => "Aero → Fld",
                    other => other,
                };
                format!("Rename channels · {label}")
            }
            Self::RadialAverage { method, param } => match method.as_str() {
                "constantwindow" => format!("Nodal average · last {param} s"),
                "periods" | "periods_omega" => format!(
                    "Nodal average · last {param} {}{}",
                    if *param == 1.0 {
                        "revolution"
                    } else {
                        "revolutions"
                    },
                    if method == "periods_omega" {
                        " (speed)"
                    } else {
                        ""
                    }
                ),
                _ => format!("Nodal average · {method} · {param}"),
            },
            Self::RadialConcat => "Nodal concatenation".into(),
            Self::Concat { common_only } => format!(
                "Concatenate tables · {}",
                if *common_only {
                    "common columns"
                } else {
                    "all columns"
                }
            ),
        }
    }

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

fn coordinates_summary(values: &[f64]) -> String {
    match values {
        [] => "no coordinates".into(),
        [a] => format!("{a}"),
        [a, b] => format!("{a}, {b}"),
        [a, b, c] => format!("{a}, {b}, {c}"),
        [a, b, c, ..] => format!("{a}, {b}, {c} … ({} total)", values.len()),
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

    #[test]
    fn catalog_defaults_serialize_and_run_on_suitable_inputs() {
        let frame = DataFrame::from_floats(
            ["Time_[s]", "AB1N001Cl_[-]", "AB1N002Cl_[-]"],
            vec![
                vec![0., 0.25, 0.5, 0.75, 1.],
                vec![1., 2., 3., 4., 5.],
                vec![2., 3., 4., 5., 6.],
            ],
        )
        .unwrap();
        let table = Table::from_df(frame, "run");
        let x = [0., 0.25, 0.5, 0.75, 1.];
        let y = [1., 2., 3., 4., 5.];
        let mut ids = std::collections::HashSet::new();
        for info in Plugin::catalog() {
            assert!(ids.insert(info.id));
            let value = serde_json::to_value(&info.default).unwrap();
            let id = match &value {
                serde_json::Value::String(name) => name.as_str(),
                serde_json::Value::Object(fields) => fields.keys().next().unwrap().as_str(),
                _ => panic!("unexpected plugin representation"),
            };
            assert_eq!(id, info.id);
            assert_eq!(
                serde_json::from_value::<Plugin>(value).unwrap(),
                info.default
            );

            match &info.default {
                Plugin::Mask { .. } | Plugin::Units { .. } | Plugin::RenameChannels { .. } => {
                    info.default.apply_table(&mut table.clone()).unwrap();
                }
                Plugin::RadialAverage { .. } | Plugin::RadialConcat => {
                    assert!(!info.default.add_tables(&table, 1).unwrap().is_empty());
                }
                Plugin::Concat { .. } => {
                    let mut tables = vec![table.clone(), table.clone()];
                    let mut action = crate::pipeline::Action::new("concat", info.default.clone());
                    action.apply(&mut tables, false);
                    assert!(action.errors.is_empty(), "{:?}", action.errors);
                    assert_eq!(tables.len(), 3);
                }
                _ => {
                    info.default.apply_xy(&x, &y).unwrap();
                }
            }
        }
        assert_eq!(ids.len(), 10);
    }

    #[test]
    fn sampler_and_filter_menu_defaults_are_actionable() {
        let x = [0., 0.25, 0.5, 0.75, 1.];
        let y = [1., 2., 3., 4., 5.];
        for definition in dv_tools::signal_analysis::FILTERS {
            let plugin = Plugin::Filter {
                name: definition.name.into(),
                param: definition.parameter_info().default,
            };
            assert!(plugin.apply_xy(&x, &y).is_ok(), "{}", definition.name);
        }
        for definition in dv_tools::signal_analysis::SAMPLERS {
            let plugin = Plugin::Sampler {
                name: definition.name.into(),
                param: definition.default_parameters().to_vec(),
            };
            assert!(plugin.apply_xy(&x, &y).is_ok(), "{}", definition.name);
            let encoded = serde_json::to_string(&plugin).unwrap();
            assert_eq!(serde_json::from_str::<Plugin>(&encoded).unwrap(), plugin);
        }
    }

    #[test]
    fn summaries_describe_parameters_and_legacy_bounds() {
        assert_eq!(
            Plugin::Filter {
                name: "Low pass 1st order".into(),
                param: 2.0,
            }
            .summary(),
            "Low-pass 1st order · 2 Hz"
        );
        assert_eq!(
            Plugin::Sampler {
                name: "Delta x".into(),
                param: vec![0.5, f64::NAN, 10.0],
            }
            .summary(),
            "Delta x · 0.5 · auto–10"
        );
        assert_eq!(
            Plugin::Sampler {
                name: "Insert".into(),
                param: vec![1., 2., 3., 4., 5.],
            }
            .summary(),
            "Insert coordinates · 1, 2, 3 … (5 total)"
        );
        assert_eq!(
            Plugin::RenameChannels {
                flavor: "OF2To3".into()
            }
            .summary(),
            "Rename channels · OpenFAST 2 → 3"
        );
        assert_eq!(
            Plugin::Concat { common_only: true }.summary(),
            "Concatenate tables · common columns"
        );
        assert_eq!(
            Plugin::Mask {
                expression: " {Speed}>10 ".into()
            }
            .summary(),
            "Mask · {Speed}>10"
        );
        assert_eq!(
            Plugin::Binning {
                bins: 1,
                min: -1.,
                max: 3.
            }
            .summary(),
            "Binning · 1 bin · -1–3"
        );
        assert_eq!(
            Plugin::Outliers {
                median_deviation: 0.
            }
            .summary(),
            "Remove outliers · disabled"
        );
        assert_eq!(
            Plugin::RadialAverage {
                method: "periods".into(),
                param: 1.
            }
            .summary(),
            "Nodal average · last 1 revolution"
        );
        assert_eq!(Plugin::RadialConcat.summary(), "Nodal concatenation");
    }
}
