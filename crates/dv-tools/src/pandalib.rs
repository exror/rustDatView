//! Dataframe interpolation and SI/wind-energy unit conversion from pyDatView.
//!
//! Conversion tables intentionally match upstream, including its conversion
//! of angular positions from radians to degrees in both unit flavors.
use dv_core::{
    dataframe::{Column, ColumnData, DataFrame},
    error::{DvError, Result},
};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnitFlavor {
    #[default]
    SI,
    WE,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitLabel {
    pub variable: String,
    pub unit: String,
    pub separator: String,
    pub brackets: String,
}
pub fn split_unit(label: &str) -> UnitLabel {
    let bracket = label
        .char_indices()
        .rev()
        .find(|(_, c)| *c == '[' || *c == '(');
    if let Some((index, open)) = bracket {
        let close = if open == '[' { ']' } else { ')' };
        if label.ends_with(close) {
            let prefix = &label[..index];
            let separator = if prefix.ends_with('_') {
                "_"
            } else if prefix.ends_with(' ') {
                " "
            } else {
                ""
            };
            return UnitLabel {
                variable: prefix.strip_suffix(separator).unwrap_or(prefix).to_owned(),
                unit: label[index + 1..label.len() - 1].to_owned(),
                separator: separator.to_owned(),
                brackets: if open == '[' { "[]" } else { "()" }.to_owned(),
            };
        }
    }
    UnitLabel {
        variable: label.to_owned(),
        unit: String::new(),
        separator: String::new(),
        brackets: String::new(),
    }
}
pub fn unit_conversion(unit: &str, flavor: UnitFlavor) -> Option<(f64, &'static str)> {
    let unit = unit.to_lowercase();
    match flavor {
        UnitFlavor::WE => match unit.as_str() {
            "rad/s" => Some((30.0 / PI, "rpm")),
            "rad" => Some((180.0 / PI, "deg")),
            "n" => Some((1e-3, "kN")),
            "mn" => Some((1e3, "kN")),
            "nm" | "n-m" | "n*m" => Some((1e-3, "kNm")),
            "mnm" | "mn-m" | "mn*m" => Some((1e3, "kNm")),
            "w" => Some((1e-3, "kW")),
            "mw" => Some((1e3, "kW")),
            _ => None,
        },
        UnitFlavor::SI => match unit.as_str() {
            "rpm" => Some((PI / 30.0, "rad/s")),
            "rad" => Some((180.0 / PI, "deg")),
            "deg/s" => Some((PI / 180.0, "rad/s")),
            "mn" => Some((1e6, "N")),
            "kn" => Some((1e3, "N")),
            "mnm" | "mn-m" | "mn*m" => Some((1e6, "Nm")),
            "knm" | "kn-m" | "kn*m" => Some((1e3, "Nm")),
            "kw" => Some((1e3, "W")),
            "mw" => Some((1e6, "W")),
            _ => None,
        },
    }
}
pub fn converted_label(label: &str, flavor: UnitFlavor) -> (String, f64) {
    let parts = split_unit(label);
    if let Some((scale, unit)) = unit_conversion(&parts.unit, flavor) {
        let mut chars = parts.brackets.chars();
        let open = chars.next().unwrap();
        let close = chars.next().unwrap();
        (
            format!(
                "{}{}{}{}{}",
                parts.variable, parts.separator, open, unit, close
            ),
            scale,
        )
    } else {
        (label.to_owned(), 1.0)
    }
}
/// Convert numeric columns in place; duplicate names remain positional.
/// Unknown units are unchanged, as in upstream. Recognized units on a
/// nonnumeric column are an error and leave the whole frame unchanged.
pub fn change_units(df: &mut DataFrame, flavor: UnitFlavor) -> Result<()> {
    for column in df.columns() {
        let parts = split_unit(&column.name);
        if unit_conversion(&parts.unit, flavor).is_some()
            && !matches!(&column.data, ColumnData::Float(_))
        {
            return Err(DvError::msg(format!(
                "unit conversion requires numeric column `{}`",
                column.name
            )));
        }
    }
    for column in df.columns_mut() {
        let (label, scale) = converted_label(&column.name, flavor);
        if let ColumnData::Float(values) = &mut column.data {
            if scale != 1.0 {
                for v in values {
                    *v *= scale;
                }
            }
        }
        column.name = label;
    }
    Ok(())
}
pub fn pd_interp1(x_new: &[f64], x_column: usize, df: &DataFrame) -> Result<DataFrame> {
    let mut result = crate::signal_analysis::interp_df(
        x_new,
        x_column,
        df,
        crate::signal_analysis::Extrapolation::Bounded,
    )?;
    let xp = df.f64_col(x_column);
    let values = crate::signal_analysis::interpolate(
        x_new,
        &xp,
        &xp,
        crate::signal_analysis::Extrapolation::Bounded,
    )?;
    result.column_mut(x_column).data = if df.column(x_column).is_datetime() {
        ColumnData::DateTime(values)
    } else {
        ColumnData::Float(values)
    };
    Ok(result)
}

/// Ordered alternatives for a rename or formula-derived column. Formula
/// expressions use the existing dv-core `{Column}` expression evaluator.
#[derive(Debug, Clone)]
pub struct ColumnMapping {
    pub name: String,
    pub alternatives: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct RemapResult {
    pub dataframe: DataFrame,
    pub missing: Vec<String>,
}
pub fn remap_df(
    df: &DataFrame,
    mappings: &[ColumnMapping],
    keep_new_only: bool,
) -> Result<RemapResult> {
    let mut result = df.clone();
    let mut renames = Vec::new();
    let mut produced = Vec::new();
    let mut missing = Vec::new();
    for mapping in mappings {
        let name = mapping.name.trim();
        for source in &mapping.alternatives {
            let source = source.trim();
            if source.contains('{') {
                // Missing alternatives are reported rather than silently discarded.
                let mut valid = true;
                let mut remainder = source;
                while let Some(start) = remainder.find('{') {
                    let after = &remainder[start + 1..];
                    let end = after.find('}').ok_or_else(|| {
                        DvError::msg("unterminated column reference in remapping")
                    })?;
                    let reference = &after[..end];
                    if result.find(reference).is_none() {
                        missing.push(reference.to_owned());
                        valid = false;
                    }
                    remainder = &after[end + 1..];
                }
                if !valid {
                    continue;
                }
                let values = dv_core::formula::eval_formula(&result, source)?;
                let column = Column::float(name, values);
                if let Some(i) = result.find(name) {
                    result.set(i, column)?;
                } else {
                    result.push(column)?;
                }
                produced.push(name.to_owned());
                break;
            } else if let Some(i) = result.find(source) {
                renames.push((i, name.to_owned()));
                produced.push(name.to_owned());
                break;
            } else {
                missing.push(source.to_owned());
            }
        }
    }
    for (i, name) in renames {
        result.column_mut(i).name = name;
    }
    if keep_new_only {
        let indices: Vec<_> = mappings
            .iter()
            .filter(|m| produced.iter().any(|p| p == m.name.trim()))
            .filter_map(|m| result.find(m.name.trim()))
            .collect();
        result = result.select_columns(&indices);
    }
    Ok(RemapResult {
        dataframe: result,
        missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wind_energy_conversion() {
        let mut df =
            DataFrame::from_floats(["Speed_[rad/s]", "Force (N)"], vec![vec![PI], vec![2000.]])
                .unwrap();
        change_units(&mut df, UnitFlavor::WE).unwrap();
        assert_eq!(df.names(), vec!["Speed_[rpm]", "Force (kN)"]);
        assert_eq!(&*df.f64_col(0), &[30.]);
        assert_eq!(&*df.f64_col(1), &[2.]);
    }
    #[test]
    fn si_angular_convention() {
        let (label, scale) = converted_label("Pitch_[rad]", UnitFlavor::SI);
        assert_eq!(label, "Pitch_[deg]");
        assert_eq!(scale, 180. / PI);
    }
    #[test]
    fn duplicate_names_convert_by_position() {
        let mut df =
            DataFrame::from_floats(["F_[N]", "F_[N]"], vec![vec![1000.], vec![2000.]]).unwrap();
        change_units(&mut df, UnitFlavor::WE).unwrap();
        assert_eq!(&*df.f64_col(1), &[2.]);
    }
}
