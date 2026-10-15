//! Standard TDMS segments: both byte orders, contiguous/interleaved samples,
//! metadata reuse, UTF-8 strings, booleans and UTC timestamp channels.
//! DAQmx scaler streams and extended/complex floats are explicitly rejected.
use super::broken;
use dv_core::{Column, ColumnData, FileObject, IoError, NamedFrame};
use std::{collections::BTreeMap, path::Path, sync::Arc};

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
    little: bool,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], IoError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| broken("TDMS offset overflow"))?;
        let data = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| broken("truncated TDMS payload"))?;
        self.pos = end;
        Ok(data)
    }
    fn u32(&mut self) -> Result<u32, IoError> {
        let b = self.take(4)?.try_into().unwrap();
        Ok(if self.little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
    fn u64(&mut self) -> Result<u64, IoError> {
        let b = self.take(8)?.try_into().unwrap();
        Ok(if self.little {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        })
    }
    fn string(&mut self) -> Result<String, IoError> {
        let n = self.u32()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(broken)
    }
    fn sample(&mut self, kind: u32) -> Result<Sample, IoError> {
        if kind == 0x20 {
            return Ok(Sample::Text(self.string()?));
        }
        if kind == 0x44 {
            let a = self.u64()?;
            let b = self.u64()?;
            let (seconds, fraction) = if self.little {
                (b as i64, a)
            } else {
                (a as i64, b)
            };
            return Ok(Sample::Number(
                seconds as f64 - 2_082_844_800.0 + fraction as f64 / 18_446_744_073_709_551_616.0,
            ));
        }
        let width = width(kind)?;
        let bytes = self.take(width)?;
        let mut raw = [0u8; 8];
        if self.little {
            raw[..width].copy_from_slice(bytes);
        } else {
            for (i, b) in bytes.iter().rev().enumerate() {
                raw[i] = *b;
            }
        }
        let n = u64::from_le_bytes(raw);
        let value = match kind {
            1 => n as i8 as f64,
            2 => n as i16 as f64,
            3 => n as i32 as f64,
            4 => n as i64 as f64,
            5..=8 => n as f64,
            9 | 0x19 => f32::from_bits(n as u32) as f64,
            10 | 0x1a => f64::from_bits(n),
            0x21 => {
                if n == 0 {
                    0.0
                } else {
                    1.0
                }
            }
            _ => unreachable!(),
        };
        Ok(Sample::Number(value))
    }
}
enum Sample {
    Number(f64),
    Text(String),
}
fn width(kind: u32) -> Result<usize, IoError> {
    match kind { 1|5|0x21 => Ok(1), 2|6 => Ok(2), 3|7|9|0x19 => Ok(4), 4|8|10|0x1a => Ok(8), 0x44 => Ok(16),
        _ => Err(IoError::Other(format!("unsupported TDMS raw/property datatype 0x{kind:x}; DAQmx, extended and complex samples require a specialized decoder"))) }
}
#[derive(Clone)]
struct RawIndex {
    kind: u32,
    count: usize,
    bytes: usize,
}
struct Object {
    path: String,
    raw: Option<RawIndex>,
    values: Option<ColumnData>,
    properties: BTreeMap<String, f64>,
}
impl Object {
    fn append(&mut self, kind: u32, sample: Sample) -> Result<(), IoError> {
        if self.values.is_none() {
            self.values = Some(if kind == 0x20 {
                ColumnData::Str(Vec::new())
            } else if kind == 0x44 {
                ColumnData::DateTime(Vec::new())
            } else {
                ColumnData::Float(Vec::new())
            });
        }
        match (self.values.as_mut().unwrap(), sample) {
            (ColumnData::Float(v), Sample::Number(x)) if kind != 0x44 => v.push(x),
            (ColumnData::DateTime(v), Sample::Number(x)) if kind == 0x44 => v.push(x),
            (ColumnData::Str(v), Sample::Text(x)) => v.push(x),
            _ => return Err(broken("TDMS channel changes datatype between segments")),
        }
        Ok(())
    }
}
fn paths(path: &str) -> Result<Vec<String>, IoError> {
    if path == "/" {
        return Ok(Vec::new());
    }
    let mut chars = path.chars().peekable();
    let mut parts = Vec::new();
    while chars.peek().is_some() {
        if chars.next() != Some('/') || chars.next() != Some('\'') {
            return Err(broken("invalid TDMS object path"));
        }
        let mut part = String::new();
        let mut closed = false;
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    part.push('\'');
                } else {
                    closed = true;
                    break;
                }
            } else {
                part.push(c);
            }
        }
        if !closed {
            return Err(broken("unterminated TDMS object path"));
        }
        parts.push(part);
    }
    Ok(parts)
}
fn decode(bytes: &[u8]) -> Result<Vec<Object>, IoError> {
    let mut pos = 0usize;
    let mut objects: Vec<Object> = Vec::new();
    let mut lookup = BTreeMap::new();
    let mut order: Vec<(usize, bool)> = Vec::new();
    let mut previous = false;
    while pos < bytes.len() {
        let header = bytes
            .get(
                pos..pos
                    .checked_add(28)
                    .ok_or_else(|| broken("TDMS offset overflow"))?,
            )
            .ok_or_else(|| broken("truncated TDMS lead-in"))?;
        if &header[..4] != b"TDSm" {
            return Err(IoError::WrongFormat(
                "missing TDMS segment signature".into(),
            ));
        }
        let flags = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let little = flags & (1 << 6) == 0;
        if flags & (1 << 7) != 0 {
            return Err(IoError::Other(
                "DAQmx TDMS scaler streams are not standard TDMS channels".into(),
            ));
        }
        let mut lead = Cursor {
            bytes: &header[8..],
            pos: 0,
            little,
        };
        let version = lead.u32()?;
        if ![4712, 4713].contains(&version) {
            return Err(broken(format!("unsupported TDMS version {version}")));
        }
        let next = lead.u64()?;
        let offset = lead.u64()?;
        let base = pos + 28;
        let end = if next == u64::MAX {
            bytes.len()
        } else {
            base.checked_add(usize::try_from(next).map_err(broken)?)
                .ok_or_else(|| broken("TDMS segment offset overflow"))?
        };
        if end > bytes.len() {
            return Err(broken("TDMS segment extends past end of file"));
        }
        let raw = base
            .checked_add(usize::try_from(offset).map_err(broken)?)
            .ok_or_else(|| broken("TDMS raw offset overflow"))?;
        if raw > end {
            return Err(broken("TDMS raw offset exceeds segment"));
        }
        if flags & (1 << 1) != 0 {
            if flags & (1 << 2) != 0 || !previous {
                order.clear();
            }
            let mut meta = Cursor {
                bytes: &bytes[base..raw],
                pos: 0,
                little,
            };
            let count = meta.u32()?;
            for _ in 0..count {
                let path = meta.string()?;
                paths(&path)?;
                let index = if let Some(&i) = lookup.get(&path) {
                    i
                } else {
                    let i = objects.len();
                    lookup.insert(path.clone(), i);
                    objects.push(Object {
                        path,
                        raw: None,
                        values: None,
                        properties: BTreeMap::new(),
                    });
                    i
                };
                let length = meta.u32()?;
                let active = length != u32::MAX;
                if active && length == 0 && objects[index].raw.is_none() {
                    return Err(broken("TDMS raw index reuses an unknown channel"));
                }
                if active && length != 0 {
                    if length < 20 {
                        return Err(broken("invalid TDMS raw index length"));
                    }
                    let start = meta.pos;
                    let kind = meta.u32()?;
                    let dim = meta.u32()?;
                    let count = usize::try_from(meta.u64()?).map_err(broken)?;
                    if dim != 1 {
                        return Err(broken("TDMS raw index dimension is not one"));
                    }
                    let size = if kind == 0x20 {
                        usize::try_from(meta.u64()?).map_err(broken)?
                    } else {
                        count
                            .checked_mul(width(kind)?)
                            .ok_or_else(|| broken("TDMS channel size overflow"))?
                    };
                    let consumed = meta.pos - start;
                    if consumed > length as usize - 4 {
                        return Err(broken("TDMS raw index is shorter than its datatype"));
                    }
                    meta.take(length as usize - 4 - consumed)?;
                    objects[index].raw = Some(RawIndex {
                        kind,
                        count,
                        bytes: size,
                    });
                }
                if let Some(entry) = order.iter_mut().find(|e| e.0 == index) {
                    entry.1 = active;
                } else {
                    order.push((index, active));
                }
                let properties = meta.u32()?;
                for _ in 0..properties {
                    let name = meta.string()?;
                    let kind = meta.u32()?;
                    let value = meta.sample(kind)?;
                    if let Sample::Number(v) = value {
                        objects[index].properties.insert(name, v);
                    }
                }
            }
        } else if !previous {
            return Err(broken("first TDMS segment has no metadata"));
        }
        if flags & (1 << 3) != 0 && raw < end {
            let active: Vec<(usize, RawIndex)> = order
                .iter()
                .filter(|(_, on)| *on)
                .map(|(i, _)| {
                    objects[*i]
                        .raw
                        .clone()
                        .map(|r| (*i, r))
                        .ok_or_else(|| broken("active TDMS object has no raw index"))
                })
                .collect::<Result<_, _>>()?;
            let chunk = active
                .iter()
                .try_fold(0usize, |a, (_, r)| a.checked_add(r.bytes))
                .ok_or_else(|| broken("TDMS chunk size overflow"))?;
            if chunk == 0 {
                return Err(broken("TDMS raw data has no samples"));
            }
            let mut data = Cursor {
                bytes: &bytes[raw..end],
                pos: 0,
                little,
            };
            if flags & (1 << 5) != 0 {
                if active.iter().any(|(_, r)| r.kind == 0x20) {
                    return Err(broken("TDMS strings cannot be interleaved because they have variable sample sizes"));
                }
                let count = active[0].1.count;
                if count == 0 || active.iter().any(|(_, r)| r.count != count) {
                    return Err(broken(
                        "interleaved TDMS channels have unequal chunk counts",
                    ));
                }
                let stride = chunk / count;
                if data.bytes.len() % stride != 0 {
                    return Err(broken("truncated TDMS interleaved sample"));
                }
                for _ in 0..data.bytes.len() / stride {
                    for (i, r) in &active {
                        let sample = data.sample(r.kind)?;
                        objects[*i].append(r.kind, sample)?;
                    }
                }
            } else {
                while data.pos < data.bytes.len() {
                    for (i, r) in &active {
                        let remaining = data.bytes.len() - data.pos;
                        if remaining == 0 {
                            break;
                        }
                        if r.kind == 0x20 {
                            if remaining < r.bytes {
                                return Err(broken("truncated TDMS string chunk"));
                            }
                            let payload = data.take(r.bytes)?;
                            let mut strings = Cursor {
                                bytes: payload,
                                pos: 0,
                                little,
                            };
                            let mut ends = Vec::with_capacity(r.count);
                            for _ in 0..r.count {
                                ends.push(strings.u32()? as usize);
                            }
                            let content = &payload[strings.pos..];
                            let mut begin = 0;
                            for end in ends {
                                if end < begin {
                                    return Err(broken("nonmonotonic TDMS string offsets"));
                                }
                                let text = String::from_utf8(
                                    content
                                        .get(begin..end)
                                        .ok_or_else(|| {
                                            broken("TDMS string offset exceeds payload")
                                        })?
                                        .to_vec(),
                                )
                                .map_err(broken)?;
                                objects[*i].append(r.kind, Sample::Text(text))?;
                                begin = end;
                            }
                            if begin != content.len() {
                                return Err(broken("TDMS string data size mismatch"));
                            }
                        } else {
                            let w = width(r.kind)?;
                            let count = r.count.min(remaining / w);
                            for _ in 0..count {
                                let sample = data.sample(r.kind)?;
                                objects[*i].append(r.kind, sample)?;
                            }
                            if count < r.count && remaining % w != 0 {
                                return Err(broken("truncated TDMS sample"));
                            }
                        }
                    }
                    if active.iter().all(|(_, r)| r.count == 0) {
                        return Err(broken("TDMS chunk makes no progress"));
                    }
                }
            }
        }
        previous = true;
        pos = end;
    }
    Ok(objects)
}
fn align(data: &ColumnData, positions: &[usize], n: usize) -> ColumnData {
    match data {
        ColumnData::Float(v) | ColumnData::DateTime(v) => {
            let mut out = vec![f64::NAN; n];
            for (&i, &x) in positions.iter().zip(v) {
                out[i] = x;
            }
            if matches!(data, ColumnData::DateTime(_)) {
                ColumnData::DateTime(out)
            } else {
                ColumnData::Float(out)
            }
        }
        ColumnData::Str(v) => {
            let mut out = vec![String::new(); n];
            for (&i, x) in positions.iter().zip(v) {
                out[i] = x.clone();
            }
            ColumnData::Str(out)
        }
    }
}
pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let objects = decode(&std::fs::read(path)?)?;
    let mut groups: Vec<(String, Vec<(&Object, String)>)> = Vec::new();
    for object in &objects {
        let Some(values) = &object.values else {
            continue;
        };
        if values.len() == 0 {
            continue;
        }
        let parts = paths(&object.path)?;
        if parts.len() != 2 {
            return Err(broken("TDMS raw data is not a group channel"));
        }
        let index = if let Some(i) = groups.iter().position(|g| g.0 == parts[0]) {
            i
        } else {
            groups.push((parts[0].clone(), Vec::new()));
            groups.len() - 1
        };
        groups[index].1.push((object, parts[1].clone()));
    }
    let mut frames = Vec::new();
    for (group, channels) in groups {
        let timed = channels
            .iter()
            .all(|(c, _)| c.properties.contains_key("wf_increment"));
        let times: Vec<Vec<f64>> = channels
            .iter()
            .map(|(c, _)| {
                let n = c.values.as_ref().unwrap().len();
                let dt = if timed {
                    c.properties["wf_increment"]
                } else {
                    1.0
                };
                let offset = if timed {
                    c.properties.get("wf_start_offset").copied().unwrap_or(0.0)
                } else {
                    0.0
                };
                (0..n).map(|i| offset + i as f64 * dt).collect()
            })
            .collect();
        let mut index: Vec<f64> = times.iter().flatten().copied().collect();
        if index.iter().any(|x| !x.is_finite()) {
            return Err(broken("nonfinite TDMS waveform time"));
        }
        index.sort_by(f64::total_cmp);
        index.dedup_by(|a, b| *a == *b);
        let mut columns = Vec::new();
        if timed {
            columns.push(Column::float("Time_[s]", index.clone()));
        }
        for ((channel, name), times) in channels.into_iter().zip(times) {
            let positions = times
                .iter()
                .map(|t| {
                    index
                        .binary_search_by(|x| x.total_cmp(t))
                        .map_err(|_| broken("TDMS time alignment failed"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            columns.push(Column::new(
                name,
                align(channel.values.as_ref().unwrap(), &positions, index.len()),
            ));
        }
        frames.push(NamedFrame::new(group, super::frame(columns)?));
    }
    if frames.len() == 1 {
        frames[0].name.clear();
    }
    Ok(super::object(path,"TDMS file",frames,None,"Standard contiguous/interleaved TDMS, both byte orders; numeric/string/UTC timestamp channels; relative waveform time alignment".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escaped_paths() {
        assert_eq!(paths("/'a''b'/'c/d'").unwrap(), vec!["a'b", "c/d"]);
        assert!(paths("/'unclosed").is_err());
    }
    #[test]
    fn timestamp_byte_orders() {
        let seconds = 2_082_844_800u64;
        let fraction = 1u64 << 63;
        let mut little = fraction.to_le_bytes().to_vec();
        little.extend_from_slice(&seconds.to_le_bytes());
        let mut big = seconds.to_be_bytes().to_vec();
        big.extend_from_slice(&fraction.to_be_bytes());
        for (bytes, little) in [(&little, true), (&big, false)] {
            match (Cursor {
                bytes,
                pos: 0,
                little,
            })
            .sample(0x44)
            .unwrap()
            {
                Sample::Number(v) => assert_eq!(v, 0.5),
                _ => panic!("timestamp became text"),
            }
        }
    }
    #[test]
    fn big_endian_signed_and_float() {
        match (Cursor {
            bytes: &(-123i16).to_be_bytes(),
            pos: 0,
            little: false,
        })
        .sample(2)
        .unwrap()
        {
            Sample::Number(v) => assert_eq!(v, -123.0),
            _ => panic!(),
        }
        match (Cursor {
            bytes: &1.25f64.to_be_bytes(),
            pos: 0,
            little: false,
        })
        .sample(10)
        .unwrap()
        {
            Sample::Number(v) => assert_eq!(v, 1.25),
            _ => panic!(),
        }
    }
    fn push32(b: &mut Vec<u8>, v: u32, little: bool) {
        b.extend_from_slice(&if little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        });
    }
    fn push64(b: &mut Vec<u8>, v: u64, little: bool) {
        b.extend_from_slice(&if little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        });
    }
    fn string(b: &mut Vec<u8>, s: &str, little: bool) {
        push32(b, s.len() as u32, little);
        b.extend_from_slice(s.as_bytes());
    }
    fn channel(
        b: &mut Vec<u8>,
        name: &str,
        kind: u32,
        count: u64,
        size: Option<u64>,
        little: bool,
    ) {
        string(b, &format!("/'group'/'{name}'"), little);
        push32(b, if size.is_some() { 28 } else { 20 }, little);
        push32(b, kind, little);
        push32(b, 1, little);
        push64(b, count, little);
        if let Some(size) = size {
            push64(b, size, little);
        }
        push32(b, 0, little);
    }
    fn segment(meta: &[u8], raw: &[u8], flags: u32, little: bool) -> Vec<u8> {
        let mut b = b"TDSm".to_vec();
        push32(&mut b, flags | if little { 0 } else { 1 << 6 }, true);
        push32(&mut b, 4713, little);
        push64(&mut b, (meta.len() + raw.len()) as u64, little);
        push64(&mut b, meta.len() as u64, little);
        b.extend_from_slice(meta);
        b.extend_from_slice(raw);
        b
    }
    #[test]
    fn interleaved_segments_reuse_metadata_in_both_orders() {
        for little in [true, false] {
            let mut meta = Vec::new();
            push32(&mut meta, 2, little);
            channel(&mut meta, "a", 2, 2, None, little);
            channel(&mut meta, "b", 2, 2, None, little);
            let raw = |v: &[i16]| {
                v.iter()
                    .flat_map(|v| {
                        if little {
                            v.to_le_bytes()
                        } else {
                            v.to_be_bytes()
                        }
                    })
                    .collect::<Vec<_>>()
            };
            let mut b = segment(
                &meta,
                &raw(&[1, 10, 2, 20]),
                (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5),
                little,
            );
            b.extend(segment(
                &[],
                &raw(&[3, 30, 4, 40]),
                (1 << 3) | (1 << 5),
                little,
            ));
            let objects = decode(&b).unwrap();
            assert_eq!(
                objects[0].values,
                Some(ColumnData::Float(vec![1., 2., 3., 4.]))
            );
            assert_eq!(
                objects[1].values,
                Some(ColumnData::Float(vec![10., 20., 30., 40.]))
            );
        }
    }
    #[test]
    fn contiguous_strings_and_timestamp() {
        for little in [true, false] {
            let mut meta = Vec::new();
            push32(&mut meta, 2, little);
            channel(&mut meta, "text", 0x20, 2, Some(11), little);
            channel(&mut meta, "stamp", 0x44, 1, None, little);
            let mut raw = Vec::new();
            push32(&mut raw, 1, little);
            push32(&mut raw, 3, little);
            raw.extend_from_slice(b"abc");
            let seconds = 2_082_844_801u64;
            let fraction = 1u64 << 63;
            push64(&mut raw, if little { fraction } else { seconds }, little);
            push64(&mut raw, if little { seconds } else { fraction }, little);
            let objects = decode(&segment(
                &meta,
                &raw,
                (1 << 1) | (1 << 2) | (1 << 3),
                little,
            ))
            .unwrap();
            assert_eq!(
                objects[0].values,
                Some(ColumnData::Str(vec!["a".into(), "bc".into()]))
            );
            assert_eq!(objects[1].values, Some(ColumnData::DateTime(vec![1.5])));
        }
    }
}
