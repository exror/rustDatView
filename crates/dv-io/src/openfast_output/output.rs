//! OpenFAST ASCII and binary time-series output, including all four file IDs.
use super::binary::{broken, fixed_text, frame, number, text_file, BinaryReader};
use dv_core::{DataFrame, Field2D, FileObject, IoError, NamedFrame};
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FastOutputFile {
    pub filename: PathBuf,
    pub data: DataFrame,
    pub description: String,
    pub binary: bool,
    pub file_id: Option<i16>,
}
fn column_name(name: &str, unit: &str) -> String {
    format!("{}_[{}]", name, unit.replace("sec", "s"))
}
impl FastOutputFile {
    pub fn read(path: &Path) -> Result<Self, IoError> {
        let mut r = BinaryReader(BufReader::new(File::open(path)?));
        let id = r.i16()?;
        if (1..=4).contains(&id) {
            load_binary_output(path)
        } else {
            load_ascii_output(path)
        }
    }
    pub fn write(&self, path: &Path, binary: bool, file_id: i16) -> Result<(), IoError> {
        if binary {
            write_binary(&self.data, path, file_id, &self.description)
        } else {
            write_ascii(&self.data, path, &self.description)
        }
    }
    pub fn driver_file(&self) -> Option<(PathBuf, bool)> {
        find_driver_file(&self.filename)
    }
    pub fn fields(
        &self,
        delta_azi: f64,
        n_periods: usize,
        rcoords: Option<&[f64]>,
        timewise: bool,
        azimuthwise: bool,
    ) -> Result<Vec<Field2D>, IoError> {
        use crate::postpro::{
            azimuthal_average_df, fast_spanwise_outputs, spanwise_postpro_rows, AzimuthOptions,
        };
        if !delta_azi.is_finite() || delta_azi <= 0.0 {
            return Err(broken("azimuth spacing must be positive"));
        }
        let positions = match self.driver_file() {
            Some((path, _)) => Some(
                fast_spanwise_outputs(&path, &self.data.names())
                    .map_err(|e| broken(e.to_string()))?,
            ),
            None => None,
        };
        let mut out = Vec::new();
        let mut append = |df: &DataFrame,
                          x: Vec<f64>,
                          prefix: &str,
                          sx: &str|
         -> Result<(), IoError> {
            let fields =
                spanwise_postpro_rows(df, positions.as_ref()).map_err(|e| broken(e.to_string()))?;
            for f in fields.ad {
                if f.nrows != x.len() {
                    return Err(broken("spanwise field and coordinate lengths differ"));
                }
                let (y, sy) = if let Some(r) = rcoords {
                    if r.len() != f.nodes {
                        return Err(broken(format!("Length of rcoords should be: {}", f.nodes)));
                    }
                    (r.to_vec(), "r [m]")
                } else {
                    (
                        f.radii,
                        if positions.as_ref().and_then(|p| p.ad.as_ref()).is_some() {
                            "r [m]"
                        } else {
                            "r [-]"
                        },
                    )
                };
                out.push(Field2D {
                    name: format!("{prefix}_{}", f.name),
                    x: x.clone(),
                    y,
                    sx: sx.into(),
                    sy: sy.into(),
                    m: f.values,
                });
            }
            Ok(())
        };
        if timewise {
            let x = self
                .data
                .get("Time_[s]")
                .map(|c| c.as_f64().into_owned())
                .unwrap_or_else(|| (0..self.data.nrows()).map(|i| i as f64).collect());
            append(&self.data, x, "(t,r)", "t [s]")?;
        }
        if azimuthwise {
            let bins = (0..=((360.0 + delta_azi / 10.0) / delta_azi).floor() as usize)
                .map(|i| i as f64 * delta_azi)
                .collect::<Vec<_>>();
            let options = AzimuthOptions {
                bins: bins.clone(),
                periodic: true,
                n_periods: Some(n_periods),
                ..Default::default()
            };
            let df =
                azimuthal_average_df(&self.data, &options).map_err(|e| broken(e.to_string()))?;
            append(&df, bins, "(psi,r)", "psi [deg]")?;
        }
        Ok(out)
    }
}
impl FileObject for FastOutputFile {
    fn format_name(&self) -> &str {
        "FAST output file"
    }
    fn filename(&self) -> &Path {
        &self.filename
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        Ok(vec![NamedFrame::unnamed(self.data.clone())])
    }
    fn to_2d_fields(&self) -> Option<Result<Vec<Field2D>, IoError>> {
        // No spanwise channels means this output has no 2D representation.
        match crate::postpro::spanwise_postpro_rows(&self.data, None) {
            Ok(fields) if fields.ad.is_empty() => None,
            Err(e) => Some(Err(broken(e.to_string()))),
            _ => Some(self.fields(5.0, 3, None, true, true)),
        }
    }
    fn info(&self) -> String {
        format!(
            "FAST output: {} rows, {} channels, binary={}, ID={:?}\n{}",
            self.data.nrows(),
            self.data.ncols(),
            self.binary,
            self.file_id,
            self.description
        )
    }
}
pub fn load_ascii_output(path: &Path) -> Result<FastOutputFile, IoError> {
    let text = text_file(path)?;
    let lines = text.lines().collect::<Vec<_>>();
    let elm = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("elm"));
    let header = if elm {
        Some(1)
    } else {
        lines.iter().take(35).position(|l| {
            matches!(
                l.split_whitespace()
                    .next()
                    .map(str::to_lowercase)
                    .as_deref(),
                Some("time" | "alpha")
            )
        })
    };
    let h = header.ok_or_else(|| {
        IoError::WrongFormat("FAST output Time or Alpha header missing in first 35 lines".into())
    })?;
    let names = lines
        .get(h)
        .ok_or_else(|| broken("channel header missing"))?
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let units = lines
        .get(h + 1)
        .ok_or_else(|| broken("units header missing"))?
        .split_whitespace()
        .map(|u| u.trim_matches(['(', ')', '[', ']']).replace("sec", "s"))
        .collect::<Vec<_>>();
    let names = if units.len() == names.len() {
        names
            .iter()
            .zip(&units)
            .map(|(n, u)| column_name(n, u))
            .collect()
    } else {
        names
    };
    let mut columns = vec![Vec::new(); names.len()];
    for (i, l) in lines.iter().enumerate().skip(h + 2) {
        let l = l.split("This").next().unwrap_or("").trim();
        if l.is_empty() {
            continue;
        }
        let values = l.split_whitespace().collect::<Vec<_>>();
        if values.len() != columns.len() {
            return Err(broken(format!(
                "FAST output line {} has {} values, expected {}",
                i + 1,
                values.len(),
                columns.len()
            )));
        }
        for (c, v) in columns.iter_mut().zip(values) {
            c.push(number(v)?);
        }
    }
    let data = frame(names, columns)?;
    if data.is_empty() {
        return Err(IoError::Empty(path.into()));
    }
    Ok(FastOutputFile {
        filename: path.into(),
        data,
        description: lines[..h].join("\n") + "\n",
        binary: false,
        file_id: None,
    })
}
pub fn load_binary_output(path: &Path) -> Result<FastOutputFile, IoError> {
    let mut r = BinaryReader(BufReader::new(File::open(path)?));
    let id = r.i16()?;
    if !(1..=4).contains(&id) {
        return Err(IoError::WrongFormat(format!(
            "unsupported FAST binary ID {id}"
        )));
    }
    let width = if id == 4 {
        usize::try_from(r.i16()?).map_err(|_| broken("negative channel width"))?
    } else {
        10
    };
    if width == 0 {
        return Err(broken("zero channel width"));
    }
    let nc = r.count("channel count")?;
    let nt = r.count("time count")?;
    let t1 = r.f64()?;
    let dt = r.f64()?;
    let mut scales = vec![1.0; nc];
    let mut offsets = vec![0.0; nc];
    if id != 3 {
        for s in &mut scales {
            *s = r.f32()? as f64;
        }
        for o in &mut offsets {
            *o = r.f32()? as f64;
        }
    }
    let nd = r.count("description length")?;
    let description = r.text(nd)?;
    let mut names = Vec::with_capacity(nc + 1);
    for _ in 0..=nc {
        names.push(r.text(width)?);
    }
    for name in &mut names {
        let u = r.text(width)?;
        *name = column_name(name, u.trim_matches(['(', ')', '[', ']']));
    }
    let mut columns = vec![Vec::with_capacity(nt); nc + 1];
    for i in 0..nt {
        columns[0].push(if id == 1 {
            (r.i32()? as f64 - dt) / t1
        } else {
            t1 + i as f64 * dt
        });
    }
    for _ in 0..nt {
        for j in 0..nc {
            let v = if id == 3 { r.f64()? } else { r.i16()? as f64 };
            columns[j + 1].push((v - offsets[j]) / scales[j]);
        }
    }
    if nt == 0 {
        return Err(IoError::Empty(path.into()));
    }
    Ok(FastOutputFile {
        filename: path.into(),
        data: frame(names, columns)?,
        description,
        binary: true,
        file_id: Some(id),
    })
}
pub fn channel_units(df: &DataFrame) -> (Vec<String>, Vec<String>) {
    df.columns()
        .iter()
        .map(|c| {
            let s = c.name.trim();
            let open = if s.ends_with(']') {
                '['
            } else if s.ends_with(')') {
                '('
            } else {
                '\0'
            };
            if let Some(i) = s.rfind(open).filter(|i| *i > 1) {
                (
                    s[..i].trim_end_matches('_').to_owned(),
                    s[i + 1..s.len() - 1].into(),
                )
            } else {
                (s.into(), String::new())
            }
        })
        .unzip()
}
pub fn write_outb(df: &DataFrame, path: &Path) -> Result<(), IoError> {
    write_binary(df, path, 4, "")
}
pub fn write_binary(
    df: &DataFrame,
    path: &Path,
    id: i16,
    description: &str,
) -> Result<(), IoError> {
    if !matches!(id, 2 | 4) {
        return Err(broken("FAST binary writing supports file IDs 2 and 4"));
    }
    let (names, units) = channel_units(df);
    if names.first().map(String::as_str) != Some("Time") {
        return Err(broken("Time needs to be the first channel"));
    }
    if df.nrows() < 2 {
        return Err(broken("binary output requires at least two time samples"));
    }
    if !description.is_ascii() {
        return Err(broken("binary description must be ASCII"));
    }
    let cols = df
        .columns()
        .iter()
        .map(|c| {
            c.floats()
                .ok_or_else(|| broken("FAST output requires numeric columns"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let nc = cols.len() - 1;
    let nt = df.nrows();
    let units = units
        .into_iter()
        .map(|u| format!("({u})"))
        .collect::<Vec<_>>();
    let width = if id == 4 {
        names
            .iter()
            .chain(&units)
            .map(String::len)
            .max()
            .unwrap_or(1)
    } else {
        10
    };
    let mut scales = Vec::with_capacity(nc);
    let mut offsets = Vec::with_capacity(nc);
    for c in &cols[1..] {
        if c.iter().any(|v| !v.is_finite()) {
            return Err(broken(
                "compressed binary output cannot represent non-finite values",
            ));
        }
        let min = c.iter().copied().fold(f64::INFINITY, f64::min);
        let max = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut range = (max - min) as f32;
        if range == 0.0 {
            range = 1.0;
        }
        let scale = 65535.0f32 / range;
        let offset = -32768.0f32 - (min as f32) * scale;
        scales.push(scale);
        offsets.push(offset);
    }
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(&id.to_le_bytes())?;
    if id == 4 {
        w.write_all(
            &i16::try_from(width)
                .map_err(|_| broken("channel names too long"))?
                .to_le_bytes(),
        )?;
    }
    w.write_all(
        &i32::try_from(nc)
            .map_err(|_| broken("too many channels"))?
            .to_le_bytes(),
    )?;
    w.write_all(
        &i32::try_from(nt)
            .map_err(|_| broken("too many samples"))?
            .to_le_bytes(),
    )?;
    w.write_all(&cols[0][0].to_le_bytes())?;
    w.write_all(&((cols[0][nt - 1] - cols[0][0]) / (nt - 1) as f64).to_le_bytes())?;
    for v in scales.iter().chain(&offsets) {
        w.write_all(&v.to_le_bytes())?;
    }
    w.write_all(
        &i32::try_from(description.len())
            .map_err(|_| broken("description too long"))?
            .to_le_bytes(),
    )?;
    w.write_all(description.as_bytes())?;
    for s in names.iter().chain(&units) {
        fixed_text(&mut w, s, width)?;
    }
    for i in 0..nt {
        for j in 0..nc {
            let v = (cols[j + 1][i] * scales[j] as f64 + offsets[j] as f64).clamp(-32768.0, 32767.0)
                as i16;
            w.write_all(&v.to_le_bytes())?;
        }
    }
    w.flush()?;
    Ok(())
}
pub fn write_ascii(df: &DataFrame, path: &Path, description: &str) -> Result<(), IoError> {
    let (names, units) = channel_units(df);
    let mut w = BufWriter::new(File::create(path)?);
    if !description.is_empty() {
        writeln!(w, "{}", description.trim_end())?;
    }
    writeln!(w, "{}", names.join("\t"))?;
    writeln!(
        w,
        "{}",
        units
            .iter()
            .map(|u| format!("({u})"))
            .collect::<Vec<_>>()
            .join("\t")
    )?;
    for i in 0..df.nrows() {
        for (j, c) in df.columns().iter().enumerate() {
            let values = c
                .floats()
                .ok_or_else(|| broken("FAST output requires numeric columns"))?;
            if j > 0 {
                write!(w, "\t")?;
            }
            if j == 0 {
                write!(w, "{:10.4}", values[i])?;
            } else {
                write!(w, "{:.5e}", values[i])?;
            }
        }
        writeln!(w)?;
    }
    w.flush()?;
    Ok(())
}
pub fn find_driver_file(path: &Path) -> Option<(PathBuf, bool)> {
    let mut base = path.with_extension("");
    let s = base.to_string_lossy();
    if let Some(i) = s.to_lowercase().find(".ad").filter(|i| *i > 1) {
        base = PathBuf::from(&s[..i]);
    }
    for (farm, exts) in [
        (true, &["fstf", "FSTF", "Fstf", "fmas", "FMAS", "Fmas"][..]),
        (false, &["fst", "FST", "Fst", "dvr", "Dvr", "DVR"][..]),
    ] {
        for ext in exts {
            let p = PathBuf::from(format!("{}.{}", base.display(), ext));
            if p.is_file() {
                return Some((p, farm));
            }
        }
    }
    None
}
