//! MATLAB v7.3 values, including object-reference cells and structure arrays.
use super::mat::MatValue;
use crate::generic::{broken, hdf};
use dv_core::IoError;
use hdf5_reader::{group::Group, Dataset, Datatype};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone)]
enum Object {
    Dataset(Dataset),
    Group(Group),
}
fn index(group: &Group, objects: &mut BTreeMap<u64, Object>) -> Result<(), IoError> {
    objects.insert(group.address(), Object::Group(group.clone()));
    let (groups, datasets) = group.members().map_err(broken)?;
    for dataset in datasets {
        objects.insert(dataset.address(), Object::Dataset(dataset));
    }
    for child in groups {
        if !objects.contains_key(&child.address()) {
            index(&child, objects)?;
        }
    }
    Ok(())
}
fn references(
    dataset: &Dataset,
    objects: &BTreeMap<u64, Object>,
    stack: &mut Vec<u64>,
) -> Result<Vec<MatValue>, IoError> {
    let Datatype::Reference { size, .. } = dataset.dtype() else {
        return Err(broken(
            "MATLAB cell/struct field is not an object-reference array",
        ));
    };
    let raw = dataset.read_raw_bytes().map_err(broken)?;
    let addresses = hdf5_reader::reference::read_object_references(&raw, *size).map_err(broken)?;
    addresses
        .into_iter()
        .map(|address| {
            if address == 0 {
                return Ok(MatValue::Numeric {
                    dimensions: vec![0, 0],
                    real: Vec::new(),
                    imaginary: None,
                });
            }
            let object = objects
                .get(&address)
                .ok_or_else(|| broken("MATLAB reference points outside the object tree"))?;
            decode(object, objects, stack)
        })
        .collect()
}
fn decode(
    object: &Object,
    objects: &BTreeMap<u64, Object>,
    stack: &mut Vec<u64>,
) -> Result<MatValue, IoError> {
    let address = match object {
        Object::Dataset(d) => d.address(),
        Object::Group(g) => g.address(),
    };
    if stack.contains(&address) {
        return Err(broken(
            "cyclic MATLAB object references are not tabular values",
        ));
    }
    stack.push(address);
    let result = decode_inner(object, objects, stack);
    stack.pop();
    result
}
fn decode_inner(
    object: &Object,
    objects: &BTreeMap<u64, Object>,
    stack: &mut Vec<u64>,
) -> Result<MatValue, IoError> {
    match object {
        Object::Dataset(d) => {
            let class = d
                .attribute("MATLAB_class")
                .map_err(broken)?
                .read_string()
                .map_err(broken)?;
            if d.attributes().iter().any(|a| a.name == "MATLAB_empty") {
                let dimensions = hdf::numbers(d)?
                    .into_iter()
                    .map(|n| {
                        if n < 0.0 || n.fract() != 0.0 || n >= usize::MAX as f64 {
                            Err(broken("invalid MATLAB empty dimensions"))
                        } else {
                            Ok(n as usize)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(MatValue::Numeric {
                    dimensions,
                    real: Vec::new(),
                    imaginary: None,
                });
            }
            if class == "cell" {
                return Ok(MatValue::Cell(references(d, objects, stack)?));
            }
            if class == "char" {
                let units = hdf::numbers(d)?
                    .into_iter()
                    .map(|n| {
                        if !(0.0..=65535.0).contains(&n) || n.fract() != 0.0 {
                            Err(broken("invalid MATLAB UTF-16 code unit"))
                        } else {
                            Ok(n as u16)
                        }
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                return String::from_utf16(&units)
                    .map(MatValue::Text)
                    .map_err(broken);
            }
            if ![
                "double", "single", "logical", "int8", "uint8", "int16", "uint16", "int32",
                "uint32", "int64", "uint64",
            ]
            .contains(&class.as_str())
            {
                return Err(IoError::Other(format!(
                    "unsupported MATLAB v7.3 dataset class `{class}`"
                )));
            }
            let mut dimensions = hdf::shape(d)?;
            dimensions.reverse();
            let (real, imaginary) = if let Datatype::Compound { size, fields } = d.dtype() {
                let r = fields
                    .iter()
                    .find(|f| f.name == "real")
                    .ok_or_else(|| broken("MATLAB complex array has no real component"))?;
                let i = fields
                    .iter()
                    .find(|f| f.name == "imag")
                    .ok_or_else(|| broken("MATLAB complex array has no imaginary component"))?;
                if *size == 0 {
                    return Err(broken("zero-width MATLAB complex datatype"));
                }
                let raw = d.read_raw_bytes().map_err(broken)?;
                let component = |field: &hdf5_reader::CompoundField| {
                    raw.chunks_exact(*size as usize)
                        .map(|b| {
                            let begin = field.byte_offset as usize;
                            let width =
                                hdf5_reader::dtype_element_size(&field.datatype).map_err(broken)?;
                            hdf::number(
                                b.get(
                                    begin
                                        ..begin.checked_add(width).ok_or_else(|| {
                                            broken("complex field offset overflow")
                                        })?,
                                )
                                .ok_or_else(|| broken("complex field extends beyond element"))?,
                                &field.datatype,
                            )
                        })
                        .collect::<Result<Vec<_>, _>>()
                };
                (component(r)?, Some(component(i)?))
            } else {
                (hdf::numbers(d)?, None)
            };
            Ok(MatValue::Numeric {
                dimensions,
                real,
                imaginary,
            })
        }
        Object::Group(g) => {
            let class = g
                .attribute("MATLAB_class")
                .map_err(broken)?
                .read_string()
                .map_err(broken)?;
            if class != "struct" {
                return Err(IoError::Other(format!("unsupported MATLAB v7.3 group class `{class}` (including sparse/object values)")));
            }
            let (groups, datasets) = g.members().map_err(broken)?;
            let mut members: Vec<(String, Object)> = groups
                .into_iter()
                .map(|g| (g.name().to_owned(), Object::Group(g)))
                .chain(
                    datasets
                        .into_iter()
                        .map(|d| (d.name().to_owned(), Object::Dataset(d))),
                )
                .collect();
            members.sort_by(|a, b| a.0.cmp(&b.0));
            let mut fields = Vec::new();
            let mut columns = Vec::new();
            for (name, member) in members {
                let values = match &member {
                    Object::Dataset(d) if matches!(d.dtype(), Datatype::Reference { .. }) => {
                        references(d, objects, stack)?
                    }
                    _ => vec![decode(&member, objects, stack)?],
                };
                fields.push(name);
                columns.push(values);
            }
            let n = columns.first().map(Vec::len).unwrap_or(1);
            if columns.iter().any(|v| v.len() != n) {
                return Err(broken("MATLAB structure field shapes differ"));
            }
            let mut elements: Vec<Vec<MatValue>> =
                (0..n).map(|_| Vec::with_capacity(fields.len())).collect();
            for column in columns {
                for (row, value) in elements.iter_mut().zip(column) {
                    row.push(value);
                }
            }
            Ok(MatValue::Struct { fields, elements })
        }
    }
}

pub(super) fn read(path: &Path) -> Result<BTreeMap<String, MatValue>, IoError> {
    let file = hdf::open(path)?;
    let root = file.root_group().map_err(broken)?;
    let mut objects = BTreeMap::new();
    index(&root, &mut objects)?;
    let (groups, datasets) = root.members().map_err(broken)?;
    let mut values = BTreeMap::new();
    for object in groups
        .into_iter()
        .map(Object::Group)
        .chain(datasets.into_iter().map(Object::Dataset))
    {
        let name = match &object {
            Object::Dataset(d) => d.name(),
            Object::Group(g) => g.name(),
        };
        if name.starts_with('#') {
            continue;
        }
        values.insert(name.to_owned(), decode(&object, &objects, &mut Vec::new())?);
    }
    Ok(values)
}
