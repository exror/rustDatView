//! Safe tabular pickle reader. Python constructors are inspected, never executed.
//! Supports built-in column dictionaries/matrices, NumPy numeric/string arrays,
//! and pandas BlockManager dataframes containing those arrays.
#[path = "pickle_vm.rs"]
mod vm;
use crate::generic::broken;
use dv_core::{Column, ColumnData, FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};
use vm::{integer, sequence, text, Node, Value};

fn object(n: &Node) -> Option<(String, String, Node, Option<Node>)> {
    if let Value::Object {
        constructor,
        args,
        state,
    } = &*n.borrow()
    {
        if let Value::Global(module, name) = &*constructor.borrow() {
            return Some((module.clone(), name.clone(), args.clone(), state.clone()));
        }
    }
    None
}
fn dict(n: &Node, key: &str) -> Option<Node> {
    if let Value::Dict(d) = &*n.borrow() {
        d.iter()
            .find_map(|(k, v)| (text(k).ok().as_deref() == Some(key)).then(|| v.clone()))
    } else {
        None
    }
}
fn scalar(n: &Node) -> Result<(Option<f64>, String), IoError> {
    match &*n.borrow() {
        Value::None => Ok((Some(f64::NAN), String::new())),
        Value::Float(v) => Ok((Some(*v), v.to_string())),
        Value::Int(v) => Ok((Some(*v as f64), v.to_string())),
        Value::Bool(v) => Ok((Some(f64::from(u8::from(*v))), v.to_string())),
        Value::Text(s) => Ok((None, s.clone())),
        Value::Bytes(b) => Ok((None, String::from_utf8(b.clone()).map_err(super::broken)?)),
        _ => Err(IoError::Other(
            "pickle table contains non-scalar objects".into(),
        )),
    }
}
#[derive(Debug)]
struct Array {
    shape: Vec<usize>,
    data: ColumnData,
    fortran: bool,
}
fn bytes(n: &Node) -> Result<Vec<u8>, IoError> {
    match &*n.borrow() {
        Value::Bytes(b) => return Ok(b.clone()),
        _ => {}
    }
    if let Some((module, name, args, _)) = object(n) {
        if module == "_codecs" && name == "encode" {
            let a = sequence(&args)?;
            if a.len() >= 2 && text(&a[1])? == "latin1" {
                return text(&a[0])?
                    .chars()
                    .map(|c| u8::try_from(c as u32).map_err(super::broken))
                    .collect();
            }
        }
        if module == "builtins" && name == "bytearray" {
            let args = sequence(&args)?;
            let value = args
                .first()
                .ok_or_else(|| super::broken("bytearray constructor missing data"))?;
            return bytes(value);
        }
    }
    Err(IoError::Other(
        "pickle array requires an in-band byte buffer".into(),
    ))
}
fn dtype(n: &Node) -> Result<(String, bool), IoError> {
    let Some((module, name, args, state)) = object(n) else {
        return Ok((text(n)?, cfg!(target_endian = "little")));
    };
    if !(module == "numpy" && name == "dtype") {
        return Err(IoError::Other(
            "unsupported pickle dtype constructor".into(),
        ));
    }
    let args = sequence(&args)?;
    let code = text(
        args.first()
            .ok_or_else(|| super::broken("missing NumPy dtype"))?,
    )?;
    let mut little = !code.starts_with('>');
    if let Some(state) = state {
        let state = sequence(&state)?;
        if let Some(order) = state.get(1) {
            match text(order)?.as_str() {
                ">" => little = false,
                "<" => little = true,
                "=" | "|" => little = cfg!(target_endian = "little"),
                _ => return Err(super::broken("invalid NumPy endian marker")),
            }
        }
    }
    Ok((
        code.trim_start_matches(['<', '>', '=', '|']).to_owned(),
        little,
    ))
}
fn decode_buffer(raw: Vec<u8>, code: &str, little: bool) -> Result<ColumnData, IoError> {
    if code.is_empty() || !code.is_ascii() {
        return Err(super::broken("invalid NumPy dtype code"));
    }
    let (kind, width) = code.split_at(1);
    let width: usize = width.parse().map_err(super::broken)?;
    let size = if kind == "U" {
        width
            .checked_mul(4)
            .ok_or_else(|| super::broken("NumPy string width overflow"))?
    } else {
        width
    };
    if size == 0 || raw.len() % size != 0 {
        return Err(super::broken("NumPy data buffer size does not match dtype"));
    }
    if kind == "S" || kind == "U" {
        let strings = raw
            .chunks(size)
            .map(|chunk| {
                if kind == "S" {
                    return String::from_utf8(
                        chunk.iter().copied().take_while(|&b| b != 0).collect(),
                    )
                    .map_err(super::broken);
                }
                let mut s = String::new();
                for b in chunk.chunks(4) {
                    let b = b.try_into().unwrap();
                    let c = if little {
                        u32::from_le_bytes(b)
                    } else {
                        u32::from_be_bytes(b)
                    };
                    if c == 0 {
                        break;
                    }
                    s.push(
                        char::from_u32(c)
                            .ok_or_else(|| super::broken("invalid NumPy Unicode character"))?,
                    );
                }
                Ok(s)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(ColumnData::Str(strings));
    }
    let mut out = Vec::with_capacity(raw.len() / size);
    for b in raw.chunks(size) {
        let mut uint = 0u64;
        if size > 8 {
            return Err(IoError::Other(format!("unsupported NumPy dtype `{code}`")));
        }
        for (i, &v) in b.iter().enumerate() {
            let shift = if little { i } else { size - 1 - i };
            uint |= (v as u64) << (8 * shift);
        }
        let v = match (kind,size) {
            ("f",4) => f32::from_bits(uint as u32) as f64, ("f",8) => f64::from_bits(uint),
            ("i",1|2|4|8) => ((uint << (64-size*8)) as i64 >> (64-size*8)) as f64,
            ("u",1|2|4|8) => uint as f64, ("b",1) | ("?",1) => f64::from(u8::from(uint != 0)),
            _ => return Err(IoError::Other(format!("unsupported NumPy dtype `{code}`; complex, datetime, structured and extended-precision dtypes are not represented by dv-core"))),
        };
        out.push(v);
    }
    Ok(ColumnData::Float(out))
}
fn ndarray(n: &Node) -> Result<Option<Array>, IoError> {
    let Some((module, name, args, state)) = object(n) else {
        return Ok(None);
    };
    if !module.starts_with("numpy") {
        return Ok(None);
    }
    let args = sequence(&args)?;
    let (shape, dtype_node, fortran, payload) = if name == "_reconstruct" {
        let state =
            sequence(&state.ok_or_else(|| super::broken("NumPy array missing restore state"))?)?;
        if state.len() != 5 {
            return Err(IoError::Other(
                "unsupported NumPy ndarray restore state".into(),
            ));
        }
        let fortran = matches!(*state[3].borrow(), Value::Bool(true));
        (
            state[1].clone(),
            state[2].clone(),
            fortran,
            state[4].clone(),
        )
    } else if name == "_frombuffer" {
        if args.len() != 4 {
            return Err(super::broken("invalid NumPy frombuffer arguments"));
        }
        (
            args[2].clone(),
            args[1].clone(),
            text(&args[3])? == "F",
            args[0].clone(),
        )
    } else {
        return Ok(None);
    };
    let shape = sequence(&shape)?
        .iter()
        .map(integer)
        .collect::<Result<Vec<_>, _>>()?;
    if shape.len() > 2 {
        return Err(IoError::Other(
            "pickle ndarray tabular conversion supports at most two dimensions".into(),
        ));
    }
    let (code, little) = dtype(&dtype_node)?;
    let data = if code.starts_with('O') {
        let values = sequence(&payload)?
            .iter()
            .map(scalar)
            .collect::<Result<Vec<_>, _>>()?;
        if values.iter().all(|v| v.0.is_some()) {
            ColumnData::Float(values.into_iter().map(|v| v.0.unwrap()).collect())
        } else {
            ColumnData::Str(values.into_iter().map(|v| v.1).collect())
        }
    } else {
        decode_buffer(bytes(&payload)?, &code, little)?
    };
    let len = shape
        .iter()
        .try_fold(1usize, |n, &d| n.checked_mul(d))
        .ok_or_else(|| super::broken("NumPy shape overflow"))?;
    if data.len() != len {
        return Err(super::broken("NumPy shape does not match payload"));
    }
    Ok(Some(Array {
        shape,
        data,
        fortran,
    }))
}
fn columns(name: &str, array: Array) -> Result<Vec<Column>, IoError> {
    let rows = array.shape.first().copied().unwrap_or(1);
    let width = array.shape.get(1).copied().unwrap_or(1);
    Ok((0..width)
        .map(|j| {
            let indices: Vec<usize> = (0..rows)
                .map(|i| {
                    if array.fortran {
                        j * rows + i
                    } else {
                        i * width + j
                    }
                })
                .collect();
            Column::new(
                if array.shape.len() <= 1 {
                    name.into()
                } else {
                    format!("C{j}")
                },
                array.data.take(&indices),
            )
        })
        .collect())
}
fn index(n: &Node) -> Result<Vec<String>, IoError> {
    if let Some(a) = ndarray(n)? {
        return Ok((0..a.data.len()).map(|i| a.data.display(i)).collect());
    }
    if let Ok(s) = sequence(n) {
        return s.iter().map(|n| scalar(n).map(|v| v.1)).collect();
    }
    if let Some((module, name, args, state)) = object(n) {
        if module.starts_with("pandas") && name == "_new_Index" {
            let args = sequence(&args)?;
            let data = args
                .get(1)
                .and_then(|n| dict(n, "data"))
                .ok_or_else(|| super::broken("pandas index missing data"))?;
            return index(&data);
        }
        if module.starts_with("pandas") && name == "RangeIndex" {
            if let Some(state) = state {
                let start = dict(&state, "start")
                    .map(|n| integer(&n))
                    .transpose()?
                    .unwrap_or(0);
                let stop =
                    dict(&state, "stop").ok_or_else(|| super::broken("RangeIndex missing stop"))?;
                return Ok((start..integer(&stop)?).map(|i| i.to_string()).collect());
            }
        }
    }
    Err(IoError::Other("unsupported pandas index encoding".into()))
}
fn dataframe(n: &Node) -> Result<Option<dv_core::DataFrame>, IoError> {
    let Some((module, name, _, state)) = object(n) else {
        return Ok(None);
    };
    if !module.starts_with("pandas") || name != "DataFrame" {
        return Ok(None);
    }
    let state = state.ok_or_else(|| super::broken("pandas DataFrame missing state"))?;
    let manager = dict(&state, "_mgr").ok_or_else(|| {
        IoError::Other("unsupported pandas DataFrame state (expected BlockManager)".into())
    })?;
    let (_, name, args, _) =
        object(&manager).ok_or_else(|| super::broken("invalid pandas manager"))?;
    if name != "BlockManager" {
        return Err(IoError::Other(
            "pandas ArrayManager pickles are not supported".into(),
        ));
    }
    let args = sequence(&args)?;
    if args.len() != 2 {
        return Err(super::broken("invalid pandas BlockManager arguments"));
    }
    let axes = sequence(&args[1])?;
    let names = index(
        axes.first()
            .ok_or_else(|| super::broken("missing pandas column axis"))?,
    )?;
    let mut cols: Vec<Option<Column>> = vec![None; names.len()];
    for block in sequence(&args[0])? {
        let (_, name, args, _) =
            object(&block).ok_or_else(|| super::broken("invalid pandas block"))?;
        if name != "_unpickle_block" {
            return Err(IoError::Other(format!(
                "unsupported pandas block constructor `{name}`"
            )));
        }
        let args = sequence(&args)?;
        let array = ndarray(
            args.first()
                .ok_or_else(|| super::broken("missing pandas block data"))?,
        )?
        .ok_or_else(|| {
            IoError::Other(
                "pandas extension arrays (categorical/timezone/datetime) are not supported".into(),
            )
        })?;
        let placement = args
            .get(1)
            .ok_or_else(|| super::broken("missing pandas block placement"))?;
        let places = if let Some((_, name, a, _)) = object(placement) {
            if name == "slice" {
                let a = sequence(&a)?;
                if a.len() != 3 {
                    return Err(super::broken("invalid pandas slice arguments"));
                }
                let start = integer(&a[0])?;
                let stop = integer(&a[1])?;
                let step = match &*a[2].borrow() {
                    Value::None => 1,
                    _ => integer(&a[2])?,
                };
                if step == 0 {
                    return Err(super::broken("zero pandas slice step"));
                }
                (start..stop).step_by(step).collect::<Vec<_>>()
            } else if let Some(a) = ndarray(placement)? {
                match a.data {
                    ColumnData::Float(v) => v.into_iter().map(|v| v as usize).collect(),
                    _ => return Err(super::broken("non-numeric block placement")),
                }
            } else {
                return Err(IoError::Other("unsupported pandas block placement".into()));
            }
        } else {
            sequence(placement)?
                .iter()
                .map(integer)
                .collect::<Result<Vec<_>, _>>()?
        };
        let width = array.shape.first().copied().unwrap_or(1);
        let rows = array.shape.get(1).copied().unwrap_or(1);
        if width != places.len() {
            return Err(super::broken("pandas block width does not match placement"));
        }
        for (j, place) in places.into_iter().enumerate() {
            if place >= names.len() {
                return Err(super::broken("pandas block column outside axis"));
            }
            let ids: Vec<usize> = (0..rows)
                .map(|i| {
                    if array.fortran {
                        i * width + j
                    } else {
                        j * rows + i
                    }
                })
                .collect();
            cols[place] = Some(Column::new(&names[place], array.data.take(&ids)));
        }
    }
    Ok(Some(super::frame(
        cols.into_iter()
            .map(|c| c.ok_or_else(|| super::broken("pandas column has no data block")))
            .collect::<Result<Vec<_>, _>>()?,
    )?))
}
fn table(name: &str, n: &Node) -> Result<Option<dv_core::DataFrame>, IoError> {
    if let Some(df) = dataframe(n)? {
        return Ok(Some(df));
    }
    if let Some(a) = ndarray(n)? {
        return Ok(Some(super::frame(columns(name, a)?)?));
    }
    if let Value::Seq(seq) = &*n.borrow() {
        if seq.is_empty() {
            return Ok(Some(dv_core::DataFrame::new()));
        }
        if seq.iter().all(|v| scalar(v).is_ok()) {
            let v = seq.iter().map(scalar).collect::<Result<Vec<_>, _>>()?;
            let data = if v.iter().all(|v| v.0.is_some()) {
                ColumnData::Float(v.into_iter().map(|v| v.0.unwrap()).collect())
            } else {
                ColumnData::Str(v.into_iter().map(|v| v.1).collect())
            };
            return Ok(Some(super::frame(vec![Column::new(name, data)])?));
        }
        let rows = seq.iter().map(sequence).collect::<Result<Vec<_>, _>>()?;
        let width = rows[0].len();
        if rows.iter().any(|r| r.len() != width) {
            return Err(super::broken("ragged pickle matrix"));
        }
        let mut cols = Vec::new();
        for j in 0..width {
            let v = rows
                .iter()
                .map(|r| scalar(&r[j]))
                .collect::<Result<Vec<_>, _>>()?;
            let data = if v.iter().all(|v| v.0.is_some()) {
                ColumnData::Float(v.into_iter().map(|v| v.0.unwrap()).collect())
            } else {
                ColumnData::Str(v.into_iter().map(|v| v.1).collect())
            };
            cols.push(Column::new(format!("C{j}"), data));
        }
        return Ok(Some(super::frame(cols)?));
    }
    Ok(None)
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let data = std::fs::read(path)?;
    let root = vm::parse(&data)?;
    let mut frames = Vec::new();
    if let Some(df) = table("data", &root)? {
        frames.push(NamedFrame::unnamed(df));
    } else if let Value::Dict(entries) = &*root.borrow() {
        let mut column_table = Vec::new();
        let mut all_columns = true;
        for (key, value) in entries {
            let name = text(key)?;
            if let Some(df) = table(&name, value)? {
                if df.ncols() == 1 && matches!(*value.borrow(), Value::Seq(_)) {
                    let mut c = df.column(0).clone();
                    c.name = name.clone();
                    column_table.push(c);
                } else {
                    all_columns = false;
                }
                frames.push(NamedFrame::new(name, df));
            } else if matches!(
                *value.borrow(),
                Value::None
                    | Value::Bool(_)
                    | Value::Int(_)
                    | Value::Float(_)
                    | Value::Text(_)
                    | Value::Bytes(_)
            ) {
                all_columns = false;
            } else {
                return Err(IoError::Other(format!(
                    "pickle entry `{name}` is not a supported table or NumPy/pandas object"
                )));
            }
        }
        if all_columns
            && !column_table.is_empty()
            && column_table
                .iter()
                .all(|c| c.len() == column_table[0].len())
        {
            frames = vec![NamedFrame::unnamed(super::frame(column_table)?)];
        } else if frames.len() == 1 {
            frames[0].name.clear();
        }
    }
    if frames.is_empty() {
        return Err(IoError::Other(
            "pickle contains no supported tabular data; arbitrary Python objects are not executed"
                .into(),
        ));
    }
    Ok(super::object(
        path,
        "Pickle file",
        frames,
        None,
        "Inert binary pickle decoder; numeric/string NumPy arrays and pandas BlockManager tables"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_numeric_list() {
        let n = vm::parse(b"\x80\x04]\x94(K\x01K\x02e.").unwrap();
        let df = table("data", &n).unwrap().unwrap();
        assert_eq!(df.column(0).floats().unwrap(), &[1., 2.]);
    }
    #[test]
    fn numpy_endian() {
        let a = decode_buffer(vec![0, 0, 0, 1, 255, 255, 255, 254], "i4", false).unwrap();
        assert_eq!(a, ColumnData::Float(vec![1., -2.]));
    }
    #[test]
    fn arbitrary_constructor_is_inert() {
        let n = vm::parse(b"cos\nsystem\n(Vexit\ntR.").unwrap();
        assert!(table("data", &n).unwrap().is_none());
        assert!(object(&n).is_some());
    }
    #[test]
    fn recursive_pickle_rejected() {
        assert!(vm::parse(b"\x80\x04]\x94h\x00a.").is_err());
    }
}
