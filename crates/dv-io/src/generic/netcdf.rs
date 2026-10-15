//! Safe Rust NetCDF-3/4 reader, following xarray's <=2D root-group projection.
use dv_core::{Column, Field2D, FileObject, IoError, NamedFrame};
use netcdf3::{DataType, DataVector, FileReader, Variable};
use std::{io::Read, path::Path, sync::Arc};

fn values(v: DataVector) -> Vec<f64> {
    match v {
        DataVector::I8(v) => v.into_iter().map(|x| x as f64).collect(),
        DataVector::U8(v) => v.into_iter().map(|x| x as f64).collect(),
        DataVector::I16(v) => v.into_iter().map(|x| x as f64).collect(),
        DataVector::I32(v) => v.into_iter().map(|x| x as f64).collect(),
        DataVector::F32(v) => v.into_iter().map(|x| x as f64).collect(),
        DataVector::F64(v) => v,
    }
}
fn attr(var: &Variable, name: &str) -> Option<f64> {
    var.get_attr_f64(name)
        .and_then(|v| v.first().copied())
        .or_else(|| {
            var.get_attr_f32(name)
                .and_then(|v| v.first().map(|&x| x as f64))
        })
        .or_else(|| {
            var.get_attr_i32(name)
                .and_then(|v| v.first().map(|&x| x as f64))
        })
        .or_else(|| {
            var.get_attr_i16(name)
                .and_then(|v| v.first().map(|&x| x as f64))
        })
        .or_else(|| {
            var.get_attr_i8(name)
                .and_then(|v| v.first().map(|&x| x as f64))
        })
}
fn decode(var: &Variable, mut v: Vec<f64>) -> Vec<f64> {
    let missing = attr(var, "_FillValue");
    let missing2 = attr(var, "missing_value");
    let scale = attr(var, "scale_factor").unwrap_or(1.0);
    let offset = attr(var, "add_offset").unwrap_or(0.0);
    for x in &mut v {
        if missing == Some(*x) || missing2 == Some(*x) {
            *x = f64::NAN;
        } else {
            *x = *x * scale + offset;
        }
    }
    v
}
fn time_values(var: &Variable, values: &mut [f64]) -> Result<bool, IoError> {
    decode_time(
        var.get_attr_as_string("units"),
        var.get_attr_as_string("calendar"),
        values,
    )
}
pub(super) fn decode_time(
    units: Option<String>,
    calendar: Option<String>,
    values: &mut [f64],
) -> Result<bool, IoError> {
    let Some(units) = units else {
        return Ok(false);
    };
    let Some((unit, origin)) = units.split_once(" since ") else {
        return Ok(false);
    };
    if let Some(calendar) = calendar {
        if !["standard", "gregorian", "proleptic_gregorian"].contains(&calendar.as_str()) {
            return Err(IoError::Other(format!(
                "NetCDF calendar `{calendar}` cannot be represented by UTC datetime columns"
            )));
        }
    }
    let scale = match unit {
        "seconds" | "second" | "s" => 1.0,
        "minutes" | "minute" => 60.0,
        "hours" | "hour" => 3600.0,
        "days" | "day" => 86400.0,
        "milliseconds" => 1e-3,
        "microseconds" => 1e-6,
        _ => {
            return Err(IoError::Other(format!(
                "unsupported NetCDF time unit `{unit}`"
            )))
        }
    };
    let origin = origin.trim().trim_end_matches(" UTC");
    let dt = chrono::NaiveDateTime::parse_from_str(origin, "%Y-%m-%d %H:%M:%S%.f")
        .ok()
        .or_else(|| chrono::NaiveDateTime::parse_from_str(origin, "%Y-%m-%dT%H:%M:%S%.f").ok())
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(origin, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
        .ok_or_else(|| IoError::Other(format!("unsupported NetCDF time origin `{origin}`")))?;
    let offset = dv_core::dataframe::datetime_to_seconds(&dt);
    for x in values {
        *x = *x * scale + offset;
    }
    Ok(true)
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let mut signature = [0; 8];
    let n = std::fs::File::open(path)?.read(&mut signature)?;
    if n >= 8 && signature == *b"\x89HDF\r\n\x1a\n" {
        return super::netcdf_hdf::read(path);
    }
    if n < 4 || &signature[..3] != b"CDF" {
        return Err(IoError::WrongFormat("missing NetCDF signature".into()));
    }
    if ![1, 2].contains(&signature[3]) {
        return Err(IoError::Other(
            "NetCDF CDF-5 is not supported; only classic/CDF-2 files are supported".into(),
        ));
    }
    let mut reader = FileReader::open(path).map_err(super::broken)?;
    let vars: Vec<Variable> = reader.data_set().get_vars().into_iter().cloned().collect();
    let mut frames = Vec::new();
    let mut fields = Vec::new();
    for var in &vars {
        let dims = var.dim_names();
        if dims.len() == 1 && dims[0] == var.name() {
            continue;
        } // xarray coordinate, not data variable
        if !(1..=2).contains(&dims.len()) {
            continue;
        } // upstream intentionally projects only <=2D data
        let sizes: Vec<usize> = dims
            .iter()
            .map(|d| reader.data_set().dim_size(d).unwrap_or(0))
            .collect();
        if var.data_type() == DataType::U8 {
            let bytes = reader.read_var_u8(var.name()).map_err(super::broken)?;
            let width = if sizes.len() == 2 { sizes[1] } else { 1 };
            if width == 0 {
                continue;
            }
            let strings = bytes
                .chunks(width)
                .map(|s| {
                    String::from_utf8(s.to_vec())
                        .map(|s| s.trim_end_matches('\0').to_owned())
                        .map_err(super::broken)
                })
                .collect::<Result<Vec<_>, _>>()?;
            frames.push(NamedFrame::new(
                var.name(),
                super::frame(vec![Column::string("0", strings)])?,
            ));
            continue;
        }
        let mut data = decode(
            var,
            values(reader.read_var(var.name()).map_err(super::broken)?),
        );
        let datetime = time_values(var, &mut data)?;
        let width = if sizes.len() == 2 { sizes[1] } else { 1 };
        if width == 0 {
            continue;
        }
        let columns = (0..width)
            .map(|j| {
                let v = data.iter().skip(j).step_by(width).copied().collect();
                if datetime {
                    Column::datetime(j.to_string(), v)
                } else {
                    Column::float(j.to_string(), v)
                }
            })
            .collect();
        frames.push(NamedFrame::new(var.name(), super::frame(columns)?));
        if sizes.len() == 2 {
            let mut axes = Vec::new();
            for (d, &size) in dims.iter().zip(&sizes) {
                let axis =
                    if let Some(coord) = vars.iter().find(|v| v.name() == d && v.num_dims() == 1) {
                        decode(coord, values(reader.read_var(d).map_err(super::broken)?))
                    } else {
                        (0..size).map(|i| i as f64).collect()
                    };
                axes.push(axis);
            }
            fields.push(Field2D {
                name: var.name().into(),
                x: axes.remove(0),
                y: axes.remove(0),
                sx: dims[0].clone(),
                sy: dims[1].clone(),
                m: data,
            });
        }
    }
    Ok(super::object(
        path,
        "NetCDF file (<=2D)",
        frames,
        if fields.is_empty() {
            None
        } else {
            Some(fields)
        },
        "NetCDF-3; coordinate variables excluded from tables, packing/fill/time attributes decoded"
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cf_time_origin_and_units() {
        let mut values = vec![0., 1., f64::NAN];
        assert!(decode_time(
            Some("hours since 1970-01-01 00:00:00".into()),
            None,
            &mut values
        )
        .unwrap());
        assert_eq!(&values[..2], &[0., 3600.]);
        assert!(values[2].is_nan());
        assert!(decode_time(
            Some("days since 2000-01-01".into()),
            Some("360_day".into()),
            &mut values
        )
        .is_err());
    }
}
