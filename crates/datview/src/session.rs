//! Explicit, versioned workspace sessions. Data arrays and transient chart caches
//! are deliberately not included: sources are reloaded in the background.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    path::{Path, PathBuf},
};

use dv_chart::ChartState;
use dv_core::table::Formula;
use dv_plot::{Action, Pipeline};
use serde::{Deserialize, Serialize};

use crate::{
    export::FigureOptions,
    panes::{Annotation, PaneSettings, RangeRule},
    ui::{chart_view::ChartOptions, stats_dock::StatsSettings},
    workspace::{Mode, ModeOptions, Source, Workspace},
};

const SESSION_VERSION: u32 = 1;

/// File sources reload by path; generated sources replay their pipeline or
/// source-operation recipe without embedding the underlying measurements.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SessionSource {
    pub source: Source,
    pub formulas: Vec<Formula>,
    pub generated: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Session {
    pub version: u32,
    pub sources: Vec<SessionSource>,
    pub selected: Vec<String>,
    pub x_key: Option<String>,
    pub mode: Mode,
    pub options: ModeOptions,
    pub pipeline: Pipeline,
    pub panes: PaneSettings,
    pub annotations: Vec<Annotation>,
    pub rules: Vec<RangeRule>,
    pub follow: bool,
    pub chart: ChartState,
    pub chart_options: ChartOptions,
    pub figure_options: FigureOptions,
    pub stats: StatsSettings,
}

/// Chart and dock state returned exactly once when an asynchronous restore
/// finishes. Transient hover/drag/decimation state is intentionally absent.
pub(crate) struct RestoredView {
    pub chart: ChartState,
    pub chart_options: ChartOptions,
    pub figure_options: FigureOptions,
    pub stats: StatsSettings,
}

/// An unavailable source remains in the restore record and will be included
/// if the workspace is saved again. `reason` is also added to Problems.
pub(crate) struct UnresolvedSource {
    pub entry: SessionSource,
    pub reason: String,
    position: usize,
}

struct FormulaFailure {
    source: Source,
    generated: bool,
    original: Vec<Formula>,
    failed: Vec<Formula>,
    errors: Vec<String>,
}

/// Retain this after `finish` while unresolved sources or formulas exist.
/// Pass it to `Session::capture` so saving cannot silently discard them.
pub(crate) struct Restore {
    pending: Option<Session>,
    unresolved: Vec<UnresolvedSource>,
    failed_formulas: Vec<FormulaFailure>,
}

impl Session {
    /// Take a snapshot only on an explicit save; the workspace's data arrays
    /// and the chart's runtime caches are never cloned.
    pub(crate) fn capture(
        workspace: &Workspace,
        chart: &ChartState,
        chart_options: &ChartOptions,
        figure_options: &FigureOptions,
        stats: &StatsSettings,
        restore: Option<&Restore>,
    ) -> Result<Self, String> {
        if workspace.loading.is_some() || restore.is_some_and(Restore::is_pending) {
            return Err("Wait for the current file or session load to finish before saving".into());
        }
        if workspace.sources.len() != workspace.tables.tabs.len() {
            return Err("Cannot save session: source and table counts do not match".into());
        }
        let mut sources = Vec::with_capacity(workspace.sources.len());
        for (index, (source, table)) in workspace
            .sources
            .iter()
            .zip(&workspace.tables.tabs)
            .enumerate()
        {
            let pipeline_generated = workspace
                .pipeline
                .actions
                .iter()
                .any(|action| action.generated_table(table));
            let generated = source.path.as_os_str().is_empty();
            if generated && !pipeline_generated && source.recipe.is_none() {
                // Analysis results have no reproducible recipe; the app reports
                // that they must be recalculated when the session is saved.
                continue;
            }
            let mut source = source.clone();
            if !source.path.as_os_str().is_empty() {
                source.path = absolute(&source.path)?;
            }
            sources.push(SessionSource {
                source,
                formulas: workspace.source_formulas(index),
                generated,
            });
        }
        if let Some(restore) = restore {
            for failure in &restore.failed_formulas {
                if let Some(entry) = sources.iter_mut().find(|entry| {
                    same_source(
                        &entry.source,
                        entry.generated,
                        &failure.source,
                        failure.generated,
                    )
                }) {
                    merge_failed_formulas(&mut entry.formulas, &failure.original, &failure.failed);
                }
            }
            for missing in &restore.unresolved {
                if let Some(entry) = sources.iter_mut().find(|entry| {
                    same_source(
                        &entry.source,
                        entry.generated,
                        &missing.entry.source,
                        missing.entry.generated,
                    )
                }) {
                    // A missing file may have been opened separately after the
                    // restore; retain any formulas not yet re-created there.
                    merge_failed_formulas(
                        &mut entry.formulas,
                        &missing.entry.formulas,
                        &missing.entry.formulas,
                    );
                } else {
                    sources.insert(missing.position.min(sources.len()), missing.entry.clone());
                }
            }
        }

        let session = Self {
            version: SESSION_VERSION,
            sources,
            selected: workspace.selected.clone(),
            x_key: workspace.x_key.clone(),
            mode: workspace.mode,
            options: workspace.options.clone(),
            pipeline: saved_pipeline(&workspace.pipeline),
            panes: workspace.panes.clone(),
            annotations: workspace.annotations.clone(),
            rules: workspace.rules.clone(),
            follow: workspace.follow,
            chart: chart.persistent_snapshot(),
            chart_options: chart_options.clone(),
            figure_options: figure_options.clone(),
            stats: stats.clone(),
        };
        session.validate()?;
        Ok(session)
    }

    /// Replace the workspace and start loading the distinct input files on its
    /// worker. Call `Restore::finish` after each `Workspace::poll`.
    pub(crate) fn begin(self, workspace: &mut Workspace) -> Result<Restore, String> {
        self.validate()?;
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for entry in &self.sources {
            if !entry.generated && seen.insert(entry.source.path.clone()) {
                files.push((entry.source.path.clone(), entry.source.format.clone()));
            }
        }
        let dayfirst = workspace.tables.dayfirst;
        *workspace = Workspace::new(dayfirst);
        if !files.is_empty() {
            workspace.load_files(files, false);
        }
        Ok(Restore {
            pending: Some(self),
            unresolved: Vec::new(),
            failed_formulas: Vec::new(),
        })
    }

    /// Read and validate before changing the current workspace. Relative input
    /// paths in a session are interpreted relative to the session file itself.
    pub(crate) fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .map_err(|error| format!("Could not read session {}: {error}", path.display()))?;
        #[derive(Deserialize)]
        struct Header {
            version: u32,
        }
        let header: Header = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid session {}: {error}", path.display()))?;
        if header.version != SESSION_VERSION {
            return Err(format!(
                "Unsupported session version {} in {} (supported: {})",
                header.version,
                path.display(),
                SESSION_VERSION
            ));
        }
        let mut session: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("Invalid session {}: {error}", path.display()))?;
        let session_path = absolute(path)?;
        let parent = session_path.parent().unwrap_or(Path::new(""));
        for entry in &mut session.sources {
            let path = &mut entry.source.path;
            if !path.as_os_str().is_empty() && !path.is_absolute() {
                *path = parent.join(&*path);
            }
        }
        session
            .validate()
            .map_err(|error| format!("Invalid session {}: {error}", path.display()))?;
        Ok(session)
    }

    /// Serialize completely before opening the destination, so invalid data
    /// cannot truncate an existing session.
    pub(crate) fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        self.validate()
            .map_err(|error| format!("Could not save session {}: {error}", path.display()))?;
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("Could not encode session: {error}"))?;
        std::fs::write(path, bytes)
            .map_err(|error| format!("Could not save session {}: {error}", path.display()))
    }

    /// Replace all references to one missing file before starting another
    /// restore. The replacement may itself be missing; the loader will report
    /// the actual I/O error instead of silently dropping it.
    pub(crate) fn relocate(&mut self, old: &Path, new: impl AsRef<Path>) -> Result<usize, String> {
        if new.as_ref().as_os_str().is_empty() {
            return Err("A replacement source path cannot be empty".into());
        }
        let replacement = absolute(new.as_ref())?;
        let count = self
            .sources
            .iter()
            .filter(|entry| !entry.generated && entry.source.path == old)
            .count();
        if count == 0 {
            return Err(format!("No session source refers to {}", old.display()));
        }
        if replacement != old
            && self
                .sources
                .iter()
                .any(|entry| !entry.generated && entry.source.path == replacement)
        {
            return Err(format!(
                "The replacement {} is already another session source",
                replacement.display()
            ));
        }
        // Source-derived color slots and series IDs include the path. Re-key
        // them as well, so Locate does not change styles or hidden series.
        let keys: HashSet<String> = self
            .selected
            .iter()
            .chain(self.panes.slots.keys())
            .chain(
                self.panes
                    .styles
                    .keys()
                    .filter(|key| RangeRule::target_id(key).is_none()),
            )
            .chain(self.panes.colors.keys())
            .chain(self.panes.custom.keys())
            .cloned()
            .collect();
        let mut relocated_ids = HashMap::new();
        let mut relocated_parents = Vec::new();
        for entry in &mut self.sources {
            if !entry.generated && entry.source.path == old {
                let old_slot = entry.source.stable_key();
                let old_ids: Vec<_> = keys
                    .iter()
                    .map(|key| crate::series::series_id(&entry.source, key))
                    .collect();
                entry.source.path = replacement.clone();
                let new_slot = entry.source.stable_key();
                if let Some(slot) = self.panes.source_slots.remove(&old_slot) {
                    self.panes.source_slots.insert(new_slot.clone(), slot);
                }
                relocated_parents.push((old_slot, new_slot));
                for (key, previous) in keys.iter().zip(old_ids) {
                    let next = crate::series::series_id(&entry.source, key);
                    relocated_ids.insert(previous, next);
                }
            }
        }
        for entry in &mut self.sources {
            if let Some(recipe) = &mut entry.source.recipe {
                for (old, new) in &relocated_parents {
                    recipe.relocate_parent(old, new);
                }
            }
        }
        for (&previous, &next) in &relocated_ids {
            if self.chart.hidden.remove(&previous) {
                self.chart.hidden.insert(next);
            }
            if let Some(style) = self
                .panes
                .styles
                .remove(&PaneSettings::series_style_key(previous))
            {
                self.panes
                    .styles
                    .insert(PaneSettings::series_style_key(next), style);
            }
            if let Some(pane) = self.panes.custom_series.remove(&previous) {
                self.panes.custom_series.insert(next, pane);
            }
            if let Some(label) = self.panes.removed.remove(&previous) {
                self.panes.removed.insert(next, label);
            }
        }
        for rule in &mut self.rules {
            if let Some(next) =
                RangeRule::target_id(&rule.target).and_then(|id| relocated_ids.get(&id))
            {
                rule.target = RangeRule::series_target(*next);
            }
        }
        for annotation in &mut self.annotations {
            if let Annotation::Point { curve, .. } = annotation {
                if let Some(next) =
                    RangeRule::target_id(curve).and_then(|id| relocated_ids.get(&id))
                {
                    *curve = RangeRule::series_target(*next);
                }
            }
        }
        Ok(count)
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != SESSION_VERSION {
            return Err(format!(
                "Unsupported session version {} (supported: {})",
                self.version, SESSION_VERSION
            ));
        }
        // serde_json otherwise serializes NaN and infinity as null, silently
        // losing optional values and producing unreadable required numbers.
        self.serialize(FiniteValidator)
            .map_err(|error| format!("Invalid session: {error}"))?;
        let mut frames = HashSet::new();
        let mut formats: HashMap<&Path, &Option<String>> = HashMap::new();
        for entry in &self.sources {
            let source = &entry.source;
            if entry.generated != source.path.as_os_str().is_empty() {
                return Err(format!("Invalid source “{}”: generated sources must have no file path, and file sources must have one", source.name));
            }
            if let Some(format) = &source.format {
                if format.is_empty() {
                    return Err(format!(
                        "Invalid empty format for source “{}”; use null for automatic detection",
                        source.name
                    ));
                }
            }
            if !frames.insert((entry.generated, source.path.as_path(), source.frame)) {
                return Err(format!(
                    "Duplicate session source {} frame {}",
                    source.path.display(),
                    source.frame
                ));
            }
            if !entry.generated {
                if let Some(previous) = formats.insert(source.path.as_path(), &source.format) {
                    if previous != &source.format {
                        return Err(format!(
                            "Conflicting formats for session source {}",
                            source.path.display()
                        ));
                    }
                }
            }
            if let Some(formula) = entry.formulas.iter().find(|formula| formula.pos == 0) {
                return Err(format!(
                    "Invalid position 0 for formula “{}” in source “{}”",
                    formula.name, source.name
                ));
            }
        }
        Ok(())
    }
}

impl Restore {
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn unresolved(&self) -> &[UnresolvedSource] {
        &self.unresolved
    }

    /// Finish only after the background loader has delivered its results.
    /// File frames are matched individually, so a failed file or a changed
    /// multi-frame file never shifts the state onto another source.
    pub(crate) fn finish(&mut self, workspace: &mut Workspace) -> Option<RestoredView> {
        if workspace.loading.is_some() {
            return None;
        }
        let session = self.pending.take()?;
        let Session {
            sources,
            selected,
            x_key,
            mode,
            options,
            pipeline,
            panes,
            annotations,
            rules,
            follow,
            chart,
            chart_options,
            figure_options,
            stats,
            ..
        } = session;

        workspace.mode = mode;
        workspace.options = options;
        workspace.panes = panes;
        workspace.annotations = annotations;
        workspace.rules = rules;
        workspace.follow = follow;

        if workspace.sources.len() != workspace.tables.tabs.len() {
            let reason =
                "Cannot restore session: the loaded source and table counts differ".to_owned();
            workspace.problems.push(reason.clone());
            for (position, entry) in sources.into_iter().enumerate() {
                self.unresolved.push(UnresolvedSource {
                    entry,
                    position,
                    reason: reason.clone(),
                });
            }
            workspace.selected = selected;
            workspace.x_key = x_key;
            workspace.pipeline = pipeline;
            return Some(RestoredView {
                chart,
                chart_options,
                figure_options,
                stats,
            });
        }

        // Move, rather than clone, tables. Unmatched frames are not part of
        // the saved workspace (e.g. a table removed from a multi-frame file).
        let mut loaded: Vec<_> = std::mem::take(&mut workspace.sources)
            .into_iter()
            .zip(std::mem::take(&mut workspace.tables.tabs))
            .map(Some)
            .collect();
        let mut files = Vec::new();
        let mut generated = Vec::new();
        for (position, entry) in sources.into_iter().enumerate() {
            if entry.generated {
                generated.push((position, entry));
                continue;
            }
            let found = loaded.iter().position(|item| {
                item.as_ref().is_some_and(|(source, _)| {
                    same_source(source, false, &entry.source, false)
                        && source.format == entry.source.format
                })
            });
            if let Some(index) = found {
                let (source, table) = loaded[index].take().expect("matched loaded source");
                workspace.sources.push(source);
                workspace.tables.tabs.push(table);
                files.push((position, entry));
            } else {
                self.missing(workspace, position, entry);
            }
        }
        for (source, _) in loaded.into_iter().flatten() {
            workspace.exclude_source(&source);
        }

        let mut order = Vec::with_capacity(files.len() + generated.len());
        for (index, (position, entry)) in files.into_iter().enumerate() {
            self.restore_entry(workspace, index, &entry);
            order.push((position, index));
        }
        workspace.pipeline = pipeline;
        // Replay in order: a newly generated table can have a formula that
        // a later processing step depends on. Retry formulas after every step
        // because a rename/unit action can make a referenced column available.
        let mut generated: Vec<_> = generated.into_iter().map(Some).collect();
        let mut stage = 0usize;
        self.restore_recipes(workspace, &mut generated, &mut order, stage);
        workspace.apply_pipeline_with(|workspace, added| {
            stage += 1;
            for index in added {
                let found = generated.iter().position(|entry| {
                    entry.as_ref().is_some_and(|(_, entry)| {
                        entry.source.recipe.is_none()
                            && same_source(&workspace.sources[index], true, &entry.source, true)
                    })
                });
                if let Some(found) = found {
                    let (position, entry) =
                        generated[found].take().expect("matched generated source");
                    self.restore_entry(workspace, index, &entry);
                    order.push((position, index));
                }
            }
            self.restore_recipes(workspace, &mut generated, &mut order, stage);
            self.retry_formulas(workspace);
        });
        self.restore_recipes(workspace, &mut generated, &mut order, usize::MAX);
        for (position, entry) in generated.into_iter().flatten() {
            self.missing(workspace, position, entry);
        }
        self.retry_formulas(workspace);
        for failure in &mut self.failed_formulas {
            workspace.problems.append(&mut failure.errors);
        }
        self.failed_formulas
            .retain(|failure| !failure.failed.is_empty());

        // Recreate saved source order including generated tables. Drop only
        // unrequested frames/outputs, not unresolved session records.
        order.sort_by_key(|(position, _)| *position);
        let mut available: Vec<_> = std::mem::take(&mut workspace.sources)
            .into_iter()
            .zip(std::mem::take(&mut workspace.tables.tabs))
            .map(Some)
            .collect();
        for (_, index) in order {
            if let Some((source, table)) = available[index].take() {
                workspace.sources.push(source);
                workspace.tables.tabs.push(table);
            }
        }
        for (source, _) in available.into_iter().flatten() {
            workspace.exclude_source(&source);
        }
        workspace.selected = selected;
        workspace.x_key = x_key;
        workspace.data_changed();
        workspace.rebuild();
        let missing_keys: Vec<_> = workspace
            .selected
            .iter()
            .filter(|key| {
                !workspace
                    .channels
                    .iter()
                    .any(|channel| &channel.key == *key)
            })
            .cloned()
            .collect();
        if !missing_keys.is_empty() {
            workspace.problems.push(format!(
                "Session channels unavailable: {}",
                missing_keys.join(", ")
            ));
        }
        if let Some(key) = &workspace.x_key {
            if !workspace.channels.iter().any(|channel| &channel.key == key) {
                workspace
                    .problems
                    .push(format!("Session X channel unavailable: {key}"));
            }
        }
        Some(RestoredView {
            chart,
            chart_options,
            figure_options,
            stats,
        })
    }

    fn restore_recipes(
        &mut self,
        workspace: &mut Workspace,
        pending: &mut [Option<(usize, SessionSource)>],
        order: &mut Vec<(usize, usize)>,
        stage: usize,
    ) {
        loop {
            let ready = pending.iter().position(|entry| {
                entry.as_ref().is_some_and(|(_, entry)| {
                    entry.source.recipe_stage <= stage
                        && entry.source.recipe.as_ref().is_some_and(|recipe| {
                            recipe.parent_keys().iter().all(|key| {
                                workspace
                                    .sources
                                    .iter()
                                    .any(|source| source.stable_key() == *key)
                            })
                        })
                })
            });
            let Some(ready) = ready else { break };
            let Some((position, entry)) = pending[ready].take() else {
                continue;
            };
            let Some(recipe) = entry.source.recipe.as_ref() else {
                continue;
            };
            match workspace.regenerate(recipe) {
                Ok(index) => {
                    self.restore_entry(workspace, index, &entry);
                    order.push((position, index));
                }
                Err(error) => {
                    workspace.problems.push(format!(
                        "Could not regenerate “{}”: {error}",
                        entry.source.name
                    ));
                    self.missing(workspace, position, entry);
                }
            }
        }
    }

    fn missing(&mut self, workspace: &mut Workspace, position: usize, entry: SessionSource) {
        let reason = if entry.generated {
            if entry.source.recipe.is_some() {
                format!("Could not regenerate “{}”: a parent source is missing or the source operation failed", entry.source.name)
            } else {
                format!(
                    "Could not regenerate “{}”; see the processing errors",
                    entry.source.name
                )
            }
        } else {
            let sheet = if entry.source.frame_name.is_empty() {
                String::new()
            } else {
                format!(" (sheet “{}”)", entry.source.frame_name)
            };
            format!(
                "Could not restore “{}”{sheet} from {}; see the file loading errors",
                entry.source.name,
                entry.source.path.display()
            )
        };
        workspace.problems.push(reason.clone());
        self.unresolved.push(UnresolvedSource {
            entry,
            reason,
            position,
        });
    }

    fn restore_entry(&mut self, workspace: &mut Workspace, index: usize, entry: &SessionSource) {
        let errors = workspace.restore_source(index, entry.source.clone(), &entry.formulas);
        if !errors.is_empty() {
            // Some formulas refer to columns created or renamed by the
            // pipeline. Defer reporting until they can be retried afterward.
            let applied = &workspace.tables.tabs[index].formulas;
            let failed: Vec<_> = entry
                .formulas
                .iter()
                .filter(|formula| {
                    !applied.iter().any(|actual| {
                        actual.name == formula.name && actual.formula == formula.formula
                    })
                })
                .cloned()
                .collect();
            if !failed.is_empty() {
                self.failed_formulas.push(FormulaFailure {
                    source: entry.source.clone(),
                    generated: entry.generated,
                    original: entry.formulas.clone(),
                    failed,
                    errors,
                });
            } else {
                workspace.problems.extend(errors);
            }
        }
    }

    fn retry_formulas(&mut self, workspace: &mut Workspace) {
        for failure in &mut self.failed_formulas {
            if failure.failed.is_empty() {
                continue;
            }
            let Some(index) = workspace.sources.iter().position(|source| {
                same_source(
                    source,
                    source.path.as_os_str().is_empty(),
                    &failure.source,
                    failure.generated,
                )
            }) else {
                continue;
            };
            let Some(table) = workspace.tables.tabs.get_mut(index) else {
                continue;
            };
            let mut remaining = Vec::new();
            let mut errors = Vec::new();
            for formula in std::mem::take(&mut failure.failed) {
                if table
                    .formulas
                    .iter()
                    .any(|actual| actual.name == formula.name && actual.formula == formula.formula)
                {
                    continue;
                }
                match table.add_column_by_formula(
                    &formula.name,
                    &formula.formula,
                    Some(formula.pos.saturating_sub(1)),
                ) {
                    Ok(()) => {}
                    Err(error) => {
                        errors.push(format!(
                            "Formula “{}” failed for {}: {error}",
                            formula.name, failure.source.name
                        ));
                        remaining.push(formula);
                    }
                }
            }
            failure.failed = remaining;
            failure.errors = errors;
        }
    }
}

fn same_source(a: &Source, a_generated: bool, b: &Source, b_generated: bool) -> bool {
    a_generated == b_generated
        && a.path == b.path
        && a.frame == b.frame
        && a.frame_name == b.frame_name
}

/// Keep failed formulas in their original dependency order. A currently
/// existing formula with the same name represents a user's subsequent edit
/// and takes precedence over the saved failure.
fn merge_failed_formulas(current: &mut Vec<Formula>, original: &[Formula], failed: &[Formula]) {
    for (index, formula) in original.iter().enumerate() {
        if !failed.iter().any(|item| item == formula)
            || current.iter().any(|item| item.name == formula.name)
        {
            continue;
        }
        let next = original[index + 1..]
            .iter()
            .find_map(|later| current.iter().position(|item| item.name == later.name));
        current.insert(next.unwrap_or(current.len()), formula.clone());
    }
}

/// Pipeline's runtime undo snapshots can contain whole dataframes. Only copy
/// the serializable action configuration into a session.
fn saved_pipeline(pipeline: &Pipeline) -> Pipeline {
    let actions = pipeline
        .actions
        .iter()
        .map(|old| {
            let mut action = Action::new(old.name.clone(), old.plugin.clone());
            action.kind = old.kind;
            action.enabled = old.enabled;
            action.unique = old.unique;
            action.x_column = old.x_column;
            action
        })
        .collect();
    Pipeline {
        actions,
        errors: Vec::new(),
    }
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(path))
            .map_err(|error| format!("Could not resolve {}: {error}", path.display()))
    }
}

// A non-allocating serializer that checks the entire serializable model for
// non-finite floats, including nested optional pipeline/chart/UI settings.
#[derive(Debug)]
struct FiniteError(String);

impl fmt::Display for FiniteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for FiniteError {}
impl serde::ser::Error for FiniteError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

struct FiniteValidator;

macro_rules! scalar_methods {
    ($($name:ident($ty:ty)),* $(,)?) => {$ (
        fn $name(self, _: $ty) -> Result<(), FiniteError> { Ok(()) }
    )*};
}

impl serde::Serializer for FiniteValidator {
    type Ok = ();
    type Error = FiniteError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    scalar_methods! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32),
        serialize_i64(i64), serialize_i128(i128), serialize_u8(u8), serialize_u16(u16),
        serialize_u32(u32), serialize_u64(u64), serialize_u128(u128), serialize_char(char),
        serialize_str(&str), serialize_bytes(&[u8]),
    }
    fn serialize_f32(self, value: f32) -> Result<(), FiniteError> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(FiniteError("non-finite number".into()))
        }
    }
    fn serialize_f64(self, value: f64) -> Result<(), FiniteError> {
        if value.is_finite() {
            Ok(())
        } else {
            Err(FiniteError("non-finite number".into()))
        }
    }
    fn serialize_none(self) -> Result<(), FiniteError> {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), FiniteError> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<(), FiniteError> {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<(), FiniteError> {
        Ok(())
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<(), FiniteError> {
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), FiniteError> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<(), FiniteError> {
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self, FiniteError> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> Result<Self, FiniteError> {
        Ok(self)
    }
    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self, FiniteError> {
        Ok(self)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, FiniteError> {
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self, FiniteError> {
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self, FiniteError> {
        Ok(self)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self, FiniteError> {
        Ok(self)
    }
}

macro_rules! sequence_validator {
    ($trait:ident, $method:ident) => {
        impl serde::ser::$trait for FiniteValidator {
            type Ok = ();
            type Error = FiniteError;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), FiniteError> {
                value.serialize(FiniteValidator)
            }
            fn end(self) -> Result<(), FiniteError> {
                Ok(())
            }
        }
    };
}
sequence_validator!(SerializeSeq, serialize_element);
sequence_validator!(SerializeTuple, serialize_element);
sequence_validator!(SerializeTupleStruct, serialize_field);
sequence_validator!(SerializeTupleVariant, serialize_field);

impl serde::ser::SerializeMap for FiniteValidator {
    type Ok = ();
    type Error = FiniteError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), FiniteError> {
        key.serialize(FiniteValidator)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), FiniteError> {
        value.serialize(FiniteValidator)
    }
    fn end(self) -> Result<(), FiniteError> {
        Ok(())
    }
}
macro_rules! struct_validator {
    ($trait:ident) => {
        impl serde::ser::$trait for FiniteValidator {
            type Ok = ();
            type Error = FiniteError;
            fn serialize_field<T: ?Sized + Serialize>(
                &mut self,
                _: &'static str,
                value: &T,
            ) -> Result<(), FiniteError> {
                value.serialize(FiniteValidator)
            }
            fn end(self) -> Result<(), FiniteError> {
                Ok(())
            }
        }
    };
}
struct_validator!(SerializeStruct);
struct_validator!(SerializeStructVariant);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::{FilterMode, PaneGrouping, SeriesStyle};
    use crate::ui::stats_dock::{StatsColumn, StatsScope};
    use dv_chart::Tool;
    use dv_core::common::clean_col;
    use dv_plot::Plugin;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "datview-session-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn csv(dir: &TestDir) -> PathBuf {
        let path = dir.file("signals.csv");
        std::fs::write(&path, "Time_[s],A_[W],B_[V]\n0,1,2\n1,3,4\n2,5,6\n").unwrap();
        path
    }

    fn wait_for_load(workspace: &mut Workspace) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while workspace.loading.is_some() {
            workspace.poll();
            assert!(Instant::now() < deadline, "background load did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn defaults(workspace: &Workspace) -> Session {
        Session::capture(
            workspace,
            &ChartState::default(),
            &ChartOptions::default(),
            &FigureOptions::default(),
            &StatsSettings::default(),
            None,
        )
        .unwrap()
    }

    #[test]
    fn round_trip_restores_files_formulas_pipeline_and_presentation() {
        let dir = TestDir::new();
        let input = csv(&dir);
        let mut workspace = Workspace::new(false);
        workspace.load(vec![input.clone()], false, Some("CSV file".into()));
        wait_for_load(&mut workspace);
        assert_eq!(workspace.sources.len(), 1, "{:?}", workspace.problems);

        let column = workspace.tables.tabs[0].data.column(2).name.clone();
        workspace.tables.tabs[0]
            .add_column_by_formula("Double_[W]", &format!("{{{column}}} * 2"), None)
            .unwrap();
        workspace.sources[0].name = "Run A".into();
        workspace.sources[0].offset = 2.25;
        workspace.sources[0].align_start = true;
        workspace.pipeline.actions.push(Action::new(
            "Concatenate",
            Plugin::Concat { common_only: true },
        ));
        workspace.apply_pipeline();
        assert_eq!(workspace.sources.len(), 2, "{:?}", workspace.problems);
        let generated_column = workspace.tables.tabs[1].data.column(2).name.clone();
        workspace.tables.tabs[1]
            .add_column_by_formula(
                "Generated_[W]",
                &format!("{{{generated_column}}} + 1"),
                None,
            )
            .unwrap();
        // A later table action depends on the generated table's formula.
        workspace.pipeline.actions.push(Action::new(
            "Mask generated",
            Plugin::Mask {
                expression: "{Generated_[W]} > 2".into(),
            },
        ));
        workspace.apply_pipeline();
        let generated_mask = workspace.tables.tabs[1].mask.clone();
        assert!(generated_mask.is_some(), "{:?}", workspace.problems);
        workspace.sources[1].name = "Derived".into();
        workspace.sources[1].active = false;
        workspace.sources[1].offset = -0.5;

        workspace.mode = Mode::Normalized;
        workspace.options.normalized.y_center = "Mean=0".into();
        workspace.selected = vec![clean_col("Double_[W]"), clean_col("B_[V]")];
        workspace.x_key = Some(clean_col("Time_[s]"));
        workspace.panes.grouping = PaneGrouping::Custom;
        workspace
            .panes
            .custom
            .insert(clean_col("Double_[W]"), "Power pane".into());
        workspace.panes.styles.insert(
            clean_col("Double_[W]"),
            SeriesStyle {
                color: Some([4, 5, 6]),
                width: Some(2.5),
                dash: Some(2),
                ..Default::default()
            },
        );
        workspace.annotations.push(Annotation::Region {
            mode: Mode::Normalized,
            x_axis: "time".into(),
            min: 0.25,
            max: 1.5,
            label: "Transient".into(),
            color: [100, 150, 200],
        });
        workspace.rules.push(RangeRule {
            enabled: true,
            mode: Mode::Normalized,
            target: clean_col("Double_[W]"),
            bounds: [Some(0.2), Some(1.8), None, Some(20.0)],
            action: FilterMode::Exclude,
        });
        workspace.follow = true;
        workspace.data_changed();
        workspace.rebuild();

        let mut chart = ChartState::default();
        chart.x_view = Some([0.25, 1.75]);
        chart.pane_x_views.insert(42, [0.5, 1.5]);
        chart.y_views.insert(42, [-2.0, 8.0]);
        chart.pane_weights.insert(42, 1.75);
        chart.maximized = Some(42);
        chart.selection = Some([0.6, 1.2]);
        chart.tool = Tool::Zoom;
        chart.hidden.insert(123);
        chart.highlighted = Some(999);
        let mut chart_options = ChartOptions::default();
        chart_options.log_y = true;
        chart_options.title = "A recorded chart".into();
        let mut figure = FigureOptions::default();
        figure.width = 1920;
        figure.include_selection = true;
        let stats = StatsSettings {
            scope: StatsScope::All,
            columns: vec![StatsColumn::N, StatsColumn::Rms, StatsColumn::Integral],
            threshold: 3.5,
        };

        let before =
            Session::capture(&workspace, &chart, &chart_options, &figure, &stats, None).unwrap();
        let path = dir.file("view.dvsession");
        before.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        let mut target = Workspace::new(false);
        let mut restore = loaded.begin(&mut target).unwrap();
        assert!(restore.is_pending());
        if target.loading.is_some() {
            assert!(restore.finish(&mut target).is_none());
        }
        wait_for_load(&mut target);
        let view = restore.finish(&mut target).unwrap();
        assert!(!restore.is_pending());
        assert!(restore.unresolved().is_empty(), "{:?}", target.problems);
        assert_eq!(target.sources.len(), 2);
        assert_eq!(target.tables.tabs[0].formulas.len(), 1);
        assert_eq!(target.tables.tabs[1].formulas.len(), 1);
        assert_eq!(target.tables.tabs[1].mask, generated_mask);
        assert!(view.chart.highlighted.is_none());
        let after = Session::capture(
            &target,
            &view.chart,
            &view.chart_options,
            &view.figure_options,
            &view.stats,
            Some(&restore),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }

    #[test]
    fn mixed_formats_and_source_order_survive_a_partial_load() {
        let dir = TestDir::new();
        let first = csv(&dir);
        let second = dir.file("second.csv");
        std::fs::write(&second, "Time_[s],A_[W],B_[V]\n0,2,3\n1,4,5\n2,6,7\n").unwrap();
        let mut workspace = Workspace::new(false);
        workspace.load_files(
            vec![
                (first.clone(), Some("CSV file".into())),
                (second.clone(), None),
            ],
            false,
        );
        wait_for_load(&mut workspace);
        assert_eq!(workspace.sources.len(), 2, "{:?}", workspace.problems);
        workspace.sources.swap(0, 1);
        workspace.tables.tabs.swap(0, 1);
        workspace.sources[0].name = "Second".into();
        workspace.sources[1].offset = 3.0;
        workspace.data_changed();
        workspace.rebuild();
        let before = defaults(&workspace);
        std::fs::remove_file(&second).unwrap();

        let mut target = Workspace::new(false);
        let mut restore = before.clone().begin(&mut target).unwrap();
        wait_for_load(&mut target);
        let view = restore.finish(&mut target).unwrap();
        assert_eq!(target.sources.len(), 1);
        assert_eq!(target.sources[0].path, first);
        assert_eq!(target.sources[0].format.as_deref(), Some("CSV file"));
        assert_eq!(target.sources[0].offset, 3.0);
        assert_eq!(restore.unresolved().len(), 1);
        assert_eq!(restore.unresolved()[0].entry.source.path, second);
        let after = Session::capture(
            &target,
            &view.chart,
            &view.chart_options,
            &view.figure_options,
            &view.stats,
            Some(&restore),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }

    #[test]
    fn split_and_merged_sources_are_rebuilt_from_recipes() {
        let dir = TestDir::new();
        let gpus = dir.file("gpus.csv");
        std::fs::write(
            &gpus,
            "Time_[s],name,Power_[W]\n0,RTX 4090,100\n0,RTX 3060,50\n1,RTX 4090,110\n1,RTX 3060,55\n2,RTX 4090,120\n2,RTX 3060,60\n",
        )
        .unwrap();
        let mut workspace = Workspace::new(false);
        workspace.load(vec![gpus], false, Some("CSV file".into()));
        wait_for_load(&mut workspace);
        workspace.x_key = Some(clean_col("Time_[s]"));
        let column = workspace.tables.tabs[0]
            .data
            .columns()
            .iter()
            .position(|c| c.name == "name")
            .unwrap();
        workspace.split_source(0, column).unwrap();
        assert_eq!(workspace.sources.len(), 3, "{:?}", workspace.problems);
        workspace.selected = vec![clean_col("Power_[W]")];
        workspace.merge_sources().unwrap();
        assert_eq!(workspace.sources.len(), 4, "{:?}", workspace.problems);
        workspace.data_changed();
        workspace.rebuild();
        let merged_columns: Vec<String> = workspace.tables.tabs[3]
            .data
            .columns()
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let before = defaults(&workspace);

        let mut target = Workspace::new(false);
        let mut restore = before.clone().begin(&mut target).unwrap();
        wait_for_load(&mut target);
        let view = restore.finish(&mut target).unwrap();
        assert!(restore.unresolved().is_empty(), "{:?}", target.problems);
        let names: Vec<&str> = target.sources.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "gpus.csv",
                "gpus.csv · RTX 4090",
                "gpus.csv · RTX 3060",
                "RTX 4090 + RTX 3060 (merged)"
            ]
        );
        let active: Vec<bool> = target.sources.iter().map(|s| s.active).collect();
        assert_eq!(active, [false, false, false, true]);
        let restored_columns: Vec<String> = target.tables.tabs[3]
            .data
            .columns()
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(restored_columns, merged_columns);
        let after = Session::capture(
            &target,
            &view.chart,
            &view.chart_options,
            &view.figure_options,
            &view.stats,
            Some(&restore),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }

    #[test]
    fn missing_file_is_reported_retained_and_relocatable() {
        let dir = TestDir::new();
        let input = csv(&dir);
        let mut workspace = Workspace::new(false);
        workspace.load(vec![input.clone()], false, None);
        wait_for_load(&mut workspace);
        assert_eq!(workspace.sources.len(), 1, "{:?}", workspace.problems);
        workspace.rebuild();
        let key = workspace.selected[0].clone();
        let old_id = crate::series::series_id(&workspace.sources[0], &key);
        let old_slot_key = workspace.sources[0].stable_key();
        let old_slot = workspace.panes.source_slots.get(&old_slot_key).copied();
        let mut chart = ChartState::default();
        chart.hidden.insert(old_id);
        let before = Session::capture(
            &workspace,
            &chart,
            &ChartOptions::default(),
            &FigureOptions::default(),
            &StatsSettings::default(),
            None,
        )
        .unwrap();
        std::fs::remove_file(&input).unwrap();

        let mut target = Workspace::new(false);
        let mut restore = before.clone().begin(&mut target).unwrap();
        wait_for_load(&mut target);
        let view = restore.finish(&mut target).unwrap();
        assert_eq!(restore.unresolved().len(), 1);
        assert_eq!(restore.unresolved()[0].entry.source.path, input);
        assert!(!restore.unresolved()[0].reason.is_empty());
        assert!(target
            .problems
            .iter()
            .any(|problem| problem.contains("signals.csv")));
        let mut saved = Session::capture(
            &target,
            &view.chart,
            &view.chart_options,
            &view.figure_options,
            &view.stats,
            Some(&restore),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&saved).unwrap()
        );

        let replacement = dir.file("replacement.csv");
        std::fs::write(&replacement, "Time_[s],A_[W],B_[V]\n0,1,2\n1,3,4\n").unwrap();
        assert_eq!(saved.relocate(&input, &replacement).unwrap(), 1);
        let new_id = crate::series::series_id(&saved.sources[0].source, &key);
        assert!(!saved.chart.hidden.contains(&old_id));
        assert!(saved.chart.hidden.contains(&new_id));
        assert_eq!(
            saved
                .panes
                .source_slots
                .get(&saved.sources[0].source.stable_key())
                .copied(),
            old_slot
        );
        let mut retry = saved.begin(&mut target).unwrap();
        wait_for_load(&mut target);
        retry.finish(&mut target).unwrap();
        assert!(retry.unresolved().is_empty(), "{:?}", target.problems);
        assert_eq!(target.sources[0].path, replacement);
    }

    #[test]
    fn relocating_source_rekeys_inspector_presentation() {
        let dir = TestDir::new();
        let input = csv(&dir);
        let mut workspace = Workspace::new(false);
        workspace.load(vec![input.clone()], false, None);
        wait_for_load(&mut workspace);
        workspace.rebuild();
        let key = workspace.selected[0].clone();
        let old = crate::series::series_id(&workspace.sources[0], &key);
        let mut session = defaults(&workspace);
        let mut derived = session.sources[0].clone();
        derived.generated = true;
        derived.source.path = PathBuf::new();
        derived.source.recipe = Some(crate::workspace::GeneratedRecipe::Transpose {
            parent: workspace.sources[0].stable_key(),
        });
        session.sources.push(derived);
        session.panes.styles.insert(
            PaneSettings::series_style_key(old),
            SeriesStyle {
                color: Some([8, 30, 80]),
                width: Some(2.5),
                ..Default::default()
            },
        );
        session.panes.custom_series.insert(old, "Thermal".into());
        session.panes.removed.insert(old, "A".into());
        session.rules.push(RangeRule {
            enabled: true,
            mode: Mode::Signals,
            target: RangeRule::series_target(old),
            bounds: [None, None, Some(1.0), None],
            action: FilterMode::Keep,
        });
        session.annotations.push(Annotation::Point {
            mode: Mode::Signals,
            curve: RangeRule::series_target(old),
            x: 1.0,
            y: 3.0,
            label: "Peak".into(),
            color: [66, 101, 218],
        });
        session.chart.hidden.insert(old);
        let replacement = dir.file("replacement.csv");
        session.relocate(&input, &replacement).unwrap();
        let new = crate::series::series_id(&session.sources[0].source, &key);
        assert_ne!(old, new);
        assert!(!session
            .panes
            .styles
            .contains_key(&PaneSettings::series_style_key(old)));
        assert_eq!(
            session.panes.styles[&PaneSettings::series_style_key(new)].width,
            Some(2.5)
        );
        assert_eq!(
            session.panes.custom_series.get(&new).map(String::as_str),
            Some("Thermal")
        );
        assert_eq!(
            session.panes.removed.get(&new).map(String::as_str),
            Some("A")
        );
        assert_eq!(RangeRule::target_id(&session.rules[0].target), Some(new));
        assert!(
            matches!(&session.annotations[0], Annotation::Point { curve, .. } if RangeRule::target_id(curve) == Some(new))
        );
        assert!(session.chart.hidden.contains(&new));
        assert!(!session.chart.hidden.contains(&old));
        assert!(matches!(&session.sources[1].source.recipe,
            Some(crate::workspace::GeneratedRecipe::Transpose { parent })
                if parent == &session.sources[0].source.stable_key()));
    }

    #[test]
    fn failed_formula_is_reported_and_preserved_when_resaving() {
        let dir = TestDir::new();
        let input = csv(&dir);
        let mut workspace = Workspace::new(false);
        workspace.load(vec![input.clone()], false, None);
        wait_for_load(&mut workspace);
        assert_eq!(workspace.sources.len(), 1, "{:?}", workspace.problems);
        let column = workspace.tables.tabs[0].data.column(2).name.clone();
        workspace.tables.tabs[0]
            .add_column_by_formula("Double_[W]", &format!("{{{column}}} * 2"), None)
            .unwrap();
        workspace.data_changed();
        workspace.selected = vec![clean_col("Double_[W]")];
        let before = defaults(&workspace);
        std::fs::write(&input, "Time_[s],B_[V]\n0,2\n1,4\n2,6\n").unwrap();

        let mut target = Workspace::new(false);
        let mut restore = before.clone().begin(&mut target).unwrap();
        wait_for_load(&mut target);
        let view = restore.finish(&mut target).unwrap();
        assert!(restore.unresolved().is_empty());
        assert!(target
            .problems
            .iter()
            .any(|problem| problem.contains("Formula")));
        assert!(target.tables.tabs[0].formulas.is_empty());
        assert_eq!(target.selected, vec![clean_col("Double_[W]")]);
        let after = Session::capture(
            &target,
            &view.chart,
            &view.chart_options,
            &view.figure_options,
            &view.stats,
            Some(&restore),
        )
        .unwrap();
        assert_eq!(after.sources[0].formulas, before.sources[0].formulas);
    }

    #[test]
    fn formula_can_depend_on_a_pipeline_renamed_channel() {
        let dir = TestDir::new();
        let input = dir.file("renamed.csv");
        std::fs::write(&input, "Time_[s],FldSignal_[W]\n0,1\n1,2\n2,3\n").unwrap();
        let mut workspace = Workspace::new(false);
        workspace.load(vec![input], false, None);
        wait_for_load(&mut workspace);
        assert_eq!(workspace.sources.len(), 1, "{:?}", workspace.problems);
        workspace.pipeline.actions.push(Action::new(
            "Rename",
            Plugin::RenameChannels {
                flavor: "FldToAero".into(),
            },
        ));
        workspace.apply_pipeline();
        let renamed = workspace.tables.tabs[0].data.column(2).name.clone();
        assert!(renamed.starts_with("Aero"));
        workspace.tables.tabs[0]
            .add_column_by_formula("Twice_[W]", &format!("{{{renamed}}} * 2"), None)
            .unwrap();
        workspace.data_changed();
        let before = defaults(&workspace);

        let mut target = Workspace::new(false);
        let mut restore = before.clone().begin(&mut target).unwrap();
        wait_for_load(&mut target);
        let view = restore.finish(&mut target).unwrap();
        assert!(restore.unresolved().is_empty());
        assert!(
            !target
                .problems
                .iter()
                .any(|problem| problem.contains("Formula")),
            "{:?}",
            target.problems
        );
        assert_eq!(target.tables.tabs[0].formulas.len(), 1);
        let after = Session::capture(
            &target,
            &view.chart,
            &view.chart_options,
            &view.figure_options,
            &view.stats,
            Some(&restore),
        )
        .unwrap();
        assert_eq!(after.sources[0].formulas, before.sources[0].formulas);
    }

    #[test]
    fn rejects_invalid_versions_and_nonfinite_values_without_truncation() {
        let dir = TestDir::new();
        let path = dir.file("bad.dvsession");
        std::fs::write(&path, r#"{"version":2}"#).unwrap();
        assert!(Session::load(&path)
            .err()
            .unwrap()
            .contains("Unsupported session version"));
        std::fs::write(&path, b"not JSON").unwrap();
        assert!(Session::load(&path)
            .err()
            .unwrap()
            .contains("Invalid session"));
        std::fs::write(&path, r#"{"version":1}"#).unwrap();
        assert!(Session::load(&path)
            .err()
            .unwrap()
            .contains("missing field"));

        let workspace = Workspace::new(false);
        let mut session = defaults(&workspace);
        session.chart.selection = Some([f64::NAN, 1.0]);
        std::fs::write(&path, b"existing contents").unwrap();
        assert!(session.save(&path).err().unwrap().contains("non-finite"));
        assert_eq!(
            std::fs::read(&path).unwrap().as_slice(),
            b"existing contents"
        );
    }

    #[test]
    fn relative_paths_are_resolved_from_session_directory() {
        let dir = TestDir::new();
        let input = csv(&dir);
        let mut workspace = Workspace::new(false);
        workspace.load(vec![input.clone()], false, None);
        wait_for_load(&mut workspace);
        let mut session = defaults(&workspace);
        session.sources[0].source.path = "signals.csv".into();
        let path = dir.file("view.dvsession");
        session.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.sources[0].source.path, input);
    }
}
