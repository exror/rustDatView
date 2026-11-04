use crate::{
    channels::{self, SearchCache},
    loader::{self, FileRequest, FormulaSnapshot, Job, LoadKind, LoadedFile, Loader},
    selection,
};
use dv_core::{common::clean_col, Formula, Table, TableList};
use dv_plot::{
    ActionKind, CompareMethod, CompareMode, FftOptions, MinMaxOptions, PdfOptions, Pipeline,
    PolarOptions,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) use crate::channels::Channel;
pub(crate) use crate::panes::{
    Annotation, ColorBy, FilterMode, LineDraw, PaneGrouping, PaneSettings, RangeRule, SeriesStyle,
};
pub(crate) use crate::series::SeriesData;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Mode {
    #[default]
    Signals,
    Distribution,
    Spectrum,
    Normalized,
    Compare,
    Polar,
}
impl Mode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Signals => "Signals",
            Self::Distribution => "Distribution",
            Self::Spectrum => "Spectrum",
            Self::Normalized => "Normalized",
            Self::Compare => "Compare",
            Self::Polar => "Polar",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ModeOptions {
    pub pdf: PdfOptions,
    pub fft: FftOptions,
    pub normalized: MinMaxOptions,
    pub polar: PolarOptions,
    pub compare_mode: Option<CompareMode>,
    pub compare_method: CompareMethod,
}
impl Default for ModeOptions {
    fn default() -> Self {
        Self {
            pdf: PdfOptions::default(),
            fft: FftOptions::default(),
            normalized: MinMaxOptions::default(),
            polar: PolarOptions::default(),
            compare_mode: None,
            compare_method: CompareMethod::Relative,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Source {
    pub path: PathBuf,
    pub name: String,
    pub active: bool,
    pub offset: f64,
    pub align_start: bool,
    pub format: Option<String>,
    /// Ordinal in the file (or among generated outputs for fileless tables).
    pub frame: usize,
    pub frame_name: String,
}
impl Default for Source {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            name: String::new(),
            active: true,
            offset: 0.0,
            align_start: false,
            format: None,
            frame: 0,
            frame_name: String::new(),
        }
    }
}
impl Source {
    pub(crate) fn stable_key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.path.to_string_lossy(),
            self.frame_name,
            self.frame
        )
    }
}
/// A model curve (fit, envelope) drawn dashed in the pane of its target series.
/// Overlays are presentation only: never saved, exported as data or analyzed.
#[derive(Clone, Debug)]
pub(crate) struct Overlay {
    pub target: u64,
    pub label: String,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    /// Changes whenever x/y change, for the chart's decimation cache.
    pub generation: u64,
}

struct Watch {
    stamp: Option<loader::FileStamp>,
    changed: Option<Instant>,
}

pub(crate) struct Workspace {
    pub tables: TableList,
    pub sources: Vec<Source>,
    pub channels: Vec<Channel>,
    pub selected: Vec<String>,
    pub x_key: Option<String>,
    pub mode: Mode,
    pub options: ModeOptions,
    pub pipeline: Pipeline,
    pub series: Vec<SeriesData>,
    pub generation: u64,
    pub loading: Option<String>,
    pub problems: Vec<String>,
    pub follow: bool,
    pub panes: PaneSettings,
    pub annotations: Vec<Annotation>,
    pub rules: Vec<RangeRule>,
    pub overlays: Vec<Overlay>,
    loader: Loader,
    epoch: u64,
    pending: usize,
    requested: BTreeSet<PathBuf>,
    removed_frames: BTreeSet<(PathBuf, usize, String)>,
    deferred_formulas: HashMap<String, Vec<Formula>>,
    watching: HashMap<PathBuf, Watch>,
    next_watch: Instant,
    search_cache: SearchCache,
    pub(crate) series_signature: Option<String>,
    pub(crate) item_signatures: HashMap<u64, String>,
    pub(crate) data_revision: u64,
    table_pipeline_key: String,
    replay_key: Option<String>,
}
impl Workspace {
    pub(crate) fn new(dayfirst: bool) -> Self {
        let pipeline = Pipeline::default();
        let table_pipeline_key = table_actions_key(&pipeline);
        Self {
            tables: TableList {
                dayfirst,
                ..Default::default()
            },
            sources: Vec::new(),
            channels: Vec::new(),
            selected: Vec::new(),
            x_key: None,
            mode: Mode::Signals,
            options: ModeOptions::default(),
            pipeline,
            series: Vec::new(),
            generation: 0,
            loading: None,
            problems: Vec::new(),
            follow: false,
            panes: PaneSettings::default(),
            annotations: Vec::new(),
            rules: Vec::new(),
            overlays: Vec::new(),
            loader: Loader::new(),
            epoch: 0,
            pending: 0,
            requested: BTreeSet::new(),
            removed_frames: BTreeSet::new(),
            deferred_formulas: HashMap::new(),
            watching: HashMap::new(),
            next_watch: Instant::now(),
            search_cache: SearchCache::default(),
            series_signature: None,
            item_signatures: HashMap::new(),
            data_revision: 0,
            table_pipeline_key,
            replay_key: None,
        }
    }
    fn supersede(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.loader.set_epoch(self.epoch);
        self.pending = 0;
        self.requested.clear();
        self.loading = None;
        self.replay_key = None;
    }
    pub(crate) fn load(&mut self, paths: Vec<PathBuf>, add: bool, format: Option<String>) {
        self.load_files(
            paths
                .into_iter()
                .map(|path| (path, format.clone()))
                .collect(),
            add,
        );
    }
    pub(crate) fn load_files(&mut self, files: Vec<(PathBuf, Option<String>)>, add: bool) {
        if files.is_empty() {
            return;
        }
        if !add {
            self.supersede();
        }
        let mut seen = BTreeSet::new();
        let mut requests = Vec::new();
        for (path, format) in files {
            if path.as_os_str().is_empty() {
                continue;
            }
            let path = if path.is_absolute() {
                path
            } else {
                match std::env::current_dir() {
                    Ok(current) => current.join(path),
                    Err(error) => {
                        self.problems
                            .push(format!("Could not resolve {}: {error}", path.display()));
                        continue;
                    }
                }
            };
            if !seen.insert(path.clone()) {
                continue;
            }
            if add
                && (self.sources.iter().any(|s| s.path == path) || self.requested.contains(&path))
            {
                self.problems
                    .push(format!("File is already open: {}", path.display()));
                continue;
            }
            requests.push(FileRequest {
                path,
                format,
                formulas: Vec::new(),
            });
        }
        self.enqueue(
            if add {
                LoadKind::Add
            } else {
                LoadKind::Replace
            },
            requests,
        );
    }
    fn enqueue(&mut self, kind: LoadKind, files: Vec<FileRequest>) {
        if files.is_empty() {
            return;
        }
        let label = if files.len() == 1 {
            files[0]
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            format!("{} files", files.len())
        };
        for file in &files {
            self.requested.insert(file.path.clone());
        }
        if let Err(error) = self.loader.submit(Job {
            epoch: self.epoch,
            kind,
            dayfirst: self.tables.dayfirst,
            files,
        }) {
            self.problems.push(error);
            return;
        }
        self.pending += 1;
        self.loading = Some(format!("Loading {label}"));
    }
    fn file_requests(&self, only: Option<&Path>) -> Vec<FileRequest> {
        let mut files: Vec<FileRequest> = Vec::new();
        for (source, table) in self.sources.iter().zip(&self.tables.tabs) {
            if source.path.as_os_str().is_empty()
                || only.is_some_and(|path| path != source.path.as_path())
            {
                continue;
            }
            let index = files
                .iter()
                .position(|file| file.path == source.path)
                .unwrap_or_else(|| {
                    files.push(FileRequest {
                        path: source.path.clone(),
                        format: source.format.clone(),
                        formulas: Vec::new(),
                    });
                    files.len() - 1
                });
            files[index].formulas.push(FormulaSnapshot {
                frame: source.frame,
                frame_name: source.frame_name.clone(),
                formulas: self.formulas_for(source, table),
            });
        }
        files
    }
    fn formulas_for(&self, source: &Source, table: &Table) -> Vec<Formula> {
        let Some(saved) = self.deferred_formulas.get(&source.stable_key()) else {
            return table.formulas.clone();
        };
        let mut used = vec![false; table.formulas.len()];
        let mut merged = Vec::with_capacity(saved.len().max(table.formulas.len()));
        for old in saved {
            if let Some((index, current)) = table
                .formulas
                .iter()
                .enumerate()
                .find(|(index, current)| !used[*index] && current.name == old.name)
            {
                used[index] = true;
                merged.push(current.clone());
            } else {
                merged.push(old.clone());
            }
        }
        merged.extend(
            table
                .formulas
                .iter()
                .enumerate()
                .filter(|(index, _)| !used[*index])
                .map(|(_, formula)| formula.clone()),
        );
        merged
    }
    /// Includes definitions that could not be evaluated during a prior reload.
    pub(crate) fn source_formulas(&self, index: usize) -> Vec<Formula> {
        self.sources
            .get(index)
            .zip(self.tables.tabs.get(index))
            .map(|(source, table)| self.formulas_for(source, table))
            .unwrap_or_default()
    }
    fn has_table_actions(&self) -> bool {
        self.pipeline
            .actions
            .iter()
            .any(|action| action.kind != ActionKind::PlotData)
            || table_actions_key(&self.pipeline) != self.table_pipeline_key
    }
    pub(crate) fn reload(&mut self) {
        let files = self.file_requests(None);
        let kind = if self.has_table_actions() {
            LoadKind::Replay
        } else {
            LoadKind::Reload
        };
        if kind == LoadKind::Replay {
            self.replay_key = Some(table_actions_key(&self.pipeline));
        }
        self.enqueue(kind, files);
    }
    pub(crate) fn reload_source(&mut self, index: usize) {
        if self.has_table_actions() {
            self.reload();
            return;
        }
        if let Some(source) = self.sources.get(index) {
            let files = self.file_requests(Some(&source.path));
            self.enqueue(LoadKind::Reload, files);
        }
    }
    pub(crate) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.loader.try_recv() {
            if result.epoch != self.epoch {
                continue;
            }
            self.pending = self.pending.saturating_sub(1);
            for file in &result.files {
                self.requested.remove(&file.path);
            }
            if self.pending == 0 {
                self.loading = None;
            }
            let all_succeeded =
                !result.files.is_empty() && result.files.iter().all(|file| !file.tables.is_empty());
            let replace = (result.kind == LoadKind::Replace && all_succeeded)
                || (result.kind == LoadKind::Replay && all_succeeded);
            if result.kind == LoadKind::Replay && !all_succeeded {
                for file in result.files {
                    self.problems.extend(file.messages);
                }
                changed = true;
                continue;
            }
            let was_empty = self.tables.tabs.is_empty();
            if replace && result.kind == LoadKind::Replace {
                self.tables.tabs.clear();
                self.sources.clear();
                self.watching.clear();
                self.removed_frames.clear();
                self.deferred_formulas.clear();
            }
            let mut generated = Vec::new();
            if result.kind == LoadKind::Replay {
                // Preserve options and formulas for pipeline-generated sources.
                let mut i = self.sources.len();
                while i > 0 {
                    i -= 1;
                    if self.sources[i].path.as_os_str().is_empty() {
                        let formulas = self.source_formulas(i);
                        generated.push((self.sources.remove(i), formulas));
                        self.tables.tabs.remove(i);
                    }
                }
            }
            let mut loaded_any = false;
            for mut file in result.files {
                self.problems.append(&mut file.messages);
                if file.tables.is_empty() {
                    continue;
                }
                if result.kind == LoadKind::Replace
                    && !all_succeeded
                    && self.sources.iter().any(|source| source.path == file.path)
                {
                    continue;
                }
                if result.kind == LoadKind::Add
                    && !self.sources.iter().any(|source| source.path == file.path)
                {
                    self.removed_frames
                        .retain(|(path, _, _)| path != &file.path);
                }
                self.watching.insert(
                    file.path.clone(),
                    Watch {
                        stamp: file.stamp.clone(),
                        changed: None,
                    },
                );
                self.install_file(file);
                loaded_any = true;
            }
            if loaded_any {
                if (result.kind == LoadKind::Add
                    || (result.kind == LoadKind::Replace && !all_succeeded))
                    && !was_empty
                    && self.has_table_actions()
                {
                    // All table actions must be replayed in order on fresh inputs.
                    self.refresh_catalog(false);
                    self.reload();
                } else {
                    if result.kind == LoadKind::Replay
                        || (replace && result.kind == LoadKind::Replace)
                    {
                        if let Ok(json) = self.pipeline.to_json() {
                            if let Ok(fresh) = Pipeline::from_json(&json) {
                                self.pipeline = fresh;
                            }
                        }
                        self.replay_key = None;
                    }
                    self.apply_pipeline_with(|workspace, new_sources| {
                        for index in new_sources {
                            let current = &workspace.sources[index];
                            if let Some(position) = generated.iter().position(|(source, _)| {
                                current.frame == source.frame
                                    && current.frame_name == source.frame_name
                            }) {
                                let (source, formulas) = generated.remove(position);
                                let errors = workspace.restore_source(index, source, &formulas);
                                workspace.problems.extend(errors);
                            }
                        }
                    });
                    for (source, formulas) in generated {
                        if !formulas.is_empty() {
                            self.problems.push(format!(
                                "Could not restore formulas for generated source {}",
                                source.name
                            ));
                        }
                    }
                    self.refresh_catalog(
                        was_empty || (replace && result.kind == LoadKind::Replace),
                    );
                }
            }
            changed = true;
        }
        if self.follow && Instant::now() >= self.next_watch {
            let now = Instant::now();
            self.next_watch = now + Duration::from_millis(250);
            let paths: BTreeSet<_> = self
                .sources
                .iter()
                .filter(|s| !s.path.as_os_str().is_empty())
                .map(|s| s.path.clone())
                .collect();
            for path in paths {
                let current = loader::stamp(&path);
                let watch = self.watching.entry(path).or_insert_with(|| Watch {
                    stamp: current.clone(),
                    changed: None,
                });
                if watch.stamp != current {
                    watch.stamp = current;
                    watch.changed = Some(now);
                }
            }
            if self.pending == 0 {
                let ready: Vec<_> =
                    self.watching
                        .iter_mut()
                        .filter_map(|(path, watch)| {
                            if watch.changed.is_some_and(|at| {
                                now.duration_since(at) >= Duration::from_millis(500)
                            }) {
                                watch.changed = None;
                                Some(path.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                if !ready.is_empty() && self.has_table_actions() {
                    self.reload();
                    changed = true;
                } else {
                    for path in ready {
                        let files = self.file_requests(Some(&path));
                        self.enqueue(LoadKind::Reload, files);
                        changed = true;
                    }
                }
            }
        }
        changed
    }
    fn install_file(&mut self, file: LoadedFile) {
        let path = file.path;
        let insert_at = self
            .sources
            .iter()
            .position(|s| s.path == path)
            .unwrap_or(self.sources.len());
        let mut old = Vec::new();
        let mut i = 0;
        while i < self.sources.len() {
            if self.sources[i].path == path {
                old.push(Some((self.sources.remove(i), self.tables.tabs.remove(i))));
            } else {
                i += 1;
            }
        }
        let mut inserted = 0;
        for (frame, mut table) in file.tables.into_iter().enumerate() {
            let frame_name = table.source_frame_name().to_owned();
            if self
                .removed_frames
                .contains(&(path.clone(), frame, frame_name.clone()))
            {
                continue;
            }
            let matched = old
                .iter()
                .position(|entry| {
                    entry
                        .as_ref()
                        .is_some_and(|(s, _)| !frame_name.is_empty() && s.frame_name == frame_name)
                })
                .or_else(|| {
                    old.iter()
                        .position(|entry| entry.as_ref().is_some_and(|(s, _)| s.frame == frame))
                });
            let mut desired_formulas = None;
            let source =
                if let Some((mut source, previous)) = matched.and_then(|index| old[index].take()) {
                    // A formula may have been edited after the background request began.
                    let old_key = source.stable_key();
                    let desired = self.formulas_for(&source, &previous);
                    let snapshot = file
                        .snapshots
                        .iter()
                        .find(|s| s.frame == source.frame && s.frame_name == source.frame_name);
                    if snapshot.is_some_and(|s| s.formulas != desired) {
                        let positions: Vec<_> = table.formulas.iter().map(|f| f.pos).collect();
                        table.delete_columns(&positions);
                        self.problems
                            .extend(loader::apply_formulas(&mut table, &desired));
                    }
                    self.deferred_formulas.remove(&old_key);
                    desired_formulas = Some(desired);
                    source.frame = frame;
                    source.frame_name = frame_name;
                    source.format = file.format.clone();
                    source
                } else {
                    let file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());
                    let name = if frame_name.is_empty() {
                        file_name
                    } else {
                        format!("{file_name} · {frame_name}")
                    };
                    Source {
                        path: path.clone(),
                        name,
                        active: true,
                        offset: 0.0,
                        align_start: false,
                        format: file.format.clone(),
                        frame,
                        frame_name,
                    }
                };
            if let Some(desired) = desired_formulas {
                if desired.iter().any(|formula| {
                    !table.formulas.iter().any(|actual| {
                        actual.name == formula.name && actual.formula == formula.formula
                    })
                }) {
                    self.deferred_formulas.insert(source.stable_key(), desired);
                }
            }
            let at = (insert_at + inserted).min(self.sources.len());
            self.sources.insert(at, source);
            self.tables.tabs.insert(at, table);
            inserted += 1;
        }
    }
    fn refresh_catalog(&mut self, defaults: bool) {
        self.channels = channels::catalog(&self.tables);
        self.search_cache.reset(&self.channels);
        if defaults {
            if let Some(table) = self.tables.tabs.first() {
                let x = selection::default_x(table);
                self.x_key = table
                    .data
                    .columns()
                    .get(x)
                    .map(|column| clean_col(&column.name));
                self.selected = selection::default_ys(table, x)
                    .into_iter()
                    .map(|index| clean_col(&table.data.column(index).name))
                    .collect();
                let mut unique = BTreeSet::new();
                self.selected.retain(|key| unique.insert(key.clone()));
            } else {
                self.x_key = None;
                self.selected.clear();
            }
        }
        self.generation = self.generation.wrapping_add(1);
        self.data_revision = self.data_revision.wrapping_add(1);
        self.series_signature = None;
        self.item_signatures.clear();
    }
    /// Call after modifying tables or formulas directly.
    pub(crate) fn data_changed(&mut self) {
        self.refresh_catalog(false);
    }
    pub(crate) fn toggle(&mut self, key: &str) {
        if let Some(index) = self.selected.iter().position(|s| s == key) {
            self.selected.remove(index);
        } else if self.x_key.as_deref() != Some(key) && self.channels.iter().any(|c| c.key == key) {
            self.selected.push(key.to_owned());
        }
    }
    pub(crate) fn set_x(&mut self, key: impl Into<String>) {
        let key = key.into();
        if self.channels.iter().any(|channel| channel.key == key) {
            self.selected.retain(|selected| selected != &key);
            self.x_key = Some(key);
        }
    }
    pub(crate) fn search(&mut self, query: &str) -> Vec<usize> {
        self.search_cache.search(query)
    }
    /// Add an in-memory table (fit result, merge, stack) as a new source.
    pub(crate) fn add_generated_table(&mut self, df: dv_core::DataFrame, name: &str) {
        let ordinal = self
            .sources
            .iter()
            .filter(|source| source.path.as_os_str().is_empty())
            .count();
        let mut table = dv_core::Table::from_df(df, name);
        table.rename(name);
        self.tables.tabs.push(table);
        self.sources.push(Source {
            name: name.to_owned(),
            frame: ordinal,
            frame_name: name.to_owned(),
            ..Source::default()
        });
        self.refresh_catalog(false);
    }
    pub(crate) fn remove_source(&mut self, index: usize) {
        if index >= self.sources.len() {
            return;
        }
        self.supersede();
        let source = self.sources.remove(index);
        self.deferred_formulas.remove(&source.stable_key());
        self.exclude_source(&source);
        self.tables.tabs.remove(index);
        if !self.sources.iter().any(|s| s.path == source.path) {
            self.watching.remove(&source.path);
        }
        self.refresh_catalog(false);
        if !source.path.as_os_str().is_empty() && self.has_table_actions() {
            if self.sources.iter().any(|s| !s.path.as_os_str().is_empty()) {
                self.reload();
            } else {
                let mut i = self.sources.len();
                while i > 0 {
                    i -= 1;
                    if self.sources[i].path.as_os_str().is_empty() {
                        self.sources.remove(i);
                        self.tables.tabs.remove(i);
                    }
                }
                self.refresh_catalog(false);
            }
        }
    }
    /// Retain a session-pruned or explicitly removed file frame across reloads.
    pub(crate) fn exclude_source(&mut self, source: &Source) {
        if !source.path.as_os_str().is_empty() {
            self.removed_frames.insert((
                source.path.clone(),
                source.frame,
                source.frame_name.clone(),
            ));
        }
    }
    /// Restore source options and replay ordered formula definitions on a fresh table.
    pub(crate) fn restore_source(
        &mut self,
        index: usize,
        source: Source,
        formulas: &[Formula],
    ) -> Vec<String> {
        let Some(table) = self.tables.tabs.get_mut(index) else {
            return vec!["Source is no longer available".into()];
        };
        let positions: Vec<_> = table.formulas.iter().map(|formula| formula.pos).collect();
        table.delete_columns(&positions);
        let errors = loader::apply_formulas(table, formulas);
        if errors.is_empty() {
            self.deferred_formulas.remove(&source.stable_key());
        } else {
            self.deferred_formulas
                .insert(source.stable_key(), formulas.to_vec());
        }
        if let Some(current) = self.sources.get_mut(index) {
            *current = source;
        }
        self.refresh_catalog(false);
        errors
    }
    /// Apply the current pipeline to newly loaded or restored tables.
    pub(crate) fn apply_pipeline(&mut self) {
        self.apply_pipeline_with(|_, _| {});
    }
    /// Allow session restoration to populate generated tables before a later
    /// action consumes them. The callback also runs after non-adder actions.
    pub(crate) fn apply_pipeline_with(
        &mut self,
        mut on_step: impl FnMut(&mut Self, std::ops::Range<usize>),
    ) {
        for action in 0..self.pipeline.actions.len() {
            if self.pipeline.actions[action].kind == ActionKind::PlotData {
                continue;
            }
            self.pipeline.actions[action].apply(&mut self.tables.tabs, false);
            let first_new = self.sources.len();
            let mut ordinal = self
                .sources
                .iter()
                .filter(|source| source.path.as_os_str().is_empty())
                .count();
            while self.sources.len() < self.tables.tabs.len() {
                let table = &self.tables.tabs[self.sources.len()];
                self.sources.push(Source {
                    name: table.nickname().to_owned(),
                    frame: ordinal,
                    frame_name: table.source_frame_name().to_owned(),
                    ..Source::default()
                });
                ordinal += 1;
            }
            let last_new = self.sources.len();
            on_step(self, first_new..last_new);
        }
        self.pipeline.collect_errors();
        self.problems.extend(self.pipeline.errors.iter().cloned());
        self.table_pipeline_key = table_actions_key(&self.pipeline);
        self.refresh_catalog(false);
    }
    pub(crate) fn ensure_pipeline(&mut self) {
        let current = table_actions_key(&self.pipeline);
        if current == self.table_pipeline_key
            || self.pending > 0
            || self.replay_key.as_deref() == Some(&current)
        {
            return;
        }
        let files = self.file_requests(None);
        if files.is_empty() {
            self.apply_pipeline();
        } else {
            self.replay_key = Some(current);
            self.enqueue(LoadKind::Replay, files);
        }
    }
}
fn table_actions_key(pipeline: &Pipeline) -> String {
    let actions: Vec<_> = pipeline
        .actions
        .iter()
        .filter(|action| action.kind != ActionKind::PlotData)
        .collect();
    serde_json::to_string(&actions).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "datview-model-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str, contents: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, contents).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn finish_loading(workspace: &mut Workspace) {
        let start = Instant::now();
        while workspace.loading.is_some() {
            assert!(
                start.elapsed() < Duration::from_secs(15),
                "background loader timed out"
            );
            workspace.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn reload_preserves_source_options_and_retries_failed_formulas() {
        let fixture = Fixture::new();
        let path = fixture.file("data.csv", "Time [s],Value\n0,2\n1,4\n");
        let mut w = Workspace::new(false);
        w.load(vec![path.clone()], false, Some("CSV file".into()));
        finish_loading(&mut w);
        assert_eq!(w.sources.len(), 1, "{:?}", w.problems);
        w.sources[0].name = "Renamed".into();
        w.sources[0].offset = 2.5;
        w.sources[0].align_start = true;
        w.tables.tabs[0]
            .add_column_by_formula("Double", "{Value} * 2", None)
            .unwrap();
        w.data_changed();

        std::fs::write(&path, "Time [s],Other\n0,3\n1,5\n").unwrap();
        w.reload_source(0);
        finish_loading(&mut w);
        assert_eq!(w.sources[0].name, "Renamed");
        assert_eq!(w.sources[0].offset, 2.5);
        assert!(w.sources[0].align_start);
        assert_eq!(w.source_formulas(0).len(), 1);
        assert!(w.tables.tabs[0].data.find("Double").is_none());

        std::fs::write(&path, "Time [s],Value\n0,3\n1,5\n").unwrap();
        w.reload_source(0);
        finish_loading(&mut w);
        let index = w.tables.tabs[0].data.find("Double").unwrap();
        assert_eq!(
            w.tables.tabs[0].data.column(index).floats().unwrap(),
            &[6., 10.]
        );
        assert_eq!(w.source_formulas(0).len(), 1);
    }

    #[test]
    fn failed_replacement_keeps_existing_data() {
        let fixture = Fixture::new();
        let path = fixture.file("data.csv", "Time [s],Value\n0,2\n1,4\n");
        let mut w = Workspace::new(false);
        w.load(vec![path], false, None);
        finish_loading(&mut w);
        let identity = w.tables.tabs[0].identity();
        w.load(vec![fixture.0.join("missing.csv")], false, None);
        finish_loading(&mut w);
        assert_eq!(w.tables.tabs[0].identity(), identity);
        assert!(!w.problems.is_empty());
    }

    #[test]
    fn background_reader_honors_dayfirst_for_auto_and_explicit_csv() {
        let fixture = Fixture::new();
        let path = fixture.file(
            "dates.csv",
            "Timestamp,Value\n01/02/2024 12:00:00,2\n02/02/2024 12:00:00,4\n",
        );
        let expected = dv_core::table::parse_datetime_str("01/02/2024 12:00:00", true).unwrap();
        for format in [None, Some("CSV file".to_owned())] {
            let mut w = Workspace::new(true);
            w.load(vec![path.clone()], false, format);
            finish_loading(&mut w);
            let index = w.tables.tabs[0].data.find("Timestamp").unwrap();
            match &w.tables.tabs[0].data.column(index).data {
                dv_core::ColumnData::DateTime(values) => assert_eq!(values[0], expected),
                _ => panic!("Expected a datetime column"),
            }
        }
    }
}
