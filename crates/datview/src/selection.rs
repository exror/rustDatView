use dv_core::{
    common::no_unit,
    units::{parse_unit, split_name_unit, UnitFamily},
    Column, ColumnData, Table,
};

/// Choose the X axis: a column named like time (in several languages and
/// logger conventions) whose values never decrease, then any datetime column,
/// then a non-decreasing column measured in a time unit, then the index.
pub(crate) fn default_x(table: &Table) -> usize {
    let columns = table.data.columns();
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_datetime() || (c.is_float() && non_decreasing(c)))
        .filter_map(|(i, c)| time_priority(&c.name).map(|rank| (rank, i)))
        .min()
        .map(|(_, i)| i)
        .or_else(|| columns.iter().position(Column::is_datetime))
        .or_else(|| {
            columns.iter().position(|c| {
                c.is_float() && !is_index(&c.name) && has_time_unit(&c.name) && non_decreasing(c)
            })
        })
        .or_else(|| columns.iter().position(|c| is_index(&c.name)))
        .or_else(|| columns.iter().position(Column::is_float))
        .unwrap_or(0)
}

pub(crate) fn is_index(name: &str) -> bool {
    no_unit(name).trim().eq_ignore_ascii_case("index")
}

/// Lowercase letters and digits only: "Time_Of_Day_Seconds" → "timeofdayseconds".
fn compact(name: &str) -> String {
    no_unit(name).chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

fn has_time_unit(name: &str) -> bool {
    let (_, unit) = split_name_unit(name);
    parse_unit(&unit).family == UnitFamily::Time
}

/// Lower is better. Names alone are not enough when a unit contradicts them.
fn time_priority(name: &str) -> Option<u8> {
    let (_, unit) = split_name_unit(name);
    let unit_ok = unit.trim().is_empty() || parse_unit(&unit).family == UnitFamily::Time;
    if !unit_ok {
        return None;
    }
    let rank = match compact(name).as_str() {
        "time" | "zeit" | "temps" | "tiempo" | "tempo" | "tid" | "czas" | "aika" => 0,
        "elapsedtime" | "elapsed" | "timeelapsed" | "relativetime" | "runtime" => 1,
        "timestamp" | "timestamps" => 2,
        "datetime" | "dateandtime" => 3,
        "timeinseconds" | "timeofdayseconds" | "timeofday" | "seconds" | "sec" | "secs" | "t" => 4,
        "x" | "xaxis" if !unit.trim().is_empty() => 5,
        _ => return None,
    };
    Some(rank)
}

/// Finite values never decrease (repeated values are fine: interleaved rows).
fn non_decreasing(column: &Column) -> bool {
    let ColumnData::Float(values) = &column.data else { return column.is_datetime() };
    let mut previous = f64::NEG_INFINITY;
    let mut finite = 0usize;
    for &value in values.iter().filter(|v| v.is_finite()) {
        if value < previous {
            return false;
        }
        previous = value;
        finite += 1;
    }
    finite >= 2
}

/// Identifier columns (core number, device index, process id) hold few
/// distinct whole numbers and no unit; they are rarely what someone wants to
/// plot first.
fn identifier_like(column: &Column) -> bool {
    if !split_name_unit(&column.name).1.trim().is_empty() {
        return false;
    }
    let Some(values) = column.floats() else { return false };
    let mut distinct = std::collections::BTreeSet::new();
    for &value in values.iter().filter(|v| v.is_finite()) {
        if value.fract() != 0.0 {
            return false;
        }
        distinct.insert(value as i64);
        if distinct.len() > 16 {
            return false;
        }
    }
    values.len() > distinct.len() * 4
}

fn priority(family: UnitFamily, name: &str) -> usize {
    let name = name.to_lowercase().replace(['_', '-'], " ");
    let has = |s: &str| name.contains(s);
    match family {
        UnitFamily::Power => {
            if has("package power") {
                0
            } else if has("ppt") {
                1
            } else if has("cpu power") {
                2
            } else if has("total power") {
                3
            } else {
                4
            }
        }
        UnitFamily::Frequency => {
            let avg = has("avg") || has("average");
            if has("effective clock") && avg {
                0
            } else if has("core clock") && avg {
                1
            } else if has("cpu clock") {
                2
            } else if has("effective clock") {
                3
            } else if has("core clock") {
                4
            } else {
                5
            }
        }
        UnitFamily::Temperature => {
            if has("cpu package") {
                0
            } else if has("tctl") {
                1
            } else if has("tdie") {
                2
            } else if has("core max") {
                3
            } else {
                4
            }
        }
        _ => usize::MAX,
    }
}

/// Small tables show every usable numeric signal. Large telemetry tables pick
/// one representative from power, frequency and temperature. Identifier
/// columns (core numbers, device indices) are skipped when anything else is
/// available.
pub(crate) fn default_ys(table: &Table, x: usize) -> Vec<usize> {
    let columns = table.data.columns();
    let mut usable: Vec<_> = columns
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            *i != x
                && !is_index(&c.name)
                && c.floats().is_some_and(|v| v.iter().any(|n| n.is_finite()))
        })
        .collect();
    if usable.iter().any(|(_, c)| !identifier_like(c)) {
        usable.retain(|(_, c)| !identifier_like(c));
    }
    let numeric_count = columns
        .iter()
        .filter(|c| (c.is_float() || c.is_datetime()) && !is_index(&c.name))
        .count();
    if numeric_count <= 8 {
        return usable.into_iter().take(8).map(|(i, _)| i).collect();
    }
    let telemetry = [
        UnitFamily::Power,
        UnitFamily::Frequency,
        UnitFamily::Temperature,
        UnitFamily::Percent,
    ]
    .into_iter()
    .filter(|family| {
        usable.iter().any(|(_, c)| {
            let (_, unit) = split_name_unit(&c.name);
            parse_unit(&unit).family == *family
        })
    })
    .count()
        >= 2;
    let mut picks = Vec::new();
    for family in [
        UnitFamily::Power,
        UnitFamily::Frequency,
        UnitFamily::Temperature,
    ] {
        if let Some((_, index)) = usable
            .iter()
            .filter_map(|(i, c)| {
                let (_, unit) = split_name_unit(&c.name);
                (parse_unit(&unit).family == family).then_some((priority(family, &c.name), *i))
            })
            .min()
        {
            picks.push(index);
        }
    }
    if telemetry && !picks.is_empty() {
        picks
    } else {
        usable
            .into_iter()
            .find(|(_, c)| time_priority(&c.name).is_none())
            .map(|(i, _)| vec![i])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::{Column, DataFrame};
    fn table(names: &[&str]) -> Table {
        Table::from_df(
            DataFrame::from_floats(names.iter().copied(), vec![vec![1., 2.]; names.len()]).unwrap(),
            "test",
        )
    }
    #[test]
    fn chooses_named_time_then_datetime_then_index() {
        let t = table(&["Timestamp", "Elapsed Time [s]", "Time [ms]", "Power [W]"]);
        assert_eq!(default_x(&t), 3);
        let t = Table::from_df(
            DataFrame::from_columns(vec![
                Column::float("Power", vec![1.]),
                Column::datetime("Recorded", vec![2.]),
            ])
            .unwrap(),
            "test",
        );
        assert_eq!(default_x(&t), 2);
        assert_eq!(default_x(&table(&["Power"])), 0);
    }
    #[test]
    fn small_tables_select_all_finite_non_index_signals() {
        let t = Table::from_df(
            DataFrame::from_floats(
                ["Time [s]", "Power", "Missing", "Temperature"],
                vec![
                    vec![0., 1.],
                    vec![3., 4.],
                    vec![f64::NAN; 2],
                    vec![35., 35.],
                ],
            )
            .unwrap(),
            "test",
        );
        assert_eq!(default_ys(&t, default_x(&t)), vec![2, 4]);
    }
    #[test]
    fn telemetry_prefers_representative_signals() {
        let t = table(&[
            "Time [s]",
            "GPU Power [W]",
            "CPU Package Power [W]",
            "Core Clock [MHz]",
            "Core Effective Clock Average [MHz]",
            "GPU [°C]",
            "CPU Tctl [°C]",
            "Core 1 Load [%]",
            "CPU Total [%]",
            "Other",
        ]);
        assert_eq!(default_ys(&t, default_x(&t)), vec![3, 5, 7]);
    }
    fn frame(columns: &[(&str, &[f64])]) -> Table {
        let df = DataFrame::from_floats(columns.iter().map(|(name, _)| *name), columns.iter().map(|(_, v)| v.to_vec()).collect()).unwrap();
        Table::from_df(df, "test")
    }
    #[test]
    fn recognizes_logger_and_localized_time_axes_but_not_unordered_ones() {
        // PresentMon: the frame interval has a time unit but is not an axis.
        let t = frame(&[("Dropped", &[0., 0., 1.]), ("TimeInSeconds", &[0.01, 0.02, 0.03]), ("msBetweenPresents [ms]", &[8., 9., 7.])]);
        assert_eq!(default_x(&t), 2);
        // Power analyzer (German), scope exports and Joulescope.
        for name in ["Zeit [s]", "X [s]", "x-axis [s]", "#time", "Time_Of_Day_Seconds"] {
            let t = frame(&[("Value [V]", &[3., 1., 2.]), (name, &[0., 1., 2.])]);
            assert_eq!(default_x(&t), 2, "{name}");
        }
        // A time-named column that goes backwards, or carries a non-time unit, is data.
        let t = frame(&[("Time [s]", &[2., 1., 0.]), ("Time [°C]", &[0., 1., 2.]), ("Elapsed [ms]", &[0., 5., 10.])]);
        assert_eq!(default_x(&t), 3);
    }
    #[test]
    fn identifier_columns_are_not_default_signals() {
        // Five samples of four cores, interleaved as turbostat and PTAT write them.
        let core: Vec<f64> = (0..20).map(|i| f64::from(i % 4)).collect();
        let time: Vec<f64> = (0..20).map(|i| f64::from(i / 4)).collect();
        let clock: Vec<f64> = (0..20).map(|i| 1000. + 37. * f64::from(i)).collect();
        let t = frame(&[("Time [s]", &time), ("Core", &core), ("Avg_MHz", &clock)]);
        assert_eq!(default_ys(&t, default_x(&t)), vec![3]);
    }
}
