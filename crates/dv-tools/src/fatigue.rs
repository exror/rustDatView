//! Rainflow counting and damage equivalent loads, following pyDatView's ASTM and WINDAP algorithms.
use dv_core::error::{DvError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RainflowMethod {
    Windap,
    Astm,
    Fatpack,
}
impl RainflowMethod {
    pub fn parse(method: &str) -> Result<Self> {
        match method {
            "rainflow_windap" | "windap" => Ok(Self::Windap),
            "rainflow_astm" | "astm" => Ok(Self::Astm),
            "fatpack" => Ok(Self::Fatpack),
            _ => Err(DvError::msg(format!(
                "Unknown rainflow algorithm: {method}"
            ))),
        }
    }
}
#[derive(Debug, Clone)]
pub enum Bins {
    Count(usize),
    Edges(Vec<f64>),
}
#[derive(Debug, Clone)]
pub struct FatigueOptions {
    pub m: f64,
    pub teq: f64,
    pub bins: Bins,
    pub method: RainflowMethod,
    pub mean_bin: bool,
    pub bin_start_at_zero: bool,
}
impl Default for FatigueOptions {
    fn default() -> Self {
        Self {
            m: 3.0,
            teq: 1.0,
            bins: Bins::Count(100),
            method: RainflowMethod::Windap,
            mean_bin: true,
            bin_start_at_zero: false,
        }
    }
}
/// Each entry is a half cycle, and range is peak-to-peak, not peak amplitude.
#[derive(Debug, Clone, Copy)]
pub struct HalfCycle {
    pub range: f64,
    pub mean: f64,
}
#[derive(Debug, Clone)]
pub struct RangeCount {
    pub cycles: Vec<f64>,
    pub ranges: Vec<f64>,
    pub bin_edges: Vec<f64>,
}
#[derive(Debug, Clone)]
pub struct EquivalentLoad {
    pub load: f64,
    pub ranges: Vec<f64>,
    pub cycles: Vec<f64>,
    pub bin_edges: Vec<f64>,
    pub damage: Vec<f64>,
}
#[derive(Debug, Clone)]
pub struct CycleMatrix {
    pub cycles: Vec<Vec<f64>>,
    pub amplitude_mean: Vec<f64>,
    pub amplitude_edges: Vec<f64>,
    pub mean_mean: Vec<f64>,
    pub mean_edges: Vec<f64>,
}

fn check_signal(signal: &[f64]) -> Result<()> {
    if signal.len() < 2 || signal.iter().any(|v| !v.is_finite()) {
        return Err(DvError::msg(
            "Rainflow requires at least two finite signal values",
        ));
    }
    if signal.iter().all(|v| *v == signal[0]) {
        return Err(DvError::msg(
            "Signal is constant, cannot compute rainflow ranges",
        ));
    }
    Ok(())
}
pub fn find_extremes(signal: &[f64]) -> Vec<f64> {
    if signal.len() < 2 {
        return signal.to_vec();
    }
    let mut out = vec![signal[0]];
    let mut direction = 0.0_f64;
    for i in 1..signal.len() {
        let d = signal[i] - signal[i - 1];
        if d == 0.0 {
            continue;
        }
        if direction != 0.0 && d.signum() != direction {
            out.push(signal[i - 1]);
        }
        direction = d.signum();
    }
    out.push(signal[signal.len() - 1]);
    out
}
pub fn rainflowcount(extremes: &[f64]) -> Vec<HalfCycle> {
    let mut stack = Vec::with_capacity(extremes.len());
    let mut cycles = Vec::new();
    for &v in extremes {
        stack.push(v);
        while stack.len() > 2 {
            let n = stack.len();
            let a: f64 = stack[n - 3];
            let b: f64 = stack[n - 2];
            if (a - b).abs() > (b - stack[n - 1]).abs() {
                break;
            }
            let cycle = HalfCycle {
                range: (a - b).abs(),
                mean: (a + b) / 2.0,
            };
            if n == 3 {
                stack.remove(0);
            } else {
                let last = stack[n - 1];
                stack.truncate(n - 3);
                stack.push(last);
            }
            if cycle.range > 0.0 {
                cycles.push(cycle);
                if n > 3 {
                    cycles.push(cycle);
                }
            }
        }
    }
    for p in stack.windows(2) {
        if p[0] != p[1] {
            cycles.push(HalfCycle {
                range: (p[0] - p[1]).abs(),
                mean: (p[0] + p[1]) / 2.0,
            });
        }
    }
    cycles
}
pub fn rainflow_astm(signal: &[f64]) -> Result<Vec<HalfCycle>> {
    check_signal(signal)?;
    Ok(rainflowcount(&find_extremes(signal)))
}
/// Thresholded reversals used by WINDAP. Integer quantization is performed by its caller.
pub fn peak_trough(x: &[f64], threshold: f64) -> Vec<f64> {
    if x.is_empty() {
        return Vec::new();
    }
    let mut peak = x[0];
    let mut trough = x[0];
    let mut state = 0;
    let mut out = Vec::new();
    for &v in &x[1..] {
        match state {
            0 => {
                if v > peak {
                    peak = v;
                    if peak - trough >= threshold {
                        out.push(trough);
                        state = 1;
                    }
                } else if v < trough {
                    trough = v;
                    if peak - trough >= threshold {
                        out.push(peak);
                        state = -1;
                    }
                }
            }
            1 => {
                if v > peak {
                    peak = v;
                } else if peak - v >= threshold {
                    out.push(peak);
                    trough = v;
                    state = -1;
                }
            }
            _ => {
                if v < trough {
                    trough = v;
                } else if v - trough >= threshold {
                    out.push(trough);
                    peak = v;
                    state = 1;
                }
            }
        }
    }
    if state == 0 {
        out.push(0.0);
        out.push(((trough + peak) / 2.0).trunc());
    } else {
        out.push(if state == 1 { peak } else { trough });
    }
    out
}
pub fn pair_range_amplitude_mean(x: &[f64]) -> Vec<HalfCycle> {
    if x.len() < 2 {
        return Vec::new();
    }
    let offset = x.iter().copied().fold(f64::INFINITY, f64::min);
    let mut s = Vec::with_capacity(x.len());
    let mut out = Vec::new();
    for &v in x {
        s.push(v - offset);
        while s.len() >= 4 {
            let n = s.len();
            let (a, b, c, d) = (s[n - 4], s[n - 3], s[n - 2], s[n - 1]);
            if !((b > a && c >= a && d >= b) || (b < a && c <= a && d <= b)) {
                break;
            }
            let cycle = HalfCycle {
                range: (b - c).abs(),
                mean: (b + c) / 2.0,
            };
            out.push(cycle);
            out.push(cycle);
            s.truncate(n - 3);
            s.push(d);
        }
    }
    for p in s.windows(2) {
        out.push(HalfCycle {
            range: (p[1] - p[0]).abs(),
            mean: (p[1] + p[0]) / 2.0,
        });
    }
    out
}
pub fn rainflow_windap(signal: &[f64], levels: f64, threshold: f64) -> Result<Vec<HalfCycle>> {
    check_signal(signal)?;
    if !levels.is_finite() || levels <= 0.0 || !threshold.is_finite() || threshold <= 0.0 {
        return Err(DvError::msg("WINDAP levels and threshold must be positive"));
    }
    let offset = signal.iter().copied().fold(f64::INFINITY, f64::min);
    let max = signal.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let gain = (max - offset) / levels;
    let quantized: Vec<_> = signal
        .iter()
        .map(|v| ((v - offset) / gain).round_ties_even())
        .collect();
    let mut cycles = pair_range_amplitude_mean(&peak_trough(&quantized, threshold));
    for c in &mut cycles {
        c.range = (c.range / threshold).round_ties_even() * gain * threshold;
        c.mean = (c.mean / threshold).round_ties_even() * gain * threshold + offset;
    }
    Ok(cycles)
}
/// Fatpack's closed-record counting: quantized reversals, closed cycles, then a doubled residual record.
fn rainflow_fatpack(signal: &[f64]) -> Result<Vec<HalfCycle>> {
    check_signal(signal)?;
    let min = signal.iter().copied().fold(f64::INFINITY, f64::min);
    let max = signal.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let step = (max - min) / 64.0;
    let q: Vec<_> = signal
        .iter()
        .map(|v| min + ((v - min) / step + 0.5).floor() * step)
        .collect();
    let mut changes: Vec<_> = q
        .windows(2)
        .enumerate()
        .filter(|(_, p)| p[0] != p[1])
        .map(|(i, _)| i)
        .collect();
    let last = changes
        .last()
        .copied()
        .ok_or_else(|| DvError::msg("No quantized reversals"))?
        + 1;
    changes.push(last);
    let mut reversals = vec![q[changes[0]]];
    for p in changes.windows(3) {
        if (q[p[1]] - q[p[0]]) * (q[p[2]] - q[p[1]]) < 0.0 {
            reversals.push(q[p[1]]);
        }
    }
    if reversals.len() < 2 {
        return Err(DvError::msg("Fatpack requires a reversing signal"));
    }
    let n = reversals.len();
    if (reversals[n - 1] - reversals[n - 2]) * (q[last] - reversals[n - 1]) < 0.0 {
        reversals.push(q[last]);
    }
    fn closed(input: &[f64], out: &mut Vec<HalfCycle>) -> Vec<f64> {
        let mut stack = Vec::new();
        for &v in input {
            stack.push(v);
            while stack.len() >= 4 {
                let n = stack.len();
                let (a, b, c, d): (f64, f64, f64, f64) =
                    (stack[n - 4], stack[n - 3], stack[n - 2], stack[n - 1]);
                if (b - c).abs() > (a - b).abs() || (b - c).abs() > (c - d).abs() {
                    break;
                }
                let cycle = HalfCycle {
                    range: (b - c).abs(),
                    mean: (b + c) / 2.0,
                };
                out.push(cycle);
                out.push(cycle);
                stack.truncate(n - 3);
                stack.push(d);
            }
        }
        stack
    }
    let mut out = Vec::new();
    let residual = closed(&reversals, &mut out);
    if residual.len() < 2 {
        return Err(DvError::msg("Fatpack residual has too few reversals"));
    }
    let n = residual.len();
    let d_start = residual[1] - residual[0];
    let d_end = residual[n - 1] - residual[n - 2];
    let d_join = residual[0] - residual[n - 1];
    let (end, start) = match (d_end * d_start > 0.0, d_end * d_join < 0.0) {
        (true, true) => (n, 0),
        (true, false) => (n - 1, 1),
        (false, false) => (n, 1),
        (false, true) => (n - 1, 0),
    };
    if d_end * d_start == 0.0 {
        return Err(DvError::msg("Fatpack residual endpoints must be reversals"));
    }
    let mut joined = residual[..end].to_vec();
    joined.extend_from_slice(&residual[start..]);
    closed(&joined, &mut out);
    if out.is_empty() {
        return Err(DvError::msg("Could not find any rainflow cycles"));
    }
    Ok(out)
}
fn count(signal: &[f64], method: RainflowMethod) -> Result<Vec<HalfCycle>> {
    match method {
        RainflowMethod::Windap => rainflow_windap(signal, 255.0, 255.0 / 50.0),
        RainflowMethod::Astm => rainflow_astm(signal),
        RainflowMethod::Fatpack => rainflow_fatpack(signal),
    }
}
pub fn create_bins(x: &[f64], bins: &Bins, start_at_zero: bool) -> Result<Vec<f64>> {
    match bins {
        Bins::Edges(edges) => {
            if edges.len() < 2
                || edges.iter().any(|v| !v.is_finite())
                || edges.windows(2).any(|p| p[0] >= p[1])
            {
                return Err(DvError::msg(
                    "Bin edges must be finite and strictly increasing",
                ));
            }
            Ok(edges.clone())
        }
        Bins::Count(n) => {
            if *n == 0 || x.is_empty() {
                return Err(DvError::msg("Histogram requires data and at least one bin"));
            }
            let mut low = if start_at_zero {
                0.0
            } else {
                x.iter().copied().fold(f64::INFINITY, f64::min)
            };
            let mut high = x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if low == high {
                low -= 0.5;
                high += 0.5;
            }
            Ok((0..=*n)
                .map(|i| low + (high - low) * i as f64 / *n as f64)
                .collect())
        }
    }
}
fn bin_index(v: f64, edges: &[f64]) -> Option<usize> {
    if v < edges[0] || v > edges[edges.len() - 1] {
        return None;
    }
    Some(
        edges
            .partition_point(|edge| *edge <= v)
            .saturating_sub(1)
            .min(edges.len() - 2),
    )
}
pub fn cycle_matrix(
    signals: &[(f64, &[f64])],
    amplitude_bins: &Bins,
    mean_bins: &Bins,
    method: RainflowMethod,
    start_at_zero: bool,
) -> Result<CycleMatrix> {
    let mut data = Vec::new();
    for &(weight, signal) in signals {
        if weight < 0.0 || !weight.is_finite() {
            return Err(DvError::msg(
                "Signal weights must be finite and nonnegative",
            ));
        }
        for c in count(signal, method)? {
            data.push((weight, c));
        }
    }
    let ranges: Vec<_> = data
        .iter()
        .filter(|(w, _)| *w > 0.0)
        .map(|(_, c)| c.range)
        .collect();
    let means: Vec<_> = data.iter().map(|(_, c)| c.mean).collect();
    let ae = create_bins(&ranges, amplitude_bins, start_at_zero)?;
    let me = create_bins(&means, mean_bins, false)?;
    let (na, nm) = (ae.len() - 1, me.len() - 1);
    let mut counts = vec![vec![0.0; nm]; na];
    let mut asum = counts.clone();
    let mut msum = counts.clone();
    for (w, c) in data {
        if let (Some(i), Some(j)) = (bin_index(c.range, &ae), bin_index(c.mean, &me)) {
            counts[i][j] += w;
            asum[i][j] += w * c.range;
            msum[i][j] += w * c.mean;
        }
    }
    let mut am = vec![f64::NAN; na];
    let mut mm = am.clone();
    for i in 0..na {
        let mut a = 0.0;
        let mut m = 0.0;
        let mut n = 0;
        for j in 0..nm {
            if counts[i][j] > 0.0 {
                a += asum[i][j] / counts[i][j];
                m += msum[i][j] / counts[i][j];
                n += 1;
            }
            counts[i][j] /= 2.0;
        }
        if n > 0 {
            am[i] = a / n as f64;
            mm[i] = m / n as f64;
        }
    }
    Ok(CycleMatrix {
        cycles: counts,
        amplitude_mean: am,
        amplitude_edges: ae,
        mean_mean: mm,
        mean_edges: me,
    })
}
pub fn find_range_count(
    signal: &[f64],
    bins: &Bins,
    method: RainflowMethod,
    mean_bin: bool,
    start_at_zero: bool,
) -> Result<RangeCount> {
    if method == RainflowMethod::Fatpack {
        let data = count(signal, method)?;
        let values: Vec<_> = data.iter().map(|c| c.range).collect();
        let edges = create_bins(&values, bins, start_at_zero)?;
        let mut cycles = vec![0.0; edges.len() - 1];
        let mut sums = cycles.clone();
        for value in values {
            let index = if mean_bin {
                if value <= edges[0] || value > edges[edges.len() - 1] {
                    None
                } else {
                    Some(edges.partition_point(|e| *e < value).saturating_sub(1))
                }
            } else {
                bin_index(value, &edges)
            };
            if let Some(i) = index {
                cycles[i] += 0.5;
                sums[i] += 0.5 * value;
            }
        }
        let ranges = cycles
            .iter()
            .enumerate()
            .map(|(i, n)| {
                if mean_bin {
                    if *n > 0.0 {
                        sums[i] / n
                    } else {
                        0.0
                    }
                } else {
                    (edges[i] + edges[i + 1]) / 2.0
                }
            })
            .collect();
        return Ok(RangeCount {
            cycles,
            ranges,
            bin_edges: edges,
        });
    }
    let matrix = cycle_matrix(
        &[(1.0, signal)],
        bins,
        &Bins::Count(1),
        method,
        start_at_zero,
    )?;
    let mut ranges = matrix.amplitude_mean;
    let mut cycles: Vec<_> = matrix.cycles.iter().map(|row| row[0]).collect();
    for i in 0..ranges.len() {
        if !mean_bin {
            ranges[i] = (matrix.amplitude_edges[i] + matrix.amplitude_edges[i + 1]) / 2.0;
        }
        if ranges[i].is_nan() {
            ranges[i] = 0.0;
            cycles[i] = 0.0;
        }
    }
    Ok(RangeCount {
        cycles,
        ranges,
        bin_edges: matrix.amplitude_edges,
    })
}
pub fn equivalent_load(
    time: &[f64],
    signal: &[f64],
    m: f64,
    teq: f64,
    bins: usize,
    method: &str,
) -> Result<f64> {
    Ok(equivalent_load_with_options(
        time,
        signal,
        &FatigueOptions {
            m,
            teq,
            bins: Bins::Count(bins),
            method: RainflowMethod::parse(method)?,
            ..Default::default()
        },
    )?
    .load)
}
pub fn equivalent_load_with_options(
    time: &[f64],
    signal: &[f64],
    options: &FatigueOptions,
) -> Result<EquivalentLoad> {
    if time.len() != signal.len() {
        return Err(DvError::msg("Time and signal lengths differ"));
    }
    if options.m <= 0.0 || !options.m.is_finite() || options.teq <= 0.0 || !options.teq.is_finite()
    {
        return Err(DvError::msg(
            "Wohler exponent and equivalent period must be finite and positive",
        ));
    }
    let valid: Vec<_> = time
        .iter()
        .zip(signal)
        .filter(|(_, v)| !v.is_nan())
        .map(|(&t, &v)| (t, v))
        .collect();
    if valid.len() < 2 {
        return Err(DvError::msg(
            "Equivalent load requires at least two valid samples",
        ));
    }
    let duration = valid[valid.len() - 1].0 - valid[0].0;
    if !duration.is_finite() || duration <= 0.0 {
        return Err(DvError::msg(
            "Equivalent load requires a positive finite duration",
        ));
    }
    let y: Vec<_> = valid.iter().map(|p| p.1).collect();
    let histogram = find_range_count(
        &y,
        &options.bins,
        options.method,
        options.mean_bin,
        options.bin_start_at_zero,
    )?;
    let damage: Vec<_> = histogram
        .ranges
        .iter()
        .zip(&histogram.cycles)
        .map(|(s, n)| s.powf(options.m) * n / (duration / options.teq))
        .collect();
    let load = damage.iter().sum::<f64>().powf(1.0 / options.m);
    Ok(EquivalentLoad {
        load,
        ranges: histogram.ranges,
        cycles: histogram.cycles,
        bin_edges: histogram.bin_edges,
        damage,
    })
}
pub fn eq_load(
    signals: &[(f64, &[f64])],
    bins: usize,
    m: &[f64],
    neq: &[f64],
    method: RainflowMethod,
) -> Result<Vec<Vec<f64>>> {
    let matrix = cycle_matrix(signals, &Bins::Count(bins), &Bins::Count(1), method, true)?;
    if m.iter().chain(neq).any(|v| !v.is_finite() || *v <= 0.0) {
        return Err(DvError::msg(
            "Exponents and equivalent cycle counts must be positive",
        ));
    }
    Ok(neq
        .iter()
        .map(|n| {
            m.iter()
                .map(|p| {
                    let damage = matrix
                        .cycles
                        .iter()
                        .zip(&matrix.amplitude_mean)
                        .filter(|(_, s)| s.is_finite())
                        .map(|(c, s)| c[0] * s.powf(*p))
                        .sum::<f64>();
                    (damage / n).powf(1.0 / p)
                })
                .collect()
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windap_reference() {
        let s = [
            -2., 0., 1., 0., -3., 0., 5., 0., -1., 0., 3., 0., -4., 0., 4., 0., -2.,
        ];
        let loads = eq_load(
            &[(1.0, &s)],
            50,
            &[3., 4., 6.],
            &[1., 17.],
            RainflowMethod::Windap,
        )
        .unwrap();
        let expected = [10.348414123746581, 9.635653414943068, 9.122399471334054];
        for (a, b) in loads[0].iter().zip(expected) {
            assert!((a - b).abs() < 1e-5);
        }
    }
    #[test]
    fn astm_half_cycles() {
        let c = rainflow_astm(&[0., 1., 0., 1., 0.]).unwrap();
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|c| c.range == 1.0));
    }
}
