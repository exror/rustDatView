//! TurbSim user-provided point time series.
use super::binary::{broken, number, text_file};
use dv_core::{DataFrame, FileObject, IoError, NamedFrame};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn header_start(text: &str) -> Option<usize> {
    let mut lines = text.lines();
    for i in 0..12 {
        let line = lines.next()?;
        let mut words = line.split_whitespace();
        let Some(value) = words.next() else {
            continue;
        };
        if !words.any(|word| word.eq_ignore_ascii_case("nComp")) {
            continue;
        }
        let mut following = lines.clone();
        let points = following.next()?;
        let reference = following.next()?;
        let has_labels = points
            .split_whitespace()
            .skip(1)
            .any(|word| word.eq_ignore_ascii_case("nPoints"))
            && reference
                .split_whitespace()
                .skip(1)
                .any(|word| word.eq_ignore_ascii_case("RefPtID"));
        let has_counts = value.parse::<usize>().is_ok()
            && points.trim().parse::<usize>().is_ok()
            && reference.trim().parse::<usize>().is_ok();
        if has_labels || has_counts {
            return Some(i);
        }
    }
    None
}

pub(crate) fn looks_like_turbsim_ts(text: &str) -> bool {
    header_start(text).is_some()
}

#[derive(Debug, Clone)]
pub struct TurbSimTsFile {
    pub filename: PathBuf,
    pub header: Vec<String>,
    pub n_components: usize,
    pub reference_point: usize,
    pub points: Vec<[f64; 2]>,
    pub data: DataFrame,
}
impl TurbSimTsFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        if !crate::registry::text_prefix(path)?
            .as_deref()
            .is_some_and(looks_like_turbsim_ts)
        {
            return Err(IoError::WrongFormat("TurbSim nComp header missing".into()));
        }
        let text = text_file(path)?;
        let start = header_start(&text)
            .ok_or_else(|| IoError::WrongFormat("TurbSim nComp header missing".into()))?;
        let lines = text.lines().collect::<Vec<_>>();
        let integer = |i: usize| -> Result<usize, IoError> {
            lines
                .get(i)
                .and_then(|l| l.split_whitespace().next())
                .ok_or_else(|| broken("truncated TurbSim time-series header"))?
                .parse()
                .map_err(|_| broken("invalid TurbSim time-series count"))
        };
        let n_components = integer(start)?;
        let npoints = integer(start + 1)?;
        let reference_point = integer(start + 2)?;
        if !(1..=3).contains(&n_components)
            || npoints == 0
            || !(1..=npoints).contains(&reference_point)
        {
            return Err(broken(
                "invalid TurbSim component count, point count or reference point",
            ));
        }
        if npoints > lines.len().saturating_sub(start + 5) {
            return Err(broken("truncated TurbSim point list"));
        }
        let mut points = Vec::with_capacity(npoints);
        for i in 0..npoints {
            let row = lines
                .get(start + 5 + i)
                .ok_or_else(|| broken("truncated TurbSim point list"))?
                .split_whitespace()
                .map(number)
                .collect::<Result<Vec<_>, _>>()?;
            if row.len() != 2 {
                return Err(broken("TurbSim point requires y,z"));
            }
            points.push([row[0], row[1]]);
        }
        let mut rows = Vec::new();
        for line in lines
            .iter()
            .skip(start + 8 + npoints)
            .take_while(|l| !l.trim().is_empty())
        {
            rows.push(
                line.split_whitespace()
                    .map(number)
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        let mut names = vec!["Time_[s]".into()];
        for comp in ["u", "v", "w"].iter().take(n_components) {
            for i in 1..=npoints {
                names.push(format!("Point{i}{comp}_[m/s]"));
            }
        }
        let data = DataFrame::from_rows(names, &rows).map_err(|e| broken(e.to_string()))?;
        if data.is_empty() {
            return Err(IoError::Empty(path.into()));
        }
        Ok(Self {
            filename: path.into(),
            header: lines[..start].iter().map(|l| l.trim().into()).collect(),
            n_components,
            reference_point,
            points,
            data,
        })
    }
    pub fn to_text(&self) -> Result<String, IoError> {
        let mut s = self.header.join("\n") + "\n";
        writeln!(
            s,
            "{:<13} {:<13} - Number of velocity components in the file",
            self.n_components, "nComp"
        )
        .unwrap();
        writeln!(
            s,
            "{:<13} {:<13} - Number of time series points contained in this file(-)",
            self.points.len(),
            "nPoints"
        )
        .unwrap();
        writeln!(
            s,
            "{:<13} {:<13} - Index of the reference point (1-nPoints)",
            self.reference_point, "RefPtID"
        )
        .unwrap();
        writeln!(
            s,
            "{:^16}{:^16} ! nPoints listed in order of increasing height",
            "Pointyi", "Pointzi"
        )
        .unwrap();
        writeln!(s, "{:^16}{:^16}", "(m)", "(m)").unwrap();
        for p in &self.points {
            writeln!(s, "{:16.8e}{:16.8e}", p[0], p[1]).unwrap();
        }
        s.push_str(
            "--------Time Series-------------------------------------------------------------\n",
        );
        for (i, c) in self.data.columns().iter().enumerate() {
            write!(s, "{:^16}", c.name.split("_[").next().unwrap_or("")).unwrap();
            if i + 1 == self.data.ncols() {
                s.push('\n');
            }
        }
        for i in 0..self.data.ncols() {
            write!(s, "{:^16}", if i == 0 { "(s)" } else { "(m/s)" }).unwrap();
        }
        s.push('\n');
        for i in 0..self.data.nrows() {
            for c in self.data.columns() {
                let v = c
                    .floats()
                    .ok_or_else(|| broken("TurbSim time series requires numeric columns"))?;
                write!(s, "{:16.8e}", v[i]).unwrap();
            }
            s.push('\n');
        }
        Ok(s)
    }
    pub fn write(&self, path: &Path) -> Result<(), IoError> {
        std::fs::write(path, self.to_text()?)?;
        Ok(())
    }
}
impl FileObject for TurbSimTsFile {
    fn format_name(&self) -> &str {
        "TurbSim time series"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        let rows = self.points.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
        let points = DataFrame::from_rows(["PointYi", "PointZi"], &rows)
            .map_err(|e| broken(e.to_string()))?;
        Ok(vec![
            NamedFrame::new("Points", points),
            NamedFrame::new("TimeSeries", self.data.clone()),
        ])
    }
}
