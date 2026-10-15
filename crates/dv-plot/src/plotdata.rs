//! Plot-data transformations and statistics, following `pydatview/plotdata.py`.
use crate::Pipeline;
use dv_core::common::{get_dt, inverse_unit, no_unit, splitunit, unit};
use dv_core::{ColumnData, DvError, Result, Table};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfOptions {
    pub bins: usize,
    pub smooth: bool,
}
impl Default for PdfOptions {
    fn default() -> Self {
        Self {
            bins: 30,
            smooth: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinMaxOptions {
    pub x_scale: bool,
    pub y_scale: bool,
    pub y_center: String,
    pub y_ref: f64,
}
impl Default for MinMaxOptions {
    fn default() -> Self {
        Self {
            x_scale: false,
            y_scale: true,
            y_center: "None".into(),
            y_ref: 0.0,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FftOptions {
    pub y_type: String,
    pub x_type: String,
    pub averaging: String,
    pub window: String,
    pub detrend: bool,
    pub n_exp: u32,
    pub points_per_decade: usize,
}
impl Default for FftOptions {
    fn default() -> Self {
        Self {
            y_type: "Amplitude".into(),
            x_type: "1/x".into(),
            averaging: "Welch".into(),
            window: "Hamming".into(),
            detrend: true,
            n_exp: 8,
            points_per_decade: 10,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolarOptions {
    pub degrees: bool,
    pub bins: Option<usize>,
    pub about: String,
    pub radius_ref: Option<f64>,
}
impl Default for PolarOptions {
    fn default() -> Self {
        Self {
            degrees: true,
            bins: None,
            about: "z".into(),
            radius_ref: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum PlotType {
    #[default]
    Regular,
    Pdf(PdfOptions),
    MinMax(MinMaxOptions),
    Fft(FftOptions),
    Polar(PolarOptions),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareMode {
    TablesOneColumn,
    OneTableColumns,
    TablesSameColumns,
    TablesSimilarColumns,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareMethod {
    Relative,
    AbsRelative,
    Ratio,
    Absolute,
    YY,
}

/// Numeric statistics are NaN when they are not defined (e.g. a string axis).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotStats {
    pub n: usize,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub x_at_y_min: f64,
    pub x_at_y_max: f64,
    pub mean: f64,
    pub median: f64,
    pub std: f64,
    pub variance: f64,
    pub turbulence_intensity: f64,
    pub abs_max: f64,
    pub x_range: f64,
    pub y_range: f64,
    pub dx: f64,
    pub integral: f64,
    pub integral_average: f64,
    pub integral_x: f64,
    pub integral_x_normalized: f64,
    pub integral_x2: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    pub index: usize,
    pub x: f64,
    pub y: f64,
}
#[derive(Debug, Clone)]
pub struct PlotData {
    pub x: ColumnData,
    pub y: ColumnData,
    pub sx: String,
    pub sy: String,
    pub st: String,
    pub syl: String,
    pub filename: String,
    pub tabname: String,
    pub it: usize,
    pub ix: usize,
    pub iy: usize,
    pub spectral_info: Option<dv_tools::spectral::SpectralInfo>,
    original_x: ColumnData,
    original_y: ColumnData,
    original_sx: String,
    original_sy: String,
    current_stats: PlotStats,
    original_stats: PlotStats,
}

fn numeric(data: &ColumnData) -> Result<&[f64]> {
    match data {
        ColumnData::Float(v) | ColumnData::DateTime(v) => Ok(v),
        ColumnData::Str(_) => Err(DvError::msg(
            "Cannot use a string axis for a numeric transformation",
        )),
    }
}
fn floating(data: &ColumnData) -> Result<&[f64]> {
    match data {
        ColumnData::Float(v) => Ok(v),
        _ => Err(DvError::msg(
            "Cannot transform dates or strings as numeric signal values",
        )),
    }
}
fn numeric_mut(data: &mut ColumnData) -> Result<&mut Vec<f64>> {
    match data {
        ColumnData::Float(v) | ColumnData::DateTime(v) => Ok(v),
        _ => Err(DvError::msg("Cannot scale strings")),
    }
}
fn nan_mean(y: &[f64]) -> f64 {
    let (sum, n) = y
        .iter()
        .filter(|v| !v.is_nan())
        .fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 {
        f64::NAN
    } else {
        sum / n as f64
    }
}
fn extrema(y: &[f64]) -> (f64, f64, usize, usize) {
    let mut min = f64::NAN;
    let mut max = f64::NAN;
    let mut imin = 0;
    let mut imax = 0;
    for (i, &v) in y.iter().enumerate() {
        if !v.is_nan() {
            if min.is_nan() || v < min {
                min = v;
                imin = i;
            }
            if max.is_nan() || v > max {
                max = v;
                imax = i;
            }
        }
    }
    (min, max, imin, imax)
}
fn stats(x: &ColumnData, y: &ColumnData) -> PlotStats {
    let empty: &[f64] = &[];
    let xv = numeric(x).unwrap_or(empty);
    let yv = numeric(y).unwrap_or(empty);
    let (mut xmin, mut xmax, _, _) = extrema(xv);
    let (mut ymin, mut ymax, imin, imax) = extrema(yv);
    if matches!(x, ColumnData::DateTime(_)) {
        xmin = xv.first().copied().unwrap_or(f64::NAN);
        xmax = xv.last().copied().unwrap_or(f64::NAN);
    }
    if matches!(y, ColumnData::DateTime(_)) {
        ymin = yv.first().copied().unwrap_or(f64::NAN);
        ymax = yv.last().copied().unwrap_or(f64::NAN);
    }
    let yn = if matches!(y, ColumnData::Float(_)) {
        yv
    } else {
        empty
    };
    let mean = nan_mean(yn);
    let mut ordered: Vec<f64> = yn.iter().copied().filter(|v| !v.is_nan()).collect();
    ordered.sort_by(f64::total_cmp);
    let n = ordered.len();
    let median = if n == 0 {
        f64::NAN
    } else if n % 2 == 0 {
        (ordered[n / 2 - 1] + ordered[n / 2]) / 2.0
    } else {
        ordered[n / 2]
    };
    let variance = if n == 0 {
        f64::NAN
    } else {
        yn.iter()
            .filter(|v| !v.is_nan())
            .map(|v| (v - mean).powi(2))
            .sum::<f64>()
            / n as f64
    };
    let mut integral = f64::NAN;
    let mut integral_x = f64::NAN;
    let mut integral_x2 = f64::NAN;
    if matches!(x, ColumnData::Float(_)) && matches!(y, ColumnData::Float(_)) {
        integral = 0.0;
        integral_x = 0.0;
        integral_x2 = 0.0;
        for (xx, yy) in xv.windows(2).zip(yv.windows(2)) {
            let h = (xx[1] - xx[0]) * 0.5;
            integral += h * (yy[0] + yy[1]);
            integral_x += h * (yy[0] * xx[0] + yy[1] * xx[1]);
            integral_x2 += h * (yy[0] * xx[0] * xx[0] + yy[1] * xx[1] * xx[1]);
        }
    }
    let x_at_y_min = if matches!(x, ColumnData::DateTime(_)) {
        xv.first().copied()
    } else {
        xv.get(imin).copied()
    }
    .unwrap_or(f64::NAN);
    let x_at_y_max = if matches!(x, ColumnData::DateTime(_)) {
        xv.last().copied()
    } else {
        xv.get(imax).copied()
    }
    .unwrap_or(f64::NAN);
    let span = xv.last().copied().unwrap_or(f64::NAN) - xv.first().copied().unwrap_or(f64::NAN);
    PlotStats {
        n: y.len(),
        x_min: xmin,
        x_max: xmax,
        y_min: ymin,
        y_max: ymax,
        x_at_y_min,
        x_at_y_max,
        mean,
        median,
        std: variance.sqrt(),
        variance,
        turbulence_intensity: if mean == 0.0 {
            f64::NAN
        } else {
            variance.sqrt() / mean
        },
        abs_max: ymin.abs().max(ymax.abs()),
        x_range: xmax - xmin,
        y_range: ymax - ymin,
        dx: get_dt(xv),
        integral,
        integral_average: integral / span,
        integral_x,
        integral_x_normalized: integral_x / integral,
        integral_x2,
    }
}

impl PlotData {
    pub fn new(
        x: Vec<f64>,
        y: Vec<f64>,
        sx: impl Into<String>,
        sy: impl Into<String>,
    ) -> Result<Self> {
        Self::from_columns(
            ColumnData::Float(x),
            ColumnData::Float(y),
            sx.into(),
            sy.into(),
        )
    }
    pub fn from_columns(x: ColumnData, y: ColumnData, sx: String, sy: String) -> Result<Self> {
        if x.len() != y.len() {
            return Err(DvError::msg("x and y have different lengths"));
        }
        if y.len() > 1000 && (matches!(x, ColumnData::Str(_)) || matches!(y, ColumnData::Str(_))) {
            return Err(DvError::msg(
                "More than 1000 string values are unsuitable for plotting",
            ));
        }
        let s = stats(&x, &y);
        let sx = sx.replace('_', " ");
        let sy = sy.replace('_', " ");
        Ok(Self {
            original_x: x.clone(),
            original_y: y.clone(),
            original_sx: sx.clone(),
            original_sy: sy.clone(),
            x,
            y,
            sx,
            sy,
            st: String::new(),
            syl: String::new(),
            filename: String::new(),
            tabname: String::new(),
            it: 0,
            ix: 0,
            iy: 0,
            spectral_info: None,
            current_stats: s,
            original_stats: s,
        })
    }
    pub fn from_table(
        table: &Table,
        ix: usize,
        iy: usize,
        pipeline: Option<&Pipeline>,
    ) -> Result<Self> {
        if ix >= table.ncols() || iy >= table.ncols() {
            return Err(DvError::msg("Column index out of bounds"));
        }
        let mut x = table.get_column(ix);
        let mut y = table.get_column(iy);
        if let Some(p) = pipeline {
            if p.has_plot_actions() {
                let (xx, yy) = p.apply_plot(numeric(&x.data)?, floating(&y.data)?)?;
                x.data = if matches!(x.data, ColumnData::DateTime(_)) {
                    ColumnData::DateTime(xx)
                } else {
                    ColumnData::Float(xx)
                };
                y.data = ColumnData::Float(yy);
            }
        }
        let mut pd = Self::from_columns(x.data, y.data, x.name, y.name)?;
        pd.st = table.active_name.clone();
        pd.tabname = table.active_name.clone();
        pd.filename = table.filename.clone();
        pd.ix = ix;
        pd.iy = iy;
        Ok(pd)
    }
    pub fn numeric_xy(&self) -> Result<(&[f64], &[f64])> {
        Ok((numeric(&self.x)?, numeric(&self.y)?))
    }
    pub fn stats(&self) -> PlotStats {
        self.current_stats
    }
    pub fn original_stats(&self) -> PlotStats {
        self.original_stats
    }
    pub fn recompute_stats(&mut self) {
        self.current_stats = stats(&self.x, &self.y);
    }
    pub fn reset(&mut self) {
        self.x = self.original_x.clone();
        self.y = self.original_y.clone();
        self.sx = self.original_sx.clone();
        self.sy = self.original_sy.clone();
        self.current_stats = self.original_stats;
        self.spectral_info = None;
    }
    /// Apply to the current values. Call `reset` to switch modes without chaining.
    pub fn transform(&mut self, kind: &PlotType) -> Result<()> {
        match kind {
            PlotType::Regular => {}
            PlotType::Pdf(o) => {
                self.to_pdf(o)?;
            }
            PlotType::MinMax(o) => self.to_min_max(o)?,
            PlotType::Fft(o) => self.to_fft(o)?,
            PlotType::Polar(o) => self.to_polar(o)?,
        }
        Ok(())
    }
    pub fn to_pdf(&mut self, o: &PdfOptions) -> Result<usize> {
        let bins = o.bins.min(self.y.len());
        if bins == 0 {
            return Err(DvError::msg("PDF requires data and at least one bin"));
        }
        let (x, y) = match &self.y {
            ColumnData::Str(v) => {
                if v.len() > 100 {
                    return Err(DvError::msg("String PDF dataset is too large to display"));
                }
                let mut counts = BTreeMap::<&str, usize>::new();
                for s in v {
                    *counts.entry(s).or_default() += 1;
                }
                let mut labels = Vec::with_capacity(counts.len());
                let mut values = Vec::with_capacity(counts.len());
                for (s, n) in counts {
                    labels.push(s.to_string());
                    values.push(n as f64 / v.len() as f64);
                }
                (ColumnData::Str(labels), values)
            }
            ColumnData::DateTime(_) => return Err(DvError::msg("Cannot plot PDF of dates")),
            ColumnData::Float(v) => {
                let (x, y) = if o.smooth {
                    let opts = dv_tools::stats::KdeOptions {
                        points: bins,
                        ..Default::default()
                    };
                    // Like scipy's singular covariance fallback, a constant signal uses a histogram.
                    if extrema(v).0 == extrema(v).1 {
                        dv_tools::stats::pdf_histogram(v, bins, true, false)?
                    } else {
                        dv_tools::stats::pdf_gaussian_kde(v, &opts)?
                    }
                } else {
                    dv_tools::stats::pdf_histogram(v, bins, true, false)?
                };
                (ColumnData::Float(x), y)
            }
        };
        let old = self.sy.clone();
        let inv = inverse_unit(&old);
        self.sx = old.clone();
        self.sy = format!(
            "PDF({}){}",
            no_unit(&old),
            if inv.is_empty() {
                String::new()
            } else {
                format!(" [{inv}]")
            }
        );
        self.x = x;
        self.y = ColumnData::Float(y);
        self.recompute_stats();
        Ok(bins)
    }
    pub fn to_min_max(&mut self, o: &MinMaxOptions) -> Result<()> {
        let s = self.original_stats;
        match o.y_center.as_str() {
            "None" => {
                if o.y_scale {
                    let v = numeric_mut(&mut self.y)?;
                    for y in v {
                        *y = if s.y_max == s.y_min {
                            0.0
                        } else {
                            (*y - s.y_min) / (s.y_max - s.y_min)
                        };
                    }
                }
            }
            "Mean=0" | "Mean=ref" | "Mid=0" | "Mid=ref" => {
                floating(&self.y)?;
                let reference = if o.y_center.ends_with("=0") {
                    0.0
                } else {
                    o.y_ref
                };
                let center = if o.y_center.starts_with("Mean") {
                    s.mean
                } else {
                    (s.y_max + s.y_min) / 2.0
                };
                for y in numeric_mut(&mut self.y)? {
                    *y += reference - center;
                }
            }
            _ => return Err(DvError::msg(format!("Unsupported y center {}", o.y_center))),
        }
        if o.x_scale {
            for x in numeric_mut(&mut self.x)? {
                *x = if s.x_max == s.x_min {
                    0.0
                } else {
                    (*x - s.x_min) / (s.x_max - s.x_min)
                };
            }
        }
        if o.y_center == "None" && o.y_scale {
            if let ColumnData::DateTime(values) = &mut self.y {
                self.y = ColumnData::Float(std::mem::take(values));
            }
        }
        if o.x_scale {
            if let ColumnData::DateTime(values) = &mut self.x {
                self.x = ColumnData::Float(std::mem::take(values));
            }
        }
        self.recompute_stats();
        Ok(())
    }
    pub fn to_fft(&mut self, o: &FftOptions) -> Result<()> {
        use dv_tools::spectral::{
            Averaging, Detrend, FftOptions as SpectralOptions, OutputType, Window,
        };
        let output_type = match o.y_type.as_str() {
            "Amplitude" => OutputType::Amplitude,
            "PSD" => OutputType::Psd,
            "f x PSD" => OutputType::FrequencyPsd,
            _ => return Err(DvError::msg("Unsupported FFT output type")),
        };
        let averaging = match o.averaging.as_str() {
            "None" => Averaging::None,
            "Welch" => Averaging::Welch,
            "Binning" => Averaging::Binning,
            _ => return Err(DvError::msg("Unsupported FFT averaging")),
        };
        let window = match o.window.as_str() {
            "Hamming" => Window::Hamming,
            "Hann" => Window::Hann,
            "Rectangular" => Window::Rectangular,
            _ => return Err(DvError::msg("Unsupported FFT window")),
        };
        if !matches!(o.x_type.as_str(), "1/x" | "x" | "2pi/x") {
            return Err(DvError::msg("Unsupported FFT x type"));
        }
        let (x, y) = (numeric(&self.x)?, floating(&self.y)?);
        let opts = SpectralOptions {
            dt: if matches!(self.x, ColumnData::DateTime(_)) {
                Some(get_dt(x))
            } else {
                None
            },
            output_type,
            averaging,
            window,
            detrend: if o.detrend {
                Detrend::Constant
            } else {
                Detrend::None
            },
            n_exp: Some(o.n_exp),
            points_per_decade: o.points_per_decade,
        };
        let mut spectrum = dv_tools::spectral::fft_wrap(x, y, &opts)?;
        let (yn, yu) = splitunit(&self.sy);
        let xu = unit(&self.sx);
        self.sy = match o.y_type.as_str() {
            "PSD" => format!("PSD({yn}) [({yu})^2/{xu}]"),
            "f x PSD" => format!("f-weighted PSD({yn}) [({yu})^2]"),
            _ => format!("FFT({yn}) [{yu}]"),
        };
        self.sx = if xu == "s" {
            match o.x_type.as_str() {
                "1/x" => "Frequency [Hz]",
                "x" => "Period [s]",
                _ => "Cyclic frequency [rad/s]",
            }
            .into()
        } else {
            String::new()
        };
        if o.x_type == "x" {
            for f in &mut spectrum.frequency {
                *f = 1.0 / *f;
            }
        } else if o.x_type == "2pi/x" {
            for f in &mut spectrum.frequency {
                *f *= std::f64::consts::TAU;
            }
        }
        self.x = ColumnData::Float(spectrum.frequency);
        self.y = ColumnData::Float(spectrum.values);
        self.spectral_info = Some(spectrum.info);
        self.recompute_stats();
        Ok(())
    }
    pub fn to_polar(&mut self, o: &PolarOptions) -> Result<()> {
        if !o.about.starts_with('x') && !o.about.starts_with('z') {
            return Err(DvError::msg("Polar axis must be x or z"));
        }
        let mut theta = floating(&self.x)?.to_vec();
        let mut radius = floating(&self.y)?.to_vec();
        if o.degrees {
            for t in &mut theta {
                *t = t.to_radians();
            }
        }
        if let Some(r) = o.radius_ref {
            let mean = radius.iter().sum::<f64>() / radius.len() as f64;
            for v in &mut radius {
                *v += r - mean;
            }
        }
        if let Some(n) = o.bins {
            if n < 2 {
                return Err(DvError::msg("Polar binning requires at least two edges"));
            }
            let edges: Vec<f64> = (0..n)
                .map(|i| std::f64::consts::TAU * i as f64 / (n - 1) as f64)
                .collect();
            (theta, radius) = dv_tools::stats::bin_signal(
                &theta,
                &radius,
                &edges,
                &dv_tools::stats::BinStatistic::Mean,
            )?;
            if let (Some(&t), Some(&r)) = (theta.first(), radius.first()) {
                theta.push(t);
                radius.push(r);
            }
        }
        let (name, unt) = splitunit(&self.sy);
        let mut x = Vec::with_capacity(theta.len());
        let mut y = Vec::with_capacity(theta.len());
        for (t, r) in theta.into_iter().zip(radius) {
            if o.about.starts_with('x') {
                x.push(-r * t.sin());
                y.push(r * t.cos());
            } else {
                x.push(r * t.cos());
                y.push(r * t.sin());
            }
        }
        let (a, b) = if o.about.starts_with('x') {
            ("y", "z")
        } else {
            ("x", "y")
        };
        self.sx = format!("{}_{a} [{unt}]", name.trim());
        self.sy = format!("{}_{b} [{unt}]", name.trim());
        self.x = ColumnData::Float(x);
        self.y = ColumnData::Float(y);
        self.recompute_stats();
        Ok(())
    }
    pub fn measure(&self, x: f64, y: f64) -> Result<Measurement> {
        let (xx, yy) = self.numeric_xy()?;
        let index = xx
            .iter()
            .zip(yy)
            .enumerate()
            .filter_map(|(i, (&a, &b))| {
                let d = (a - x).powi(2) + (b - y).powi(2);
                d.is_finite().then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|v| v.0)
            .ok_or_else(|| DvError::msg("No finite measurement point"))?;
        Ok(Measurement {
            index,
            x: xx[index],
            y: yy[index],
        })
    }
    /// Statistics between the two nearest x samples; the right endpoint is excluded like numpy slicing.
    pub fn measurement_stats(&self, a: f64, b: f64) -> Result<PlotStats> {
        let (x, y) = self.numeric_xy()?;
        floating(&self.y)?;
        let nearest = |q: f64| {
            x.iter()
                .enumerate()
                .filter(|(_, v)| v.is_finite())
                .min_by(|a, b| (a.1 - q).abs().total_cmp(&(b.1 - q).abs()))
                .map(|(i, _)| i)
                .ok_or_else(|| DvError::msg("No finite measurement point"))
        };
        let mut i = nearest(a)?;
        let mut j = nearest(b)?;
        if i > j {
            std::mem::swap(&mut i, &mut j);
        }
        if i == j {
            return Err(DvError::msg("Empty measurement interval"));
        }
        Ok(stats(
            &ColumnData::Float(x[i..j].to_vec()),
            &ColumnData::Float(y[i..j].to_vec()),
        ))
    }
    pub fn equivalent_load(&self, m: f64, teq: f64, bins: usize) -> Result<f64> {
        dv_tools::fatigue::equivalent_load(
            numeric(&self.x)?,
            floating(&self.y)?,
            m,
            teq,
            bins,
            "rainflow_windap",
        )
    }
}

pub fn compare(
    data: &[PlotData],
    mode: CompareMode,
    method: CompareMethod,
) -> Result<Vec<PlotData>> {
    if data.len() < 2 {
        return Err(DvError::msg("Comparison requires at least two signals"));
    }
    for p in data {
        floating(&p.y)?;
    }
    let mut groups: Vec<Vec<&PlotData>> = Vec::new();
    for p in data {
        let group = match mode {
            CompareMode::TablesSameColumns => groups.iter().position(|g| g[0].iy == p.iy),
            CompareMode::TablesSimilarColumns => groups.iter().position(|g| {
                dv_core::common::clean_col(&g[0].sy) == dv_core::common::clean_col(&p.sy)
            }),
            _ => Some(0),
        };
        if let Some(i) = group {
            if i < groups.len() {
                groups[i].push(p);
            } else {
                groups.push(vec![p]);
            }
        } else {
            groups.push(vec![p]);
        }
    }
    let mut output = Vec::new();
    for group in groups {
        let reference = group[0];
        let yr = floating(&reference.y)?;
        let near_zero = yr.iter().map(|v| v.abs()).sum::<f64>() / (yr.len() as f64) < 1e-7;
        let compared: &[&PlotData] = if mode == CompareMode::TablesOneColumn {
            &group[1..group.len().min(2)]
        } else {
            &group[1..]
        };
        for p in compared {
            let mut q = (*p).clone();
            let y = if mode == CompareMode::OneTableColumns
                || matches!(reference.x, ColumnData::Str(_))
                || matches!(p.x, ColumnData::Str(_))
            {
                if yr.len() != p.y.len() {
                    return Err(DvError::msg(
                        "Cannot compare different lengths without a numeric x axis",
                    ));
                }
                floating(&p.y)?.to_vec()
            } else {
                dv_tools::signal_analysis::interpolate(
                    numeric(&reference.x)?,
                    numeric(&p.x)?,
                    floating(&p.y)?,
                    dv_tools::signal_analysis::Extrapolation::Bounded,
                )?
            };
            if method == CompareMethod::YY {
                q.x = ColumnData::Float(yr.to_vec());
                q.y = ColumnData::Float(y);
                q.sx = reference.sy.clone();
            } else {
                q.x = reference.x.clone();
                q.y = ColumnData::Float(
                    y.iter()
                        .zip(yr)
                        .map(|(&v, &r)| match method {
                            CompareMethod::Relative => {
                                100.0 * (v - r) / (r + if near_zero { 1.0 } else { 0.0 })
                            }
                            CompareMethod::AbsRelative => {
                                (100.0 * (v - r) / (r + if near_zero { 1.0 } else { 0.0 })).abs()
                            }
                            CompareMethod::Ratio => {
                                (v + if near_zero { 1.0 } else { 0.0 })
                                    / (r + if near_zero { 1.0 } else { 0.0 })
                            }
                            CompareMethod::Absolute => v - r,
                            CompareMethod::YY => unreachable!(),
                        })
                        .collect(),
                );
                q.sx = reference.sx.clone();
                q.sy = match method {
                    CompareMethod::Relative => "Relative error [%]".into(),
                    CompareMethod::AbsRelative => "Abs. relative error [%]".into(),
                    CompareMethod::Ratio => "Ratio [-]".into(),
                    CompareMethod::Absolute => format!("Absolute error [{}]", unit(&reference.sy)),
                    CompareMethod::YY => unreachable!(),
                };
            }
            q.syl = if mode == CompareMode::OneTableColumns {
                format!("{} wrt. {}", no_unit(&p.sy), no_unit(&reference.sy))
            } else {
                format!("{} wrt. {}, {}", p.st, reference.st, p.sy)
            };
            q.recompute_stats();
            output.push(q);
        }
    }
    if output.is_empty() {
        return Err(DvError::msg("No matching columns to compare"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_and_raw_stats() {
        let mut p =
            PlotData::new(vec![0., 1., 2.], vec![2., 4., 6.], "Time [s]", "Load [N]").unwrap();
        p.transform(&PlotType::MinMax(Default::default())).unwrap();
        assert_eq!(p.numeric_xy().unwrap().1, &[0., 0.5, 1.]);
        assert_eq!(p.original_stats().mean, 4.);
        assert_eq!(p.stats().integral, 1.);
    }
    #[test]
    fn comparison_interpolates() {
        let a = PlotData::new(vec![0., 1., 2.], vec![1., 2., 3.], "t", "y").unwrap();
        let b = PlotData::new(vec![0., 2.], vec![2., 6.], "t", "y").unwrap();
        let c = compare(&[a, b], CompareMode::TablesOneColumn, CompareMethod::Ratio).unwrap();
        assert_eq!(c[0].numeric_xy().unwrap().1, &[2., 2., 2.]);
    }
    #[test]
    fn categorical_pdf() {
        let mut p = PlotData::from_columns(
            ColumnData::Float(vec![0., 1., 2.]),
            ColumnData::Str(vec!["b".into(), "a".into(), "b".into()]),
            "x".into(),
            "category".into(),
        )
        .unwrap();
        p.to_pdf(&PdfOptions::default()).unwrap();
        assert_eq!(p.x, ColumnData::Str(vec!["a".into(), "b".into()]));
        assert_eq!(p.y, ColumnData::Float(vec![1. / 3., 2. / 3.]));
    }
    #[test]
    fn date_normalization_becomes_numeric() {
        let mut p = PlotData::from_columns(
            ColumnData::DateTime(vec![100., 200., 300.]),
            ColumnData::DateTime(vec![400., 500., 600.]),
            "x".into(),
            "y".into(),
        )
        .unwrap();
        p.to_min_max(&MinMaxOptions {
            x_scale: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(p.x, ColumnData::Float(vec![0., 0.5, 1.]));
        assert_eq!(p.y, ColumnData::Float(vec![0., 0.5, 1.]));
        assert_eq!(p.stats().mean, 0.5);
    }
}
