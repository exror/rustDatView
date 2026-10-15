use dv_core::{DataFrame, Field2D, FileObject, IoError, NamedFrame};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
pub struct ParsedFile {
    pub path: PathBuf,
    pub format: &'static str,
    pub frames: Vec<NamedFrame>,
    pub fields: Option<Vec<Field2D>>,
    pub description: String,
}
impl FileObject for ParsedFile {
    fn format_name(&self) -> &str {
        self.format
    }
    fn filename(&self) -> &Path {
        &self.path
    }
    fn to_dataframes(&self) -> Result<Vec<NamedFrame>, IoError> {
        Ok(self.frames.clone())
    }
    fn to_2d_fields(&self) -> Option<Result<Vec<Field2D>, IoError>> {
        self.fields.clone().map(Ok)
    }
    fn info(&self) -> String {
        format!("{}\n{}", self.format, self.description)
    }
}
pub fn object(path: &Path, format: &'static str, frames: Vec<NamedFrame>) -> Arc<dyn FileObject> {
    Arc::new(ParsedFile {
        path: path.into(),
        format,
        frames,
        fields: None,
        description: String::new(),
    })
}
pub fn broken(s: impl Into<String>) -> IoError {
    IoError::BrokenFormat(s.into())
}
pub fn wrong(s: impl Into<String>) -> IoError {
    IoError::WrongFormat(s.into())
}
pub fn text(path: &Path) -> Result<String, IoError> {
    let bytes = std::fs::read(path)?;
    if bytes.is_empty() {
        return Err(IoError::Empty(path.into()));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
pub fn number(s: &str) -> Result<f64, IoError> {
    s.replace(['D', 'd'], "E")
        .parse()
        .map_err(|_| broken(format!("Invalid number: {s}")))
}
pub fn numbers(s: &str) -> Result<Vec<f64>, IoError> {
    s.split_whitespace().map(number).collect()
}
pub fn count(s: &str) -> Result<usize, IoError> {
    s.split_whitespace()
        .next()
        .ok_or_else(|| broken("Missing count"))?
        .parse()
        .map_err(|_| broken("Invalid count"))
}
pub fn table(names: &[String], rows: &[Vec<f64>]) -> Result<DataFrame, IoError> {
    let mut cols: Vec<Vec<f64>> = names
        .iter()
        .map(|_| Vec::with_capacity(rows.len()))
        .collect();
    for row in rows {
        if row.len() != cols.len() {
            return Err(broken(format!(
                "Expected {} columns, found {}",
                cols.len(),
                row.len()
            )));
        }
        for (col, value) in cols.iter_mut().zip(row) {
            col.push(*value);
        }
    }
    DataFrame::from_floats(names.iter().cloned(), cols).map_err(|e| broken(e.to_string()))
}
pub fn names(s: &str) -> Vec<String> {
    s.split('|').map(str::to_owned).collect()
}
pub fn numeric_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<Vec<f64>>, IoError> {
    lines
        .into_iter()
        .filter(|s| !s.trim().is_empty() && !s.trim_start().starts_with('#'))
        .map(numbers)
        .collect()
}
pub struct Lines<'a> {
    lines: Vec<&'a str>,
    pub pos: usize,
}
impl<'a> Lines<'a> {
    pub fn new(s: &'a str) -> Self {
        Self {
            lines: s.lines().collect(),
            pos: 0,
        }
    }
    pub fn next(&mut self) -> Result<&'a str, IoError> {
        let l = self
            .lines
            .get(self.pos)
            .ok_or_else(|| broken("Unexpected end of file"))?;
        self.pos += 1;
        Ok(l)
    }
    pub fn done(&self) -> bool {
        self.pos >= self.lines.len()
    }
    pub fn rows(&mut self, n: usize, width: usize) -> Result<Vec<Vec<f64>>, IoError> {
        (0..n)
            .map(|_| {
                let row: Vec<f64> = self
                    .next()?
                    .split_whitespace()
                    .take(width)
                    .map(number)
                    .collect::<Result<_, _>>()?;
                if row.len() != width {
                    Err(broken("Truncated row"))
                } else {
                    Ok(row)
                }
            })
            .collect()
    }
}
pub fn quoted(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for c in s.chars() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c.is_whitespace() || c == ',' {
            if !word.is_empty() {
                result.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        result.push(word);
    }
    result
}
pub fn float_matches(s: &str) -> Vec<f64> {
    let re =
        regex::Regex::new(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[EeDd][-+]?\d+)?").expect("constant regex");
    re.find_iter(s)
        .filter_map(|m| number(m.as_str()).ok())
        .collect()
}
pub fn f32_at(b: &[u8], p: usize) -> Result<f64, IoError> {
    Ok(f32::from_le_bytes(
        b.get(p..p + 4)
            .ok_or_else(|| broken("Truncated binary file"))?
            .try_into()
            .unwrap(),
    ) as f64)
}
pub fn i32_at(b: &[u8], p: usize) -> Result<i32, IoError> {
    Ok(i32::from_le_bytes(
        b.get(p..p + 4)
            .ok_or_else(|| broken("Truncated binary file"))?
            .try_into()
            .unwrap(),
    ))
}
pub const POLAR: &str = "Alpha_[deg]|Cl_[-]|Cd_[-]|Cm_[-]";
