//! Statistics, histogram/KDE densities and right-closed binning from pyDatView.
use dv_core::{
    dataframe::{Column, ColumnData, DataFrame},
    error::{DvError, Result},
};
use std::f64::consts::PI;

/// Statistics of finite `(x, y)` sample pairs in a selected x range.
///
/// `std` and `variance` use the population convention (`ddof = 0`), matching
/// pyDatView's `numpy.nanstd`. Percentiles use NumPy's linear interpolation.
/// `start` and `end` are the first and last selected sample values in input
/// order. Undefined values are NaN; empty selections have `n = 0`.
/// `mean_dt` is `(last_x - first_x) / (n - 1)` for the selected finite
/// samples, including any gaps between them. The least-squares `slope`
/// and `mean_dt` are undefined if there is no variation in x or fewer than
/// two samples, respectively.
///
/// Integration follows consecutive input samples in their original order,
/// clipping each linearly interpolated segment to the requested range. A
/// non-finite x or y breaks the line rather than joining across missing data.
/// `duration` is the sum of the absolute widths of the covered segments, so
/// missing intervals are excluded; the signed integral divided by that
/// duration is `integral_mean`. This also gives a meaningful clipped integral
/// when the selected range lies entirely between two samples.
/// `integral_x` and `integral_x2` are trapezoidal integrals of `y*x` and
/// `y*x²`; `integral_x_normalized` divides the former by the integral.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SignalSummary {
    pub n: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub std: f64,
    pub rms: f64,
    pub p1: f64,
    pub p5: f64,
    pub p95: f64,
    pub p99: f64,
    pub peak_to_peak: f64,
    pub start: f64,
    pub end: f64,
    pub delta: f64,
    pub slope: f64,
    pub integral: f64,
    pub integral_mean: f64,
    pub integral_x: f64,
    pub integral_x_normalized: f64,
    pub integral_x2: f64,
    pub x_at_min: f64,
    pub x_at_max: f64,
    pub duration: f64,
    pub mean_dt: f64,
    pub ti: f64,
    pub variance: f64,
    pub abs_max: f64,
}

impl Default for SignalSummary {
    fn default() -> Self {
        Self {
            n: 0,
            min: f64::NAN,
            max: f64::NAN,
            mean: f64::NAN,
            median: f64::NAN,
            std: f64::NAN,
            rms: f64::NAN,
            p1: f64::NAN,
            p5: f64::NAN,
            p95: f64::NAN,
            p99: f64::NAN,
            peak_to_peak: f64::NAN,
            start: f64::NAN,
            end: f64::NAN,
            delta: f64::NAN,
            slope: f64::NAN,
            integral: f64::NAN,
            integral_mean: f64::NAN,
            integral_x: f64::NAN,
            integral_x_normalized: f64::NAN,
            integral_x2: f64::NAN,
            x_at_min: f64::NAN,
            x_at_max: f64::NAN,
            duration: f64::NAN,
            mean_dt: f64::NAN,
            ti: f64::NAN,
            variance: f64::NAN,
            abs_max: f64::NAN,
        }
    }
}

type Bounds = Option<(f64, f64)>;

fn bounds(range: Option<[f64; 2]>) -> Option<Bounds> {
    match range {
        None => Some(None),
        Some([a, b]) if !a.is_nan() && !b.is_nan() => Some(Some((a.min(b), a.max(b)))),
        Some(_) => None,
    }
}

// Sorted selections can be found in logarithmic time; unsorted or non-finite
// x data must be filtered linearly. The caller also visits neighbouring points
// for segments that intersect the range without containing a sample.
fn sample_interval(x: &[f64], bounds: Bounds) -> (usize, usize, bool) {
    if let Some((low, high)) = bounds {
        if x.windows(2).all(|w| w[0].is_finite() && w[0] <= w[1])
            && x.last().is_none_or(|v| v.is_finite())
        {
            return (
                x.partition_point(|v| *v < low),
                x.partition_point(|v| *v <= high),
                true,
            );
        }
    }
    (0, x.len(), bounds.is_none())
}

fn in_bounds(x: f64, bounds: Bounds) -> bool {
    bounds.is_none_or(|(low, high)| x >= low && x <= high)
}

// Returns the clipped x and y endpoints, in the input segment's direction.
fn clipped_segment(
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    bounds: Bounds,
) -> Option<(f64, f64, f64, f64)> {
    if !x0.is_finite() || !x1.is_finite() || !y0.is_finite() || !y1.is_finite() || x0 == x1 {
        return None;
    }
    let dx = x1 - x0;
    let Some((low, high)) = bounds else {
        return Some((x0, y0, x1, y1));
    };
    let a = x0.min(x1).max(low);
    let b = x0.max(x1).min(high);
    if a >= b {
        return None;
    }
    let (start, end) = if dx > 0.0 { (a, b) } else { (b, a) };
    let y_at = |v: f64| {
        if v == x0 {
            y0
        } else if v == x1 {
            y1
        } else {
            let t = (v - x0) / dx;
            y0 * (1.0 - t) + y1 * t
        }
    };
    Some((start, y_at(start), end, y_at(end)))
}

// Select all percentile ranks together, reusing partitions and a single
// scratch buffer rather than sorting millions of observations.
fn select_ranks(values: &mut [f64], offset: usize, ranks: &[(usize, usize)], out: &mut [f64; 10]) {
    if ranks.is_empty() {
        return;
    }
    let rank = ranks[ranks.len() / 2].0;
    let first = ranks.partition_point(|&(r, _)| r < rank);
    let last = ranks.partition_point(|&(r, _)| r <= rank);
    let (left, value, right) = values.select_nth_unstable_by(rank - offset, f64::total_cmp);
    for &(_, index) in &ranks[first..last] {
        out[index] = *value;
    }
    select_ranks(left, offset, &ranks[..first], out);
    select_ranks(right, rank + 1, &ranks[last..], out);
}

fn percentiles(values: &mut [f64]) -> [f64; 5] {
    let quantiles = [0.01, 0.05, 0.5, 0.95, 0.99];
    let mut ranks = [(0, 0); 10];
    let mut fraction = [0.0; 5];
    for (i, quantile) in quantiles.into_iter().enumerate() {
        let position: f64 = quantile * (values.len() - 1) as f64;
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        ranks[2 * i] = (lower, 2 * i);
        ranks[2 * i + 1] = (upper, 2 * i + 1);
        fraction[i] = position - lower as f64;
    }
    // Adjacent percentiles can share bracketing ranks, especially for small
    // samples, so sort the rank/index pairs before partitioning.
    ranks.sort_unstable_by_key(|&(rank, _)| rank);
    let mut selected = [0.0; 10];
    select_ranks(values, 0, &ranks, &mut selected);
    std::array::from_fn(|i| {
        selected[2 * i] * (1.0 - fraction[i]) + selected[2 * i + 1] * fraction[i]
    })
}

/// Summarize finite pairs whose x lies in `range`, inclusive.
///
/// Reversed bounds are accepted. If lengths differ, only the common prefix
/// is used. A range with a NaN bound produces an empty summary. On sorted
/// finite x, binary searches locate the selected samples; otherwise the
/// common prefix is filtered in input order.
pub fn summarize(x: &[f64], y: &[f64], range: Option<[f64; 2]>) -> SignalSummary {
    let Some(bounds) = bounds(range) else {
        return SignalSummary::default();
    };
    let len = x.len().min(y.len());
    let (x, y) = (&x[..len], &y[..len]);
    let (start, end, in_range) = sample_interval(x, bounds);
    let mut result = SignalSummary::default();
    let capacity = if in_range {
        end - start
    } else {
        (end - start).min(1024)
    };
    let mut values = Vec::with_capacity(capacity);
    let mut x_origin = 0.0;
    let mut y_origin = 0.0;
    let mut x_offsets = 0.0;
    let mut y_offsets = 0.0;
    let mut first_x = 0.0;
    let mut last_x = 0.0;

    for (&xv, &yv) in x[start..end].iter().zip(&y[start..end]) {
        if !xv.is_finite() || !yv.is_finite() || (!in_range && !in_bounds(xv, bounds)) {
            continue;
        }
        if values.is_empty() {
            x_origin = xv;
            y_origin = yv;
            first_x = xv;
            result.start = yv;
            result.min = yv;
            result.max = yv;
            result.x_at_min = xv;
            result.x_at_max = xv;
        }
        if yv < result.min {
            result.min = yv;
            result.x_at_min = xv;
        }
        if yv > result.max {
            result.max = yv;
            result.x_at_max = xv;
        }
        x_offsets += xv - x_origin;
        y_offsets += yv - y_origin;
        last_x = xv;
        result.end = yv;
        values.push(yv);
    }
    result.n = values.len();
    if result.n != 0 {
        let count = result.n as f64;
        let x_mean_offset = x_offsets / count;
        let y_mean_offset = y_offsets / count;
        result.mean = y_origin + y_mean_offset;
        let mut sum_y2 = 0.0;
        let mut sum_x2 = 0.0;
        let mut sum_xy = 0.0;
        for (&xv, &yv) in x[start..end].iter().zip(&y[start..end]) {
            if !xv.is_finite() || !yv.is_finite() || (!in_range && !in_bounds(xv, bounds)) {
                continue;
            }
            let dx = (xv - x_origin) - x_mean_offset;
            let dy = (yv - y_origin) - y_mean_offset;
            sum_x2 += dx * dx;
            sum_y2 += dy * dy;
            sum_xy += dx * dy;
        }
        result.variance = sum_y2 / count;
        result.std = result.variance.sqrt();
        result.rms = result.mean.hypot(result.std);
        result.slope = if sum_x2 > 0.0 {
            sum_xy / sum_x2
        } else {
            f64::NAN
        };
        result.ti = if result.mean != 0.0 {
            result.std / result.mean
        } else {
            f64::NAN
        };
        result.mean_dt = if result.n > 1 {
            (last_x - first_x) / (count - 1.0)
        } else {
            f64::NAN
        };
        result.delta = result.end - result.start;
        result.peak_to_peak = result.max - result.min;
        result.abs_max = result.min.abs().max(result.max.abs());
        let [p1, p5, median, p95, p99] = percentiles(&mut values);
        result.p1 = p1;
        result.p5 = p5;
        result.median = median;
        result.p95 = p95;
        result.p99 = p99;
    }

    let left = start.saturating_sub(1);
    let right = end.saturating_add(1).min(len);
    let mut integral = 0.0;
    let mut integral_x = 0.0;
    let mut integral_x2 = 0.0;
    let mut duration = 0.0;
    for i in left..right.saturating_sub(1) {
        if let Some((xa, a, xb, b)) = clipped_segment(x[i], y[i], x[i + 1], y[i + 1], bounds) {
            let dx = xb - xa;
            integral += (a * 0.5 + b * 0.5) * dx;
            integral_x += (a * xa * 0.5 + b * xb * 0.5) * dx;
            integral_x2 += (a * xa * xa * 0.5 + b * xb * xb * 0.5) * dx;
            duration += dx.abs();
        }
    }
    if result.n != 0 || duration > 0.0 {
        result.integral = integral;
        result.integral_x = integral_x;
        result.integral_x2 = integral_x2;
        if integral != 0.0 {
            result.integral_x_normalized = integral_x / integral;
        }
        result.duration = duration;
        if duration > 0.0 {
            result.integral_mean = integral / duration;
        }
    }
    result
}

/// Duration where the linearly interpolated signal is strictly above
/// `threshold`, within `range`. Non-finite pairs break the line, so gaps are
/// never counted. Descending segments contribute positive duration.
pub fn time_above(x: &[f64], y: &[f64], threshold: f64, range: Option<[f64; 2]>) -> f64 {
    let Some(bounds) = bounds(range) else {
        return f64::NAN;
    };
    if !threshold.is_finite() {
        return f64::NAN;
    }
    let len = x.len().min(y.len());
    let (x, y) = (&x[..len], &y[..len]);
    let (start, end, _) = sample_interval(x, bounds);
    let left = start.saturating_sub(1);
    let right = end.saturating_add(1).min(len);
    let mut duration = 0.0;
    for i in left..right.saturating_sub(1) {
        if let Some((xa, a, xb, b)) = clipped_segment(x[i], y[i], x[i + 1], y[i + 1], bounds) {
            let fraction = match (a > threshold, b > threshold) {
                (true, true) => 1.0,
                (false, false) => 0.0,
                (true, false) => (a - threshold) / (a - b),
                (false, true) => (b - threshold) / (b - a),
            };
            duration += (xb - xa).abs() * fraction;
        }
    }
    duration
}

pub fn nanmean(y: &[f64]) -> f64 {
    let mut sum = 0.0;
    let mut n = 0;
    for &v in y {
        if !v.is_nan() {
            sum += v;
            n += 1;
        }
    }
    sum / n as f64
}
pub fn nanstd(y: &[f64], ddof: usize) -> f64 {
    let mean = nanmean(y);
    let mut sum = 0.0;
    let mut n = 0;
    for &v in y {
        if !v.is_nan() {
            sum += (v - mean).powi(2);
            n += 1;
        }
    }
    if n <= ddof {
        f64::NAN
    } else {
        (sum / (n - ddof) as f64).sqrt()
    }
}
pub fn nanmedian(y: &[f64]) -> f64 {
    let mut values = y.to_vec();
    nanmedian_in_place(&mut values)
}

// Reuses an owned scratch buffer for callers such as outlier rejection.
pub(crate) fn nanmedian_in_place(values: &mut Vec<f64>) -> f64 {
    values.retain(|v| !v.is_nan());
    let n = values.len();
    if n == 0 {
        return f64::NAN;
    }
    let (lower, middle, _) = values.select_nth_unstable_by(n / 2, f64::total_cmp);
    if n.is_multiple_of(2) {
        let previous = lower
            .iter()
            .copied()
            .max_by(f64::total_cmp)
            .unwrap_or(f64::NAN);
        (previous + *middle) / 2.0
    } else {
        *middle
    }
}
pub fn rsquare(y: &[f64], fitted: &[f64], constant: bool) -> Result<(f64, f64)> {
    if y.len() != fitted.len() {
        return Err(DvError::msg("Y and F must be the same size"));
    }
    let pairs: Vec<_> = y
        .iter()
        .zip(fitted)
        .filter(|(a, b)| !a.is_nan() && !b.is_nan())
        .collect();
    let mean = pairs.iter().map(|(a, _)| **a).sum::<f64>() / pairs.len() as f64;
    let error = pairs.iter().map(|(a, b)| (**a - **b).powi(2)).sum::<f64>();
    let denominator = pairs
        .iter()
        .map(|(a, _)| (**a - if constant { mean } else { 0.0 }).powi(2))
        .sum::<f64>();
    let r2 = if denominator.abs() > 0.0 {
        (1.0 - error / denominator).max(0.0)
    } else {
        f64::INFINITY
    };
    Ok((r2, (error / pairs.len() as f64).sqrt()))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RelativeErrorMethod {
    Mean,
    #[default]
    MeanAbs,
    Local,
    MinMax,
    OneTwo,
}
pub fn mean_rel_err(
    y: &[f64],
    other: &[f64],
    method: RelativeErrorMethod,
    absolute: bool,
) -> Result<f64> {
    if y.len() != other.len() {
        return Err(DvError::msg("relative error requires equal signal lengths"));
    }
    let abs = |v: f64| if absolute { v.abs() } else { v };
    let mut errors = Vec::with_capacity(y.len());
    match method {
        RelativeErrorMethod::Mean | RelativeErrorMethod::MeanAbs => {
            let reference = if method == RelativeErrorMethod::Mean {
                nanmean(y)
            } else {
                y.iter()
                    .filter(|v| !v.is_nan())
                    .map(|v| v.abs())
                    .sum::<f64>()
                    / y.iter().filter(|v| !v.is_nan()).count() as f64
            };
            if reference == 0.0 {
                return Ok(f64::NAN);
            }
            errors.extend(y.iter().zip(other).map(|(a, b)| abs(b - a) / reference));
        }
        RelativeErrorMethod::Local => {
            errors.extend(y.iter().zip(other).map(|(a, b)| abs(b - a) / a.abs()))
        }
        RelativeErrorMethod::MinMax | RelativeErrorMethod::OneTwo => {
            let (min, max) = finite_range(y.iter().chain(other).copied())?;
            let offset = if method == RelativeErrorMethod::MinMax {
                0.5
            } else {
                1.0
            };
            errors.extend(y.iter().zip(other).map(|(a, b)| {
                let a = (a - min) / (max - min) + offset;
                let b = (b - min) / (max - min) + offset;
                abs(b - a) / a.abs()
            }));
        }
    }
    Ok(100.0 * nanmean(&errors))
}
pub fn mean_rel_err_time(
    t: &[f64],
    y: &[f64],
    other_t: &[f64],
    other: &[f64],
    method: RelativeErrorMethod,
    absolute: bool,
) -> Result<f64> {
    if y.len() == other.len() {
        mean_rel_err(y, other, method, absolute)
    } else {
        let interpolated = crate::signal_analysis::interpolate(
            t,
            other_t,
            other,
            crate::signal_analysis::Extrapolation::Bounded,
        )?;
        mean_rel_err(y, &interpolated, method, absolute)
    }
}
fn finite_range(values: impl Iterator<Item = f64>) -> Result<(f64, f64)> {
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    for v in values {
        if !v.is_nan() {
            min = min.min(v);
            max = max.max(v);
        }
    }
    if !min.is_finite() || !max.is_finite() {
        Err(DvError::msg(
            "statistics require finite, nonmissing observations",
        ))
    } else {
        Ok((min, max))
    }
}
pub fn linspace(min: f64, max: f64, n: usize) -> Vec<f64> {
    match n {
        0 => Vec::new(),
        1 => vec![min],
        _ => (0..n)
            .map(|i| min + (max - min) * i as f64 / (n - 1) as f64)
            .collect(),
    }
}
pub fn trapezoid(y: &[f64], x: &[f64]) -> f64 {
    y.windows(2)
        .zip(x.windows(2))
        .map(|(y, x)| (x[1] - x[0]) * (y[1] + y[0]) / 2.0)
        .sum()
}
/// Upstream normalization uses a trapezoidal integral over bin centers, not
/// the sum over bin widths. `count` selects conventional sample-count scaling
/// before that optional normalization (the Python count branch has a typo).
pub fn pdf_histogram(
    y: &[f64],
    bins: usize,
    normalize: bool,
    count: bool,
) -> Result<(Vec<f64>, Vec<f64>)> {
    if bins == 0 {
        return Err(DvError::msg("histogram requires at least one bin"));
    }
    let (mut min, mut max) = if y.iter().all(|v| v.is_nan()) {
        (0.0, 1.0)
    } else {
        finite_range(y.iter().copied())?
    };
    if min == max {
        min -= 0.5;
        max += 0.5;
    }
    let width = (max - min) / bins as f64;
    let mut density = vec![0.0; bins];
    let mut samples = 0;
    for &v in y {
        if !v.is_nan() {
            let i = (((v - min) / width).floor() as usize).min(bins - 1);
            density[i] += 1.0;
            samples += 1;
        }
    }
    let denominator = if count {
        samples as f64 * width
    } else {
        bins as f64 * width
    };
    for v in &mut density {
        *v /= denominator;
    }
    let x: Vec<_> = (0..bins).map(|i| min + (i as f64 + 0.5) * width).collect();
    if normalize {
        let area = trapezoid(&density, &x);
        for v in &mut density {
            *v /= area;
        }
    }
    Ok((x, density))
}
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Bandwidth {
    #[default]
    Scott,
    Silverman,
    Factor(f64),
}
#[derive(Debug, Clone)]
pub struct KdeOptions {
    pub bandwidth: Bandwidth,
    pub points: usize,
    pub cut: f64,
    pub clip: (f64, f64),
}
impl Default for KdeOptions {
    fn default() -> Self {
        Self {
            bandwidth: Bandwidth::Scott,
            points: 100,
            cut: 3.0,
            clip: (f64::NEG_INFINITY, f64::INFINITY),
        }
    }
}
pub fn pdf_gaussian_kde(data: &[f64], options: &KdeOptions) -> Result<(Vec<f64>, Vec<f64>)> {
    let values: Vec<_> = data.iter().copied().filter(|v| !v.is_nan()).collect();
    if values.len() < 2
        || options.points == 0
        || !options.cut.is_finite()
        || options.cut < 0.0
        || options.clip.0 > options.clip.1
    {
        return Err(DvError::msg("invalid KDE data, support or output length"));
    }
    let (min, max) = finite_range(values.iter().copied())?;
    let n = values.len() as f64;
    let factor = match options.bandwidth {
        Bandwidth::Scott => n.powf(-0.2),
        Bandwidth::Silverman => (n * 0.75).powf(-0.2),
        Bandwidth::Factor(v) => v,
    };
    let sigma = nanstd(&values, 1) * factor;
    if !sigma.is_finite() || sigma <= 0.0 {
        return Err(DvError::msg(
            "KDE covariance is singular or bandwidth is invalid",
        ));
    }
    // Numeric bw is a covariance factor to SciPy, but a raw support width in pyDatView.
    let support = match options.bandwidth {
        Bandwidth::Factor(v) => v,
        _ => nanstd(&values, 0) * factor,
    };
    let lower = (min - support * options.cut).max(options.clip.0);
    let upper = (max + support * options.cut).min(options.clip.1);
    if lower > upper {
        return Err(DvError::msg("KDE clip excludes its support"));
    }
    let x = linspace(lower, upper, options.points);
    let normalizer = n * sigma * (2.0 * PI).sqrt();
    let y = x
        .iter()
        .map(|x| {
            values
                .iter()
                .map(|v| (-0.5 * ((x - v) / sigma).powi(2)).exp())
                .sum::<f64>()
                / normalizer
        })
        .collect();
    Ok((x, y))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PdfMethod {
    #[default]
    Histogram,
    GaussianKde,
}
pub fn pdf(data: &[f64], method: PdfMethod, points: usize) -> Result<(Vec<f64>, Vec<f64>)> {
    match method {
        PdfMethod::Histogram => pdf_histogram(data, points, true, false),
        PdfMethod::GaussianKde => pdf_gaussian_kde(
            data,
            &KdeOptions {
                points,
                ..Default::default()
            },
        ),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinStatistic {
    #[default]
    Mean,
    Std,
}
fn check_edges(edges: &[f64]) -> Result<()> {
    if edges.len() < 2
        || edges.iter().any(|v| !v.is_finite())
        || edges.windows(2).any(|w| w[1] <= w[0])
    {
        Err(DvError::msg(
            "bin edges must be finite and strictly increasing",
        ))
    } else {
        Ok(())
    }
}
fn bin_index(x: f64, edges: &[f64]) -> Option<usize> {
    if x.is_nan() || x <= edges[0] || x > *edges.last().unwrap() {
        return None;
    }
    Some(edges.partition_point(|v| *v < x) - 1)
}
#[derive(Clone, Copy, Default)]
struct Accumulator {
    n: usize,
    mean: f64,
    m2: f64,
}
impl Accumulator {
    fn push(&mut self, v: f64) {
        if v.is_nan() {
            return;
        }
        self.n += 1;
        let delta = v - self.mean;
        self.mean += delta / self.n as f64;
        self.m2 += delta * (v - self.mean);
    }
    fn value(&self, stat: BinStatistic) -> f64 {
        match stat {
            BinStatistic::Mean => {
                if self.n == 0 {
                    f64::NAN
                } else {
                    self.mean
                }
            }
            BinStatistic::Std => {
                if self.n < 2 {
                    f64::NAN
                } else {
                    (self.m2 / (self.n - 1) as f64).sqrt()
                }
            }
        }
    }
}
pub fn bin_signal(
    x: &[f64],
    y: &[f64],
    edges: &[f64],
    stat: &BinStatistic,
) -> Result<(Vec<f64>, Vec<f64>)> {
    check_edges(edges)?;
    if x.len() != y.len() {
        return Err(DvError::msg("binning requires equal x/y lengths"));
    }
    let mut xs = vec![Accumulator::default(); edges.len() - 1];
    let mut ys = xs.clone();
    for (&x, &y) in x.iter().zip(y) {
        if let Some(i) = bin_index(x, edges) {
            xs[i].push(x);
            ys[i].push(y);
        }
    }
    Ok((
        xs.iter().map(|v| v.value(*stat)).collect(),
        ys.iter().map(|v| v.value(*stat)).collect(),
    ))
}
pub fn bin_signal_n(
    x: &[f64],
    y: &[f64],
    bins: usize,
    stat: BinStatistic,
) -> Result<(Vec<f64>, Vec<f64>)> {
    if bins == 0 {
        return Err(DvError::msg("number of bins must be positive"));
    }
    let (min, max) = finite_range(x.iter().copied())?;
    bin_signal(x, y, &linspace(min, max, bins + 1), &stat)
}
pub fn bin_df(
    df: &DataFrame,
    edges: &[f64],
    column: usize,
    stat: BinStatistic,
) -> Result<DataFrame> {
    check_edges(edges)?;
    if column >= df.ncols() {
        return Err(DvError::msg("bin column is out of range"));
    }
    let x = df.f64_col(column);
    let mut counts = vec![0.0; edges.len() - 1];
    for &v in x.iter() {
        if let Some(i) = bin_index(v, edges) {
            counts[i] += 1.0;
        }
    }
    let mut columns = Vec::new();
    for c in df.columns() {
        if matches!(&c.data, ColumnData::Str(_)) {
            return Err(DvError::msg("binning requires numeric columns"));
        }
        let (_, y) = bin_signal(&x, &c.as_f64(), edges, &stat)?;
        columns.push(Column::float(c.name.clone(), y));
    }
    columns.push(Column::float("Counts", counts));
    DataFrame::from_columns(columns)
}
#[derive(Debug, Clone)]
pub struct Binned2D {
    pub x_mean: Vec<Vec<f64>>,
    pub y_mean: Vec<Vec<f64>>,
    pub z_mean: Vec<Vec<f64>>,
    pub z_std: Vec<Vec<f64>>,
    pub counts: Vec<Vec<usize>>,
    pub x_mid: Vec<Vec<f64>>,
    pub y_mid: Vec<Vec<f64>>,
}
/// The upstream 2-D algorithm includes both bin boundaries; observations on
/// an internal edge therefore contribute to both adjacent bins.
pub fn bin2d_signal(
    x: &[f64],
    y: &[f64],
    z: &[f64],
    xedges: &[f64],
    yedges: &[f64],
) -> Result<Binned2D> {
    check_edges(xedges)?;
    check_edges(yedges)?;
    if x.len() != y.len() || x.len() != z.len() {
        return Err(DvError::msg("2-D binning requires equal signal lengths"));
    }
    let nx = xedges.len() - 1;
    let ny = yedges.len() - 1;
    let blank = vec![vec![f64::NAN; ny]; nx];
    let mut result = Binned2D {
        x_mean: blank.clone(),
        y_mean: blank.clone(),
        z_mean: blank.clone(),
        z_std: blank.clone(),
        counts: vec![vec![0; ny]; nx],
        x_mid: blank.clone(),
        y_mid: blank,
    };
    for i in 0..nx {
        for j in 0..ny {
            result.x_mid[i][j] = (xedges[i] + xedges[i + 1]) / 2.0;
            result.y_mid[i][j] = (yedges[j] + yedges[j + 1]) / 2.0;
            let mut sx = 0.0;
            let mut sy = 0.0;
            let mut sz = 0.0;
            for k in 0..x.len() {
                if x[k] >= xedges[i]
                    && x[k] <= xedges[i + 1]
                    && y[k] >= yedges[j]
                    && y[k] <= yedges[j + 1]
                {
                    result.counts[i][j] += 1;
                    sx += x[k];
                    sy += y[k];
                    sz += z[k];
                }
            }
            let n = result.counts[i][j];
            if n > 0 {
                result.x_mean[i][j] = sx / n as f64;
                result.y_mean[i][j] = sy / n as f64;
                result.z_mean[i][j] = sz / n as f64;
                let mut variance = 0.0;
                for k in 0..x.len() {
                    if x[k] >= xedges[i]
                        && x[k] <= xedges[i + 1]
                        && y[k] >= yedges[j]
                        && y[k] <= yedges[j + 1]
                    {
                        variance += (z[k] - result.z_mean[i][j]).powi(2);
                    }
                }
                result.z_std[i][j] = (variance / n as f64).sqrt();
            }
        }
    }
    Ok(result)
}
pub fn azimuthal_stats_df(
    df: &DataFrame,
    edges: &[f64],
    azimuth: usize,
    time: Option<(usize, f64)>,
    stat: BinStatistic,
) -> Result<DataFrame> {
    if let Some((column, start)) = time {
        if column >= df.ncols() {
            return Err(DvError::msg("time column out of range"));
        }
        let mask: Vec<_> = df.f64_col(column).iter().map(|v| *v > start).collect();
        bin_df(&df.filter_rows(&mask), edges, azimuth, stat)
    } else {
        bin_df(df, edges, azimuth, stat)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }

    #[test]
    fn median_handles_unordered_even_odd_and_missing_values() {
        close(nanmedian(&[7., f64::NAN, 1., 4., 2.]), 3.);
        close(nanmedian(&[9., f64::NAN, 3., 5.]), 5.);
        assert!(nanmedian(&[f64::NAN]).is_nan());
        assert_eq!(nanmedian(&[f64::INFINITY, 4., 2.]), 4.);
    }

    #[test]
    fn summary_matches_linear_percentiles_and_population_statistics() {
        let x = [0., 1., 2., 3., 4.];
        let stats = summarize(&x, &x, None);
        assert_eq!(stats.n, 5);
        close(stats.min, 0.);
        close(stats.max, 4.);
        close(stats.mean, 2.);
        close(stats.median, 2.);
        close(stats.variance, 2.);
        close(stats.std, 2_f64.sqrt());
        close(stats.rms, 6_f64.sqrt());
        close(stats.p1, 0.04);
        close(stats.p5, 0.2);
        close(stats.p95, 3.8);
        close(stats.p99, 3.96);
        close(stats.peak_to_peak, 4.);
        close(stats.start, 0.);
        close(stats.end, 4.);
        close(stats.delta, 4.);
        close(stats.slope, 1.);
        close(stats.integral, 8.);
        close(stats.integral_mean, 2.);
        close(stats.integral_x, 22.);
        close(stats.integral_x_normalized, 2.75);
        close(stats.integral_x2, 68.);
        close(stats.x_at_min, 0.);
        close(stats.x_at_max, 4.);
        close(stats.duration, 4.);
        close(stats.mean_dt, 1.);
        close(stats.ti, 2_f64.sqrt() / 2.);
        close(stats.abs_max, 4.);
    }

    #[test]
    fn summary_clips_boundary_segments_and_range_can_contain_no_samples() {
        let x = [0., 1., 2., 3., 4.];
        let stats = summarize(&x, &x, Some([2.5, 0.5]));
        assert_eq!(stats.n, 2);
        close(stats.mean, 1.5);
        close(stats.p1, 1.01);
        close(stats.p5, 1.05);
        close(stats.p95, 1.95);
        close(stats.p99, 1.99);
        close(stats.integral, 3.);
        close(stats.integral_x, 5.375);
        close(stats.integral_x2, 10.6875);
        close(stats.duration, 2.);
        close(stats.integral_mean, 1.5);
        close(time_above(&x, &x, 1.5, Some([0.5, 2.5])), 1.);

        let stats = summarize(&[0., 1.], &[0., 10.], Some([0.2, 0.4]));
        assert_eq!(stats.n, 0);
        assert!(stats.mean.is_nan());
        close(stats.integral, 0.6);
        close(stats.duration, 0.2);
        close(stats.integral_mean, 3.);
        close(time_above(&[0., 1.], &[0., 10.], 3., Some([0.2, 0.4])), 0.1);
    }

    #[test]
    fn summary_filters_unsorted_input_and_does_not_bridge_nan_gaps() {
        let x = [3., 0., 2., 1.];
        let stats = summarize(&x, &x, Some([0.5, 2.5]));
        assert_eq!(stats.n, 2);
        close(stats.start, 2.);
        close(stats.end, 1.);
        close(stats.min, 1.);
        close(stats.max, 2.);
        close(stats.integral, -2.625);
        close(stats.duration, 4.5);

        let x = [0., 1., 2., 3., 4.];
        let y = [0., 1., f64::NAN, 3., 4.];
        let stats = summarize(&x, &y, None);
        assert_eq!(stats.n, 4);
        close(stats.mean, 2.);
        close(stats.integral, 4.);
        close(stats.duration, 2.);
        close(stats.integral_mean, 2.);
        close(time_above(&x, &y, 0.5, None), 1.5);
    }

    #[test]
    fn summary_handles_single_empty_and_mismatched_input() {
        let empty = summarize(&[], &[], None);
        assert_eq!(empty.n, 0);
        assert!(empty.mean.is_nan());
        assert!(empty.integral.is_nan());
        let stats = summarize(&[0., 1.], &[3.], None);
        assert_eq!(stats.n, 1);
        close(stats.median, 3.);
        close(stats.p1, 3.);
        close(stats.p99, 3.);
        close(stats.std, 0.);
        close(stats.integral, 0.);
        assert!(stats.slope.is_nan());
        assert!(stats.mean_dt.is_nan());
        assert!(stats.integral_mean.is_nan());
        assert_eq!(summarize(&[0.], &[1.], Some([f64::NAN, 1.])).n, 0);
    }

    #[test]
    fn slope_and_variance_stay_accurate_with_large_offsets() {
        let x = [1e12, 1e12 + 1., 1e12 + 2., 1e12 + 3.];
        let y = [1e12 + 1., 1e12 + 3., 1e12 + 5., 1e12 + 7.];
        let stats = summarize(&x, &y, None);
        close(stats.variance, 5.);
        close(stats.slope, 2.);
    }

    #[test]
    #[ignore = "release-mode performance measurement"]
    fn summary_ten_million_samples_benchmark() {
        let n = 10_000_000;
        let x: Vec<f64> = (0..n).map(|i| i as f64 * 0.001).collect();
        let y: Vec<f64> = (0..n)
            .map(|i| {
                let bits = (i as u64).wrapping_mul(6364136223846793005).wrapping_add(1);
                (bits >> 11) as f64 / (1_u64 << 53) as f64
            })
            .collect();
        let start = std::time::Instant::now();
        let stats = std::hint::black_box(summarize(&x, &y, None));
        let elapsed = start.elapsed();
        println!("10,000,000-sample summary: {elapsed:?}");
        assert_eq!(stats.n, n);
    }

    #[test]
    fn right_closed_bins() {
        let (x, y) = bin_signal(
            &[0., 1., 2., 3.],
            &[0., 2., 4., 6.],
            &[0., 2., 4.],
            &BinStatistic::Mean,
        )
        .unwrap();
        assert_eq!(x, vec![1.5, 3.]);
        assert_eq!(y, vec![3., 6.]);
    }
    #[test]
    fn r_square_ignores_nan() {
        assert_eq!(
            rsquare(&[1., 2., f64::NAN], &[1., 2., 7.], true).unwrap(),
            (1., 0.)
        );
    }
    #[test]
    fn kde_singular_error() {
        assert!(pdf_gaussian_kde(&[1., 1.], &KdeOptions::default()).is_err());
    }
    #[test]
    fn histogram_normalization() {
        let (x, y) = pdf_histogram(&[0., 1., 2., 3.], 4, true, false).unwrap();
        assert!((trapezoid(&y, &x) - 1.).abs() < 1e-12);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonStatistic {
    SigmaRatio,
    RelativeError,
    RSquared,
    EquivalentLoadError,
}
#[derive(Debug, Clone)]
pub struct ComparisonOptions {
    pub statistics: Vec<ComparisonStatistic>,
    pub error_method: RelativeErrorMethod,
    pub absolute: bool,
}
impl Default for ComparisonOptions {
    fn default() -> Self {
        Self {
            statistics: vec![
                ComparisonStatistic::SigmaRatio,
                ComparisonStatistic::RelativeError,
                ComparisonStatistic::RSquared,
            ],
            error_method: RelativeErrorMethod::Mean,
            absolute: true,
        }
    }
}
#[derive(Debug, Clone)]
pub struct ComparisonStats {
    pub values: std::collections::BTreeMap<String, f64>,
    pub summary: String,
}
pub fn comparison_stats(
    t1: &[f64],
    y1: &[f64],
    t2: &[f64],
    y2: &[f64],
    options: &ComparisonOptions,
) -> Result<ComparisonStats> {
    if t1.len() != y1.len() || t2.len() != y2.len() {
        return Err(DvError::msg("comparison time and signal lengths differ"));
    }
    let mut values = std::collections::BTreeMap::new();
    let mut labels = Vec::new();
    for statistic in &options.statistics {
        let (name, value, label) = match statistic {
            ComparisonStatistic::SigmaRatio => {
                let denominator = nanstd(y1, 0);
                let ratio = if denominator == 0.0 {
                    f64::NAN
                } else {
                    nanstd(y2, 0) / denominator
                };
                ("sigRatio", ratio, format!("σ_est/σ_ref = {ratio:.3}"))
            }
            ComparisonStatistic::RelativeError => {
                let value =
                    mean_rel_err_time(t1, y1, t2, y2, options.error_method, options.absolute)?;
                ("eps", value, format!("ε = {value:.1}%"))
            }
            ComparisonStatistic::RSquared => {
                let value = rsquare(y2, y1, true)?.0;
                ("R2", value, format!("R² = {value:.3}"))
            }
            ComparisonStatistic::EquivalentLoadError => {
                let reference = crate::fatigue::equivalent_load(t1, y1, 5.0, 1.0, 100, "fatpack")?;
                let other = crate::fatigue::equivalent_load(t2, y2, 5.0, 1.0, 100, "fatpack")?;
                let value = (other - reference) / reference * 100.0;
                ("epsLeq", value, format!("ε Leq = {value:.1}%"))
            }
        };
        values.insert(name.to_owned(), value);
        labels.push(label);
    }
    Ok(ComparisonStats {
        values,
        summary: labels.join(" - "),
    })
}
