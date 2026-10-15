//! NetCDF-4 projection using HDF5 dimension scales and CF decoding.
use super::{broken, hdf};
use dv_core::{Column, Field2D, FileObject, IoError, NamedFrame};
use hdf5_reader::{Dataset, Datatype, StringSize, VarLenKind};
use std::{collections::BTreeSet, path::Path, sync::Arc};

fn text(d: &Dataset, name: &str) -> Result<Option<String>, IoError> {
    d.attributes()
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| a.read_string().map_err(broken))
        .transpose()
}
fn number(d: &Dataset, name: &str) -> Result<Option<f64>, IoError> {
    d.attributes()
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| a.read_as_f64().map_err(broken))
        .transpose()
}
fn packed(d: &Dataset) -> Result<Vec<f64>, IoError> {
    let fill = number(d, "_FillValue")?;
    let missing = number(d, "missing_value")?;
    let scale = number(d, "scale_factor")?.unwrap_or(1.0);
    let offset = number(d, "add_offset")?.unwrap_or(0.0);
    let mut values = hdf::numbers(d)?;
    for v in &mut values {
        if fill == Some(*v) || missing == Some(*v) {
            *v = f64::NAN;
        } else {
            *v = *v * scale + offset;
        }
    }
    Ok(values)
}
fn dimensions(d: &Dataset, datasets: &[Dataset]) -> Result<Vec<Option<usize>>, IoError> {
    let Some(attr) = d
        .attributes()
        .into_iter()
        .find(|a| a.name == "DIMENSION_LIST")
    else {
        return Ok(vec![None; d.ndim()]);
    };
    let Datatype::VarLen {
        base,
        kind: VarLenKind::Sequence,
        ..
    } = attr.datatype
    else {
        return Err(broken("invalid NetCDF DIMENSION_LIST datatype"));
    };
    let Datatype::Reference { size, .. } = *base else {
        return Err(broken(
            "NetCDF dimension list contains non-object references",
        ));
    };
    let width = d.vlen_reference_size();
    if width == 0 || attr.raw_data.len() % width != 0 {
        return Err(broken("invalid NetCDF dimension-list payload"));
    }
    let dims = attr
        .raw_data
        .chunks_exact(width)
        .map(|reference| {
            let raw = d
                .resolve_vlen_reference_bytes(reference, size as usize)
                .map_err(broken)?;
            let refs =
                hdf5_reader::reference::read_object_references(&raw, size).map_err(broken)?;
            let address = refs
                .first()
                .ok_or_else(|| broken("NetCDF dimension has no scale reference"))?;
            datasets
                .iter()
                .position(|d| d.address() == *address)
                .map(Some)
                .ok_or_else(|| broken("NetCDF dimension scale is not in the root group"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if dims.len() != d.ndim() {
        return Err(broken("NetCDF dimension list rank mismatch"));
    }
    Ok(dims)
}
pub(super) fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let file = hdf::open(path)?;
    let root = file.root_group().map_err(broken)?;
    let mut datasets = root.datasets().map_err(broken)?;
    datasets.sort_by(|a, b| a.name().cmp(b.name()));
    let mut coordinates = BTreeSet::new();
    for d in &datasets {
        if text(d, "CLASS")?.as_deref() == Some("DIMENSION_SCALE") {
            coordinates.insert(d.name().to_owned());
        }
        if let Some(names) = text(d, "coordinates")? {
            coordinates.extend(names.split_whitespace().map(str::to_owned));
        }
    }
    let mut frames = Vec::new();
    let mut fields = Vec::new();
    for d in &datasets {
        if !(1..=2).contains(&d.ndim()) || coordinates.contains(d.name()) {
            continue;
        }
        let shape = hdf::shape(d)?;
        let width = if shape.len() == 2 { shape[1] } else { 1 };
        if width == 0 {
            continue;
        }
        if matches!(
            d.dtype(),
            Datatype::String { .. }
                | Datatype::VarLen {
                    kind: VarLenKind::String,
                    ..
                }
        ) {
            let mut strings = d.read_strings().map_err(broken)?;
            if matches!(
                d.dtype(),
                Datatype::String {
                    size: StringSize::Fixed(1),
                    ..
                }
            ) && shape.len() == 2
            {
                strings = strings
                    .chunks(width)
                    .map(|s| s.concat().trim_end_matches('\0').to_owned())
                    .collect();
                frames.push(NamedFrame::new(
                    d.name(),
                    super::frame(vec![Column::string("0", strings)])?,
                ));
            } else {
                let columns = (0..width)
                    .map(|j| {
                        Column::string(
                            j.to_string(),
                            strings.iter().skip(j).step_by(width).cloned().collect(),
                        )
                    })
                    .collect();
                frames.push(NamedFrame::new(d.name(), super::frame(columns)?));
            }
            continue;
        }
        let mut values = packed(d)?;
        let datetime =
            super::netcdf::decode_time(text(d, "units")?, text(d, "calendar")?, &mut values)?;
        let columns = (0..width)
            .map(|j| {
                let v = values.iter().skip(j).step_by(width).copied().collect();
                if datetime {
                    Column::datetime(j.to_string(), v)
                } else {
                    Column::float(j.to_string(), v)
                }
            })
            .collect();
        frames.push(NamedFrame::new(d.name(), super::frame(columns)?));
        if shape.len() == 2 {
            let dims = dimensions(d, &datasets)?;
            let mut axes = Vec::new();
            let mut names = Vec::new();
            for (i, dim) in dims.into_iter().enumerate() {
                if let Some(j) = dim {
                    let coord = &datasets[j];
                    names.push(coord.name().to_owned());
                    if text(coord, "NAME")?.is_some_and(|s| {
                        s.starts_with("This is a netCDF dimension but not a netCDF variable")
                    }) {
                        axes.push((0..shape[i]).map(|i| i as f64).collect());
                    } else {
                        axes.push(packed(coord)?);
                    }
                } else {
                    names.push(format!("dim_{i}"));
                    axes.push((0..shape[i]).map(|i| i as f64).collect());
                }
                if axes[i].len() != shape[i] {
                    return Err(broken("NetCDF coordinate shape mismatch"));
                }
            }
            fields.push(Field2D {
                name: d.name().into(),
                x: axes.remove(0),
                y: axes.remove(0),
                sx: names.remove(0),
                sy: names.remove(0),
                m: values,
            });
        }
    }
    Ok(super::object(path,"NetCDF file (<=2D)",frames,if fields.is_empty(){None}else{Some(fields)},"NetCDF-4/HDF5; dimension scales and auxiliary coordinates excluded; CF packing/fill/time decoded".into()))
}
