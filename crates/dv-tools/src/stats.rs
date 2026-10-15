//! Statistics, histogram/KDE densities and right-closed binning from pyDatView.
use dv_core::{
    dataframe::{Column, ColumnData, DataFrame},
    error::{DvError, Result},
};
use std::f64::consts::PI;

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
    let mut values: Vec<_> = y.iter().copied().filter(|v| !v.is_nan()).collect();
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n == 0 {
        f64::NAN
    } else if n % 2 == 0 {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    } else {
        values[n / 2]
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
