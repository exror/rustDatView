//! Ordered plot and table actions, with reversible state and JSON persistence.
//! Forced reapplication first unwinds reversible actions in reverse order.
use crate::Plugin;
use dv_core::{DataFrame, DvError, Result, Table};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    PlotData,
    ReversibleTable,
    IrreversibleTable,
    Adder,
}
#[derive(Debug, Clone)]
struct TableState {
    names: Vec<String>,
    after_names: Vec<String>,
    mask: Option<Vec<bool>>,
    mask_string: String,
    mask_action: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct Addition {
    pub inputs: Vec<u64>,
    pub outputs: Vec<u64>,
    pub plugin: Plugin,
    pub x_column: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub plugin: Plugin,
    pub kind: ActionKind,
    pub enabled: bool,
    pub unique: bool,
    /// Column used by full-table filter/sampler/binning additions.
    pub x_column: usize,
    #[serde(skip)]
    pub errors: Vec<String>,
    #[serde(skip)]
    applied: HashSet<u64>,
    #[serde(skip)]
    previous: HashMap<u64, TableState>,
    #[serde(skip)]
    generated: HashSet<u64>,
    #[serde(skip)]
    pub(crate) additions: Vec<Addition>,
}
fn table_key(t: &Table) -> u64 {
    t.identity()
}
impl Action {
    pub fn new(name: impl Into<String>, plugin: Plugin) -> Self {
        let kind = if plugin.is_plot() {
            ActionKind::PlotData
        } else if plugin.is_reversible() {
            ActionKind::ReversibleTable
        } else if plugin.is_adder() {
            ActionKind::Adder
        } else {
            ActionKind::IrreversibleTable
        };
        Self {
            name: name.into(),
            plugin,
            kind,
            enabled: true,
            unique: true,
            x_column: 1,
            errors: Vec::new(),
            applied: HashSet::new(),
            previous: HashMap::new(),
            generated: HashSet::new(),
            additions: Vec::new(),
        }
    }
    pub fn needs_reload_on_remove(&self) -> bool {
        self.kind == ActionKind::IrreversibleTable && !self.applied.is_empty()
    }
    pub fn applied_to(&self, table: &Table) -> bool {
        self.applied.contains(&table_key(table))
    }
    pub fn generated_table(&self, table: &Table) -> bool {
        self.generated.contains(&table_key(table))
    }
    fn apply_existing(&mut self, tables: &mut [Table], force: bool) {
        for t in tables {
            let key = table_key(t);
            if self.generated.contains(&key) || (!force && self.applied.contains(&key)) {
                continue;
            }
            if self.kind == ActionKind::ReversibleTable && force && self.previous.contains_key(&key)
            {
                let errors = std::mem::take(&mut self.errors);
                let result = self.cancel(std::slice::from_mut(t));
                self.errors = errors;
                if let Err(e) = result {
                    self.errors.push(e.to_string());
                    continue;
                }
            }
            if self.kind == ActionKind::ReversibleTable && !self.previous.contains_key(&key) {
                let mask_action = matches!(self.plugin, Plugin::Mask { .. });
                self.previous.insert(
                    key,
                    TableState {
                        names: if mask_action { Vec::new() } else { t.columns() },
                        after_names: Vec::new(),
                        mask: if mask_action { t.mask.clone() } else { None },
                        mask_string: if mask_action {
                            t.mask_string.clone()
                        } else {
                            String::new()
                        },
                        mask_action,
                    },
                );
            }
            match self.plugin.apply_table(t) {
                Ok(()) => {
                    if let Some(state) = self.previous.get_mut(&key) {
                        if !state.mask_action {
                            state.after_names = t.columns();
                        }
                    }
                    self.applied.insert(key);
                }
                Err(e) => self.errors.push(format!(
                    "Failed to apply action {:?} to table {:?}: {e}",
                    self.name,
                    t.nickname()
                )),
            }
        }
    }
    pub fn apply(&mut self, tables: &mut Vec<Table>, force: bool) {
        self.errors.clear();
        if !self.enabled || self.kind == ActionKind::PlotData {
            return;
        }
        if self.kind != ActionKind::Adder {
            self.apply_existing(tables, force);
            return;
        }
        if let Plugin::Concat { common_only } = self.plugin {
            if tables.is_empty() {
                self.errors.push("No tables to concatenate".into());
                return;
            }
            let inputs: Vec<&Table> = tables.iter().filter(|t| !self.generated_table(t)).collect();
            let keys: Vec<_> = inputs.iter().map(|t| table_key(t)).collect();
            if !force && self.additions.iter().any(|a| a.inputs == keys) {
                return;
            }
            let frames: Vec<DataFrame> = inputs.iter().map(|t| t.data_without_index()).collect();
            let refs: Vec<&DataFrame> = frames.iter().collect();
            let combined = DataFrame::concat_vertical(&refs, common_only);
            if combined.ncols() == 0 {
                self.errors.push("No common columns to concatenate".into());
                return;
            }
            let added = Table::from_df(combined, "concat");
            self.additions.push(Addition {
                inputs: keys,
                outputs: vec![table_key(&added)],
                plugin: self.plugin.clone(),
                x_column: self.x_column,
            });
            self.generated.insert(table_key(&added));
            tables.push(added);
            return;
        }
        let mut added = Vec::new();
        for t in tables.iter_mut() {
            let key = table_key(t);
            if self.generated.contains(&key) || (!force && self.applied.contains(&key)) {
                continue;
            }
            let result = self.plugin.add_tables(t, self.x_column).map(|new| {
                self.additions.push(Addition {
                    inputs: vec![key],
                    outputs: new.iter().map(table_key).collect(),
                    plugin: self.plugin.clone(),
                    x_column: self.x_column,
                });
                added.extend(new);
            });
            match result {
                Ok(()) => {
                    self.applied.insert(key);
                }
                Err(e) => self.errors.push(format!(
                    "Failed to apply action {:?} to table {:?}: {e}",
                    self.name,
                    t.nickname()
                )),
            }
        }
        self.generated.extend(added.iter().map(table_key));
        tables.extend(added);
    }
    fn ensure_cancel(&self, tables: &[Table]) -> Result<()> {
        if self.needs_reload_on_remove() {
            return Err(DvError::msg(format!(
                "Removing {} requires reloading the original tables",
                self.name
            )));
        }
        for t in tables {
            if let Some(state) = self.previous.get(&table_key(t)) {
                if !state.mask_action && state.names.len() != t.ncols() {
                    return Err(DvError::msg(format!(
                        "Cannot restore columns for {}: table structure changed",
                        t.nickname()
                    )));
                }
            }
        }
        Ok(())
    }
    pub fn cancel(&mut self, tables: &mut [Table]) -> Result<()> {
        self.errors.clear();
        self.ensure_cancel(tables)?;
        if self.kind == ActionKind::ReversibleTable {
            for t in tables {
                let key = table_key(t);
                if let Some(state) = self.previous.get(&key) {
                    if state.mask_action {
                        t.mask = state.mask.clone();
                        t.mask_string = state.mask_string.clone();
                    } else {
                        for ((c, before), after) in t
                            .data
                            .columns_mut()
                            .iter_mut()
                            .zip(&state.names)
                            .zip(&state.after_names)
                        {
                            if c.name == *after {
                                c.name = before.clone();
                            } else if dv_core::common::no_unit(&c.name)
                                == dv_core::common::no_unit(after)
                            {
                                // A later unit conversion changes labels and values.
                                // Undo just the channel rename, retaining that unit.
                                let current = dv_tools::pandalib::split_unit(&c.name);
                                let original = dv_tools::pandalib::split_unit(before);
                                let mut brackets = current.brackets.chars();
                                c.name = if let (Some(open), Some(close)) =
                                    (brackets.next(), brackets.next())
                                {
                                    format!(
                                        "{}{}{open}{}{close}",
                                        original.variable, current.separator, current.unit
                                    )
                                } else {
                                    original.variable
                                };
                            }
                        }
                    }
                    self.previous.remove(&key);
                    self.applied.remove(&key);
                }
            }
        }
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(DvError::msg(self.errors.join("\n")))
        }
    }
    pub fn add_tables(&mut self, tables: &[Table]) -> Vec<Table> {
        self.errors.clear();
        let mut added = Vec::new();
        for t in tables {
            match self.plugin.add_tables(t, self.x_column) {
                Ok(new) => {
                    self.additions.push(Addition {
                        inputs: vec![table_key(t)],
                        outputs: new.iter().map(table_key).collect(),
                        plugin: self.plugin.clone(),
                        x_column: self.x_column,
                    });
                    added.extend(new);
                }
                Err(e) => self.errors.push(format!(
                    "Failed to add tables with {} for {}: {e}",
                    self.name,
                    t.nickname()
                )),
            }
        }
        self.generated.extend(added.iter().map(table_key));
        added
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Pipeline {
    pub actions: Vec<Action>,
    #[serde(skip)]
    pub errors: Vec<String>,
}
impl Pipeline {
    pub fn append(&mut self, action: Action, overwrite: bool) {
        if action.unique {
            if let Some(i) = self.actions.iter().position(|a| a.name == action.name) {
                if overwrite {
                    let mut action = action;
                    // Keep undo bookkeeping when updating an action's parameters.
                    action.applied = std::mem::take(&mut self.actions[i].applied);
                    action.previous = std::mem::take(&mut self.actions[i].previous);
                    action.generated = std::mem::take(&mut self.actions[i].generated);
                    action.additions = std::mem::take(&mut self.actions[i].additions);
                    self.actions[i] = action;
                }
                return;
            }
        }
        self.actions.push(action);
    }
    pub fn find(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.name == name)
    }
    /// Cancel one action without invalidating later reversible snapshots.
    /// The caller may already have disabled the target action.
    pub fn cancel(&mut self, index: usize, tables: &mut [Table]) -> Result<()> {
        if index >= self.actions.len() {
            return Err(DvError::msg("Action index out of bounds"));
        }
        if self.actions[index].kind == ActionKind::ReversibleTable {
            if self.actions[index + 1..]
                .iter()
                .any(Action::needs_reload_on_remove)
            {
                return Err(DvError::msg(
                    "Removing this action requires reloading the original tables",
                ));
            }
            for action in &self.actions[index..] {
                if action.kind == ActionKind::ReversibleTable {
                    action.ensure_cancel(tables)?;
                }
            }
            let replay_failure = self.actions[index + 1..]
                .iter()
                .any(|a| a.enabled && a.kind == ActionKind::ReversibleTable);
            let rollback = replay_failure.then(|| {
                (
                    tables
                        .iter()
                        .map(|t| (t.columns(), t.mask.clone(), t.mask_string.clone()))
                        .collect::<Vec<_>>(),
                    self.actions[index..]
                        .iter()
                        .map(|a| (a.applied.clone(), a.previous.clone(), a.errors.clone()))
                        .collect::<Vec<_>>(),
                )
            });
            for action in self.actions[index + 1..].iter_mut().rev() {
                if action.kind == ActionKind::ReversibleTable {
                    action.cancel(tables)?;
                }
            }
            self.actions[index].cancel(tables)?;
            for action in &mut self.actions[index + 1..] {
                if action.enabled && action.kind == ActionKind::ReversibleTable {
                    action.apply_existing(tables, false);
                }
            }
            let errors: Vec<_> = self.actions[index + 1..]
                .iter()
                .filter(|a| a.enabled && a.kind == ActionKind::ReversibleTable)
                .flat_map(|a| a.errors.iter().cloned())
                .collect();
            if !errors.is_empty() {
                if let Some((states, bookkeeping)) = rollback {
                    for (t, (names, mask, mask_string)) in tables.iter_mut().zip(states) {
                        for (c, name) in t.data.columns_mut().iter_mut().zip(names) {
                            c.name = name;
                        }
                        t.mask = mask;
                        t.mask_string = mask_string;
                    }
                    for (action, (applied, previous, old_errors)) in
                        self.actions[index..].iter_mut().zip(bookkeeping)
                    {
                        action.applied = applied;
                        action.previous = previous;
                        action.errors = old_errors;
                    }
                }
                self.errors = errors;
                return Err(DvError::msg(self.errors.join("\n")));
            }
        } else {
            self.actions[index].cancel(tables)?;
        }
        self.collect_errors();
        Ok(())
    }
    pub fn remove(&mut self, index: usize, tables: &mut [Table], cancel: bool) -> Result<Action> {
        if index >= self.actions.len() {
            return Err(DvError::msg("Action index out of bounds"));
        }
        if cancel {
            self.cancel(index, tables)?;
        }
        let action = self.actions.remove(index);
        self.collect_errors();
        Ok(action)
    }
    pub fn has_plot_actions(&self) -> bool {
        self.actions
            .iter()
            .any(|a| a.enabled && a.kind == ActionKind::PlotData)
    }
    pub fn apply(&mut self, tables: &mut Vec<Table>, force: bool) {
        // Upstream executes all data actions before on-the-fly plot actions.
        if force {
            for action in &self.actions {
                if action.kind == ActionKind::ReversibleTable {
                    if let Err(e) = action.ensure_cancel(tables) {
                        self.errors = vec![e.to_string()];
                        return;
                    }
                }
            }
            for action in self.actions.iter_mut().rev() {
                if action.kind == ActionKind::ReversibleTable {
                    if let Err(e) = action.cancel(tables) {
                        self.errors = vec![e.to_string()];
                        return;
                    }
                }
            }
        }
        for action in &mut self.actions {
            if action.kind != ActionKind::PlotData {
                action.apply(tables, force);
            }
        }
        self.collect_errors();
    }
    pub fn apply_plot(&self, x: &[f64], y: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
        let mut xx = x.to_vec();
        let mut yy = y.to_vec();
        for action in &self.actions {
            if action.enabled && action.kind == ActionKind::PlotData {
                (xx, yy) = action
                    .plugin
                    .apply_xy(&xx, &yy)
                    .map_err(|e| DvError::msg(format!("Plot action {}: {e}", action.name)))?;
            }
        }
        Ok((xx, yy))
    }
    pub fn collect_errors(&mut self) {
        self.errors = self
            .actions
            .iter()
            .flat_map(|a| a.errors.iter().cloned())
            .collect();
    }
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| DvError::msg(e.to_string()))
    }
    pub fn from_json(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| DvError::msg(format!("Invalid pipeline: {e}")))
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.to_json()?).map_err(|e| DvError::msg(e.to_string()))
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let json = std::fs::read_to_string(path).map_err(|e| DvError::msg(e.to_string()))?;
        Self::from_json(&json)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_preserves_order() {
        let mut p = Pipeline::default();
        p.append(Action::new("sample", Plugin::sampler()), false);
        p.append(Action::new("filter", Plugin::filter()), false);
        let q = Pipeline::from_json(&p.to_json().unwrap()).unwrap();
        assert_eq!(q.actions.len(), 2);
        assert_eq!(q.actions[0].name, "sample");
    }
    #[test]
    fn mask_is_reversible() {
        let mut t = vec![Table::from_df(
            DataFrame::from_floats(["Time [s]"], vec![vec![0., 1., 2.]]).unwrap(),
            "test",
        )];
        let mut p = Pipeline::default();
        p.append(
            Action::new(
                "mask",
                Plugin::Mask {
                    expression: "{Time}>0".into(),
                },
            ),
            false,
        );
        p.apply(&mut t, false);
        assert!(p.errors.is_empty());
        assert_eq!(t[0].get_column(1).len(), 2);
        p.remove(0, &mut t, true).unwrap();
        assert_eq!(t[0].get_column(1).len(), 3);
    }
    #[test]
    fn replacing_mask_preserves_original_undo() {
        let mut t = vec![Table::from_df(
            DataFrame::from_floats(["Time [s]"], vec![vec![0., 1., 2.]]).unwrap(),
            "test",
        )];
        let mut p = Pipeline::default();
        p.append(
            Action::new(
                "mask",
                Plugin::Mask {
                    expression: "{Time}>0".into(),
                },
            ),
            false,
        );
        p.apply(&mut t, false);
        p.append(
            Action::new(
                "mask",
                Plugin::Mask {
                    expression: "{Time}>1".into(),
                },
            ),
            true,
        );
        p.apply(&mut t, true);
        assert_eq!(t[0].get_column(1).len(), 1);
        p.remove(0, &mut t, true).unwrap();
        assert_eq!(t[0].get_column(1).len(), 3);
    }
    #[test]
    fn removing_earlier_mask_replays_later_mask() {
        let mut t = vec![Table::from_df(
            DataFrame::from_floats(["Time [s]"], vec![vec![0., 1., 2.]]).unwrap(),
            "test",
        )];
        let mut p = Pipeline::default();
        p.append(
            Action::new(
                "first",
                Plugin::Mask {
                    expression: "{Time}>0".into(),
                },
            ),
            false,
        );
        p.append(
            Action::new(
                "second",
                Plugin::Mask {
                    expression: "{Time}>1".into(),
                },
            ),
            false,
        );
        p.apply(&mut t, false);
        p.remove(0, &mut t, true).unwrap();
        assert_eq!(t[0].get_column(1).len(), 1);
        p.remove(0, &mut t, true).unwrap();
        assert_eq!(t[0].get_column(1).len(), 3);
    }
    #[test]
    fn identity_survives_rename_and_distinguishes_same_named_tables() {
        let frame = DataFrame::from_floats(["Time [s]"], vec![vec![0., 1., 2.]]).unwrap();
        let mut t = vec![
            Table::from_df(frame.clone(), "same"),
            Table::from_df(frame, "same"),
        ];
        let mut a = Action::new(
            "mask",
            Plugin::Mask {
                expression: "{Time}>0".into(),
            },
        );
        a.apply(&mut t, false);
        assert!(t.iter().all(|t| t.get_column(1).len() == 2));
        t[0].rename("renamed");
        a.cancel(&mut t).unwrap();
        assert!(t.iter().all(|t| t.get_column(1).len() == 3));
    }
    #[test]
    fn forced_rename_does_not_reconvert_units() {
        let mut tables = vec![Table::from_df(
            DataFrame::from_floats(["FldSpeed [rpm]"], vec![vec![30.]]).unwrap(),
            "t",
        )];
        let mut p = Pipeline::default();
        p.append(
            Action::new(
                "rename",
                Plugin::RenameChannels {
                    flavor: "FldToAero".into(),
                },
            ),
            false,
        );
        p.append(
            Action::new(
                "units",
                Plugin::Units {
                    flavor: "SI".into(),
                },
            ),
            false,
        );
        p.apply(&mut tables, false);
        p.apply(&mut tables, true);
        assert!(p.errors.is_empty());
        assert_eq!(tables[0].columns()[1], "AeroSpeed [rad/s]");
        assert!((tables[0].data.f64_col(1)[0] - std::f64::consts::PI).abs() < 1e-12);
    }
    #[test]
    fn failed_dependent_cancel_restores_tables_and_undo() {
        let mut tables = vec![Table::from_df(
            DataFrame::from_floats(["FldSpeed [rpm]"], vec![vec![0., 1., 2.]]).unwrap(),
            "t",
        )];
        let mut p = Pipeline::default();
        p.append(
            Action::new(
                "rename",
                Plugin::RenameChannels {
                    flavor: "FldToAero".into(),
                },
            ),
            false,
        );
        p.append(
            Action::new(
                "mask",
                Plugin::Mask {
                    expression: "{AeroSpeed}>0".into(),
                },
            ),
            false,
        );
        p.apply(&mut tables, false);
        p.actions[0].enabled = false;
        assert!(p.cancel(0, &mut tables).is_err());
        assert_eq!(tables[0].columns()[1], "AeroSpeed [rpm]");
        assert_eq!(tables[0].get_column(1).len(), 2);
        p.actions[0].enabled = true;
        p.remove(1, &mut tables, true).unwrap();
        p.remove(0, &mut tables, true).unwrap();
        assert_eq!(tables[0].columns()[1], "FldSpeed [rpm]");
    }
}
