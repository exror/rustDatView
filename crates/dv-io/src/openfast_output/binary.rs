use dv_core::IoError;
use std::io::{Read, Write};

pub(crate) fn broken(message: impl Into<String>) -> IoError {
    IoError::BrokenFormat(message.into())
}
pub(crate) fn number(text: &str) -> Result<f64, IoError> {
    text.trim()
        .replace(['D', 'd'], "E")
        .parse()
        .map_err(|_| broken(format!("invalid number `{text}`")))
}
pub(crate) struct BinaryReader<R: Read>(pub R);
impl<R: Read> BinaryReader<R> {
    pub fn bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        let mut v = vec![0; n];
        self.0.read_exact(&mut v)?;
        Ok(v)
    }
    pub fn i16(&mut self) -> Result<i16, IoError> {
        let mut b = [0; 2];
        self.0.read_exact(&mut b)?;
        Ok(i16::from_le_bytes(b))
    }
    pub fn i32(&mut self) -> Result<i32, IoError> {
        let mut b = [0; 4];
        self.0.read_exact(&mut b)?;
        Ok(i32::from_le_bytes(b))
    }
    pub fn f32(&mut self) -> Result<f32, IoError> {
        let mut b = [0; 4];
        self.0.read_exact(&mut b)?;
        Ok(f32::from_le_bytes(b))
    }
    pub fn f64(&mut self) -> Result<f64, IoError> {
        let mut b = [0; 8];
        self.0.read_exact(&mut b)?;
        Ok(f64::from_le_bytes(b))
    }
    pub fn count(&mut self, label: &str) -> Result<usize, IoError> {
        let v = self.i32()?;
        usize::try_from(v).map_err(|_| broken(format!("negative {label}")))
    }
    pub fn text(&mut self, n: usize) -> Result<String, IoError> {
        Ok(self
            .bytes(n)?
            .into_iter()
            .map(char::from)
            .collect::<String>()
            .trim()
            .into())
    }
}
pub(crate) fn fixed_text(w: &mut impl Write, text: &str, n: usize) -> Result<(), IoError> {
    if !text.is_ascii() {
        return Err(broken(
            "binary channel names, units and descriptions must be ASCII",
        ));
    }
    let bytes = text.as_bytes();
    let len = bytes.len().min(n);
    w.write_all(&bytes[..len])?;
    for _ in len..n {
        w.write_all(b" ")?;
    }
    Ok(())
}
pub(crate) fn frame(
    names: Vec<String>,
    columns: Vec<Vec<f64>>,
) -> Result<dv_core::DataFrame, IoError> {
    if names.len() != columns.len() {
        return Err(broken("inconsistent number of columns"));
    }
    dv_core::DataFrame::from_floats(names, columns).map_err(|e| broken(e.to_string()))
}
pub(crate) fn text_file(path: &std::path::Path) -> Result<String, IoError> {
    let b = std::fs::read(path)?;
    if b.starts_with(&[0xff, 0xfe])
        || b.starts_with(&[0xfe, 0xff])
        || b.iter().take(80).any(|&c| c == 0)
    {
        let be = b.starts_with(&[0xfe, 0xff]);
        let start = if b.starts_with(&[0xff, 0xfe]) || be {
            2
        } else {
            0
        };
        let units = b[start..]
            .chunks_exact(2)
            .map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            })
            .collect::<Vec<_>>();
        Ok(String::from_utf16_lossy(&units))
    } else {
        Ok(String::from_utf8_lossy(&b).into_owned())
    }
}
