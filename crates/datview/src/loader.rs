use dv_core::{Formula, Table, TableList};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    pub modified: Option<SystemTime>,
    pub len: u64,
}
pub(crate) fn stamp(path: &std::path::Path) -> Option<FileStamp> {
    std::fs::metadata(path).ok().map(|metadata| FileStamp {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

#[derive(Clone)]
pub(crate) struct FormulaSnapshot {
    pub frame: usize,
    pub frame_name: String,
    pub formulas: Vec<Formula>,
}
#[derive(Clone)]
pub(crate) struct FileRequest {
    pub path: PathBuf,
    pub format: Option<String>,
    pub formulas: Vec<FormulaSnapshot>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadKind {
    Replace,
    Add,
    Reload,
    Replay,
}
pub(crate) struct Job {
    pub epoch: u64,
    pub kind: LoadKind,
    pub dayfirst: bool,
    pub files: Vec<FileRequest>,
}
pub(crate) struct LoadedFile {
    pub path: PathBuf,
    pub format: Option<String>,
    pub tables: Vec<Table>,
    pub messages: Vec<String>,
    pub stamp: Option<FileStamp>,
    pub snapshots: Vec<FormulaSnapshot>,
}
pub(crate) struct Loaded {
    pub epoch: u64,
    pub kind: LoadKind,
    pub files: Vec<LoadedFile>,
}

pub(crate) fn apply_formulas(table: &mut Table, formulas: &[Formula]) -> Vec<String> {
    let mut errors = Vec::new();
    for formula in formulas {
        if let Err(error) = table.add_column_by_formula(
            &formula.name,
            &formula.formula,
            Some(formula.pos.saturating_sub(1)),
        ) {
            errors.push(format!(
                "Formula `{}` failed for {}: {error}",
                formula.name,
                table.name()
            ));
        }
    }
    errors
}

pub(crate) struct Loader {
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<Loaded>,
    epoch: Arc<AtomicU64>,
}
impl Loader {
    pub(crate) fn new() -> Self {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (results_tx, results_rx) = mpsc::channel::<Loaded>();
        let epoch = Arc::new(AtomicU64::new(0));
        let worker_epoch = epoch.clone();
        thread::Builder::new()
            .name("datview-loader".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    if job.epoch != worker_epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    let tables = TableList {
                        dayfirst: job.dayfirst,
                        ..Default::default()
                    };
                    let csv_options = dv_io::generic::csv::CsvOptions {
                        dayfirst: job.dayfirst,
                        ..Default::default()
                    };
                    let reader = |path: &std::path::Path, format: Option<&str>| {
                        dv_io::read_file_with_csv_options(path, format, &csv_options)
                    };
                    let mut files = Vec::with_capacity(job.files.len());
                    for request in job.files {
                        if job.epoch != worker_epoch.load(Ordering::Acquire) {
                            break;
                        }
                        let file_stamp = stamp(&request.path);
                        let perf_start = std::time::Instant::now();
                        let (mut loaded, warning) = tables.load_file_tabs(
                            &request.path,
                            request.format.as_deref(),
                            false,
                            &reader,
                        );
                        if let Ok(path) = std::env::var("DV_PROFILE") {
                            use std::io::Write as _;
                            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                                let row = format!("load\t{:.3}\t{}\n", perf_start.elapsed().as_secs_f64() * 1000.0, request.path.display());
                                let _ = file.write_all(row.as_bytes());
                            }
                        }
                        let mut messages = Vec::new();
                        if !warning.trim().is_empty() {
                            messages.push(warning.trim().to_owned());
                        }
                        let mut used = vec![false; request.formulas.len()];
                        for (frame, table) in loaded.iter_mut().enumerate() {
                            let candidate = request
                                .formulas
                                .iter()
                                .enumerate()
                                .find(|(i, old)| {
                                    !used[*i]
                                        && old.frame_name == table.source_frame_name()
                                        && !old.frame_name.is_empty()
                                })
                                .or_else(|| {
                                    request
                                        .formulas
                                        .iter()
                                        .enumerate()
                                        .find(|(i, old)| !used[*i] && old.frame == frame)
                                });
                            if let Some((index, old)) = candidate {
                                used[index] = true;
                                messages.extend(apply_formulas(table, &old.formulas));
                            }
                        }
                        files.push(LoadedFile {
                            path: request.path,
                            format: request.format,
                            tables: loaded,
                            messages,
                            stamp: file_stamp,
                            snapshots: request.formulas,
                        });
                    }
                    if job.epoch == worker_epoch.load(Ordering::Acquire)
                        && results_tx
                            .send(Loaded {
                                epoch: job.epoch,
                                kind: job.kind,
                                files,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("failed to start file loader thread");
        Self {
            sender: jobs_tx,
            receiver: results_rx,
            epoch,
        }
    }
    pub(crate) fn set_epoch(&self, epoch: u64) {
        self.epoch.store(epoch, Ordering::Release);
    }
    pub(crate) fn submit(&self, job: Job) -> Result<(), String> {
        self.sender
            .send(job)
            .map_err(|_| "The background file loader stopped".to_owned())
    }
    pub(crate) fn try_recv(&self) -> Option<Loaded> {
        self.receiver.try_recv().ok()
    }
}
