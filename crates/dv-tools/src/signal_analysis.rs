//! Filtering, interpolation, sampling and signal measurements from pyDatView.
use crate::stats::{linspace, nanmedian, nanmedian_in_place};
use dv_core::{
    dataframe::{Column, ColumnData, DataFrame},
    error::{DvError, Result},
};
use std::f64::consts::PI;

/// Numeric input metadata for typed processing forms.
///
/// Values must be finite apart from legacy sampler auto-bound sentinels.
/// `min` and `max` are optional bounds; the corresponding inclusive flags
/// specify whether the endpoint is allowed.
/// `increment` and `digits` are editing hints rather than validation rules.
/// Units of `x` refer to the currently selected x axis; Hz assumes x is in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub unit: &'static str,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_inclusive: bool,
    pub max_inclusive: bool,
    pub increment: f64,
    pub digits: usize,
    pub integer: bool,
    pub nonzero: bool,
}

impl ParameterInfo {
    const fn real(
        id: &'static str,
        label: &'static str,
        description: &'static str,
        unit: &'static str,
        default: f64,
        increment: f64,
        digits: usize,
    ) -> Self {
        Self {
            id,
            label,
            description,
            unit,
            default,
            min: None,
            max: None,
            min_inclusive: true,
            max_inclusive: true,
            increment,
            digits,
            integer: false,
            nonzero: false,
        }
    }

    const fn integer(mut self) -> Self {
        self.integer = true;
        self
    }

    const fn nonzero(mut self) -> Self {
        self.nonzero = true;
        self
    }

    const fn minimum(mut self, min: f64, inclusive: bool) -> Self {
        self.min = Some(min);
        self.min_inclusive = inclusive;
        self
    }

    const fn maximum(mut self, max: f64, inclusive: bool) -> Self {
        self.max = Some(max);
        self.max_inclusive = inclusive;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FilterDefinition {
    pub name: &'static str,
    pub parameter: f64,
    pub parameter_name: &'static str,
    pub range: (f64, f64),
    pub increment: f64,
    pub digits: usize,
}
impl FilterDefinition {
    /// Typed metadata without changing the legacy definition fields.
    pub fn parameter_info(&self) -> ParameterInfo {
        match self.name {
            "Moving average" => ParameterInfo::real(
                "window",
                "Window size",
                "Number of samples in the causal moving average.",
                "samples",
                self.parameter,
                self.increment,
                self.digits,
            )
            .integer()
            .minimum(1.0, true)
            .maximum(usize::MAX as f64, true),
            "Low pass 1st order" | "High pass 1st order" => ParameterInfo::real(
                "cutoff",
                "Cutoff frequency",
                "Cycles per x unit; Hz when x is measured in seconds.",
                "Hz",
                self.parameter,
                self.increment,
                self.digits.max(8),
            )
            .minimum(0.0, false),
            _ => {
                let mut info = ParameterInfo::real(
                    "parameter",
                    self.parameter_name,
                    "Filter parameter.",
                    "",
                    self.parameter,
                    self.increment,
                    self.digits,
                );
                info.min = Some(self.range.0);
                info.max = Some(self.range.1);
                info
            }
        }
    }
}

pub const FILTERS: [FilterDefinition; 3] = [
    FilterDefinition {
        name: "Moving average",
        parameter: 100.0,
        parameter_name: "Window Size",
        range: (1.0, 100000.0),
        increment: 1.0,
        digits: 0,
    },
    FilterDefinition {
        name: "Low pass 1st order",
        parameter: 1.0,
        parameter_name: "Cutoff Freq.",
        range: (0.0001, 100000.0),
        increment: 0.1,
        digits: 4,
    },
    FilterDefinition {
        name: "High pass 1st order",
        parameter: 0.1,
        parameter_name: "Cutoff Freq.",
        range: (0.0001, 100000.0),
        increment: 0.1,
        digits: 4,
    },
];
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    MovingAverage(usize),
    LowPass(f64),
    HighPass(f64),
}
impl Default for Filter {
    fn default() -> Self {
        Self::MovingAverage(100)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum Sampler {
    Replace(Vec<f64>),
    Insert(Vec<f64>),
    Remove(Vec<f64>),
    EveryN(isize),
    Linspace {
        min: f64,
        max: f64,
        count: usize,
    },
    TimeBased(f64),
    DeltaX {
        step: f64,
        min: Option<f64>,
        max: Option<f64>,
    },
}
impl Default for Sampler {
    fn default() -> Self {
        Self::EveryN(2)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct SamplerDefinition {
    pub name: &'static str,
    pub parameters: &'static [f64],
    pub parameter_name: &'static str,
}
/// Shape of the serialized sampler parameter vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SamplerParameterShape {
    /// A nonempty, arbitrary-length list; the single field describes each value.
    Vector,
    /// Exactly one value per field, in field order.
    Fixed,
    /// A step alone, or a step followed by both explicit bounds.
    OneOrThree,
}

impl SamplerDefinition {
    /// Form fields in serialized order. Vector shapes describe each element
    /// with one field; one-or-three shapes expose the step and both bounds.
    ///
    /// For `Delta x`, one value uses the data bounds. Three values provide the
    /// bounds explicitly; NaN in either bound also requests automatic bounds
    /// for compatibility with legacy configurations.
    pub fn parameter_info(&self) -> &'static [ParameterInfo] {
        match self.name {
            "Replace" => {
                const {
                    &[ParameterInfo::real(
                        "x",
                        "New x coordinate",
                        "Coordinates to sample, in the desired output order.",
                        "x units",
                        0.0,
                        0.1,
                        6,
                    )]
                }
            }
            "Insert" => {
                const {
                    &[ParameterInfo::real(
                        "x",
                        "Coordinate to insert",
                        "Coordinates to insert into the existing samples.",
                        "x units",
                        0.0,
                        0.1,
                        6,
                    )]
                }
            }
            "Remove" => {
                const {
                    &[ParameterInfo::real(
                        "x",
                        "Coordinate to remove",
                        "Remove coordinates less than 0.001 x units from each value.",
                        "x units",
                        0.0,
                        0.1,
                        6,
                    )]
                }
            }
            "Every n" => {
                const {
                    &[ParameterInfo::real(
                        "stride",
                        "Stride",
                        "Keep every n-th sample; a negative stride reverses the order.",
                        "samples",
                        2.0,
                        1.0,
                        0,
                    )
                    .integer()
                    .nonzero()
                    .minimum(isize::MIN as f64, true)
                    .maximum(isize::MAX as f64, true)]
                }
            }
            "Linspace" => {
                const {
                    &[
                        ParameterInfo::real(
                            "start",
                            "Start",
                            "First output coordinate.",
                            "x units",
                            0.0,
                            0.1,
                            6,
                        ),
                        ParameterInfo::real(
                            "end",
                            "End",
                            "Last output coordinate.",
                            "x units",
                            1.0,
                            0.1,
                            6,
                        ),
                        ParameterInfo::real(
                            "count",
                            "Number of points",
                            "Number of evenly spaced output coordinates.",
                            "points",
                            100.0,
                            1.0,
                            0,
                        )
                        .integer()
                        .minimum(1.0, true)
                        .maximum(usize::MAX as f64, true),
                    ]
                }
            }
            "Time-based" => {
                const {
                    &[ParameterInfo::real(
                        "interval",
                        "Sample interval",
                        "Width of each averaging interval, in x units.",
                        "x units",
                        0.01,
                        0.01,
                        6,
                    )
                    .minimum(0.0, false)]
                }
            }
            "Delta x" => {
                const {
                    &[
                        ParameterInfo::real(
                            "step",
                            "Step",
                            "Spacing of the output coordinates; must be nonzero.",
                            "x units",
                            0.1,
                            0.1,
                            6,
                        )
                        .nonzero(),
                        ParameterInfo::real(
                            "start",
                            "Start",
                            "Starting coordinate when explicit bounds are used.",
                            "x units",
                            0.0,
                            0.1,
                            6,
                        ),
                        ParameterInfo::real(
                            "end",
                            "End",
                            "Exclusive end coordinate when explicit bounds are used.",
                            "x units",
                            1.0,
                            0.1,
                            6,
                        ),
                    ]
                }
            }
            _ => &[],
        }
    }

    /// The number and organization of values accepted by this sampler.
    pub fn parameter_shape(&self) -> SamplerParameterShape {
        match self.name {
            "Replace" | "Insert" | "Remove" => SamplerParameterShape::Vector,
            "Delta x" => SamplerParameterShape::OneOrThree,
            _ => SamplerParameterShape::Fixed,
        }
    }

    /// Valid initial values for a newly selected sampler.
    ///
    /// Unlike the legacy `parameters` field, these are nonempty for vector
    /// samplers and use the one-value form of `Delta x`, so they are JSON-safe.
    pub fn default_parameters(&self) -> &'static [f64] {
        match self.name {
            "Replace" | "Insert" | "Remove" => &[0.0],
            "Delta x" => &[0.1],
            _ => self.parameters,
        }
    }
}

pub const SAMPLERS: [SamplerDefinition; 7] = [
    SamplerDefinition {
        name: "Replace",
        parameters: &[],
        parameter_name: "New x",
    },
    SamplerDefinition {
        name: "Insert",
        parameters: &[],
        parameter_name: "Insert list",
    },
    SamplerDefinition {
        name: "Remove",
        parameters: &[],
        parameter_name: "Remove list",
    },
    SamplerDefinition {
        name: "Every n",
        parameters: &[2.0],
        parameter_name: "n",
    },
    SamplerDefinition {
        name: "Linspace",
        parameters: &[0.0, 1.0, 100.0],
        parameter_name: "xmin, xmax, n",
    },
    SamplerDefinition {
        name: "Time-based",
        parameters: &[0.01],
        parameter_name: "Sample time (s)",
    },
    SamplerDefinition {
        name: "Delta x",
        parameters: &[0.1, f64::NAN, f64::NAN],
        parameter_name: "dx, xmin, xmax",
    },
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Extrapolation {
    #[default]
    Bounded,
    Nan,
}
fn equal_lengths(x: &[f64], y: &[f64]) -> Result<()> {
    if x.len() != y.len() {
        Err(DvError::msg("x and y must have the same length"))
    } else {
        Ok(())
    }
}
fn validate_interpolation_coordinates(xp: &[f64]) -> Result<()> {
    if xp.is_empty() || xp.iter().any(|v| !v.is_finite()) || xp.windows(2).any(|w| w[1] < w[0]) {
        return Err(DvError::msg(
            "interpolation coordinates must be finite, nonempty and ascending",
        ));
    }
    Ok(())
}

fn interpolation_index(v: f64, xp: &[f64], extrap: Extrapolation) -> (usize, usize, f64) {
    // NumPy chooses the last observation at duplicate coordinates.
    let after = xp.partition_point(|p| *p <= v);
    if v.is_nan() {
        return (0, 0, f64::NAN);
    }
    if after == 0 {
        return (
            0,
            0,
            if extrap == Extrapolation::Nan && v < xp[0] {
                f64::NAN
            } else {
                0.0
            },
        );
    }
    if after == xp.len() {
        let i = xp.len() - 1;
        return (
            i,
            i,
            if extrap == Extrapolation::Nan && v > xp[i] {
                f64::NAN
            } else {
                0.0
            },
        );
    }
    let before = after - 1;
    (before, after, (v - xp[before]) / (xp[after] - xp[before]))
}

fn interpolation_indices(
    x: &[f64],
    xp: &[f64],
    extrap: Extrapolation,
) -> Result<Vec<(usize, usize, f64)>> {
    validate_interpolation_coordinates(xp)?;
    Ok(x.iter()
        .map(|&v| interpolation_index(v, xp, extrap))
        .collect())
}
pub fn multi_interp(
    x: &[f64],
    xp: &[f64],
    fp: &[Vec<f64>],
    extrap: Extrapolation,
) -> Result<Vec<Vec<f64>>> {
    if fp.iter().any(|v| v.len() != xp.len()) {
        return Err(DvError::msg("all interpolation rows must match xp length"));
    }
    let indices = interpolation_indices(x, xp, extrap)?;
    Ok(fp
        .iter()
        .map(|row| {
            indices
                .iter()
                .map(|&(a, b, w)| {
                    if w == 0.0 {
                        row[a]
                    } else if w == 1.0 {
                        row[b]
                    } else {
                        (1.0 - w) * row[a] + w * row[b]
                    }
                })
                .collect()
        })
        .collect())
}
pub fn interpolate(x: &[f64], xp: &[f64], fp: &[f64], extrap: Extrapolation) -> Result<Vec<f64>> {
    equal_lengths(xp, fp)?;
    validate_interpolation_coordinates(xp)?;
    Ok(x.iter()
        .map(|&value| {
            let (a, b, w) = interpolation_index(value, xp, extrap);
            if w == 0.0 {
                fp[a]
            } else if w == 1.0 {
                fp[b]
            } else {
                (1.0 - w) * fp[a] + w * fp[b]
            }
        })
        .collect())
}
pub fn interp_array(
    x: f64,
    xp: &[f64],
    fp: &[Vec<f64>],
    extrap: Extrapolation,
) -> Result<Vec<f64>> {
    let result = multi_interp(&[x], xp, fp, extrap)?;
    Ok(result.into_iter().map(|v| v[0]).collect())
}
pub fn interp_df(
    x_new: &[f64],
    x_column: usize,
    df: &DataFrame,
    extrap: Extrapolation,
) -> Result<DataFrame> {
    if x_column >= df.ncols() {
        return Err(DvError::msg("interpolation column out of range"));
    }
    let xp = df.f64_col(x_column);
    let indices = interpolation_indices(x_new, &xp, extrap)?;
    let mut columns = Vec::with_capacity(df.ncols());
    for (i, column) in df.columns().iter().enumerate() {
        let data = if i == x_column {
            ColumnData::Float(x_new.to_vec())
        } else {
            match &column.data {
                ColumnData::Str(_) => {
                    return Err(DvError::msg(
                        "dataframe interpolation requires numeric columns",
                    ))
                }
                ColumnData::Float(v) | ColumnData::DateTime(v) => {
                    let values = indices
                        .iter()
                        .map(|&(a, b, w)| {
                            if w == 0.0 {
                                v[a]
                            } else if w == 1.0 {
                                v[b]
                            } else {
                                (1.0 - w) * v[a] + w * v[b]
                            }
                        })
                        .collect();
                    if column.is_datetime() {
                        ColumnData::DateTime(values)
                    } else {
                        ColumnData::Float(values)
                    }
                }
            }
        };
        columns.push(Column::new(column.name.clone(), data));
    }
    DataFrame::from_columns(columns)
}
fn outlier_mask(y: &[f64], m: f64) -> Vec<bool> {
    if m == 0.0 {
        return vec![true; y.len()];
    }
    let median = nanmedian(y);
    let mut distances: Vec<_> = y.iter().map(|v| (v - median).abs()).collect();
    let deviation = nanmedian_in_place(&mut distances);
    drop(distances);
    if deviation == 0.0 || deviation.is_nan() {
        vec![true; y.len()]
    } else {
        y.iter()
            .map(|v| (v - median).abs() / deviation < m)
            .collect()
    }
}
pub fn reject_outliers(y: &[f64], m: f64, replace_nan: bool) -> Vec<f64> {
    let mask = outlier_mask(y, m);
    y.iter()
        .zip(mask)
        .filter_map(|(&v, keep)| {
            if keep {
                Some(v)
            } else if replace_nan {
                Some(f64::NAN)
            } else {
                None
            }
        })
        .collect()
}
pub fn reject_outliers_xy(
    x: &[f64],
    y: &[f64],
    m: f64,
    replace_nan: bool,
) -> Result<(Vec<f64>, Vec<f64>)> {
    equal_lengths(x, y)?;
    let mask = outlier_mask(y, m);
    let new_x = if replace_nan {
        x.to_vec()
    } else {
        x.iter()
            .zip(&mask)
            .filter_map(|(&v, &keep)| if keep { Some(v) } else { None })
            .collect()
    };
    let new_y = y
        .iter()
        .zip(mask)
        .filter_map(|(&v, keep)| {
            if keep {
                Some(v)
            } else if replace_nan {
                Some(f64::NAN)
            } else {
                None
            }
        })
        .collect();
    Ok((new_x, new_y))
}
/// Causal average with the first observation repeated on the left.
pub fn moving_average(y: &[f64], n: usize) -> Result<Vec<f64>> {
    if n == 0 {
        return Err(DvError::msg("moving average window must be positive"));
    }
    if y.is_empty() {
        return Ok(Vec::new());
    }
    // Match NumPy cumulative-sum semantics, including propagation of NaNs.
    let mut cumulative = Vec::with_capacity(y.len());
    let mut sum = y[0] * (n - 1) as f64;
    for &v in y {
        sum += v;
        cumulative.push(sum);
    }
    // Work backwards so each cumulative value is read before being replaced.
    for i in (0..y.len()).rev() {
        let before = if i >= n {
            cumulative[i - n]
        } else {
            y[0] * i as f64
        };
        cumulative[i] = (cumulative[i] - before) / n as f64;
    }
    Ok(cumulative)
}
pub fn moving_average_conv(y: &[f64], n: usize) -> Result<Vec<f64>> {
    if n == 0 {
        return Err(DvError::msg("moving average window must be positive"));
    }
    if y.is_empty() {
        return Ok(Vec::new());
    }
    let mut result = Vec::with_capacity(y.len());
    let offset = n / 2;
    for i in 0..y.len() {
        let mut sum = 0.0;
        for k in 0..n {
            let index =
                (i as isize + k as isize - offset as isize).clamp(0, y.len() as isize - 1) as usize;
            sum += y[index];
        }
        result.push(sum / n as f64);
    }
    Ok(result)
}
fn filter_parameters(dt: f64, fc: f64) -> Result<()> {
    if !dt.is_finite() || dt <= 0.0 || !fc.is_finite() || fc <= 0.0 {
        Err(DvError::msg(
            "filter sample time and cutoff frequency must be positive",
        ))
    } else {
        Ok(())
    }
}
pub fn lowpass1(y: &[f64], dt: f64, fc: f64) -> Result<Vec<f64>> {
    filter_parameters(dt, fc)?;
    if y.is_empty() {
        return Ok(Vec::new());
    }
    let alpha = dt / (1.0 / (2.0 * PI * fc) + dt);
    let mut result = vec![0.0; y.len()];
    result[0] = y[0];
    for i in 1..y.len() {
        result[i] = alpha * y[i] + (1.0 - alpha) * result[i - 1];
    }
    Ok(result)
}
/// First-order high-pass followed by upstream's mean-restoring offset.
pub fn highpass1(y: &[f64], dt: f64, fc: f64) -> Result<Vec<f64>> {
    filter_parameters(dt, fc)?;
    if y.is_empty() {
        return Ok(Vec::new());
    }
    let tau = 1.0 / (2.0 * PI * fc);
    let alpha = tau / (tau + dt);
    let mut result = vec![0.0; y.len()];
    for i in 1..y.len() {
        result[i] = alpha * result[i - 1] + alpha * (y[i] - y[i - 1]);
    }
    let shift = (y.iter().sum::<f64>() - result.iter().sum::<f64>()) / y.len() as f64;
    for v in &mut result {
        *v += shift;
    }
    Ok(result)
}
pub fn apply_filter(x: &[f64], y: &[f64], filter: &Filter) -> Result<Vec<f64>> {
    equal_lengths(x, y)?;
    match filter {
        Filter::MovingAverage(n) => moving_average(y, *n),
        Filter::LowPass(fc) | Filter::HighPass(fc) => {
            if x.len() < 2 {
                return Err(DvError::msg(
                    "frequency filters require at least two samples",
                ));
            }
            if matches!(filter, Filter::LowPass(_)) {
                lowpass1(y, x[1] - x[0], *fc)
            } else {
                highpass1(y, x[1] - x[0], *fc)
            }
        }
    }
}
pub fn apply_filter_df(df: &DataFrame, x_column: usize, filter: &Filter) -> Result<DataFrame> {
    if x_column >= df.ncols() {
        return Err(DvError::msg("filter column out of range"));
    }
    let x = df.f64_col(x_column);
    let mut columns = Vec::with_capacity(df.ncols());
    for (i, c) in df.columns().iter().enumerate() {
        if i == x_column {
            columns.push(c.clone());
        } else {
            if matches!(&c.data, ColumnData::Str(_)) {
                return Err(DvError::msg("filtering requires numeric columns"));
            }
            columns.push(Column::float(
                c.name.clone(),
                apply_filter(&x, &c.as_f64(), filter)?,
            ));
        }
    }
    DataFrame::from_columns(columns)
}
fn stride_indices(length: usize, n: isize) -> Result<Vec<usize>> {
    if n == 0 {
        return Err(DvError::msg("sampling stride cannot be zero"));
    }
    if n > 0 {
        Ok((0..length).step_by(n as usize).collect())
    } else {
        Ok((0..length).rev().step_by(n.unsigned_abs()).collect())
    }
}
fn sampler_coordinates(x: &[f64], sampler: &Sampler) -> Result<Vec<f64>> {
    let coordinates = match sampler {
        Sampler::Replace(v) => {
            if v.is_empty() {
                return Err(DvError::msg("provide replacement coordinates"));
            }
            v.clone()
        }
        Sampler::Insert(v) => {
            if v.is_empty() {
                return Err(DvError::msg("provide coordinates to insert"));
            }
            let mut result = x.to_vec();
            result.extend_from_slice(v);
            result.sort_by(f64::total_cmp);
            result
        }
        Sampler::Remove(v) => {
            if v.is_empty() {
                return Err(DvError::msg("provide coordinates to remove"));
            }
            // Only the nearest coordinate on either side can be within the
            // removal tolerance. Non-finite candidates never matched before.
            let mut sorted: Vec<_> = v.iter().copied().filter(|v| v.is_finite()).collect();
            sorted.sort_unstable_by(f64::total_cmp);
            x.iter()
                .copied()
                .filter(|&value| {
                    let after = sorted.partition_point(|&v| v < value);
                    let near_after = sorted.get(after).is_some_and(|&v| (value - v).abs() < 1e-3);
                    let near_before = after > 0 && (value - sorted[after - 1]).abs() < 1e-3;
                    !near_after && !near_before
                })
                .collect()
        }
        Sampler::Linspace { min, max, count } => linspace(*min, *max, *count),
        Sampler::DeltaX { step, min, max } => {
            if !step.is_finite() || *step == 0.0 {
                return Err(DvError::msg("dx must be nonzero and finite"));
            }
            let lower = min.filter(|v| !v.is_nan()).unwrap_or_else(|| {
                x.iter()
                    .copied()
                    .filter(|v| !v.is_nan())
                    .fold(f64::INFINITY, f64::min)
            });
            let upper = max.filter(|v| !v.is_nan()).unwrap_or_else(|| {
                x.iter()
                    .copied()
                    .filter(|v| !v.is_nan())
                    .fold(f64::NEG_INFINITY, f64::max)
                    + step / 2.0
            });
            if !lower.is_finite() || !upper.is_finite() {
                return Err(DvError::msg("sampling bounds must be finite"));
            }
            let count = ((upper - lower) / step).ceil().max(0.0) as usize;
            if count == 0 {
                vec![lower, lower + step]
            } else {
                (0..count).map(|i| lower + i as f64 * step).collect()
            }
        }
        Sampler::EveryN(n) => stride_indices(x.len(), *n)?
            .into_iter()
            .map(|i| x[i])
            .collect(),
        Sampler::TimeBased(_) => {
            return Err(DvError::msg(
                "time-based coordinates require binned observations",
            ))
        }
    };
    if coordinates.iter().any(|v| !v.is_finite()) {
        return Err(DvError::msg("sampling coordinates must be finite"));
    }
    Ok(coordinates)
}
// Contiguous indices avoid an allocation per populated time bin.
struct TimeBins {
    offsets: Vec<usize>,
    indices: Vec<usize>,
}

fn time_bins(x: &[f64], sample_time: f64) -> Result<TimeBins> {
    if x.is_empty()
        || !sample_time.is_finite()
        || sample_time <= 0.0
        || x.iter().any(|v| !v.is_finite())
    {
        return Err(DvError::msg(
            "time-based sampling requires finite times and a positive sample time",
        ));
    }
    let origin = x.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let count = ((maximum - origin) / sample_time).floor() as usize + 1;
    let mut offsets = vec![0; count + 1];
    for &v in x {
        let bin = ((v - origin) / sample_time).floor() as usize;
        offsets[bin + 1] += 1;
    }
    for i in 0..count {
        offsets[i + 1] += offsets[i];
    }
    let mut next = offsets[..count].to_vec();
    let mut indices = vec![0; x.len()];
    for (i, &v) in x.iter().enumerate() {
        let bin = ((v - origin) / sample_time).floor() as usize;
        indices[next[bin]] = i;
        next[bin] += 1;
    }
    Ok(TimeBins { offsets, indices })
}

fn time_bin_values(y: &[f64], bins: &TimeBins) -> Vec<f64> {
    let mut result: Vec<_> = bins
        .offsets
        .windows(2)
        .map(|bounds| {
            let mut sum = 0.0;
            let mut n = 0;
            for &i in &bins.indices[bounds[0]..bounds[1]] {
                if !y[i].is_nan() {
                    sum += y[i];
                    n += 1;
                }
            }
            sum / n as f64
        })
        .collect();
    let mut last_good = None;
    for b in 0..result.len() {
        if result[b].is_nan() {
            continue;
        }
        if let Some(a) = last_good {
            for i in a + 1..b {
                result[i] = result[a] + (result[b] - result[a]) * (i - a) as f64 / (b - a) as f64;
            }
        }
        last_good = Some(b);
    }
    if let Some(last) = last_good {
        for i in last + 1..result.len() {
            result[i] = result[last];
        }
    }
    result
}
pub fn apply_sampler(x: &[f64], y: &[f64], sampler: &Sampler) -> Result<(Vec<f64>, Vec<f64>)> {
    equal_lengths(x, y)?;
    if let Sampler::EveryN(n) = sampler {
        if *n == 0 {
            return Err(DvError::msg("sampling stride cannot be zero"));
        }
        let stride = n.unsigned_abs();
        let count = if x.is_empty() {
            0
        } else {
            1 + (x.len() - 1) / stride
        };
        let mut sampled_x = Vec::with_capacity(count);
        let mut sampled_y = Vec::with_capacity(count);
        let mut push = |i: usize| {
            sampled_x.push(x[i]);
            sampled_y.push(y[i]);
        };
        if *n > 0 {
            (0..x.len()).step_by(stride).for_each(&mut push);
        } else {
            (0..x.len()).rev().step_by(stride).for_each(&mut push);
        }
        return Ok((sampled_x, sampled_y));
    }
    if let Sampler::TimeBased(dt) = sampler {
        let bins = time_bins(x, *dt)?;
        return Ok((time_bin_values(x, &bins), time_bin_values(y, &bins)));
    }
    let new_x = sampler_coordinates(x, sampler)?;
    let new_y = interpolate(&new_x, x, y, Extrapolation::Bounded)?;
    Ok((new_x, new_y))
}
pub fn apply_sampler_df(df: &DataFrame, x_column: usize, sampler: &Sampler) -> Result<DataFrame> {
    if x_column >= df.ncols() {
        return Err(DvError::msg("sampling column out of range"));
    }
    if let Sampler::EveryN(n) = sampler {
        return Ok(df.take_rows(&stride_indices(df.nrows(), *n)?));
    }
    let x = df.f64_col(x_column);
    if let Sampler::TimeBased(dt) = sampler {
        let bins = time_bins(&x, *dt)?;
        let mut columns = Vec::with_capacity(df.ncols());
        for c in df.columns() {
            if matches!(&c.data, ColumnData::Str(_)) {
                return Err(DvError::msg("time resampling requires numeric columns"));
            }
            columns.push(Column::float(
                c.name.clone(),
                time_bin_values(&c.as_f64(), &bins),
            ));
        }
        return DataFrame::from_columns(columns);
    }
    interp_df(
        &sampler_coordinates(&x, sampler)?,
        x_column,
        df,
        Extrapolation::Bounded,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossingDirection {
    Up,
    Down,
}
#[derive(Debug, Clone, Default)]
pub struct ZeroCrossings {
    pub positions: Vec<f64>,
    pub indices: Vec<usize>,
    pub signs: Vec<i8>,
}
pub fn zero_crossings(
    y: &[f64],
    x: Option<&[f64]>,
    direction: Option<CrossingDirection>,
    bouncing_zero: bool,
) -> Result<ZeroCrossings> {
    let generated;
    let x = if let Some(x) = x {
        x
    } else {
        generated = (0..y.len()).map(|i| i as f64).collect::<Vec<_>>();
        &generated
    };
    equal_lengths(x, y)?;
    if x.windows(2).any(|w| w[1] <= w[0]) {
        return Err(DvError::msg("x values need to be in ascending order"));
    }
    let mut crossings = Vec::new();
    for i in 0..y.len().saturating_sub(1) {
        if y[i] * y[i + 1] < 0.0 {
            crossings.push((
                x[i] - y[i] * (x[i + 1] - x[i]) / (y[i + 1] - y[i]),
                i,
                if y[i + 1] > y[i] { 1 } else { -1 },
            ));
        }
    }
    for i in 1..y.len().saturating_sub(1) {
        if y[i] == 0.0 && (bouncing_zero || y[i - 1] * y[i + 1] < 0.0) {
            crossings.push((
                x[i],
                i,
                if y[i + 1] > 0.0 {
                    1
                } else if y[i + 1] < 0.0 {
                    -1
                } else {
                    0
                },
            ));
        }
    }
    crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut result = ZeroCrossings::default();
    for (position, index, sign) in crossings {
        if direction.is_some_and(|d| match d {
            CrossingDirection::Up => sign != 1,
            CrossingDirection::Down => sign != -1,
        }) {
            continue;
        }
        result.positions.push(position);
        result.indices.push(index);
        result.signs.push(sign);
    }
    Ok(result)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CorrelationMethod {
    #[default]
    Pearson,
    Manual,
}
pub fn correlation(
    x: &[f64],
    max_lags: usize,
    dt: f64,
    method: CorrelationMethod,
) -> Result<(Vec<f64>, Vec<f64>)> {
    if max_lags == 0 || max_lags > x.len() {
        return Err(DvError::msg(
            "correlation lags must be between one and signal length",
        ));
    }
    let mean = x.iter().sum::<f64>() / x.len() as f64;
    let variance = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / x.len() as f64;
    let mut values = vec![1.0];
    for lag in 1..max_lags {
        let a = &x[..x.len() - lag];
        let b = &x[lag..];
        let n = a.len() as f64;
        let value = match method {
            CorrelationMethod::Manual => {
                a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>() / (n * variance)
            }
            CorrelationMethod::Pearson => {
                let ma = a.iter().sum::<f64>() / n;
                let mb = b.iter().sum::<f64>() / n;
                let covariance = a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| (a - ma) * (b - mb))
                    .sum::<f64>();
                covariance
                    / (a.iter().map(|a| (a - ma).powi(2)).sum::<f64>()
                        * b.iter().map(|b| (b - mb).powi(2)).sum::<f64>())
                    .sqrt()
            }
        };
        values.push(value);
    }
    Ok((values, (0..max_lags).map(|i| i as f64 * dt).collect()))
}
pub fn amplitude(y: &[f64], t: Option<&[f64]>, period: Option<f64>) -> Result<f64> {
    fn half_range(y: impl Iterator<Item = f64>) -> f64 {
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for v in y {
            if v.is_nan() {
                return f64::NAN;
            }
            min = min.min(v);
            max = max.max(v);
        }
        if min == f64::INFINITY {
            f64::NAN
        } else {
            (max - min) / 2.0
        }
    }
    if y.is_empty() {
        return Err(DvError::msg("amplitude requires observations"));
    }
    if let (Some(t), Some(period)) = (t, period) {
        equal_lengths(t, y)?;
        if !period.is_finite() || period <= 0.0 {
            return Err(DvError::msg("period must be positive"));
        }
        let duration = t[t.len() - 1] - t[0];
        if duration > period {
            let periods = (duration / period) as usize;
            let mut total = 0.0;
            if t.iter().all(|v| v.is_finite()) && t.windows(2).all(|w| w[0] <= w[1]) {
                // Inclusive period boundaries deliberately belong to both periods.
                for i in 0..periods {
                    let start = i as f64 * period;
                    let end = (i + 1) as f64 * period;
                    let first = t.partition_point(|v| *v - t[0] < start);
                    let after = t.partition_point(|v| *v - t[0] <= end);
                    total += half_range(y[first..after].iter().copied());
                }
            } else {
                for i in 0..periods {
                    total += half_range(
                        t.iter()
                            .zip(y)
                            .filter(|(time, _)| {
                                **time - t[0] >= i as f64 * period
                                    && **time - t[0] <= (i + 1) as f64 * period
                            })
                            .map(|(_, y)| *y),
                    );
                }
            }
            return Ok(total / periods as f64);
        }
    }
    Ok(half_range(y.iter().copied()))
}
pub fn find_time_offset(t: &[f64], f: &[f64], g: &[f64]) -> Result<f64> {
    equal_lengths(t, f)?;
    equal_lengths(f, g)?;
    if t.len() < 2 {
        return Err(DvError::msg("time offset requires at least two samples"));
    }
    let n = f.len();
    let mf = f.iter().sum::<f64>() / n as f64;
    let mg = g.iter().sum::<f64>() / n as f64;
    let sf = (f.iter().map(|v| (v - mf).powi(2)).sum::<f64>() / n as f64).sqrt();
    let sg = (g.iter().map(|v| (v - mg).powi(2)).sum::<f64>() / n as f64).sqrt();
    if sf == 0.0 || sg == 0.0 {
        return Err(DvError::msg(
            "time offset is undefined for a constant signal",
        ));
    }
    let nfft = (2 * n - 1).next_power_of_two();
    let mut a = vec![rustfft::num_complex::Complex::new(0.0, 0.0); nfft];
    let mut b = a.clone();
    for i in 0..n {
        a[i].re = (f[i] - mf) / sf;
        b[i].re = (g[i] - mg) / sg;
    }
    let mut planner = rustfft::FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(nfft);
    fft.process(&mut a);
    fft.process(&mut b);
    for i in 0..nfft {
        a[i] *= b[i].conj();
    }
    planner.plan_fft_inverse(nfft).process(&mut a);
    let mut best = f64::NEG_INFINITY;
    let mut lag_best = 0;
    for lag in 1 - n as isize..n as isize {
        let i = if lag < 0 {
            (nfft as isize + lag) as usize
        } else {
            lag as usize
        };
        if a[i].re > best {
            best = a[i].re;
            lag_best = lag;
        }
    }
    Ok(lag_best as f64 * (t[1] - t[0]))
}
pub fn phase_shift(
    a: &[f64],
    b: &[f64],
    t: &[f64],
    omega: f64,
    start: f64,
    degrees: bool,
) -> Result<f64> {
    equal_lengths(t, a)?;
    equal_lengths(a, b)?;
    let indices: Vec<_> = t
        .iter()
        .enumerate()
        .filter_map(|(i, v)| if *v >= start { Some(i) } else { None })
        .collect();
    let time: Vec<_> = indices.iter().map(|&i| t[i]).collect();
    let aa: Vec<_> = indices.iter().map(|&i| a[i]).collect();
    let bb: Vec<_> = indices.iter().map(|&i| b[i]).collect();
    let mut phase = (find_time_offset(&time, &aa, &bb)? * omega).rem_euclid(2.0 * PI);
    if phase > 0.8 * PI {
        phase -= 2.0 * PI;
    }
    if degrees {
        phase *= 180.0 / PI;
        if phase < -190.0 {
            phase += 360.0;
        }
    }
    Ok(phase)
}
pub fn input_output_amplitude_phase(
    t: &[f64],
    u: &[f64],
    y: &[f64],
    omega: f64,
    input_amplitude: Option<f64>,
    degrees: bool,
) -> Result<(f64, f64)> {
    let period = 2.0 * PI / omega;
    let output = amplitude(y, Some(t), Some(period))?;
    let input = match input_amplitude {
        Some(a) => a,
        None => amplitude(u, Some(t), Some(period))?,
    };
    Ok((output / input, phase_shift(u, y, t, omega, 0.0, degrees)?))
}
pub fn convolution_integral(t: &[f64], f: &[f64], g: &[f64]) -> Result<Vec<f64>> {
    equal_lengths(t, f)?;
    equal_lengths(t, g)?;
    if t.len() < 2 {
        return Err(DvError::msg("convolution requires two time samples"));
    }
    let dt = t[1] - t[0];
    if !dt.is_finite()
        || dt == 0.0
        || t.windows(2)
            .any(|w| (((w[1] - w[0]) / dt) * 1000.0).round() != 1000.0)
    {
        return Err(DvError::msg(
            "convolution integral requires uniform time samples",
        ));
    }
    let nfft = (2 * t.len() - 1).next_power_of_two();
    let mut a = vec![rustfft::num_complex::Complex::new(0.0, 0.0); nfft];
    let mut b = a.clone();
    for i in 0..t.len() {
        a[i].re = f[i];
        b[i].re = g[i];
    }
    let mut planner = rustfft::FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(nfft);
    fft.process(&mut a);
    fft.process(&mut b);
    for i in 0..nfft {
        a[i] *= b[i];
    }
    planner.plan_fft_inverse(nfft).process(&mut a);
    Ok(a[..t.len()]
        .iter()
        .map(|v| v.re * dt / nfft as f64)
        .collect())
}
#[derive(Debug, Clone)]
pub struct IntervalOptions {
    pub min_length: usize,
    pub forgiving_jump: bool,
    pub remove_small_relative: bool,
    pub remove_small_factor: f64,
    pub merge_close_relative: bool,
    pub merge_close_factor: f64,
}
impl Default for IntervalOptions {
    fn default() -> Self {
        Self {
            min_length: 1,
            forgiving_jump: true,
            remove_small_relative: true,
            remove_small_factor: 0.1,
            merge_close_relative: false,
            merge_close_factor: 0.2,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    pub start: usize,
    pub end: usize,
}
impl Interval {
    pub fn len(&self) -> usize {
        self.end - self.start + 1
    }
    pub fn is_empty(&self) -> bool {
        false
    }
}
pub fn intervals(mask: &[bool], options: &IntervalOptions) -> Vec<Interval> {
    let indices: Vec<_> = mask
        .iter()
        .enumerate()
        .filter_map(|(i, v)| if *v { Some(i) } else { None })
        .collect();
    let min = options.min_length.max(1);
    let jump = if options.forgiving_jump { min } else { 1 };
    let mut result = Vec::<Interval>::new();
    for i in indices {
        if let Some(last) = result.last_mut() {
            if i - last.end <= jump {
                last.end = i;
                continue;
            }
        }
        result.push(Interval { start: i, end: i });
    }
    result.retain(|v| v.len() >= min);
    if options.remove_small_relative {
        let largest = result.iter().map(Interval::len).max().unwrap_or(0) as f64;
        result.retain(|v| v.len() as f64 >= largest * options.remove_small_factor);
    }
    if options.merge_close_relative && result.len() > 2 {
        let distance = result
            .windows(2)
            .map(|w| w[1].start - w[0].end)
            .max()
            .unwrap();
        let new_min = ((distance as f64 * options.merge_close_factor) as usize).max(min);
        if new_min > 1 {
            return intervals(
                mask,
                &IntervalOptions {
                    min_length: new_min,
                    forgiving_jump: true,
                    merge_close_relative: false,
                    ..options.clone()
                },
            );
        }
    }
    result
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeakMethod {
    #[default]
    Intervals,
    Derivative,
}
#[derive(Debug, Clone)]
pub struct PeakOptions {
    pub threshold: f64,
    pub absolute_threshold: bool,
    pub method: PeakMethod,
    pub min_length: usize,
    pub merge_close_relative: bool,
}
impl Default for PeakOptions {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            absolute_threshold: true,
            method: PeakMethod::Intervals,
            min_length: 3,
            merge_close_relative: true,
        }
    }
}
pub fn peaks(y: &[f64], options: &PeakOptions) -> Vec<usize> {
    let min = y.iter().copied().fold(f64::INFINITY, f64::min);
    let max = y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let threshold = if options.absolute_threshold {
        options.threshold
    } else {
        options.threshold * (max - min) + min
    };
    match options.method {
        PeakMethod::Intervals => {
            let mask = y.iter().map(|v| *v > threshold).collect::<Vec<_>>();
            intervals(
                &mask,
                &IntervalOptions {
                    min_length: options.min_length,
                    merge_close_relative: options.merge_close_relative,
                    ..Default::default()
                },
            )
            .iter()
            .map(|interval| {
                let mut peak = interval.start;
                for i in interval.start + 1..=interval.end {
                    if y[i] > y[peak] {
                        peak = i;
                    }
                }
                peak
            })
            .collect()
        }
        PeakMethod::Derivative => {
            let mut candidates = Vec::new();
            let mut i = 1;
            while i + 1 < y.len() {
                if y[i] > threshold && y[i] > y[i - 1] {
                    let mut end = i;
                    while end + 1 < y.len() && y[end + 1] == y[i] {
                        end += 1;
                    }
                    if end + 1 < y.len() && y[end] > y[end + 1] {
                        candidates.push(i + (end - i) / 2);
                    }
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
            candidates.sort_by(|&a, &b| y[b].total_cmp(&y[a]));
            let mut selected = std::collections::BTreeSet::<usize>::new();
            for i in candidates {
                let too_close_before = selected
                    .range(..i)
                    .next_back()
                    .is_some_and(|&j| i - j <= options.min_length);
                let too_close_after = selected
                    .range(i..)
                    .next()
                    .is_some_and(|&j| j - i <= options.min_length);
                if !too_close_before && !too_close_after {
                    selected.insert(i);
                }
            }
            selected.into_iter().collect()
        }
    }
}
pub fn step(t: &[f64], start: f64, value_at_step: f64, amplitude: f64) -> Vec<f64> {
    t.iter()
        .map(|v| {
            amplitude
                * if *v < start {
                    0.0
                } else if *v > start {
                    1.0
                } else {
                    value_at_step
                }
        })
        .collect()
}
pub fn ramp(t: &[f64], start: f64, amplitude: f64) -> Vec<f64> {
    t.iter()
        .map(|v| {
            if *v >= start {
                amplitude * (v - start)
            } else {
                0.0
            }
        })
        .collect()
}
pub fn hat(t: &[f64], width: f64, start: f64, amplitude: f64) -> Vec<f64> {
    t.iter()
        .map(|v| {
            if (v - start).abs() <= width / 2.0 {
                amplitude * (1.0 - (2.0 * (v - start) / width).abs())
            } else {
                0.0
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn causal_average() {
        assert_eq!(
            moving_average(&[1., 2., 3., 4.], 3).unwrap(),
            vec![1., 4. / 3., 2., 3.]
        );
    }
    #[test]
    fn interpolation_bounds() {
        assert_eq!(
            interpolate(
                &[-1., 0.5, 2.],
                &[0., 1.],
                &[2., 4.],
                Extrapolation::Bounded
            )
            .unwrap(),
            vec![2., 3., 4.]
        );
    }
    #[test]
    fn negative_stride() {
        let (x, y) = apply_sampler(&[0., 1., 2.], &[3., 4., 5.], &Sampler::EveryN(-2)).unwrap();
        assert_eq!(x, vec![2., 0.]);
        assert_eq!(y, vec![5., 3.]);
        assert_eq!(
            apply_sampler(&[0., 1., 2.], &[3., 4., 5.], &Sampler::EveryN(2)).unwrap(),
            (vec![0., 2.], vec![3., 5.])
        );
        assert_eq!(
            apply_sampler(&[0., 1., 2.], &[3., 4., 5.], &Sampler::EveryN(isize::MIN)).unwrap(),
            (vec![2.], vec![5.])
        );
        assert_eq!(
            apply_sampler(&[], &[], &Sampler::EveryN(2)).unwrap(),
            (vec![], vec![])
        );
        assert!(apply_sampler(&[], &[], &Sampler::EveryN(0)).is_err());
    }
    #[test]
    fn outlier_nan() {
        let y = reject_outliers(&[1., 2., 3., 100.], 2., true);
        assert!(y[3].is_nan());
        assert_eq!(y[0], 1.);
    }

    #[test]
    fn outlier_mask_preserves_median_deviation_edge_cases() {
        fn reference(y: &[f64], m: f64) -> Vec<bool> {
            if m == 0.0 {
                return vec![true; y.len()];
            }
            let median = nanmedian(y);
            let distances: Vec<_> = y.iter().map(|v| (v - median).abs()).collect();
            let deviation = nanmedian(&distances);
            if deviation == 0.0 || deviation.is_nan() {
                vec![true; y.len()]
            } else {
                distances.iter().map(|d| d / deviation < m).collect()
            }
        }
        for y in [
            &[][..],
            &[1., 2., f64::NAN, 3., 100.],
            &[5., 5., f64::NAN, 5.],
            &[f64::NEG_INFINITY, -1., 1., f64::INFINITY],
        ] {
            for m in [0., 1., 5., f64::INFINITY, f64::NAN] {
                assert_eq!(outlier_mask(y, m), reference(y, m));
            }
        }
    }

    #[test]
    fn crossings() {
        let z = zero_crossings(&[-1., 0., 1., -1.], None, None, false).unwrap();
        assert_eq!(z.positions, vec![1., 2.5]);
        assert_eq!(z.signs, vec![1, -1]);
    }
    #[test]
    fn highpass_keeps_mean() {
        let y = [1., 2., 4., 8.];
        let filtered = highpass1(&y, 0.1, 1.).unwrap();
        assert!((filtered.iter().sum::<f64>() - 15.).abs() < 1e-12);
    }
    #[test]
    fn interpolation_exact_samples_do_not_mix_nan_neighbors() {
        let y = interpolate(
            &[0., 1., 2.],
            &[0., 1., 1., 2.],
            &[2., f64::NAN, 3., 4.],
            Extrapolation::Bounded,
        )
        .unwrap();
        assert_eq!(y, vec![2., 3., 4.]);
    }

    #[test]
    fn typed_metadata_describes_usable_sampler_shapes_and_bounds() {
        fn assert_valid(info: ParameterInfo, value: f64) {
            assert!(value.is_finite(), "{}", info.label);
            if let Some(min) = info.min {
                assert!(value > min || (info.min_inclusive && value == min));
            }
            if let Some(max) = info.max {
                assert!(value < max || (info.max_inclusive && value == max));
            }
            if info.integer {
                assert_eq!(value.fract(), 0.0);
            }
            if info.nonzero {
                assert_ne!(value, 0.0);
            }
        }

        for filter in FILTERS {
            let info = filter.parameter_info();
            assert_valid(info, info.default);
            assert_eq!(info.default, filter.parameter);
        }
        for sampler in SAMPLERS {
            let fields = sampler.parameter_info();
            let defaults = sampler.default_parameters();
            match sampler.parameter_shape() {
                SamplerParameterShape::Vector => {
                    assert_eq!(fields.len(), 1);
                    assert!(!defaults.is_empty());
                    for &value in defaults {
                        assert_valid(fields[0], value);
                    }
                }
                SamplerParameterShape::Fixed => {
                    assert_eq!(fields.len(), defaults.len());
                    for (&field, &value) in fields.iter().zip(defaults) {
                        assert_valid(field, value);
                    }
                }
                SamplerParameterShape::OneOrThree => {
                    assert_eq!(fields.len(), 3);
                    assert_eq!(defaults.len(), 1);
                    for field in fields {
                        assert_valid(*field, field.default);
                    }
                    let (x, _) = apply_sampler(
                        &[0., 0.5, 1.],
                        &[0., 1., 2.],
                        &Sampler::DeltaX {
                            step: fields[0].default,
                            min: Some(fields[1].default),
                            max: Some(fields[2].default),
                        },
                    )
                    .unwrap();
                    assert_eq!(x.len(), 10);
                }
            }
        }
        assert_eq!(SAMPLERS[6].parameters.len(), 3);
        assert!(SAMPLERS[6].parameters[1].is_nan());
        assert_eq!(SAMPLERS[6].default_parameters(), &[0.1]);
        assert!(SAMPLERS[3].parameter_info()[0].nonzero);
        assert!(!FILTERS[1].parameter_info().min_inclusive);
    }

    #[test]
    fn interpolation_paths_agree_on_duplicates_bounds_and_nan() {
        let queries = [f64::NEG_INFINITY, -1., 0., 0.5, 1., 1.5, 2., 3., f64::NAN];
        let xp = [0., 1., 1., 2.];
        let fp = [2., f64::NAN, 3., 4.];
        for extrap in [Extrapolation::Bounded, Extrapolation::Nan] {
            let single = interpolate(&queries, &xp, &fp, extrap).unwrap();
            let multi = multi_interp(&queries, &xp, &[fp.to_vec()], extrap).unwrap();
            for (&a, &b) in single.iter().zip(&multi[0]) {
                assert!(a == b || (a.is_nan() && b.is_nan()));
            }
            assert_eq!(single[2], 2.);
            assert_eq!(single[4], 3.);
            assert_eq!(single[6], 4.);
            assert!(single[8].is_nan());
            assert_eq!(single[1].is_nan(), extrap == Extrapolation::Nan);
            assert_eq!(single[7].is_nan(), extrap == Extrapolation::Nan);
        }
        assert!(interpolate(&[0.], &[], &[], Extrapolation::Bounded).is_err());
        assert!(interpolate(&[0.], &[1., 0.], &[1., 2.], Extrapolation::Bounded).is_err());
    }

    #[test]
    fn moving_average_keeps_cumulative_nan_semantics() {
        assert_eq!(
            moving_average(&[2., 4., 6.], 4).unwrap(),
            vec![2., 2.5, 3.5]
        );
        let result = moving_average(&[1., 2., f64::NAN, 4.], 2).unwrap();
        assert_eq!(&result[..2], &[1., 1.5]);
        assert!(result[2..].iter().all(|v| v.is_nan()));
    }

    #[test]
    fn removing_coordinates_matches_strict_tolerance_and_nonfinite_candidates() {
        let x = [-1., -0.001, -0.0005, -0.0, 0.0, 0.001, 1., 1.0005, 4., 5.];
        let remove = vec![4.0006, f64::NAN, f64::INFINITY, 1., -0.0];
        let expected: Vec<_> = x
            .iter()
            .copied()
            .filter(|x| !remove.iter().any(|v| (x - v).abs() < 1e-3))
            .collect();
        assert_eq!(
            sampler_coordinates(&x, &Sampler::Remove(remove)).unwrap(),
            expected
        );
    }

    #[test]
    fn time_sampling_keeps_input_order_and_fills_gaps() {
        let x = [3., 0., 1., 1.5, 4., 7.];
        let y = [f64::NAN, f64::NAN, 2., 4., 8., f64::NAN];
        let (new_x, new_y) = apply_sampler(&x, &y, &Sampler::TimeBased(1.)).unwrap();
        assert_eq!(new_x, vec![0., 1.25, 2.125, 3., 4., 5., 6., 7.]);
        assert!(new_y[0].is_nan());
        let expected = [3., 3. + 5. / 3., 3. + 10. / 3., 8., 8., 8., 8.];
        for (&actual, expected) in new_y[1..].iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn amplitude_includes_shared_period_endpoints_and_unsorted_times() {
        let sorted_t = [0., 1., 2., 3., 4.];
        let sorted_y = [0., 1., 4., 3., 0.];
        let unsorted_t = [0., 2., 1., 3., 4.];
        let unsorted_y = [0., 4., 1., 3., 0.];
        assert_eq!(amplitude(&sorted_y, Some(&sorted_t), Some(2.)).unwrap(), 2.);
        assert_eq!(
            amplitude(&unsorted_y, Some(&unsorted_t), Some(2.)).unwrap(),
            2.
        );
        assert!(amplitude(&[0., 1.], Some(&[0., 4.]), Some(1.))
            .unwrap()
            .is_nan());
    }

    #[test]
    fn derivative_peaks_resolve_ties_and_distance() {
        let options = PeakOptions {
            method: PeakMethod::Derivative,
            threshold: 0.,
            min_length: 2,
            ..Default::default()
        };
        assert_eq!(
            peaks(&[0., 5., 0., 4., 0., 5., 0., 3., 0.], &options),
            vec![1, 5]
        );
        let options = PeakOptions {
            min_length: 0,
            ..options
        };
        assert_eq!(peaks(&[0., 5., 5., 0., 5., 5., 0.], &options), vec![1, 4]);
    }
}
