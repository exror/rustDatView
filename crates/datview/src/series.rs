use crate::{
    panes,
    workspace::{Mode, Source, Workspace},
};
use dv_core::{
    common::clean_col,
    units::{parse_unit, split_name_unit},
    ColumnData, Table,
};
use dv_plot::{CompareMode, PlotData, PlotType};
use std::{borrow::Cow, collections::HashMap, sync::Arc};

#[derive(Clone, Debug)]
pub(crate) struct SeriesData {
    pub id: u64,
    pub source: usize,
    pub key: String,
    pub label: String,
    pub channel_label: String,
    pub unit: String,
    pub family: String,
    pub x_label: String,
    pub x: Arc<[f64]>,
    pub y: Arc<[f64]>,
    pub generation: u64,
    pub datetime: bool,
    pub color_slot: usize,
    pub dash: usize,
    pub pane: String,
}

// A fixed hash rather than RandomState keeps chart visibility and view IDs
// stable across reloads and sessions.
pub(crate) fn series_id(source: &Source, key: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in source.stable_key().bytes().chain([0xff]).chain(key.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    hash
}
fn numeric(column: &ColumnData) -> Option<&[f64]> {
    match column {
        ColumnData::Float(v) | ColumnData::DateTime(v) => Some(v),
        ColumnData::Str(_) => None,
    }
}
fn column_index(table: &Table, key: &str) -> Option<usize> {
    table
        .data
        .columns()
        .iter()
        .position(|c| clean_col(&c.name) == key && (c.is_float() || c.is_datetime()))
}

fn raw_xy(
    table: &Table,
    source: &Source,
    ix: usize,
    iy: usize,
    pipeline: &dv_plot::Pipeline,
) -> Result<(Vec<f64>, Vec<f64>, bool), String> {
    let xc = table.data.column(ix);
    let yc = table.data.column(iy);
    let x = numeric(&xc.data).ok_or_else(|| "X column is not numeric".to_owned())?;
    let y = numeric(&yc.data).ok_or_else(|| "Y column is not numeric".to_owned())?;
    // Align to the original source start, before masks and plot processing.
    // This also makes the plotted coordinate reversible for the table cursor.
    let start = if source.align_start {
        x.iter().copied().find(|v| v.is_finite()).unwrap_or(0.0)
    } else {
        0.0
    };
    let shift = source.offset - start;
    let (x, y): (Cow<'_, [f64]>, Cow<'_, [f64]>) = if let Some(mask) = &table.mask {
        (
            Cow::Owned(
                x.iter()
                    .zip(mask)
                    .filter_map(|(&value, &keep)| keep.then_some(value + shift))
                    .collect(),
            ),
            Cow::Owned(
                y.iter()
                    .zip(mask)
                    .filter_map(|(&value, &keep)| keep.then_some(value))
                    .collect(),
            ),
        )
    } else if shift != 0.0 {
        (
            Cow::Owned(x.iter().map(|value| value + shift).collect()),
            Cow::Borrowed(y),
        )
    } else {
        (Cow::Borrowed(x), Cow::Borrowed(y))
    };
    let (x, y) = if pipeline.has_plot_actions() {
        pipeline.apply_plot(&x, &y).map_err(|e| e.to_string())?
    } else {
        (x.into_owned(), y.into_owned())
    };
    Ok((x, y, xc.is_datetime() && !source.align_start))
}

// All unprocessed signals from the same table share their transformed X axis.
// The mask and source alignment are common to the table, whereas plot actions
// can depend on Y and must be evaluated separately.
fn signal_xy(
    table: &Table,
    source: &Source,
    ix: usize,
    iy: usize,
    shared_x: &mut HashMap<usize, Arc<[f64]>>,
    source_index: usize,
) -> Result<(Arc<[f64]>, Arc<[f64]>, bool), String> {
    let xc = table.data.column(ix);
    let x = numeric(&xc.data).ok_or_else(|| "X column is not numeric".to_owned())?;
    let y =
        numeric(&table.data.column(iy).data).ok_or_else(|| "Y column is not numeric".to_owned())?;
    let start = if source.align_start {
        x.iter().copied().find(|v| v.is_finite()).unwrap_or(0.0)
    } else {
        0.0
    };
    let shift = source.offset - start;
    let x = Arc::clone(shared_x.entry(source_index).or_insert_with(|| {
        if let Some(mask) = &table.mask {
            x.iter()
                .zip(mask)
                .filter_map(|(&value, &keep)| keep.then_some(value + shift))
                .collect::<Vec<_>>()
                .into()
        } else if shift != 0.0 {
            x.iter()
                .map(|value| value + shift)
                .collect::<Vec<_>>()
                .into()
        } else {
            Arc::from(x)
        }
    }));
    let y = if let Some(mask) = &table.mask {
        y.iter()
            .zip(mask)
            .filter_map(|(&value, &keep)| keep.then_some(value))
            .collect::<Vec<_>>()
            .into()
    } else {
        Arc::from(y)
    };
    Ok((x, y, xc.is_datetime() && !source.align_start))
}

fn build_plot(
    table: &Table,
    source: &Source,
    ix: usize,
    iy: usize,
    pipeline: &dv_plot::Pipeline,
) -> Result<PlotData, String> {
    let (x, y, datetime) = raw_xy(table, source, ix, iy, pipeline)?;
    let x = if datetime {
        ColumnData::DateTime(x)
    } else {
        ColumnData::Float(x)
    };
    let mut plot = PlotData::from_columns(
        x,
        ColumnData::Float(y),
        table.data.column(ix).name.clone(),
        table.data.column(iy).name.clone(),
    )
    .map_err(|e| e.to_string())?;
    plot.st = source.name.clone();
    plot.tabname = source.name.clone();
    plot.filename = source.path.to_string_lossy().into_owned();
    plot.ix = ix;
    plot.iy = iy;
    Ok(plot)
}

fn finish(
    workspace: &Workspace,
    source_index: usize,
    key: &str,
    mut label: String,
    unit: String,
    x_label: String,
    x: Arc<[f64]>,
    mut y: Arc<[f64]>,
    datetime: bool,
    id: u64,
) -> SeriesData {
    let rules: Vec<_> = workspace
        .rules
        .iter()
        .filter(|rule| rule.matches(workspace.mode, key, &label, id))
        .collect();
    if !rules.is_empty() {
        for (x, y) in x.iter().zip(Arc::make_mut(&mut y).iter_mut()) {
            if rules.iter().any(|rule| !rule.accepts(*x, *y)) {
                *y = f64::NAN;
            }
        }
    }
    let channel = workspace.channels.iter().find(|channel| channel.key == key);
    let channel_label = channel
        .map(|channel| channel.label.clone())
        .unwrap_or_else(|| label.clone());
    // Spectra and distributions change the unit (W²/Hz, 1/W) but should stay
    // grouped with the physical quantity they were computed from.
    let family = channel
        .map(|channel| channel.family.clone())
        .unwrap_or_else(|| parse_unit(&unit).family.to_string());
    if workspace.sources.iter().filter(|s| s.active).count() > 1 && workspace.mode != Mode::Compare
    {
        label.push_str(" · ");
        label.push_str(&workspace.sources[source_index].name);
    }
    SeriesData {
        id,
        source: source_index,
        key: key.to_owned(),
        label,
        channel_label,
        family,
        unit,
        x_label,
        x,
        y,
        generation: workspace.generation,
        datetime,
        color_slot: 0,
        dash: 0,
        pane: String::new(),
    }
}
fn full_signature(workspace: &Workspace, pipeline: &str) -> String {
    serde_json::to_string(&(
        workspace.data_revision,
        workspace.mode,
        &workspace.options,
        pipeline,
        &workspace.sources,
        &workspace.selected,
        &workspace.x_key,
        &workspace.rules,
        &workspace.panes,
    ))
    .unwrap_or_default()
}

impl Workspace {
    /// Rebuild only when input metadata/options change, and reuse unchanged
    /// vectors when selection, pane layout or source visibility changes.
    pub(crate) fn rebuild(&mut self) {
        self.ensure_pipeline();
        let pipeline = self.pipeline.to_json().unwrap_or_default();
        let signature = full_signature(self, &pipeline);
        if self.series_signature.as_deref() == Some(&signature) {
            return;
        }
        let perf_start = std::time::Instant::now();
        self.generation = self.generation.wrapping_add(1);
        let mut old: HashMap<_, _> = std::mem::take(&mut self.series)
            .into_iter()
            .map(|series| (series.id, series))
            .collect();
        let mut item_signatures = HashMap::new();
        let mut series = Vec::new();
        let mut shared_x = HashMap::new();
        let active_count = self.sources.iter().filter(|source| source.active).count();
        let base = serde_json::to_string(&(
            self.data_revision,
            self.mode,
            &self.options,
            &pipeline,
            &self.x_key,
            &self.rules,
            active_count,
        ))
        .unwrap_or_default();
        let plot_type = match self.mode {
            Mode::Signals | Mode::Compare => PlotType::Regular,
            Mode::Distribution => PlotType::Pdf(self.options.pdf.clone()),
            Mode::Spectrum => PlotType::Fft(self.options.fft.clone()),
            Mode::Normalized => PlotType::MinMax(self.options.normalized.clone()),
            Mode::Polar => PlotType::Polar(self.options.polar.clone()),
        };
        let mut comparison = Vec::new();
        let compare_mode = self.options.compare_mode.unwrap_or_else(|| {
            if active_count <= 1 {
                CompareMode::OneTableColumns
            } else if self.selected.len() <= 1 {
                CompareMode::TablesOneColumn
            } else {
                CompareMode::TablesSimilarColumns
            }
        });
        let first_active = self.sources.iter().position(|source| source.active);
        if let Some(x_key) = self.x_key.as_deref() {
            for key in &self.selected {
                if self.mode == Mode::Compare
                    && compare_mode == CompareMode::TablesOneColumn
                    && self.selected.first() != Some(key)
                {
                    continue;
                }
                for (index, (table, source)) in
                    self.tables.tabs.iter().zip(&self.sources).enumerate()
                {
                    if !source.active
                        || (self.mode == Mode::Compare
                            && compare_mode == CompareMode::OneTableColumns
                            && Some(index) != first_active)
                    {
                        continue;
                    }
                    let Some(ix) = column_index(table, x_key) else {
                        continue;
                    };
                    let Some(iy) = column_index(table, key) else {
                        continue;
                    };
                    if ix == iy {
                        continue;
                    }
                    let id = series_id(source, key);
                    if self.mode != Mode::Compare && self.panes.removed.contains_key(&id) {
                        continue;
                    }
                    let item_key =
                        serde_json::to_string(&(&base, source, table.identity(), ix, iy))
                            .unwrap_or_default();
                    if self.mode != Mode::Compare
                        && self.item_signatures.get(&id) == Some(&item_key)
                    {
                        if let Some(mut cached) = old.remove(&id) {
                            cached.source = index;
                            if self.mode == Mode::Signals && !self.pipeline.has_plot_actions() {
                                shared_x
                                    .entry(index)
                                    .or_insert_with(|| Arc::clone(&cached.x));
                            }
                            item_signatures.insert(id, item_key);
                            series.push(cached);
                            continue;
                        }
                    }
                    let result = if self.mode == Mode::Compare {
                        build_plot(table, source, ix, iy, &self.pipeline).map(|mut plot| {
                            plot.it = index;
                            comparison.push(plot);
                            None
                        })
                    } else if self.mode == Mode::Signals {
                        let values = if self.pipeline.has_plot_actions() {
                            raw_xy(table, source, ix, iy, &self.pipeline)
                                .map(|(x, y, datetime)| (x.into(), y.into(), datetime))
                        } else {
                            signal_xy(table, source, ix, iy, &mut shared_x, index)
                        };
                        values.map(|(x, y, datetime)| {
                            let (label, unit) = split_name_unit(&table.data.column(iy).name);
                            Some(finish(
                                self,
                                index,
                                key,
                                label,
                                unit,
                                table.data.column(ix).name.clone(),
                                x,
                                y,
                                datetime,
                                id,
                            ))
                        })
                    } else {
                        build_plot(table, source, ix, iy, &self.pipeline).and_then(|mut plot| {
                            plot.transform(&plot_type).map_err(|e| e.to_string())?;
                            let datetime = matches!(plot.x, ColumnData::DateTime(_));
                            let x = match plot.x {
                                ColumnData::Float(v) | ColumnData::DateTime(v) => v,
                                ColumnData::Str(_) => {
                                    return Err("This mode produced a text X axis".into())
                                }
                            };
                            let y = match plot.y {
                                ColumnData::Float(v) | ColumnData::DateTime(v) => v,
                                ColumnData::Str(_) => {
                                    return Err("This mode produced a text Y axis".into())
                                }
                            };
                            let (label, _) = split_name_unit(&table.data.column(iy).name);
                            let (_, mut unit) = split_name_unit(&plot.sy);
                            if self.mode == Mode::Normalized
                                && self.options.normalized.y_scale
                                && self.options.normalized.y_center == "None"
                            {
                                unit.clear();
                            }
                            Ok(Some(finish(
                                self,
                                index,
                                key,
                                label,
                                unit,
                                plot.sx,
                                x.into(),
                                y.into(),
                                datetime,
                                id,
                            )))
                        })
                    };
                    match result {
                        Ok(Some(data)) => {
                            item_signatures.insert(id, item_key);
                            series.push(data);
                        }
                        Ok(None) => {}
                        Err(error) => self.problems.push(format!(
                            "{} · {}: {error}",
                            source.name,
                            table.data.column(iy).name
                        )),
                    }
                }
            }
        }
        if self.mode == Mode::Compare && !comparison.is_empty() {
            match dv_plot::compare(&comparison, compare_mode, self.options.compare_method) {
                Ok(compared) => {
                    for plot in compared {
                        let Some(source) = self.sources.get(plot.it) else {
                            continue;
                        };
                        let Some(table) = self.tables.tabs.get(plot.it) else {
                            continue;
                        };
                        let Some(column) = table.data.columns().get(plot.iy) else {
                            continue;
                        };
                        let key = clean_col(&column.name);
                        let id = series_id(source, &key);
                        let datetime = matches!(plot.x, ColumnData::DateTime(_));
                        let x = match plot.x {
                            ColumnData::Float(v) | ColumnData::DateTime(v) => v,
                            ColumnData::Str(_) => continue,
                        };
                        let y = match plot.y {
                            ColumnData::Float(v) | ColumnData::DateTime(v) => v,
                            ColumnData::Str(_) => continue,
                        };
                        let (_, unit) = split_name_unit(&plot.sy);
                        let label = if plot.syl.is_empty() {
                            split_name_unit(&column.name).0
                        } else {
                            plot.syl
                        };
                        series.push(finish(
                            self,
                            plot.it,
                            &key,
                            label,
                            unit,
                            plot.sx,
                            x.into(),
                            y.into(),
                            datetime,
                            id,
                        ));
                    }
                }
                Err(error) => self.problems.push(format!("Comparison: {error}")),
            }
        }
        panes::arrange(&mut series, &self.sources, &mut self.panes);
        self.series = series;
        self.item_signatures = item_signatures;
        self.series_signature = Some(full_signature(self, &pipeline));
        if let Ok(path) = std::env::var("DV_PROFILE") {
            use std::io::Write as _;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let row = format!("series\t{:.3}\t{}\n", perf_start.elapsed().as_secs_f64() * 1000.0, self.series.len());
                let _ = file.write_all(row.as_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::{FilterMode, RangeRule};
    use dv_core::DataFrame;
    fn sample() -> Workspace {
        let mut w = Workspace::new(false);
        w.tables.tabs.push(Table::from_df(
            DataFrame::from_floats(
                ["Time [s]", "Power [W]"],
                vec![vec![10., 11., 12.], vec![4., 8., 12.]],
            )
            .unwrap(),
            "run",
        ));
        w.sources.push(Source {
            path: "run.csv".into(),
            name: "run".into(),
            align_start: true,
            offset: 2.,
            ..Default::default()
        });
        w.x_key = Some("time".into());
        w.selected.push("power".into());
        w.data_changed();
        w
    }
    #[test]
    fn caches_alignment_and_nondestructive_display_gaps() {
        let mut w = sample();
        w.rules.push(RangeRule {
            enabled: true,
            mode: Mode::Signals,
            target: "power".into(),
            bounds: [Some(3.), None, None, None],
            action: FilterMode::Keep,
        });
        w.rebuild();
        assert_eq!(w.series[0].x.as_ref(), &[2., 3., 4.]);
        assert!(w.series[0].y[0].is_nan());
        assert_eq!(&w.series[0].y[1..], &[8., 12.]);
        let address = w.series[0].x.as_ptr();
        let generation = w.series[0].generation;
        w.rebuild();
        assert_eq!(w.series[0].x.as_ptr(), address);
        assert_eq!(w.series[0].generation, generation);
        assert_eq!(
            w.tables.tabs[0].data.column(2).floats().unwrap(),
            &[4., 8., 12.]
        );
    }
    #[test]
    fn signals_share_aligned_masked_x_and_reuse_it_after_selection_changes() {
        let mut w = sample();
        w.tables.tabs[0]
            .data
            .push(dv_core::Column::float("Current [A]", vec![1., 2., 3.]))
            .unwrap();
        w.tables.tabs[0].mask = Some(vec![true, false, true]);
        w.data_changed();
        w.rebuild();
        let first_x = Arc::clone(&w.series[0].x);
        assert_eq!(first_x.as_ref(), &[2., 4.]);
        w.selected.push("current".into());
        w.rebuild();
        assert_eq!(w.series.len(), 2);
        assert!(Arc::ptr_eq(&first_x, &w.series[0].x));
        assert!(Arc::ptr_eq(&w.series[0].x, &w.series[1].x));
        assert_eq!(w.series[1].y.as_ref(), &[1., 3.]);
        w.sources[0].offset = 5.0;
        w.rebuild();
        assert_eq!(w.series[0].x.as_ref(), &[5., 7.]);
        assert!(Arc::ptr_eq(&w.series[0].x, &w.series[1].x));
        assert!(!Arc::ptr_eq(&first_x, &w.series[0].x));
    }
    #[test]
    fn source_identity_survives_rename() {
        let mut s = Source {
            path: "run.csv".into(),
            frame: 2,
            frame_name: "sheet".into(),
            name: "a".into(),
            ..Default::default()
        };
        let id = series_id(&s, "power");
        s.name = "renamed".into();
        assert_eq!(id, series_id(&s, "power"));
        s.frame = 3;
        assert_ne!(id, series_id(&s, "power"));
    }
}
