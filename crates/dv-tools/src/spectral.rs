//! One-sided spectral analysis, following pyDatView's scaling and defaults.
//!
//! `psd` deliberately retains the upstream undoubled last bin for odd input
//! lengths; Welch doubles every positive bin for odd FFT lengths.
use dv_core::error::{DvError, Result};
use rustfft::{num_complex::Complex, FftPlanner};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detrend {
    #[default]
    None,
    Constant,
    Linear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Window {
    Hann,
    #[default]
    Hamming,
    Rectangular,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputType {
    #[default]
    Amplitude,
    Psd,
    FrequencyPsd,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Averaging {
    #[default]
    None,
    Welch,
    Binning,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scaling {
    #[default]
    Density,
    Spectrum,
}
#[derive(Debug, Clone)]
pub struct FftOptions {
    pub dt: Option<f64>,
    pub output_type: OutputType,
    pub averaging: Averaging,
    pub window: Window,
    pub detrend: Detrend,
    pub n_exp: Option<u32>,
    pub points_per_decade: usize,
}
impl Default for FftOptions {
    fn default() -> Self {
        Self {
            dt: None,
            output_type: OutputType::Amplitude,
            averaging: Averaging::None,
            window: Window::Hamming,
            detrend: Detrend::None,
            n_exp: None,
            points_per_decade: 10,
        }
    }
}
#[derive(Debug, Clone)]
pub struct WelchOptions {
    pub fs: f64,
    pub window: Window,
    pub nperseg: Option<usize>,
    pub noverlap: Option<usize>,
    pub nfft: Option<usize>,
    pub detrend: Detrend,
    pub scaling: Scaling,
    pub symmetric_window: bool,
}
impl Default for WelchOptions {
    fn default() -> Self {
        Self {
            fs: 1.0,
            window: Window::Hann,
            nperseg: None,
            noverlap: None,
            nfft: None,
            detrend: Detrend::Constant,
            scaling: Scaling::Density,
            symmetric_window: false,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct SpectralInfo {
    pub df: f64,
    pub f_max: f64,
    pub frequency_len: usize,
    pub segment_len: usize,
    pub window_len: usize,
    pub overlap: usize,
    pub n_fft: usize,
    pub segments: usize,
    pub n_exp: Option<u32>,
    pub points_per_decade: Option<usize>,
    pub bin_edges: Vec<f64>,
}
#[derive(Debug, Clone)]
pub struct Spectrum {
    pub frequency: Vec<f64>,
    pub values: Vec<f64>,
    pub info: SpectralInfo,
}
#[derive(Debug, Clone)]
pub struct CrossSpectrum {
    pub frequency: Vec<f64>,
    pub values: Vec<Complex<f64>>,
    pub info: SpectralInfo,
}

pub fn fnextpow2(x: f64) -> usize {
    if x <= 1.0 {
        1
    } else {
        2usize.pow((x.log2() * 0.99999999999).ceil() as u32)
    }
}
pub fn general_cosine(n: usize, coefficients: &[f64], symmetric: bool) -> Vec<f64> {
    if n <= 1 {
        return vec![1.0; n];
    }
    let denominator = if symmetric { n - 1 } else { n } as f64;
    (0..n)
        .map(|i| {
            let phase = -PI + 2.0 * PI * i as f64 / denominator;
            coefficients
                .iter()
                .enumerate()
                .map(|(k, a)| a * (k as f64 * phase).cos())
                .sum()
        })
        .collect()
}
pub fn general_hamming(n: usize, alpha: f64, symmetric: bool) -> Vec<f64> {
    general_cosine(n, &[alpha, 1.0 - alpha], symmetric)
}
pub fn hann(n: usize, symmetric: bool) -> Vec<f64> {
    general_hamming(n, 0.5, symmetric)
}
pub fn hamming(n: usize, symmetric: bool) -> Vec<f64> {
    general_hamming(n, 0.54, symmetric)
}
pub fn boxcar(n: usize, _symmetric: bool) -> Vec<f64> {
    vec![1.0; n]
}
pub fn get_window(window: Window, n: usize, fftbins: bool) -> Vec<f64> {
    match window {
        Window::Hann => hann(n, !fftbins),
        Window::Hamming => hamming(n, !fftbins),
        Window::Rectangular => boxcar(n, !fftbins),
    }
}
pub fn detrend_in_place(y: &mut [f64], detrend: Detrend) {
    if y.is_empty() || detrend == Detrend::None {
        return;
    }
    let mean = y.iter().sum::<f64>() / y.len() as f64;
    let center = (y.len() - 1) as f64 / 2.0;
    let slope = if detrend == Detrend::Linear && y.len() > 1 {
        let numerator: f64 = y
            .iter()
            .enumerate()
            .map(|(i, v)| (i as f64 - center) * (v - mean))
            .sum();
        let denominator: f64 = (0..y.len()).map(|i| (i as f64 - center).powi(2)).sum();
        numerator / denominator
    } else {
        0.0
    };
    for (i, v) in y.iter_mut().enumerate() {
        *v -= mean + slope * (i as f64 - center);
    }
}
fn validate(y: &[f64], fs: f64) -> Result<()> {
    if y.len() < 2 {
        return Err(DvError::msg(
            "spectral analysis requires at least two samples",
        ));
    }
    if !fs.is_finite() || fs <= 0.0 {
        return Err(DvError::msg(
            "sampling frequency must be positive and finite",
        ));
    }
    Ok(())
}
fn info(fs: f64, nfft: usize, segment: usize, overlap: usize, segments: usize) -> SpectralInfo {
    let count = nfft / 2 + 1;
    SpectralInfo {
        df: fs / nfft as f64,
        f_max: (count - 1) as f64 * fs / nfft as f64,
        frequency_len: count,
        segment_len: segment,
        window_len: segment,
        overlap,
        n_fft: nfft,
        segments,
        ..Default::default()
    }
}
pub fn psd(y: &[f64], fs: f64, detrend: Detrend) -> Result<Spectrum> {
    validate(y, fs)?;
    let center = (y.len() - 1) as f64 / 2.0;
    let mean = if detrend == Detrend::None {
        0.0
    } else {
        y.iter().sum::<f64>() / y.len() as f64
    };
    let slope = if detrend == Detrend::Linear {
        y.iter()
            .enumerate()
            .map(|(i, v)| (i as f64 - center) * (v - mean))
            .sum::<f64>()
            / (0..y.len())
                .map(|i| (i as f64 - center).powi(2))
                .sum::<f64>()
    } else {
        0.0
    };
    let mut fft: Vec<_> = y
        .iter()
        .enumerate()
        .map(|(i, v)| Complex::new(v - mean - slope * (i as f64 - center), 0.0))
        .collect();
    FftPlanner::<f64>::new()
        .plan_fft_forward(y.len())
        .process(&mut fft);
    let n = y.len();
    let count = n / 2 + 1;
    let values = (0..count)
        .map(|i| {
            fft[i].norm_sqr() / (n as f64 * fs) * (if i > 0 && i < count - 1 { 2.0 } else { 1.0 })
        })
        .collect();
    Ok(Spectrum {
        frequency: (0..count).map(|i| i as f64 * fs / n as f64).collect(),
        values,
        info: info(fs, n, count, 0, 1),
    })
}
pub fn fft_amplitude(y: &[f64], fs: f64, detrend: Detrend) -> Result<Spectrum> {
    let mut result = psd(y, fs, detrend)?;
    for v in &mut result.values {
        *v = (*v * 2.0 * result.info.df).sqrt();
    }
    Ok(result)
}
pub fn psd_binned(
    y: &[f64],
    fs: f64,
    points_per_decade: usize,
    detrend: Detrend,
) -> Result<Spectrum> {
    if points_per_decade == 0 {
        return Err(DvError::msg("points per decade must be positive"));
    }
    let mut result = psd(y, fs, detrend)?;
    let low = result.frequency[1].log10();
    let high = result.frequency.last().unwrap().log10();
    let n = (high - low).ceil() as usize * points_per_decade;
    if n < 2 {
        return Err(DvError::msg("log binning needs at least two bin edges"));
    }
    let edges: Vec<_> = (0..n)
        .map(|i| low + (high - low) * i as f64 / (n - 1) as f64)
        .collect();
    let logs: Vec<_> = result.frequency[1..].iter().map(|v| v.log10()).collect();
    let (x, y) = crate::stats::bin_signal(
        &logs,
        &result.values[1..],
        &edges,
        &crate::stats::BinStatistic::Mean,
    )?;
    let mut frequencies = vec![0.0];
    let mut values = vec![result.values[0]];
    for (x, y) in x.into_iter().zip(y) {
        if !x.is_nan() {
            frequencies.push(10f64.powf(x));
            values.push(y);
        }
    }
    result.frequency = frequencies;
    result.values = values;
    result.info.bin_edges = edges;
    result.info.points_per_decade = Some(points_per_decade);
    Ok(result)
}
pub fn csd(x: &[f64], y: &[f64], options: &WelchOptions) -> Result<CrossSpectrum> {
    let length = x.len().max(y.len());
    if x.is_empty() || y.is_empty() {
        return Err(DvError::msg("cross spectrum requires nonempty signals"));
    }
    validate(if x.len() >= y.len() { x } else { y }, options.fs)?;
    let nseg = options.nperseg.unwrap_or(256).min(length);
    let win = get_window(options.window, nseg, !options.symmetric_window);
    let opts = WelchOptions {
        nperseg: Some(nseg),
        ..options.clone()
    };
    csd_with_window(x, y, &opts, &win)
}
pub fn csd_with_window(
    x: &[f64],
    y: &[f64],
    options: &WelchOptions,
    win: &[f64],
) -> Result<CrossSpectrum> {
    let length = x.len().max(y.len());
    validate(if x.len() >= y.len() { x } else { y }, options.fs)?;
    let nseg = win.len();
    let nfft = options.nfft.unwrap_or(nseg);
    let overlap = options.noverlap.unwrap_or(nseg / 2);
    if x.is_empty()
        || y.is_empty()
        || nseg < 2
        || nseg > length
        || nfft < nseg
        || overlap >= nseg
        || options.nperseg.is_some_and(|n| n != nseg)
    {
        return Err(DvError::msg(
            "invalid Welch segment, window, FFT length or overlap",
        ));
    }
    let denominator = match options.scaling {
        Scaling::Density => options.fs * win.iter().map(|v| v * v).sum::<f64>(),
        Scaling::Spectrum => win.iter().sum::<f64>().powi(2),
    };
    if denominator <= 0.0 {
        return Err(DvError::msg("window has zero spectral normalization"));
    }
    let count = nfft / 2 + 1;
    let segments = 1 + (length - nseg) / (nseg - overlap);
    let mut values = vec![Complex::new(0.0, 0.0); count];
    let same = x.len() == y.len() && x.as_ptr() == y.as_ptr();
    let mut a = vec![Complex::new(0.0, 0.0); nfft];
    let mut b = if same {
        Vec::new()
    } else {
        vec![Complex::new(0.0, 0.0); nfft]
    };
    let mut sa = vec![0.0; nseg];
    let mut sb = if same { Vec::new() } else { vec![0.0; nseg] };
    let fft = FftPlanner::<f64>::new().plan_fft_forward(nfft);
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_inplace_scratch_len()];
    for start in (0..=length - nseg).step_by(nseg - overlap) {
        for i in 0..nseg {
            sa[i] = x.get(start + i).copied().unwrap_or(0.0);
        }
        detrend_in_place(&mut sa, options.detrend);
        a.fill(Complex::new(0.0, 0.0));
        for i in 0..nseg {
            a[i].re = sa[i] * win[i];
        }
        fft.process_with_scratch(&mut a, &mut scratch);
        if same {
            for i in 0..count {
                values[i].re += a[i].norm_sqr();
            }
        } else {
            for i in 0..nseg {
                sb[i] = y.get(start + i).copied().unwrap_or(0.0);
            }
            detrend_in_place(&mut sb, options.detrend);
            b.fill(Complex::new(0.0, 0.0));
            for i in 0..nseg {
                b[i].re = sb[i] * win[i];
            }
            fft.process_with_scratch(&mut b, &mut scratch);
            for i in 0..count {
                values[i] += a[i].conj() * b[i];
            }
        }
    }
    for (i, v) in values.iter_mut().enumerate() {
        *v *= (if i > 0 && (nfft % 2 == 1 || i < count - 1) {
            2.0
        } else {
            1.0
        }) / (denominator * segments as f64);
    }
    Ok(CrossSpectrum {
        frequency: (0..count)
            .map(|i| i as f64 * options.fs / nfft as f64)
            .collect(),
        values,
        info: info(options.fs, nfft, nseg, overlap, segments),
    })
}
pub fn welch(y: &[f64], options: &WelchOptions) -> Result<Spectrum> {
    let cross = csd(y, y, options)?;
    Ok(Spectrum {
        frequency: cross.frequency,
        values: cross.values.into_iter().map(|v| v.re).collect(),
        info: cross.info,
    })
}
/// MATLAB-compatible defaults: Hamming, no detrending and sqrt-based segment length.
pub fn pwelch(y: &[f64], options: Option<&WelchOptions>) -> Result<Spectrum> {
    let mut opts = options.cloned().unwrap_or(WelchOptions {
        window: Window::Hamming,
        detrend: Detrend::None,
        ..Default::default()
    });
    if opts.nperseg.is_none() {
        if opts.noverlap.is_some_and(|n| n > 0) {
            return Err(DvError::msg(
                "pwelch requires segment length with explicit nonzero overlap",
            ));
        }
        let fraction = if opts.noverlap == Some(0) { 1.0 } else { 0.5 };
        opts.nperseg = Some(fnextpow2((y.len() as f64 / fraction).sqrt()).min(y.len()));
    }
    welch(y, &opts)
}
pub fn coherence(x: &[f64], y: &[f64], options: &WelchOptions) -> Result<Spectrum> {
    let cross = csd(x, y, options)?;
    let xx = welch(x, options)?;
    let yy = welch(y, options)?;
    if xx.frequency != cross.frequency || yy.frequency != cross.frequency {
        return Err(DvError::msg("coherence requires equal segment lengths"));
    }
    let values = cross
        .values
        .iter()
        .zip(xx.values.iter().zip(&yy.values))
        .map(|(v, (x, y))| v.norm_sqr() / (x * y))
        .collect();
    Ok(Spectrum {
        frequency: cross.frequency,
        values,
        info: cross.info,
    })
}
pub fn fft_wrap(t: &[f64], y: &[f64], options: &FftOptions) -> Result<Spectrum> {
    if t.len() != y.len() {
        return Err(DvError::msg("t and y should have the same length"));
    }
    let mut times = t.iter().copied().filter(|v| !v.is_nan());
    let first = times
        .next()
        .ok_or_else(|| DvError::msg("FFT requires at least two nonmissing times"))?;
    let mut last = first;
    let mut count = 1usize;
    for time in times {
        last = time;
        count += 1;
    }
    if count < 2 {
        return Err(DvError::msg("FFT requires at least two nonmissing times"));
    }
    let signal: Vec<_> = t
        .iter()
        .zip(y)
        .filter_map(|(t, y)| {
            if !t.is_nan() && !y.is_nan() {
                Some(*y)
            } else {
                None
            }
        })
        .collect();
    let dt = options.dt.unwrap_or((last - first) / (count - 1) as f64);
    let fs = 1.0 / dt;
    validate(&signal, fs)?;
    let mut result = match options.averaging {
        Averaging::None => psd(&signal, fs, options.detrend)?,
        Averaging::Binning => psd_binned(&signal, fs, options.points_per_decade, options.detrend)?,
        Averaging::Welch => {
            let default_exp = fnextpow2(signal.len() as f64).ilog2().saturating_sub(1);
            let mut exp = options.n_exp.unwrap_or(default_exp);
            let mut n = 1usize
                .checked_shl(exp)
                .ok_or_else(|| DvError::msg("FFT exponent is too large"))?;
            if n > signal.len() {
                exp = default_exp;
                n = 1usize << exp;
            }
            let opts = WelchOptions {
                fs,
                window: options.window,
                nperseg: Some(n),
                detrend: options.detrend,
                symmetric_window: true,
                ..Default::default()
            };
            let mut r = welch(&signal, &opts)?;
            r.info.n_exp = Some(exp);
            r
        }
    };
    let delta = if result.frequency.len() > 1 {
        result.frequency[1] - result.frequency[0]
    } else {
        result.info.df
    };
    for (f, v) in result.frequency.iter().zip(&mut result.values) {
        match options.output_type {
            OutputType::Amplitude => *v = (*v * 2.0 * delta).sqrt(),
            OutputType::Psd => {}
            OutputType::FrequencyPsd => *v *= f,
        }
    }
    if options.detrend != Detrend::None {
        result.frequency.remove(0);
        result.values.remove(0);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sinusoid_amplitude() {
        let y: Vec<_> = (0..100)
            .map(|i| 3.0 * (2.0 * PI * 10.0 * i as f64 / 100.0).sin())
            .collect();
        let s = fft_amplitude(&y, 100.0, Detrend::Constant).unwrap();
        assert!((s.values[10] - 3.0).abs() < 1e-10);
    }
    #[test]
    fn window_symmetry() {
        let w = hamming(7, true);
        assert!((w[0] - 0.08).abs() < 1e-12);
        assert!((w[3] - 1.0).abs() < 1e-12);
    }
    #[test]
    fn welch_power() {
        let y: Vec<_> = (0..1024)
            .map(|i| (2.0 * PI * i as f64 / 16.0).sin())
            .collect();
        let s = welch(&y, &WelchOptions::default()).unwrap();
        assert!((s.values.iter().sum::<f64>() * s.info.df - 0.5).abs() < 1e-10);
    }
}
