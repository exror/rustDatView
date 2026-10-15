use dv_core::{common::get_common_col_indices, TableList};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub(super) enum SelectionMode {
    #[default]
    Auto,
    Common,
    Independent,
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
