use dv_core::{
    common::no_unit,
    units::{parse_unit, split_name_unit, UnitFamily},
    Table,
};

/// Prefer the familiar time names in their historical order, then any other
/// datetime column, then the generated index.
pub(crate) fn default_x(table: &Table) -> usize {
    let columns = table.data.columns();
    columns
        .iter()
        .enumerate()
        .filter(|(_, c)| c.is_float() || c.is_datetime())
        .filter_map(|(i, c)| time_priority(&c.name).map(|rank| (rank, i)))
        .min()
        .map(|(_, i)| i)
        .or_else(|| columns.iter().position(|c| c.is_datetime()))
        .or_else(|| columns.iter().position(|c| is_index(&c.name)))
        .or_else(|| columns.iter().position(|c| c.is_float()))
        .unwrap_or(0)
}

pub(crate) fn is_index(name: &str) -> bool {
    no_unit(name).trim().eq_ignore_ascii_case("index")
}

fn time_priority(name: &str) -> Option<u8> {
    let name = no_unit(name);
    let mut words = name
        .split(|c: char| c.is_whitespace() || c == '_')
        .filter(|s| !s.is_empty());
    let first = words.next()?;
    let rank = if first.eq_ignore_ascii_case("time") {
        0
    } else if first.eq_ignore_ascii_case("timestamp") {
        2
    } else if first.eq_ignore_ascii_case("datetime") {
        3
    } else if first.eq_ignore_ascii_case("date")
        && words.next().is_some_and(|w| w.eq_ignore_ascii_case("time"))
    {
        3
    } else if first.eq_ignore_ascii_case("elapsed")
        && words.next().is_some_and(|w| w.eq_ignore_ascii_case("time"))
    {
        1
    } else {
        return None;
    };
    if let Some(unit) = words.next() {
        if !["s", "sec", "secs", "second", "seconds"]
            .iter()
            .any(|u| unit.eq_ignore_ascii_case(u))
        {
            return None;
        }
    }
    words.next().is_none().then_some(rank)
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
/// one representative from power, frequency and temperature.
pub(crate) fn default_ys(table: &Table, x: usize) -> Vec<usize> {
    let columns = table.data.columns();
    let usable: Vec<_> = columns
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            *i != x
                && !is_index(&c.name)
                && c.floats().is_some_and(|v| v.iter().any(|n| n.is_finite()))
        })
        .collect();
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
}
