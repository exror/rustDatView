//! Peak-based logarithmic decrement, natural frequency and damping envelopes.
use dv_core::error::{DvError, Result};
use std::f64::consts::PI;
#[derive(Debug, Clone)]
pub struct LogDecrement {
    pub logdec: f64,
    pub index_period: f64,
    pub std: f64,
    pub peaks: Vec<usize>,
    pub decrements: Vec<f64>,
}
#[derive(Debug, Clone)]
pub struct DampingResult {
    pub fn_: f64,
    pub zeta: f64,
    pub fd: f64,
    pub td: f64,
    pub omega0: f64,
    pub positive_peaks: Vec<usize>,
    pub negative_peaks: Vec<usize>,
    pub x_model: Vec<f64>,
    pub epos: Vec<f64>,
    pub eneg: Vec<f64>,
    pub zeta_min: f64,
    pub zeta_max: f64,
    pub zeta_mean: f64,
    pub logdec: f64,
    pub logdec_std: f64,
    pub positive_decrements: Vec<f64>,
    pub negative_decrements: Vec<f64>,
    pub decay: bool,
}
fn mean(x: &[f64]) -> f64 {
    x.iter().sum::<f64>() / x.len() as f64
}
fn median(mut x: Vec<f64>) -> f64 {
    x.sort_by(f64::total_cmp);
    let n = x.len();
    if n % 2 == 1 {
        x[n / 2]
    } else {
        (x[n / 2 - 1] + x[n / 2]) / 2.0
    }
}
/// Local maxima, including plateau centers; larger peaks win minimum-distance conflicts.
pub fn indexes(y: &[f64], threshold: f64, min_dist: usize, absolute: bool) -> Vec<usize> {
    if y.len() < 3 {
        return Vec::new();
    }
    let threshold = if absolute {
        threshold
    } else {
        threshold
            * (y.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - y.iter().copied().fold(f64::INFINITY, f64::min))
            + y.iter().copied().fold(f64::INFINITY, f64::min)
    };
    let mut candidates = Vec::new();
    let mut i = 1;
    while i + 1 < y.len() {
        let start = i;
        while i + 1 < y.len() && y[i] == y[i + 1] {
            i += 1;
        }
        if i + 1 < y.len() && y[start] > y[start - 1] && y[i] > y[i + 1] && y[start] > threshold {
            candidates.push((start + i) / 2);
        }
        i += 1;
    }
    if min_dist <= 1 {
        return candidates;
    }
    candidates.sort_by(|a, b| y[*b].total_cmp(&y[*a]).then(b.cmp(a)));
    let mut selected = Vec::new();
    for p in candidates {
        if selected.iter().all(|q: &usize| p.abs_diff(*q) > min_dist) {
            selected.push(p);
        }
    }
    selected.sort_unstable();
    selected
}
pub fn log_dec_from_threshold(
    x: &[f64],
    threshold: Option<f64>,
    decay: bool,
) -> Result<LogDecrement> {
    if x.len() < 3 || x.iter().any(|v| !v.is_finite()) {
        return Err(DvError::msg(
            "Logarithmic decrement requires finite samples",
        ));
    }
    let m = mean(x);
    let threshold = threshold
        .unwrap_or_else(|| x.iter().map(|v| (v - m).abs()).sum::<f64>() / x.len() as f64 / 3.0);
    let peaks = indexes(x, threshold, 1, true);
    if peaks.len() < 2 {
        return Err(DvError::msg(
            "At least two peaks above threshold are required",
        ));
    }
    let index_period =
        median(peaks.windows(2).map(|p| (p[1] - p[0]) as f64).collect()).round_ties_even();
    let decrements: Vec<_> = if decay {
        peaks[1..]
            .iter()
            .enumerate()
            .map(|(i, p)| (x[peaks[0]] / x[*p]).ln() / (i + 1) as f64)
            .collect()
    } else {
        peaks[..peaks.len() - 1]
            .iter()
            .rev()
            .enumerate()
            .map(|(i, p)| (x[*p] / x[peaks[peaks.len() - 1]]).ln() / (i + 1) as f64)
            .collect()
    };
    if decrements.iter().any(|v| !v.is_finite()) {
        return Err(DvError::msg("Peak ratios must be finite and positive"));
    }
    let logdec = mean(&decrements);
    let std = (decrements.iter().map(|v| (v - logdec).powi(2)).sum::<f64>()
        / decrements.len() as f64)
        .sqrt();
    Ok(LogDecrement {
        logdec,
        index_period,
        std,
        peaks,
        decrements,
    })
}
pub fn log_dec_two_times(x: &[f64], t: &[f64], i1: usize, i2: usize, td: f64) -> Result<f64> {
    if x.len() != t.len()
        || i1 >= x.len()
        || i2 >= x.len()
        || i1 == i2
        || td <= 0.0
        || t[i1] == t[i2]
    {
        return Err(DvError::msg(
            "Invalid logarithmic decrement reference times",
        ));
    }
    let delta = (x[i1] / x[i2]).ln() * td / (t[i2] - t[i1]);
    if !delta.is_finite() {
        return Err(DvError::msg("Invalid peak ratio"));
    }
    Ok(delta)
}
pub fn zeta_two_times(x: &[f64], t: &[f64], i1: usize, i2: usize, td: f64) -> Result<f64> {
    let d = log_dec_two_times(x, t, i1, i2, td)?;
    Ok(d / (4.0 * PI * PI + d * d).sqrt())
}
pub fn zeta_envelop(
    x: &[f64],
    t: &[f64],
    omega0: f64,
    zeta: f64,
    i_ref: usize,
    moff: f64,
) -> Result<(Vec<f64>, Vec<f64>)> {
    if x.len() != t.len() || i_ref >= x.len() {
        return Err(DvError::msg("Invalid envelope reference sample"));
    }
    let m = mean(x);
    let amplitude = x[i_ref] - m;
    let e: Vec<_> = t
        .iter()
        .map(|v| amplitude * (-zeta * omega0 * (v - t[i_ref])).exp())
        .collect();
    Ok((
        e.iter().map(|v| v + m + moff).collect(),
        e.iter().map(|v| -v + m + moff).collect(),
    ))
}
pub fn freq_damp_estimator(x: &[f64], t: &[f64], method: &str) -> Result<DampingResult> {
    if method != "fromPeaks" {
        return Err(DvError::msg(format!(
            "Unsupported damping method: {method}"
        )));
    }
    freq_damp_from_peaks(x, t, None)
}
pub fn freq_damp_from_peaks(x: &[f64], t: &[f64], threshold: Option<f64>) -> Result<DampingResult> {
    if x.len() != t.len()
        || x.len() < 4
        || x.iter().chain(t).any(|v| !v.is_finite())
        || t.windows(2).any(|p| p[1] <= p[0])
    {
        return Err(DvError::msg(
            "Damping estimation requires matching finite data and increasing times",
        ));
    }
    let dt = (t[t.len() - 1] - t[0]) / (t.len() - 1) as f64;
    let m = mean(x);
    let centered: Vec<_> = x.iter().map(|v| v - m).collect();
    let negative: Vec<_> = centered.iter().map(|v| -v).collect();
    let split = x.len().div_ceil(2);
    let decay = centered[..split].iter().map(|v| v.abs()).sum::<f64>() / split as f64
        > centered[split..].iter().map(|v| v.abs()).sum::<f64>() / (x.len() - split) as f64;
    let threshold = threshold
        .unwrap_or_else(|| centered.iter().map(|v| v.abs()).sum::<f64>() / x.len() as f64 / 3.0);
    let pos = log_dec_from_threshold(&centered, Some(threshold), decay)?;
    let neg = log_dec_from_threshold(&negative, Some(threshold), decay)?;
    let logdec = (pos.logdec + neg.logdec) / 2.0;
    let td = (pos.index_period + neg.index_period) / 2.0 * dt;
    let fd = 1.0 / td;
    let zeta = logdec / (4.0 * PI * PI + logdec * logdec).sqrt();
    let fn_ = fd / (1.0 - zeta * zeta).sqrt();
    let omega0 = 2.0 * PI * fn_;
    let mut zetas = Vec::new();
    for (values, peaks) in [(&centered, &pos.peaks), (&negative, &neg.peaks)] {
        zetas.push(zeta_two_times(
            values,
            t,
            peaks[0],
            peaks[peaks.len() - 1],
            td,
        )?);
        if pos.peaks.len() > 3 && neg.peaks.len() > 3 {
            let (a, b) = if decay {
                (0, peaks.len() / 2)
            } else {
                (peaks.len() - peaks.len() / 2, peaks.len() - 1)
            };
            zetas.push(zeta_two_times(values, t, peaks[a], peaks[b], td)?);
        }
    }
    let i1 = pos.peaks[pos.peaks.len() / 2];
    let alpha = logdec / td;
    let omega = 2.0 * PI * fd;
    let crossing = (i1 + 1..x.len())
        .find(|i| centered[*i] < 0.0)
        .ok_or_else(|| DvError::msg("No zero crossing after the reference peak"))?;
    let i0 = crossing - 1;
    let t0 = t[i0] + (t[crossing] - t[i0]) * centered[i0] / (centered[i0] - centered[crossing]);
    // Work relative to the reference peak, so epoch-sized timestamps do not
    // overflow the envelope or lose the fitted phase.
    let phi = PI / 2.0 - omega * (t0 - t[i1]);
    let amplitude = centered[i1] / phi.cos();
    let x_model = t
        .iter()
        .map(|v| amplitude * (-alpha * (v - t[i1])).exp() * (omega * (v - t[i1]) + phi).cos() + m)
        .collect();
    let epos = t
        .iter()
        .map(|v| amplitude * (-alpha * (v - t[i1])).exp() + m)
        .collect();
    let eneg = t
        .iter()
        .map(|v| -amplitude * (-alpha * (v - t[i1])).exp() + m)
        .collect();
    Ok(DampingResult {
        fn_,
        zeta,
        fd,
        td,
        omega0,
        positive_peaks: pos.peaks,
        negative_peaks: neg.peaks,
        x_model,
        epos,
        eneg,
        zeta_min: zetas.iter().copied().fold(f64::INFINITY, f64::min),
        zeta_max: zetas.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        zeta_mean: mean(&zetas),
        logdec,
        logdec_std: (pos.std + neg.std) / 2.0,
        positive_decrements: pos.decrements,
        negative_decrements: neg.decrements,
        decay,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decay_and_growth() {
        for zeta in [0.1_f64, -0.01] {
            let td = 10.0 / (1.0 - zeta * zeta).sqrt();
            let t: Vec<_> = (0..2000).map(|i| 30.0 * td * i as f64 / 1999.0).collect();
            let x: Vec<_> = t
                .iter()
                .map(|v| (2.0 * PI / td * v).cos() * (-2.0 * PI * zeta / 10.0 * v).exp() + 10.0)
                .collect();
            let r = freq_damp_from_peaks(&x, &t, None).unwrap();
            assert!((r.zeta - zeta).abs() < 1e-4);
            assert!((r.fn_ - 0.1).abs() < 0.005);
        }
    }
    #[test]
    fn plateau_peaks() {
        assert_eq!(indexes(&[0., 1., 1., 1., 0.], 0.0, 1, true), vec![2]);
    }
    #[test]
    fn envelope_is_invariant_to_time_origin() {
        let t: Vec<_> = (0..2000).map(|i| i as f64 / 100.).collect();
        let shifted: Vec<_> = t.iter().map(|v| v + 1_700_000_000.).collect();
        let x: Vec<_> = t
            .iter()
            .map(|v| (2. * PI * v).cos() * (-0.1 * v).exp())
            .collect();
        let a = freq_damp_from_peaks(&x, &t, None).unwrap();
        let b = freq_damp_from_peaks(&x, &shifted, None).unwrap();
        for (a, b) in a.x_model.iter().zip(b.x_model) {
            assert!((a - b).abs() < 1e-5);
        }
        assert!(b.epos.iter().all(|v| v.is_finite()));
    }
}
