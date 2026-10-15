//! Predefined and custom-formula curve fitting with bounded nonlinear least squares.
//!
//! Formulae are parsed once into a scalar expression tree; fitting never executes Python.
//! Fits use a projected Levenberg–Marquardt iteration, with parameter-scaled damping.
use dv_core::error::{DvError, Result};
use nalgebra::{DMatrix, DVector};
use rustfft::{num_complex::Complex, FftPlanner};
use std::{collections::BTreeMap, f64::consts::PI};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct FitOptions {
    pub guesses: BTreeMap<String, f64>,
    pub bounds: BTreeMap<String, (f64, f64)>,
    pub constants: BTreeMap<String, f64>,
    pub order: usize,
    pub exponents: Vec<i32>,
    pub physical: bool,
    pub max_iterations: usize,
}
impl Default for FitOptions {
    fn default() -> Self {
        Self {
            guesses: BTreeMap::new(),
            bounds: BTreeMap::new(),
            constants: BTreeMap::new(),
            order: 3,
            exponents: vec![0, 2, 3],
            physical: true,
            max_iterations: 1000,
        }
    }
}
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub formula: String,
    pub coefficients: Vec<(String, f64)>,
    pub bounds: BTreeMap<String, (f64, f64)>,
    pub constants: BTreeMap<String, f64>,
}
#[derive(Debug, Clone)]
pub struct FitResult {
    pub x: Vec<f64>,
    pub y_fit: Vec<f64>,
    pub coefficients: Vec<(String, f64)>,
    pub r2: f64,
    pub rmse: f64,
    pub formula: String,
    pub iterations: usize,
    model: CompiledModel,
}
impl FitResult {
    pub fn evaluate(&self, x: &[f64]) -> Result<Vec<f64>> {
        let p: Vec<_> = self.coefficients.iter().map(|(_, v)| *v).collect();
        let result: Vec<_> = x.iter().map(|v| self.model.eval(*v, &p)).collect();
        if result.iter().any(|v| !v.is_finite()) {
            return Err(DvError::msg(
                "Fitted model is not finite at the requested points",
            ));
        }
        Ok(result)
    }
}
fn info(
    id: &str,
    label: &str,
    formula: &str,
    coeffs: &[(&str, f64)],
    bounds: &[(&str, f64, f64)],
    constants: &[(&str, f64)],
) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        label: label.into(),
        formula: formula.into(),
        coefficients: coeffs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        bounds: bounds
            .iter()
            .map(|(k, a, b)| (k.to_string(), (*a, *b)))
            .collect(),
        constants: constants.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
    }
}
pub fn models() -> Vec<ModelInfo> {
    let inf = f64::INFINITY;
    vec![
        info("predef: gaussian","Gaussian","1/({sigma}*sqrt(2*pi))*exp(-0.5*((x-{mu})/{sigma})**2)",&[("mu",0.),("sigma",1.)],&[],&[]),
        info("predef: gaussian-yoff","Gaussian with y-offset","1/({sigma}*sqrt(2*pi))*exp(-0.5*((x-{mu})/{sigma})**2)+{y0}",&[("mu",0.),("sigma",1.),("y0",0.)],&[],&[]),
        info("predef: expdecay","Exponential","{A}*exp(-{k}*x)+{B}",&[("A",1.),("k",1.),("B",0.)],&[],&[]),
        info("predef: logarithmic","Logarithmic","{a}*log(x)+{b}",&[("a",1.),("b",0.)],&[],&[]),
        info("predef: secondorder_impulse","2nd order impulse/decay (manual)","{A}*exp(-{zeta}*{omega}*(x-{x0}))*sin({omega}*sqrt(1-{zeta}**2)*(x-{x0}))+{B}",&[("A",1.),("omega",1.),("zeta",0.001),("B",0.),("x0",0.)],&[("omega",0.,100.),("zeta",0.,1.)],&[]),
        info("predef: secondorder_step","2nd order step (manual)","{A}*(1-exp(-{zeta}*{omega}*(x-{x0}))/sqrt(1-{zeta}**2)*cos({omega}*sqrt(1-{zeta}**2)*(x-{x0})-arctan({zeta}/sqrt(1-{zeta}**2))))+{B}",&[("A",1.),("omega",1.),("zeta",0.001),("B",0.),("x0",0.)],&[("omega",0.,100.),("zeta",0.,1.)],&[]),
        info("predef: powerlaw_alpha","Power law (alpha)","u_ref*(x/z_ref)**{alpha}",&[("alpha",0.1)],&[("alpha",-1.,1.)],&[("u_ref",10.),("z_ref",100.)]),
        info("predef: powerlaw_u_alpha","Power law (alpha,u)","{u_ref}*(x/z_ref)**{alpha}",&[("alpha",0.1),("u_ref",10.)],&[("alpha",-1.,1.),("u_ref",0.,inf)],&[("z_ref",100.)]),
        info("predef: weibull_pdf","Weibull PDF","{k}*x**({k}-1)/{A}**{k}*exp(-(x/{A})**{k})",&[("A",1.),("k",1.)],&[("A",0.1,inf),("k",0.,5.)],&[]),
        info("predef: gentorque","Generator Torque","Piecewise variable-speed generator torque",&[("RtGnSp",100.),("RtTq",1000.),("Rgn2K",0.01),("SlPc",5.),("SpdGenOn",0.)],&[("RtGnSp",0.1,inf),("RtTq",1.,inf),("Rgn2K",0.,0.1),("SlPc",0.,20.),("SpdGenOn",0.,inf)],&[]),
        info("fitter: polynomial_continuous","Polynomial (full)","sum(a_i*x**i)",&[],&[],&[("order",3.)]),
        info("fitter: polynomial_discrete","Polynomial (partial)","sum(a_i*x**exponents[i])",&[],&[],&[]),
        info("fitter: sinusoid","Sinusoid","{A}*sin(2*pi*({f}*x+{phi}/360))+{B}",&[("A",1.),("f",1.),("phi",0.),("B",0.)],&[],&[]),
        info("fitter: secondorder_impulse","2nd order impulse/decay (auto)","A*exp(-zeta*omega*(x-t0))*sin(omega*sqrt(1-zeta**2)*(x-t0))+B",&[("A",1.),("omega",1.),("zeta",0.1),("B",0.),("t0",0.)],&[],&[]),
        info("fitter: secondorder_step","2nd order step (auto)","A*(1-exp(-zeta*omega*(x-t0))/sqrt(1-zeta**2)*cos(omega*sqrt(1-zeta**2)*(x-t0)-asin(zeta)))+B",&[("A",1.),("omega",1.),("zeta",0.1),("B",0.),("t0",0.)],&[],&[]),
    ]
}

#[derive(Debug, Clone)]
enum CompiledModel {
    Formula(Expr),
    Polynomial(Vec<i32>),
    SecondOrder(bool),
    GeneratorTorque,
}
impl CompiledModel {
    fn eval(&self, x: f64, p: &[f64]) -> f64 {
        match self {
            Self::Formula(expr) => expr.eval(x, p),
            Self::Polynomial(e) => p.iter().zip(e).map(|(a, k)| a * x.powi(*k)).sum(),
            Self::SecondOrder(step) => {
                if *step {
                    secondorder_step(x, p)
                } else {
                    secondorder_impulse(x, p)
                }
            }
            Self::GeneratorTorque => gentorque(x, p),
        }
    }
}
pub fn gaussian(x: f64, p: &[f64]) -> f64 {
    1.0 / (p[1] * (2.0 * PI).sqrt()) * (-0.5 * ((x - p[0]) / p[1]).powi(2)).exp()
}
pub fn gaussian_w_offset(x: f64, p: &[f64]) -> f64 {
    gaussian(x, p) + p[2]
}
pub fn logarithmic(x: f64, p: &[f64]) -> f64 {
    p[0] * x.ln() + p[1]
}
pub fn powerlaw_all(x: f64, p: &[f64]) -> f64 {
    p[1] * (x / p[2]).powf(p[0])
}
pub fn powerlaw_alpha(x: f64, p: &[f64], u_ref: f64, z_ref: f64) -> f64 {
    u_ref * (x / z_ref).powf(p[0])
}
pub fn powerlaw_u_alpha(x: f64, p: &[f64], z_ref: f64) -> f64 {
    p[1] * (x / z_ref).powf(p[0])
}
pub fn expdecay(x: f64, p: &[f64]) -> f64 {
    p[0] * (-p[1] * x).exp() + p[2]
}
pub fn weibull_pdf(x: f64, p: &[f64]) -> f64 {
    p[1] * x.powf(p[1] - 1.0) / p[0].powf(p[1]) * (-(x / p[0]).powf(p[1])).exp()
}
pub fn sinusoid(x: f64, p: &[f64]) -> f64 {
    p[0] * (p[1] * x + p[2]).sin() + p[3]
}
pub fn sinusoid_f(x: f64, p: &[f64]) -> f64 {
    p[0] * (2.0 * PI * (p[1] * x + p[2] / 360.0)).sin() + p[3]
}
pub fn secondorder_impulse(x: f64, p: &[f64]) -> f64 {
    if x < p[4] {
        p[3]
    } else {
        let t = x - p[4];
        p[0] * (p[1] * (1.0 - p[2] * p[2]).sqrt() * t).sin() * (-p[2] * p[1] * t).exp() + p[3]
    }
}
pub fn secondorder_step(x: f64, p: &[f64]) -> f64 {
    if x < p[4] {
        return p[3];
    }
    let t = x - p[4];
    let s = (1.0 - p[2] * p[2]).sqrt();
    let phase = p[1] * s * t;
    // sin(omega*s*t)/s has a finite limit at critical damping.
    let transient = phase.cos() + p[2] * if s == 0.0 { p[1] * t } else { phase.sin() / s };
    p[0] * (1.0 - (-p[2] * p[1] * t).exp() * transient) + p[3]
}
pub fn gentorque(x: f64, p: &[f64]) -> f64 {
    let start = p[0] * (1.0 - p[3] / 100.0);
    if x < p[4] {
        0.0
    } else if x >= p[0] {
        p[1]
    } else if x >= start {
        let y = p[2] * start * start;
        (p[1] - y) / (p[0] - start) * (x - start) + y
    } else if x > p[4] {
        p[2] * x * x
    } else {
        0.0
    }
}

/// Fit any registered identifier or `eval: {a}*x**2+{b}`. NaN input pairs are removed.
pub fn model_fit(model: &str, x: &[f64], y: &[f64], options: &FitOptions) -> Result<FitResult> {
    let (x, y) = clean_data(x, y)?;
    let id = model.trim();
    if id == "fitter: polynomial_continuous" {
        let order = options
            .constants
            .get("order")
            .copied()
            .unwrap_or(options.order as f64);
        if !order.is_finite() || order < 0.0 || order.fract() != 0.0 || order >= x.len() as f64 {
            return Err(DvError::msg(
                "Polynomial order must be a nonnegative integer smaller than the sample count",
            ));
        }
        return fit_polynomial_impl(&x, &y, (0..=order as i32).rev().collect(), options);
    }
    if id == "fitter: polynomial_discrete" {
        return fit_polynomial_impl(&x, &y, options.exponents.clone(), options);
    }
    let (compiled, mut spec) = if let Some(formula) = id.strip_prefix("eval:") {
        let (names, _) = extract_variables(formula)?;
        if names.is_empty() {
            return Err(DvError::msg(
                "Custom fitting formula requires at least one {coefficient}",
            ));
        }
        let expr = Parser::parse(formula, &names, &options.constants)?;
        (
            CompiledModel::Formula(expr),
            info(id, "User defined model", formula, &[], &[], &[]),
        )
    } else {
        let spec = models()
            .into_iter()
            .find(|m| m.id == id)
            .ok_or_else(|| DvError::msg(format!("Unknown fitting model: {id}")))?;
        let mut constants = spec.constants.clone();
        constants.extend(options.constants.clone());
        let names: Vec<_> = spec.coefficients.iter().map(|(k, _)| k.clone()).collect();
        let compiled = match id {
            "predef: gentorque" => CompiledModel::GeneratorTorque,
            "predef: secondorder_step" | "fitter: secondorder_step" => {
                CompiledModel::SecondOrder(true)
            }
            "predef: secondorder_impulse" | "fitter: secondorder_impulse" => {
                CompiledModel::SecondOrder(false)
            }
            "fitter: sinusoid" => CompiledModel::Formula(Parser::parse(
                if options.physical {
                    "{A}*sin(2*pi*({f}*x+{phi}/360))+{B}"
                } else {
                    "{A}*sin({f}*x+{phi})+{B}"
                },
                &names,
                &constants,
            )?),
            _ => CompiledModel::Formula(Parser::parse(&spec.formula, &names, &constants)?),
        };
        (compiled, spec)
    };
    if id.starts_with("eval:") {
        let (names, _) = extract_variables(&spec.formula)?;
        spec.coefficients = names.into_iter().map(|name| (name, 0.0)).collect();
    }
    if id == "fitter: sinusoid" {
        let offset = mean(&y);
        let amplitude =
            (2.0 * y.iter().map(|v| (v - offset).powi(2)).sum::<f64>() / y.len() as f64).sqrt();
        let omega = main_frequency(&x, &y)?;
        spec.coefficients = vec![
            ("A".into(), amplitude),
            (
                if options.physical { "f" } else { "omega" }.into(),
                if options.physical {
                    omega / (2.0 * PI)
                } else {
                    omega
                },
            ),
            ("phi".into(), 0.0),
            ("B".into(), offset),
        ];
        spec.formula = if options.physical {
            "{A}*sin(2*pi*({f}*x+{phi}/360))+{B}"
        } else {
            "{A}*sin({omega}*x+{phi})+{B}"
        }
        .into();
    }
    if id.starts_with("fitter: secondorder_") {
        let ymin = y.iter().copied().fold(f64::INFINITY, f64::min);
        let ymax = y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let a = ymax - ymin;
        let omega = main_frequency(&x, &y)?;
        let b = if id.ends_with("step") { y[0] } else { mean(&y) };
        let t0 = x[y
            .iter()
            .position(|v| (v - y[0]).abs() > 0.01 * a)
            .unwrap_or(0)];
        spec.coefficients = vec![
            ("A".into(), a),
            ("omega".into(), omega),
            ("zeta".into(), 0.1),
            ("B".into(), b),
            ("t0".into(), t0),
        ];
        spec.bounds = [
            ("A".into(), (a * 0.1, a * 3.0)),
            (
                "omega".into(),
                (PI / (x[x.len() - 1] - x[0]), PI / (x[1] - x[0])),
            ),
            ("zeta".into(), (0.0, 1.0)),
            ("B".into(), (ymin, ymax)),
            ("t0".into(), (x[0], x[x.len() - 1])),
        ]
        .into_iter()
        .collect();
    }
    let names: Vec<_> = spec.coefficients.iter().map(|(k, _)| k.clone()).collect();
    check_keys(&names, &options.guesses, "guess")?;
    check_keys(&names, &options.bounds, "bound")?;
    if !options.guesses.is_empty() && names.iter().any(|k| !options.guesses.contains_key(k)) {
        return Err(DvError::msg(
            "Guess dictionary must supply every model coefficient",
        ));
    }
    let mut p = Vec::new();
    let mut bounds = Vec::new();
    for (name, default) in &spec.coefficients {
        let (lo, hi) = options
            .bounds
            .get(name)
            .or_else(|| spec.bounds.get(name))
            .copied()
            .unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
        if lo.is_nan() || hi.is_nan() || lo >= hi {
            return Err(DvError::msg(format!("Invalid bounds for {name}")));
        }
        let default = if id.starts_with("eval:") && options.guesses.is_empty() {
            middle_of_bounds(lo, hi)
        } else {
            *default
        };
        let guess = options.guesses.get(name).copied().unwrap_or(default);
        if !guess.is_finite() || guess < lo || guess > hi {
            return Err(DvError::msg(format!(
                "Guess for {name} is nonfinite or outside its bounds"
            )));
        }
        p.push(guess);
        bounds.push((lo, hi));
    }
    let result = nonlinear_least_squares(&x, &y, &p, &bounds, options.max_iterations, |x, p| {
        compiled.eval(x, p)
    })?;
    p = result.parameters;
    finish(x, y, p, names, spec.formula, compiled, result.iterations)
}
fn check_keys<T>(names: &[String], values: &BTreeMap<String, T>, kind: &str) -> Result<()> {
    if let Some(key) = values.keys().find(|k| !names.contains(k)) {
        return Err(DvError::msg(format!("Unknown {kind} coefficient: {key}")));
    }
    Ok(())
}
fn middle_of_bounds(lo: f64, hi: f64) -> f64 {
    match (lo.is_finite(), hi.is_finite()) {
        (false, false) => 0.0,
        (true, true) => (lo + hi) / 2.0,
        (true, false) => {
            if lo == 0.0 {
                1.0
            } else {
                lo + lo.abs()
            }
        }
        (false, true) => {
            if hi == 0.0 {
                -1.0
            } else {
                hi - hi.abs()
            }
        }
    }
}
fn mean(y: &[f64]) -> f64 {
    y.iter().sum::<f64>() / y.len() as f64
}
fn clean_data(x: &[f64], y: &[f64]) -> Result<(Vec<f64>, Vec<f64>)> {
    if x.len() != y.len() {
        return Err(DvError::msg("Curve fitting x and y lengths differ"));
    }
    let (x, y): (Vec<_>, Vec<_>) = x
        .iter()
        .zip(y)
        .filter(|(x, y)| !x.is_nan() && !y.is_nan())
        .map(|(&x, &y)| (x, y))
        .unzip();
    if x.is_empty() || x.iter().chain(&y).any(|v| !v.is_finite()) {
        return Err(DvError::msg("Curve fitting requires finite nonempty data"));
    }
    Ok((x, y))
}
fn finish(
    x: Vec<f64>,
    y: Vec<f64>,
    p: Vec<f64>,
    names: Vec<String>,
    formula: String,
    model: CompiledModel,
    iterations: usize,
) -> Result<FitResult> {
    let y_fit: Vec<_> = x.iter().map(|v| model.eval(*v, &p)).collect();
    let residual = y
        .iter()
        .zip(&y_fit)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>();
    let rmse = (residual / y.len() as f64).sqrt();
    let r2 = rsquare(&y, &y_fit)?;
    Ok(FitResult {
        x,
        y_fit,
        coefficients: names.into_iter().zip(p).collect(),
        r2,
        rmse,
        formula,
        iterations,
        model,
    })
}
pub fn rsquare(y: &[f64], fit: &[f64]) -> Result<f64> {
    if y.len() != fit.len() || y.is_empty() {
        return Err(DvError::msg("R squared requires matching nonempty arrays"));
    }
    let m = mean(y);
    let total = y.iter().map(|v| (v - m).powi(2)).sum::<f64>();
    let residual = y.iter().zip(fit).map(|(a, b)| (a - b).powi(2)).sum::<f64>();
    Ok(if total == 0.0 {
        if residual == 0.0 {
            1.0
        } else {
            0.0
        }
    } else {
        (1.0 - residual / total).max(0.0)
    })
}
/// Dominant non-DC FFT angular frequency, matching upstream's initialization.
pub fn main_frequency(t: &[f64], y: &[f64]) -> Result<f64> {
    if t.len() != y.len() || t.len() < 2 || t[1] <= t[0] {
        return Err(DvError::msg(
            "Frequency initialization requires increasing sampled times",
        ));
    }
    let mut data: Vec<_> = y.iter().map(|v| Complex::new(*v, 0.0)).collect();
    let mut planner = FftPlanner::new();
    planner.plan_fft_forward(y.len()).process(&mut data);
    let k = (1..=y.len() / 2)
        .max_by(|a, b| data[*a].norm_sqr().total_cmp(&data[*b].norm_sqr()))
        .unwrap_or(1);
    Ok(2.0 * PI * k as f64 / (y.len() as f64 * (t[1] - t[0])))
}
pub fn fit_sinusoid(x: &[f64], y: &[f64], physical: bool) -> Result<FitResult> {
    model_fit(
        "fitter: sinusoid",
        x,
        y,
        &FitOptions {
            physical,
            ..Default::default()
        },
    )
}
pub fn fit_gaussian(x: &[f64], y: &[f64], offset: bool) -> Result<FitResult> {
    model_fit(
        if offset {
            "predef: gaussian-yoff"
        } else {
            "predef: gaussian"
        },
        x,
        y,
        &FitOptions::default(),
    )
}
pub fn fit_polynomial_continuous(x: &[f64], y: &[f64], order: usize) -> Result<FitResult> {
    model_fit(
        "fitter: polynomial_continuous",
        x,
        y,
        &FitOptions {
            order,
            ..Default::default()
        },
    )
}
pub fn fit_polynomial_discrete(x: &[f64], y: &[f64], exponents: &[i32]) -> Result<FitResult> {
    model_fit(
        "fitter: polynomial_discrete",
        x,
        y,
        &FitOptions {
            exponents: exponents.to_vec(),
            ..Default::default()
        },
    )
}
pub fn fit_powerlaw_u_alpha(
    x: &[f64],
    y: &[f64],
    z_ref: f64,
    guess: (f64, f64),
) -> Result<FitResult> {
    model_fit(
        "predef: powerlaw_u_alpha",
        x,
        y,
        &FitOptions {
            constants: [("z_ref".into(), z_ref)].into_iter().collect(),
            guesses: [("alpha".into(), guess.1), ("u_ref".into(), guess.0)]
                .into_iter()
                .collect(),
            ..Default::default()
        },
    )
}
fn fit_polynomial_impl(
    x: &[f64],
    y: &[f64],
    exponents: Vec<i32>,
    options: &FitOptions,
) -> Result<FitResult> {
    if exponents.is_empty() || exponents.len() > x.len() {
        return Err(DvError::msg(
            "Polynomial fit needs at least as many samples as coefficients",
        ));
    }
    if exponents
        .iter()
        .enumerate()
        .any(|(i, v)| exponents[..i].contains(v))
    {
        return Err(DvError::msg("Polynomial exponents must be distinct"));
    }
    let names: Vec<_> = (0..exponents.len())
        .map(|i| {
            if i < 26 {
                ((b'a' + i as u8) as char).to_string()
            } else {
                format!("a{i}")
            }
        })
        .collect();
    let matrix = DMatrix::from_fn(x.len(), exponents.len(), |i, j| x[i].powi(exponents[j]));
    if matrix.iter().any(|v| !v.is_finite()) {
        return Err(DvError::msg("Polynomial basis is nonfinite"));
    }
    let rhs = DVector::from_column_slice(y);
    let svd = matrix.svd(true, true);
    let tolerance = f64::EPSILON
        * x.len().max(exponents.len()) as f64
        * svd.singular_values.iter().copied().fold(0.0, f64::max);
    let solution = svd.solve(&rhs, tolerance).map_err(DvError::msg)?;
    let compiled = CompiledModel::Polynomial(exponents.clone());
    let mut p: Vec<_> = solution.iter().copied().collect();
    let mut iterations = 1;
    if !options.bounds.is_empty() || !options.guesses.is_empty() {
        check_keys(&names, &options.guesses, "guess")?;
        check_keys(&names, &options.bounds, "bound")?;
        let bounds: Vec<_> = names
            .iter()
            .map(|n| {
                options
                    .bounds
                    .get(n)
                    .copied()
                    .unwrap_or((f64::NEG_INFINITY, f64::INFINITY))
            })
            .collect();
        for (i, n) in names.iter().enumerate() {
            if bounds[i].0.is_nan() || bounds[i].1.is_nan() || bounds[i].0 >= bounds[i].1 {
                return Err(DvError::msg(format!("Invalid bounds for {n}")));
            }
            p[i] = options
                .guesses
                .get(n)
                .copied()
                .unwrap_or_else(|| p[i].clamp(bounds[i].0, bounds[i].1));
        }
        let fit = nonlinear_least_squares(x, y, &p, &bounds, options.max_iterations, |x, p| {
            compiled.eval(x, p)
        })?;
        p = fit.parameters;
        iterations = fit.iterations;
    }
    let formula = names
        .iter()
        .zip(&exponents)
        .map(|(n, e)| format!("{{{n}}}*x**{e}"))
        .collect::<Vec<_>>()
        .join("+");
    finish(
        x.to_vec(),
        y.to_vec(),
        p,
        names,
        formula,
        compiled,
        iterations,
    )
}
/// Least-squares coefficients of x^i*y^j in increasing i then j order.
pub fn polyfit2d(
    x: &[f64],
    y: &[f64],
    z: &[f64],
    kx: usize,
    ky: usize,
    order: Option<usize>,
) -> Result<Vec<f64>> {
    if x.len() != y.len() || x.len() != z.len() || x.is_empty() {
        return Err(DvError::msg(
            "2D polynomial fit requires matching nonempty coordinate arrays",
        ));
    }
    let powers: Vec<_> = (0..=kx)
        .flat_map(|i| {
            (0..=ky)
                .filter(move |j| order.is_none_or(|n| i + j <= n))
                .map(move |j| (i, j))
        })
        .collect();
    let a = DMatrix::from_fn(x.len(), powers.len(), |n, j| {
        x[n].powi(powers[j].0 as i32) * y[n].powi(powers[j].1 as i32)
    });
    let rhs = DVector::from_column_slice(z);
    let svd = a.svd(true, true);
    let tol = f64::EPSILON
        * x.len().max(powers.len()) as f64
        * svd.singular_values.iter().copied().fold(0.0, f64::max);
    Ok(svd
        .solve(&rhs, tol)
        .map_err(DvError::msg)?
        .iter()
        .copied()
        .collect())
}

#[derive(Debug, Clone)]
pub struct LeastSquaresResult {
    pub parameters: Vec<f64>,
    pub iterations: usize,
    pub residual_sum_squares: f64,
}
/// Real finite-difference, box-constrained Levenberg–Marquardt least squares.
/// Nonconvergence and nonfinite model evaluations are reported, never replaced by guesses.
pub fn nonlinear_least_squares<F>(
    x: &[f64],
    y: &[f64],
    initial: &[f64],
    bounds: &[(f64, f64)],
    max_iterations: usize,
    model: F,
) -> Result<LeastSquaresResult>
where
    F: Fn(f64, &[f64]) -> f64,
{
    let n = initial.len();
    let rows = x.len();
    if x.len() != y.len() || rows < n || n == 0 || bounds.len() != n || max_iterations == 0 {
        return Err(DvError::msg(
            "Invalid nonlinear least-squares dimensions or iteration limit",
        ));
    }
    if x.iter().chain(y).any(|v| !v.is_finite()) {
        return Err(DvError::msg("Least-squares observations must be finite"));
    }
    for (p, (lo, hi)) in initial.iter().zip(bounds) {
        if !p.is_finite() || lo.is_nan() || hi.is_nan() || lo >= hi || p < lo || p > hi {
            return Err(DvError::msg(
                "Invalid least-squares bounds or initial coefficients",
            ));
        }
    }
    let mut p = initial.to_vec();
    let mut trial = p.clone();
    let mut residual = vec![0.0; rows];
    let mut trial_residual = residual.clone();
    let mut jac = DMatrix::zeros(rows, n);
    let mut normal = DMatrix::zeros(n, n);
    let mut damped = normal.clone();
    let mut gradient = DVector::zeros(n);
    fn evaluate<F: Fn(f64, &[f64]) -> f64>(
        x: &[f64],
        y: &[f64],
        p: &[f64],
        r: &mut [f64],
        f: &F,
    ) -> f64 {
        let mut sum = 0.0;
        for i in 0..x.len() {
            r[i] = y[i] - f(x[i], p);
            sum += r[i] * r[i];
        }
        sum
    }
    let mut cost = evaluate(x, y, &p, &mut residual, &model);
    if !cost.is_finite() {
        return Err(DvError::msg(
            "Model is nonfinite at initial coefficients; supply valid guesses",
        ));
    }
    let mut lambda = 1e-3_f64;
    for iteration in 0..max_iterations {
        if cost <= f64::EPSILON * f64::EPSILON * rows as f64 {
            return Ok(LeastSquaresResult {
                parameters: p,
                iterations: iteration,
                residual_sum_squares: cost,
            });
        }
        for j in 0..n {
            let h = f64::EPSILON.cbrt() * (p[j].abs() + 1.0);
            let plus = (p[j] + h).min(bounds[j].1);
            let minus = (p[j] - h).max(bounds[j].0);
            trial.copy_from_slice(&p);
            trial[j] = plus;
            for i in 0..rows {
                jac[(i, j)] = model(x[i], &trial);
            }
            trial[j] = minus;
            for i in 0..rows {
                let low = model(x[i], &trial);
                let high = jac[(i, j)];
                jac[(i, j)] = if high.is_finite() && low.is_finite() {
                    (high - low) / (plus - minus)
                } else if high.is_finite() {
                    (high - (y[i] - residual[i])) / (plus - p[j])
                } else if low.is_finite() {
                    ((y[i] - residual[i]) - low) / (p[j] - minus)
                } else {
                    f64::NAN
                };
            }
        }
        if jac.iter().any(|v| !v.is_finite()) {
            return Err(DvError::msg(
                "Model derivative is nonfinite inside the requested bounds",
            ));
        }
        normal.fill(0.0);
        gradient.fill(0.0);
        for i in 0..rows {
            for j in 0..n {
                gradient[j] += jac[(i, j)] * residual[i];
                for k in 0..=j {
                    normal[(j, k)] += jac[(i, j)] * jac[(i, k)];
                }
            }
        }
        for j in 0..n {
            for k in 0..j {
                normal[(k, j)] = normal[(j, k)];
            }
        }
        let projected_gradient = (0..n)
            .map(|j| {
                let g = gradient[j];
                if (p[j] <= bounds[j].0 && g < 0.0) || (p[j] >= bounds[j].1 && g > 0.0) {
                    0.0
                } else {
                    g.abs() / normal[(j, j)].sqrt().max(1e-15)
                }
            })
            .fold(0.0, f64::max);
        if projected_gradient <= 1e-10 * (cost.sqrt() + 1.0) {
            return Ok(LeastSquaresResult {
                parameters: p,
                iterations: iteration,
                residual_sum_squares: cost,
            });
        }
        // Bound-active parameters must not drive the coupled free-variable
        // solve. Clamping an unconstrained step alone can stall at a boundary.
        for j in 0..n {
            if (p[j] <= bounds[j].0 && gradient[j] < 0.0)
                || (p[j] >= bounds[j].1 && gradient[j] > 0.0)
            {
                for k in 0..n {
                    normal[(j, k)] = 0.0;
                    normal[(k, j)] = 0.0;
                }
                normal[(j, j)] = 1.0;
                gradient[j] = 0.0;
            }
        }
        let mut accepted = false;
        for _ in 0..32 {
            damped.copy_from(&normal);
            for j in 0..n {
                damped[(j, j)] += lambda * normal[(j, j)].max(1e-12);
            }
            let Some(step) = damped.clone().cholesky().map(|c| c.solve(&gradient)) else {
                lambda *= 10.0;
                continue;
            };
            let mut step_norm = 0.0_f64;
            for j in 0..n {
                trial[j] = (p[j] + step[j]).clamp(bounds[j].0, bounds[j].1);
                step_norm = step_norm.max((trial[j] - p[j]).abs() / (p[j].abs() + 1.0));
            }
            let new_cost = evaluate(x, y, &trial, &mut trial_residual, &model);
            if new_cost.is_finite() && new_cost < cost {
                let reduction = cost - new_cost;
                p.copy_from_slice(&trial);
                residual.copy_from_slice(&trial_residual);
                cost = new_cost;
                lambda = (lambda / 3.0).max(1e-15);
                accepted = true;
                if step_norm <= 1e-10 || reduction <= 1e-12 * cost.max(1e-24) {
                    return Ok(LeastSquaresResult {
                        parameters: p,
                        iterations: iteration + 1,
                        residual_sum_squares: cost,
                    });
                }
                break;
            }
            if step_norm <= 1e-12 && projected_gradient <= 1e-7 * (cost.sqrt() + 1.0) {
                return Ok(LeastSquaresResult {
                    parameters: p,
                    iterations: iteration + 1,
                    residual_sum_squares: cost,
                });
            }
            lambda *= 10.0;
        }
        if !accepted {
            return Err(DvError::msg("Nonlinear least squares could not find a decreasing step; change coefficient guesses"));
        }
    }
    Err(DvError::msg(format!(
        "Nonlinear least squares did not converge in {max_iterations} iterations"
    )))
}

#[derive(Debug, Clone)]
enum Expr {
    Constant(f64),
    X,
    Parameter(usize),
    Neg(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call(Function, Vec<Expr>),
}
#[derive(Debug, Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Mod,
}
#[derive(Debug, Clone, Copy)]
enum Function {
    Exp,
    Log,
    Log10,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Abs,
    Sinh,
    Cosh,
    Tanh,
    Floor,
    Ceil,
    Sign,
    Pow,
    Min,
    Max,
}
impl Expr {
    fn eval(&self, x: f64, p: &[f64]) -> f64 {
        match self {
            Self::Constant(v) => *v,
            Self::X => x,
            Self::Parameter(i) => p[*i],
            Self::Neg(v) => -v.eval(x, p),
            Self::Binary(op, a, b) => {
                let a = a.eval(x, p);
                let b = b.eval(x, p);
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Pow => a.powf(b),
                    Op::Mod => {
                        let r = a % b;
                        if r != 0.0 && r.signum() != b.signum() {
                            r + b
                        } else {
                            r
                        }
                    }
                }
            }
            Self::Call(fun, args) => {
                let a = args[0].eval(x, p);
                match fun {
                    Function::Exp => a.exp(),
                    Function::Log => a.ln(),
                    Function::Log10 => a.log10(),
                    Function::Sqrt => a.sqrt(),
                    Function::Sin => a.sin(),
                    Function::Cos => a.cos(),
                    Function::Tan => a.tan(),
                    Function::Asin => a.asin(),
                    Function::Acos => a.acos(),
                    Function::Atan => a.atan(),
                    Function::Abs => a.abs(),
                    Function::Sinh => a.sinh(),
                    Function::Cosh => a.cosh(),
                    Function::Tanh => a.tanh(),
                    Function::Floor => a.floor(),
                    Function::Ceil => a.ceil(),
                    Function::Sign => {
                        if a == 0.0 {
                            0.0
                        } else {
                            a.signum()
                        }
                    }
                    Function::Atan2 => a.atan2(args[1].eval(x, p)),
                    Function::Pow => a.powf(args[1].eval(x, p)),
                    Function::Min => a.min(args[1].eval(x, p)),
                    Function::Max => a.max(args[1].eval(x, p)),
                }
            }
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Parameter(String),
    Plus,
    Minus,
    Star,
    Slash,
    Pow,
    Percent,
    Open,
    Close,
    Comma,
    End,
}
fn tokenize(formula: &str) -> Result<Vec<Token>> {
    let bytes = formula.as_bytes();
    let mut i = 0;
    let mut tokens = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let token = match c {
            b'0'..=b'9' | b'.' if c != b'.' || bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                Token::Number(
                    formula[start..i]
                        .parse()
                        .map_err(|_| DvError::msg("Invalid formula number"))?,
                )
            }
            b'{' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                if i == bytes.len() {
                    return Err(DvError::msg("Unclosed coefficient reference"));
                }
                let name = formula[start..i].trim().to_string();
                i += 1;
                if name.is_empty() {
                    return Err(DvError::msg("Empty coefficient name"));
                }
                Token::Parameter(name)
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
                {
                    i += 1;
                }
                Token::Name(formula[start..i].into())
            }
            b'+' => {
                i += 1;
                Token::Plus
            }
            b'-' => {
                i += 1;
                Token::Minus
            }
            b'*' => {
                i += 1;
                if bytes.get(i) == Some(&b'*') {
                    i += 1;
                    Token::Pow
                } else {
                    Token::Star
                }
            }
            b'^' => {
                i += 1;
                Token::Pow
            }
            b'/' => {
                i += 1;
                Token::Slash
            }
            b'%' => {
                i += 1;
                Token::Percent
            }
            b'(' => {
                i += 1;
                Token::Open
            }
            b')' => {
                i += 1;
                Token::Close
            }
            b',' => {
                i += 1;
                Token::Comma
            }
            _ => {
                return Err(DvError::msg(format!(
                    "Unsupported formula character: {}",
                    c as char
                )))
            }
        };
        tokens.push(token);
    }
    tokens.push(Token::End);
    Ok(tokens)
}
struct Parser<'a> {
    tokens: Vec<Token>,
    position: usize,
    names: &'a [String],
    constants: &'a BTreeMap<String, f64>,
}
impl<'a> Parser<'a> {
    fn parse(s: &str, names: &'a [String], constants: &'a BTreeMap<String, f64>) -> Result<Expr> {
        let mut parser = Self {
            tokens: tokenize(s)?,
            position: 0,
            names,
            constants,
        };
        let expr = parser.expression(0)?;
        if parser.tokens[parser.position] != Token::End {
            return Err(DvError::msg("Unexpected token after formula expression"));
        }
        Ok(expr)
    }
    fn expression(&mut self, min_binding: u8) -> Result<Expr> {
        let token = self.tokens[self.position].clone();
        self.position += 1;
        let mut lhs = match token {
            Token::Number(v) => Expr::Constant(v),
            Token::Parameter(name) => Expr::Parameter(
                self.names
                    .iter()
                    .position(|k| k == &name)
                    .ok_or_else(|| DvError::msg(format!("Unknown coefficient: {name}")))?,
            ),
            Token::Plus => self.expression(5)?,
            Token::Minus => Expr::Neg(Box::new(self.expression(5)?)),
            Token::Open => {
                let expr = self.expression(0)?;
                self.expect(Token::Close)?;
                expr
            }
            Token::Name(name) => {
                let bare = name
                    .strip_prefix("np.")
                    .or_else(|| name.strip_prefix("numpy."))
                    .or_else(|| name.strip_prefix("math."))
                    .unwrap_or(&name);
                if self.tokens[self.position] == Token::Open {
                    self.position += 1;
                    let (function, n) = match bare {
                        "exp" => (Function::Exp, 1),
                        "log" | "ln" => (Function::Log, 1),
                        "log10" => (Function::Log10, 1),
                        "sqrt" => (Function::Sqrt, 1),
                        "sin" => (Function::Sin, 1),
                        "cos" => (Function::Cos, 1),
                        "tan" => (Function::Tan, 1),
                        "arcsin" | "asin" => (Function::Asin, 1),
                        "arccos" | "acos" => (Function::Acos, 1),
                        "arctan" | "atan" => (Function::Atan, 1),
                        "arctan2" | "atan2" => (Function::Atan2, 2),
                        "abs" | "absolute" => (Function::Abs, 1),
                        "sinh" => (Function::Sinh, 1),
                        "cosh" => (Function::Cosh, 1),
                        "tanh" => (Function::Tanh, 1),
                        "floor" => (Function::Floor, 1),
                        "ceil" => (Function::Ceil, 1),
                        "sign" => (Function::Sign, 1),
                        "pow" | "power" => (Function::Pow, 2),
                        "min" | "minimum" => (Function::Min, 2),
                        "max" | "maximum" => (Function::Max, 2),
                        _ => {
                            return Err(DvError::msg(format!(
                                "Unsupported formula function: {name}"
                            )))
                        }
                    };
                    let mut args = Vec::with_capacity(n);
                    for i in 0..n {
                        if i > 0 {
                            self.expect(Token::Comma)?;
                        }
                        args.push(self.expression(0)?);
                    }
                    self.expect(Token::Close)?;
                    Expr::Call(function, args)
                } else {
                    match bare {
                        "x" | "t" | "z" => Expr::X,
                        "pi" => Expr::Constant(PI),
                        "e" => Expr::Constant(std::f64::consts::E),
                        "inf" => Expr::Constant(f64::INFINITY),
                        _ => Expr::Constant(*self.constants.get(&name).ok_or_else(|| {
                            DvError::msg(format!("Unknown formula constant: {name}"))
                        })?),
                    }
                }
            }
            _ => return Err(DvError::msg("Expected a formula operand")),
        };
        loop {
            let (op, left, right) = match self.tokens[self.position] {
                Token::Plus => (Op::Add, 1, 2),
                Token::Minus => (Op::Sub, 1, 2),
                Token::Star => (Op::Mul, 3, 4),
                Token::Slash => (Op::Div, 3, 4),
                Token::Percent => (Op::Mod, 3, 4),
                Token::Pow => (Op::Pow, 7, 6),
                _ => break,
            };
            if left < min_binding {
                break;
            }
            self.position += 1;
            let rhs = self.expression(right)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn expect(&mut self, token: Token) -> Result<()> {
        if self.tokens.get(self.position) != Some(&token) {
            return Err(DvError::msg(format!("Expected formula token {token:?}")));
        }
        self.position += 1;
        Ok(())
    }
}
/// Ordered unique coefficient names and a display/evaluation formula with p[i] references.
pub fn extract_variables(formula: &str) -> Result<(Vec<String>, String)> {
    let tokens = tokenize(formula)?;
    let mut names = Vec::new();
    let mut rewritten = formula.to_string();
    for token in tokens {
        if let Token::Parameter(name) = token {
            if !names.contains(&name) {
                let i = names.len();
                rewritten = rewritten.replace(&format!("{{{name}}}"), &format!("p[{i}]"));
                names.push(name);
            }
        }
    }
    Ok((names, rewritten))
}
fn scalar_number(s: &str) -> Result<f64> {
    Ok(Parser::parse(s, &[], &BTreeMap::new())?.eval(0.0, &[]))
}
fn comma_part(s: &str) -> Result<(&str, &str)> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| DvError::msg("Unexpected closing parenthesis"))?
            }
            ',' if depth == 0 => return Ok((&s[..i], &s[i + 1..])),
            _ => {}
        }
    }
    if depth != 0 {
        return Err(DvError::msg("Unclosed parameter expression"));
    }
    Ok((s, ""))
}
pub fn parse_guesses(s: &str) -> Result<BTreeMap<String, f64>> {
    let mut result = BTreeMap::new();
    if s.trim().is_empty() {
        return Ok(result);
    }
    let mut rest = s.trim();
    while !rest.is_empty() {
        let (pair, tail) = comma_part(rest)?;
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| DvError::msg("Guesses must be name=value pairs"))?;
        let key = key.trim();
        if key.is_empty() || result.contains_key(key) {
            return Err(DvError::msg("Empty or duplicate guess name"));
        }
        result.insert(key.into(), scalar_number(value.trim())?);
        rest = tail.trim();
    }
    Ok(result)
}
pub fn parse_bounds(s: &str) -> Result<BTreeMap<String, (f64, f64)>> {
    let mut result = BTreeMap::new();
    let mut rest = s.trim();
    while !rest.is_empty() {
        let (part, remainder) = comma_part(rest)?;
        let (key, tail) = part
            .split_once('=')
            .ok_or_else(|| DvError::msg("Bounds must be name=(lower,upper) pairs"))?;
        let key = key.trim();
        let tail = tail.trim();
        if !tail.starts_with('(') || !tail.ends_with(')') {
            return Err(DvError::msg("Bounds require parenthesized pairs"));
        }
        let (lower, upper) = comma_part(&tail[1..tail.len() - 1])?;
        if upper.trim().is_empty() {
            return Err(DvError::msg("Bound pair must have two values"));
        }
        if key.is_empty() || result.contains_key(key) {
            return Err(DvError::msg("Empty or duplicate bound name"));
        }
        let pair = (scalar_number(lower.trim())?, scalar_number(upper.trim())?);
        if pair.0.is_nan() || pair.1.is_nan() || pair.0 >= pair.1 {
            return Err(DvError::msg(format!("Invalid bounds for {key}")));
        }
        result.insert(key.into(), pair);
        rest = remainder.trim();
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn polynomial_coefficients() {
        let x: Vec<_> = (0..30).map(|i| i as f64 / 10.).collect();
        let y: Vec<_> = x.iter().map(|v| 2. * v * v + 3. * v + 4.).collect();
        let fit = fit_polynomial_continuous(&x, &y, 2).unwrap();
        for ((_, a), b) in fit.coefficients.iter().zip([2., 3., 4.]) {
            assert!((a - b).abs() < 1e-10);
        }
    }
    #[test]
    fn bounded_custom_exponential() {
        let x: Vec<_> = (0..100).map(|i| i as f64 / 20.).collect();
        let y: Vec<_> = x.iter().map(|v| 3. * (-0.7 * v).exp() + 2.).collect();
        let options = FitOptions {
            guesses: parse_guesses("A=1,k=1,B=1").unwrap(),
            bounds: parse_bounds("A=(0,10),k=(0,3),B=(-inf,inf)").unwrap(),
            ..Default::default()
        };
        let fit = model_fit("eval: {A}*np.exp(-{k}*x)+{B}", &x, &y, &options).unwrap();
        for ((_, a), b) in fit.coefficients.iter().zip([3., 0.7, 2.]) {
            assert!((a - b).abs() < 1e-6);
        }
        assert!(fit.r2 > 0.999999);
    }
    #[test]
    fn power_precedence() {
        let e = Parser::parse("-x**2+2**3**2", &[], &BTreeMap::new()).unwrap();
        assert_eq!(e.eval(2., &[]), 508.);
    }
    #[test]
    fn sinusoid_fit() {
        let x: Vec<_> = (0..500).map(|i| i as f64 / 50.).collect();
        let y: Vec<_> = x
            .iter()
            .map(|v| 2. * (2. * PI * 0.7 * v + 0.4).sin() + 3.)
            .collect();
        let fit = fit_sinusoid(&x, &y, true).unwrap();
        assert!(fit.rmse < 1e-6);
    }
    #[test]
    fn invalid_polynomial_bounds_return_error() {
        let options = FitOptions {
            bounds: [("a".into(), (2., 1.))].into_iter().collect(),
            ..Default::default()
        };
        assert!(model_fit(
            "fitter: polynomial_continuous",
            &[0., 1., 2., 3.],
            &[0., 1., 2., 3.],
            &options
        )
        .is_err());
    }
    #[test]
    fn critical_step_and_zero_slip_torque_are_finite() {
        assert!(
            (secondorder_step(1., &[1., 2., 1., 0., 0.]) - (1. - 3. * (-2.0_f64).exp())).abs()
                < 1e-12
        );
        assert_eq!(gentorque(100., &[100., 1000., 0.01, 0., 0.]), 1000.);
    }
    #[test]
    fn active_bound_does_not_stall_free_coefficient() {
        let fit = nonlinear_least_squares(
            &[0., 1., 2.],
            &[0., 0., 0.],
            &[1., 0.],
            &[(1., 10.), (-10., 10.)],
            100,
            |x, p| p[0] * x + p[1],
        )
        .unwrap();
        assert!((fit.parameters[0] - 1.).abs() < 1e-8);
        assert!((fit.parameters[1] + 1.).abs() < 1e-8);
    }
    #[test]
    fn nested_parameter_expressions() {
        assert_eq!(parse_guesses("A=pow(2,3), B=sqrt(4)").unwrap()["A"], 8.);
        assert_eq!(
            parse_bounds("A=(sqrt(4), pow(2,3)), B=(-inf,pi)").unwrap()["A"],
            (2., 8.)
        );
        assert_eq!(
            Parser::parse("x%-2", &[], &BTreeMap::new())
                .unwrap()
                .eval(3., &[]),
            -1.
        );
    }
}
