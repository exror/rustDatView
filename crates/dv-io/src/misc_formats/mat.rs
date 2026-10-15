use super::common::*;
use dv_core::{FileObject, IoError, NamedFrame};
use std::{collections::BTreeMap, io::Read, path::Path, sync::Arc};
/// MATLAB value, preserving cell/struct nesting and column-major storage.
#[derive(Debug, Clone)]
pub enum MatValue {
    Numeric {
        dimensions: Vec<usize>,
        real: Vec<f64>,
        imaginary: Option<Vec<f64>>,
    },
    Text(String),
    Cell(Vec<MatValue>),
    Struct {
        fields: Vec<String>,
        elements: Vec<Vec<MatValue>>,
    },
}
struct Decoder<'a> {
    b: &'a [u8],
    p: usize,
    le: bool,
}
impl<'a> Decoder<'a> {
    fn u32(&self, b: &[u8]) -> u32 {
        if self.le {
            u32::from_le_bytes(b.try_into().unwrap())
        } else {
            u32::from_be_bytes(b.try_into().unwrap())
        }
    }
    fn element(&mut self) -> Result<(u32, &'a [u8]), IoError> {
        let tag = self
            .b
            .get(self.p..self.p + 8)
            .ok_or_else(|| broken("Truncated MATLAB element"))?;
        let a = self.u32(&tag[..4]);
        let (kind, n, start, end) = if a >> 16 != 0 {
            let (kind, n) = if self.le {
                (a & 0xffff, (a >> 16) as usize)
            } else {
                (a >> 16, (a & 0xffff) as usize)
            };
            if n > 4 {
                return Err(broken("Invalid MATLAB small element"));
            }
            (kind, n, self.p + 4, self.p + 8)
        } else {
            let n = self.u32(&tag[4..]) as usize;
            let start = self.p + 8;
            let end = start
                .checked_add(n)
                .ok_or_else(|| broken("MATLAB size overflow"))?;
            (
                a,
                n,
                start,
                if a == 15 {
                    end
                } else {
                    end.checked_add(7)
                        .ok_or_else(|| broken("MATLAB size overflow"))?
                        & !7
                },
            )
        };
        let data = self
            .b
            .get(start..start + n)
            .ok_or_else(|| broken("Truncated MATLAB payload"))?;
        self.p = end;
        Ok((kind, data))
    }
    fn numeric(&self, kind: u32, b: &[u8]) -> Result<Vec<f64>, IoError> {
        let width = match kind {
            1 | 2 | 16 => 1,
            3 | 4 | 17 => 2,
            5 | 6 | 7 | 18 => 4,
            9 | 12 | 13 => 8,
            _ => return Err(broken(format!("Unsupported MATLAB numeric type {kind}"))),
        };
        if b.len() % width != 0 {
            return Err(broken("Truncated MATLAB numeric value"));
        }
        Ok(b.chunks_exact(width)
            .map(|v| {
                let u = match width {
                    1 => v[0] as u64,
                    2 => {
                        (if self.le {
                            u16::from_le_bytes(v.try_into().unwrap())
                        } else {
                            u16::from_be_bytes(v.try_into().unwrap())
                        }) as u64
                    }
                    4 => self.u32(v) as u64,
                    _ => {
                        if self.le {
                            u64::from_le_bytes(v.try_into().unwrap())
                        } else {
                            u64::from_be_bytes(v.try_into().unwrap())
                        }
                    }
                };
                match kind {
                    1 => u as i8 as f64,
                    3 => u as i16 as f64,
                    5 => u as i32 as f64,
                    7 => f32::from_bits(u as u32) as f64,
                    9 => f64::from_bits(u),
                    12 => u as i64 as f64,
                    _ => u as f64,
                }
            })
            .collect())
    }
    fn matrix(&mut self) -> Result<(String, MatValue), IoError> {
        let (_, flags) = self.element()?;
        if flags.len() < 8 {
            return Err(broken("Missing MATLAB array flags"));
        }
        let flags = self.u32(&flags[..4]);
        let class = flags & 255;
        let (k, d) = self.element()?;
        let dimensions: Vec<usize> = self
            .numeric(k, d)?
            .into_iter()
            .map(|v| {
                if v < 0.0 || v.fract() != 0.0 || v >= usize::MAX as f64 {
                    Err(broken("Invalid MATLAB dimension"))
                } else {
                    Ok(v as usize)
                }
            })
            .collect::<Result<_, _>>()?;
        let n = dimensions
            .iter()
            .try_fold(1usize, |a, b| a.checked_mul(*b))
            .ok_or_else(|| broken("MATLAB dimensions overflow"))?;
        let (_, name) = self.element()?;
        let name = String::from_utf8_lossy(name).into_owned();
        let value = match class {
            1 => {
                let mut values = Vec::with_capacity(n);
                for _ in 0..n {
                    values.push(self.child()?);
                }
                MatValue::Cell(values)
            }
            2 => {
                let (k, len) = self.element()?;
                let len = self.numeric(k, len)?.first().copied().unwrap_or(0.0) as usize;
                if len == 0 {
                    return Err(broken("Invalid MATLAB struct field length"));
                }
                let (_, fields) = self.element()?;
                if fields.len() % len != 0 {
                    return Err(broken("Truncated MATLAB struct field name"));
                }
                let fields: Vec<_> = fields
                    .chunks(len)
                    .map(|s| {
                        String::from_utf8_lossy(s.split(|b| *b == 0).next().unwrap_or(s))
                            .into_owned()
                    })
                    .collect();
                let mut elements = Vec::with_capacity(n);
                for _ in 0..n {
                    let mut values = Vec::with_capacity(fields.len());
                    for _ in &fields {
                        values.push(self.child()?);
                    }
                    elements.push(values);
                }
                MatValue::Struct { fields, elements }
            }
            4 => {
                let (k, b) = self.element()?;
                let text = if k == 16 || k == 1 || k == 2 {
                    String::from_utf8(b.to_vec()).map_err(|e| broken(e.to_string()))?
                } else if k == 4 || k == 17 {
                    let units = self
                        .numeric(k, b)?
                        .into_iter()
                        .map(|v| v as u16)
                        .collect::<Vec<_>>();
                    String::from_utf16(&units).map_err(|e| broken(e.to_string()))?
                } else {
                    self.numeric(k, b)?
                        .into_iter()
                        .map(|v| {
                            char::from_u32(v as u32)
                                .ok_or_else(|| broken("Invalid MATLAB Unicode scalar"))
                        })
                        .collect::<Result<String, _>>()?
                };
                MatValue::Text(text)
            }
            6..=15 => {
                let (k, b) = self.element()?;
                let real = self.numeric(k, b)?;
                if real.len() != n {
                    return Err(broken("MATLAB numeric dimensions mismatch"));
                }
                let imaginary = if flags & 0x800 != 0 {
                    let (k, b) = self.element()?;
                    let values = self.numeric(k, b)?;
                    if values.len() != n {
                        return Err(broken("MATLAB imaginary dimensions mismatch"));
                    }
                    Some(values)
                } else {
                    None
                };
                MatValue::Numeric {
                    dimensions,
                    real,
                    imaginary,
                }
            }
            _ => return Err(broken(format!("Unsupported MATLAB array class {class}"))),
        };
        Ok((name, value))
    }
    fn child(&mut self) -> Result<MatValue, IoError> {
        let (k, b) = self.element()?;
        if k != 14 {
            return Err(broken("Expected nested MATLAB matrix"));
        }
        Decoder {
            b,
            p: 0,
            le: self.le,
        }
        .matrix()
        .map(|(_, v)| v)
    }
}
/// Read level-5 or HDF5-backed MATLAB arrays without executing object constructors.
pub fn read_mat(path: &Path) -> Result<BTreeMap<String, MatValue>, IoError> {
    let mut signature = [0; 128];
    let n = std::fs::File::open(path)?.read(&mut signature)?;
    if signature[..n].starts_with(b"\x89HDF\r\n\x1a\n")
        || signature[..n].starts_with(b"MATLAB 7.3 MAT-file")
    {
        super::mat_hdf::read(path)
    } else {
        read_mat_v5(path)
    }
}
/// Read uncompressed or zlib-compressed MATLAB v5 arrays, including nested cells and structs.
pub fn read_mat_v5(path: &Path) -> Result<BTreeMap<String, MatValue>, IoError> {
    let b = std::fs::read(path)?;
    if b.len() < 128 || !b.starts_with(b"MATLAB 5.0 MAT-file") {
        return Err(wrong("Not a MATLAB v5 file"));
    }
    let le = match &b[126..128] {
        b"IM" => true,
        b"MI" => false,
        _ => return Err(broken("Invalid MATLAB byte order")),
    };
    let mut out = BTreeMap::new();
    parse_elements(&b[128..], le, &mut out)?;
    Ok(out)
}
fn parse_elements(b: &[u8], le: bool, out: &mut BTreeMap<String, MatValue>) -> Result<(), IoError> {
    let mut d = Decoder { b, p: 0, le };
    while d.p < b.len() {
        let (k, b) = d.element()?;
        match k {
            14 => {
                let (name, value) = Decoder { b, p: 0, le }.matrix()?;
                out.insert(name, value);
            }
            15 => {
                let mut inflated = Vec::new();
                flate2::read::ZlibDecoder::new(b).read_to_end(&mut inflated)?;
                parse_elements(&inflated, le, out)?;
            }
            _ => return Err(broken("Unexpected top-level MATLAB element")),
        }
    }
    Ok(())
}
fn entries(v: &MatValue) -> Option<&[MatValue]> {
    match v {
        MatValue::Cell(v) => Some(v),
        MatValue::Struct { elements, .. } if elements.len() == 1 => Some(&elements[0]),
        _ => None,
    }
}
fn unwrap(v: &MatValue) -> &MatValue {
    if let Some(e) = entries(v) {
        if e.len() == 1 {
            return unwrap(&e[0]);
        }
    }
    v
}
fn real(v: &MatValue) -> Option<&[f64]> {
    match unwrap(v) {
        MatValue::Numeric { real, .. } => Some(real),
        _ => None,
    }
}
pub fn raaw(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let m = read_mat(path)?;
    let data = m
        .get("DataCell")
        .ok_or_else(|| wrong("MATLAB file has no RAAW DataCell"))?;
    let signals = entries(data).ok_or_else(|| broken("RAAW DataCell must be a cell array"))?;
    let mut names50 = vec!["time".into()];
    let mut names1 = vec!["time".into()];
    let mut vals50 = vec![(0..30000).map(|i| i as f64 * 0.02).collect()];
    let mut vals1 = vec![(0..600).map(|i| i as f64).collect()];
    for signal in signals {
        let record =
            entries(unwrap(signal)).ok_or_else(|| broken("RAAW signal must contain a record"))?;
        if record.len() < 7 {
            return Err(broken("RAAW record needs seven entries"));
        }
        let name = match unwrap(&record[1]) {
            MatValue::Text(s) => s.clone(),
            _ => return Err(broken("RAAW channel name must be text")),
        };
        let dt = real(&record[3])
            .and_then(|v| v.first())
            .copied()
            .ok_or_else(|| broken("Missing RAAW sample interval"))?;
        let values = real(&record[6])
            .ok_or_else(|| broken("Missing RAAW samples"))?
            .to_vec();
        if dt == 0.02 {
            if values.len() != 30000 {
                return Err(broken("RAAW 50Hz channel must have 30000 samples"));
            }
            names50.push(name);
            vals50.push(values);
        } else if dt == 1.0 {
            if values.len() != 600 {
                return Err(broken("RAAW 1Hz channel must have 600 samples"));
            }
            names1.push(name);
            vals1.push(values);
        } else {
            return Err(broken(format!("Unsupported RAAW sample interval {dt}")));
        }
    }
    let frame = |n, v| dv_core::DataFrame::from_floats(n, v).map_err(|e| broken(e.to_string()));
    Ok(object(
        path,
        "RAAW .mat file",
        vec![
            NamedFrame::new("data50hz", frame(names50, vals50)?),
            NamedFrame::new("data1hz", frame(names1, vals1)?),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn word(v: u32, le: bool) -> [u8; 4] {
        if le {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    }
    fn element(out: &mut Vec<u8>, kind: u32, data: &[u8], le: bool) {
        out.extend_from_slice(&word(kind, le));
        out.extend_from_slice(&word(data.len() as u32, le));
        out.extend_from_slice(data);
        while out.len() % 8 != 0 {
            out.push(0);
        }
    }
    fn matrix(class: u32, dimensions: &[i32], kind: u32, data: &[u8], le: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut flags = word(class, le).to_vec();
        flags.extend_from_slice(&word(0, le));
        element(&mut out, 6, &flags, le);
        let dims = dimensions
            .iter()
            .flat_map(|v| word(*v as u32, le))
            .collect::<Vec<_>>();
        element(&mut out, 5, &dims, le);
        element(&mut out, 1, b"value", le);
        element(&mut out, kind, data, le);
        out
    }
    #[test]
    fn utf16_surrogate_pairs_in_both_orders() {
        for le in [true, false] {
            let units = [0x0041u16, 0xd83d, 0xde00];
            let bytes = units
                .iter()
                .flat_map(|v| if le { v.to_le_bytes() } else { v.to_be_bytes() })
                .collect::<Vec<_>>();
            let b = matrix(4, &[1, 3], 17, &bytes, le);
            let (_, value) = Decoder { b: &b, p: 0, le }.matrix().unwrap();
            match value {
                MatValue::Text(s) => assert_eq!(s, "A\u{1f600}"),
                _ => panic!("character matrix became numeric"),
            }
        }
    }
    #[test]
    fn negative_dimensions_rejected() {
        let b = matrix(6, &[-1, 1], 9, &[], true);
        assert!(Decoder {
            b: &b,
            p: 0,
            le: true
        }
        .matrix()
        .is_err());
    }
    #[test]
    fn object_class_is_not_silently_treated_as_struct() {
        let b = matrix(3, &[1, 1], 9, &[], true);
        assert!(Decoder {
            b: &b,
            p: 0,
            le: true
        }
        .matrix()
        .is_err());
    }
}
