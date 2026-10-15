use super::input::{broken, float, read_text};
use dv_core::{DataFrame, FileObject, IoError, NamedFrame};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const WIND_COLUMNS: [&str; 8] = [
    "Time_[s]",
    "WindSpeed_[m/s]",
    "WindDir_[deg]",
    "VertSpeed_[m/s]",
    "HorizShear_[-]",
    "VertShear_[-]",
    "LinVShear_[-]",
    "GustSpeed_[m/s]",
];
#[derive(Debug, Clone)]
pub struct FastWindFile {
    pub filename: PathBuf,
    pub data: DataFrame,
    pub header: Vec<String>,
}
#[derive(Debug, Clone, Copy)]
pub enum StepDuration {
    Constant(f64),
    Range(f64, f64),
}
#[derive(Debug, Clone, Copy)]
pub struct StepWindOptions {
    pub speed_step: f64,
    pub min_speed: f64,
    pub max_speed: f64,
    pub duration: StepDuration,
    pub dt: f64,
    pub min_time: f64,
    pub max_time: f64,
}
impl Default for StepWindOptions {
    fn default() -> Self {
        Self {
            speed_step: 1.0,
            min_speed: 3.0,
            max_speed: 25.0,
            duration: StepDuration::Constant(100.0),
            dt: 0.5,
            min_time: 0.0,
            max_time: 999.0,
        }
    }
}
impl FastWindFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        Self::parse(path, &read_text(path)?)
    }
    pub fn parse(path: &Path, text: &str) -> Result<Self, IoError> {
        let mut rows = Vec::new();
        let mut header = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.starts_with('!') {
                header.push(line.into());
                continue;
            }
            if line.is_empty() {
                continue;
            }
            let vals: Vec<_> = line
                .split('!')
                .next()
                .unwrap()
                .split_whitespace()
                .map(float)
                .collect();
            if vals.len() != 8 || vals.iter().any(Option::is_none) {
                return Err(IoError::WrongFormat(format!(
                    "expected eight numeric wind columns at {}",
                    i + 1
                )));
            }
            rows.push(vals.into_iter().map(Option::unwrap).collect());
        }
        if rows.is_empty() {
            return Err(IoError::WrongFormat("no deterministic wind data".into()));
        }
        let data = DataFrame::from_rows(WIND_COLUMNS, &rows).map_err(|e| broken(e.to_string()))?;
        Ok(Self {
            filename: path.into(),
            data,
            header,
        })
    }
    pub fn step_wind(options: StepWindOptions) -> Result<Self, IoError> {
        let o = options;
        if !o.speed_step.is_finite()
            || o.speed_step <= 0.0
            || o.max_speed < o.min_speed
            || !o.min_speed.is_finite()
            || !o.max_speed.is_finite()
        {
            return Err(broken("invalid step wind speed range"));
        }
        let count = ((o.max_speed + o.speed_step - o.min_speed) / o.speed_step).ceil() as usize;
        let steps: Vec<_> = (0..count)
            .map(|i| o.min_speed + i as f64 * o.speed_step)
            .collect();
        if steps.is_empty() {
            return Err(broken("empty step wind speeds"));
        }
        let durations: Vec<_> = (0..count)
            .map(|i| match o.duration {
                StepDuration::Constant(d) => d,
                StepDuration::Range(a, b) => {
                    if count == 1 {
                        a.round_ties_even()
                    } else {
                        (a + (b - a) * i as f64 / (count - 1) as f64).round_ties_even()
                    }
                }
            })
            .collect();
        if durations.iter().any(|d| !d.is_finite() || *d <= o.dt) || !o.dt.is_finite() || o.dt < 0.0
        {
            return Err(broken("step wind durations must exceed dt"));
        }
        let mut rows = vec![vec![0.0; 8]; 2 * count];
        rows[0][0] = o.min_time;
        rows[0][1] = o.min_speed;
        let mut time = o.min_time;
        for i in 0..count - 1 {
            rows[2 * i + 1][0] = time + durations[i] - o.dt;
            rows[2 * i + 2][0] = time + durations[i];
            time += durations[i];
            rows[2 * i + 1][1] = steps[i];
            rows[2 * i + 2][1] = steps[i + 1];
        }
        rows[2 * count - 1][0] = o.max_time.max(time + durations[count - 1]);
        rows[2 * count - 1][1] = o.max_speed;
        let data = DataFrame::from_rows(WIND_COLUMNS, &rows).map_err(|e| broken(e.to_string()))?;
        Ok(Self {
            filename: PathBuf::new(),
            data,
            header: vec![
                "!Wind file.".into(),
                "!Time WindSpeed WindDir VertSpeed HorizShear VertShear LinVShear GustSpeed".into(),
            ],
        })
    }
    pub fn write(&self, path: &Path) -> Result<(), IoError> {
        write_wind_dataframe(&self.data, path)
    }
}
pub fn write_wind_dataframe(df: &DataFrame, path: &Path) -> Result<(), IoError> {
    if df.ncols() != 8 {
        return Err(broken("deterministic wind files require eight columns"));
    }
    let values: Vec<_> = df
        .columns()
        .iter()
        .map(|c| {
            c.floats()
                .ok_or_else(|| broken("wind columns must be numeric"))
        })
        .collect::<Result<_, _>>()?;
    let mut s = String::from(
        "!Wind file.\n!Time WindSpeed WindDir VertSpeed HorizShear VertShear LinVShear GustSpeed\n",
    );
    for i in 0..df.nrows() {
        for (j, c) in values.iter().enumerate() {
            if j > 0 {
                s.push(' ');
            }
            s.push_str(&format!("{}", c[i]));
        }
        s.push('\n');
    }
    fs::write(path, s)?;
    Ok(())
}
impl FileObject for FastWindFile {
    fn format_name(&self) -> &str {
        "FAST determ. wind file"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        Ok(vec![NamedFrame::unnamed(self.data.clone())])
    }
}
