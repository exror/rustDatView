use dv_core::{
    common::{get_common_col_indices, no_unit},
    Table, TableList,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub(super) enum SelectionMode {
    #[default]
    Auto,
    Common,
    Independent,
}

/// Prefer a real time axis, then the generated sample index. Ties follow the
/// explicit Time / Elapsed Time / Timestamp order, then source column order.
pub(crate) fn default_x(table: &Table) -> usize {
    let columns = table.data.columns();
    columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.is_float() || column.is_datetime())
        .filter_map(|(index, column)| time_priority(&column.name).map(|rank| (rank, index)))
        .min()
        .map(|(_, index)| index)
        .or_else(|| columns.iter().position(|column| is_index(&column.name)))
        .unwrap_or(0)
}

/// Start with one plottable signal, not another time/index channel or an
/// entirely missing numeric column. Constant signals remain useful telemetry.
pub(crate) fn default_ys(table: &Table, x: usize) -> Vec<usize> {
    table
        .data
        .columns()
        .iter()
        .enumerate()
        .find(|(index, column)| {
            *index != x
                && !is_index(&column.name)
                && time_priority(&column.name).is_none()
                && column
                    .floats()
                    .is_some_and(|values| values.iter().any(|value| value.is_finite()))
        })
        .map(|(index, _)| vec![index])
        .unwrap_or_default()
}

pub(super) fn is_index(name: &str) -> bool {
    no_unit(name).trim().eq_ignore_ascii_case("index")
}

fn time_priority(name: &str) -> Option<u8> {
    let name = no_unit(name);
    let mut words = name
        .split(|character: char| character.is_whitespace() || character == '_')
        .filter(|word| !word.is_empty());
    let first = words.next()?;
    let rank = if first.eq_ignore_ascii_case("time") {
        0
    } else if first.eq_ignore_ascii_case("timestamp") {
        2
    } else if first.eq_ignore_ascii_case("elapsed")
        && words.next().is_some_and(|word| word.eq_ignore_ascii_case("time"))
    {
        1
    } else {
        return None;
    };
    if let Some(unit) = words.next() {
        if !["s", "sec", "secs", "second", "seconds"]
            .iter()
            .any(|candidate| unit.eq_ignore_ascii_case(candidate))
        {
            return None;
        }
    }
    words.next().is_none().then_some(rank)
}

/// Map primary-table choices by cleaned channel name, not position, when
/// selected files share channels in a different order or use different units.
pub(super) fn shared_columns(
    tables: &TableList,
    selected: &[usize],
    mode: SelectionMode,
) -> Option<Vec<Vec<usize>>> {
    if mode == SelectionMode::Independent || selected.is_empty() {
        return None;
    }
    let names = selected
        .iter()
        .map(|&i| tables.tabs[i].columns())
        .collect::<Vec<_>>();
    let common = get_common_col_indices(&names);
    if mode == SelectionMode::Auto
        && !tables.have_same_columns(Some(selected))
        && common.keep.iter().any(|c| c.len() < 2)
    {
        return None;
    }
    Some(common.keep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::{DataFrame, Table};

    fn numeric_table(names: &[&str]) -> Table {
        Table::from_df(
            DataFrame::from_floats(names.iter().copied(), vec![vec![1., 2.]; names.len()])
                .unwrap(),
            "telemetry",
        )
    }

    #[test]
    fn time_axis_wins_over_reordered_metrics() {
        let table = numeric_table(&["CPU Power [W]", "Core Clock [MHz]", "Time [s]"]);
        assert_eq!(default_x(&table), 3);
        assert_eq!(default_ys(&table, 3), vec![1]);
        let reordered = numeric_table(&["Time (seconds)", "CPU Power [W]"]);
        assert_eq!(default_x(&reordered), 1);
        assert_eq!(default_ys(&reordered, 1), vec![2]);
    }

    #[test]
    fn time_alias_ties_have_an_explicit_preference() {
        let table = numeric_table(&[
            "Timestamp",
            "Elapsed Time [s]",
            "Time [ms]",
            "Time [s]",
            "Temperature",
        ]);
        assert_eq!(default_x(&table), 3);
        let elapsed = numeric_table(&["Power", "Elapsed Time seconds", "Timestamp"]);
        assert_eq!(default_x(&elapsed), 2);
    }

    #[test]
    fn datetime_time_is_a_valid_default_axis() {
        let table = Table::from_df(
            DataFrame::from_columns(vec![
                dv_core::Column::float("CPU Power", vec![10., 11.]),
                dv_core::Column::datetime("Timestamp", vec![1., 2.]),
            ])
            .unwrap(),
            "telemetry",
        );
        assert_eq!(default_x(&table), 2);
        assert_eq!(default_ys(&table, 2), vec![1]);
    }

    #[test]
    fn metrics_without_time_use_index_not_first_metric() {
        let table = numeric_table(&["CPU Power [W]", "CPU Temperature"]);
        assert_eq!(default_x(&table), 0);
        assert_eq!(default_ys(&table, 0), vec![1]);
        let empty_signal = Table::from_df(
            DataFrame::from_floats(
                ["Missing", "Temperature"],
                vec![vec![f64::NAN; 2], vec![35., 35.]],
            )
            .unwrap(),
            "telemetry",
        );
        assert_eq!(default_ys(&empty_signal, default_x(&empty_signal)), vec![2]);
        assert!(default_ys(&numeric_table(&["Time [s]"]), 1).is_empty());
    }
    #[test]
    fn shared_selection_matches_reordered_unit_channels() {
        let first = Table::from_df(
            DataFrame::from_floats(
                ["Time [s]", "Speed [m/s]"],
                vec![vec![0., 1.], vec![2., 3.]],
            )
            .unwrap(),
            "a",
        );
        let second = Table::from_df(
            DataFrame::from_floats(
                ["Speed [km/h]", "Time (s)", "Other"],
                vec![vec![7.2, 10.8], vec![0., 1.], vec![1., 1.]],
            )
            .unwrap(),
            "b",
        );
        let tables = TableList::new(vec![first, second]);
        assert_eq!(
            shared_columns(&tables, &[0, 1], SelectionMode::Common),
            Some(vec![vec![0, 1, 2], vec![0, 2, 1]])
        );
        assert_eq!(
            shared_columns(&tables, &[0, 1], SelectionMode::Independent),
            None
        );
    }
    #[test]
    fn auto_does_not_map_unrelated_channels() {
        let a = Table::from_df(DataFrame::from_floats(["A"], vec![vec![1.]]).unwrap(), "a");
        let b = Table::from_df(DataFrame::from_floats(["B"], vec![vec![2.]]).unwrap(), "b");
        assert!(
            shared_columns(&TableList::new(vec![a, b]), &[0, 1], SelectionMode::Auto).is_none()
        );
    }
}
