//! Legacy ASCII/binary and XML VTK, with real point/cell tables (no dummy data).
use dv_core::{Column, Field2D, FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};
use vtkio::model::{Attribute, Attributes, DataSet, IOBuffer, Vtk};

fn buffer(b: IOBuffer) -> Result<Vec<f64>, IoError> {
    b.cast_into::<f64>()
        .ok_or_else(|| super::broken("VTK buffer cannot be converted to numeric data"))
}
fn arrays(attributes: Vec<Attribute>) -> Result<Vec<(String, usize, Vec<f64>)>, IoError> {
    let mut result = Vec::new();
    for a in attributes {
        match a {
            Attribute::DataArray(a) => {
                let n = a.num_comp();
                result.push((a.name, n, buffer(a.data)?));
            }
            Attribute::Field { data_array, .. } => {
                for a in data_array {
                    let n = a.num_comp();
                    result.push((a.name, n, buffer(a.data)?));
                }
            }
        }
    }
    Ok(result)
}
fn components(name: &str, n: usize, data: &[f64]) -> Result<Vec<Column>, IoError> {
    if n == 0 || data.len() % n != 0 {
        return Err(super::broken(
            "VTK attribute has inconsistent component count",
        ));
    }
    Ok((0..n)
        .map(|j| {
            let name = if n == 1 {
                name.to_owned()
            } else if n == 3 {
                format!("{name}_{}", ["x", "y", "z"][j])
            } else {
                format!("{name}_{j}")
            };
            Column::float(name, data.iter().skip(j).step_by(n).copied().collect())
        })
        .collect())
}
fn table(
    frames: &mut Vec<NamedFrame>,
    prefix: &str,
    points: Option<Vec<f64>>,
    attributes: Attributes,
) -> Result<(), IoError> {
    let mut cols = Vec::new();
    if let Some(p) = points {
        if p.len() % 3 != 0 {
            return Err(super::broken("VTK points are not triples"));
        }
        for (j, name) in ["x", "y", "z"].iter().enumerate() {
            cols.push(Column::float(
                *name,
                p.iter().skip(j).step_by(3).copied().collect(),
            ));
        }
    }
    for (name, n, v) in arrays(attributes.point)? {
        cols.extend(components(&name, n, &v)?);
    }
    if !cols.is_empty() {
        frames.push(NamedFrame::new(
            format!("{prefix}points"),
            super::frame(cols)?,
        ));
    }
    let mut cols = Vec::new();
    for (name, n, v) in arrays(attributes.cell)? {
        cols.extend(components(&name, n, &v)?);
    }
    if !cols.is_empty() {
        frames.push(NamedFrame::new(
            format!("{prefix}cells"),
            super::frame(cols)?,
        ));
    }
    Ok(())
}
fn fields(
    attributes: &[Attribute],
    axes: &[Vec<f64>; 3],
    prefix: &str,
) -> Result<Vec<Field2D>, IoError> {
    let varying: Vec<usize> = (0..3).filter(|&a| axes[a].len() > 1).collect();
    if varying.len() != 2 {
        return Ok(Vec::new());
    }
    let a = varying[0];
    let b = varying[1];
    let mut result = Vec::new();
    let count = axes
        .iter()
        .try_fold(1usize, |n, a| n.checked_mul(a.len()))
        .ok_or_else(|| super::broken("VTK grid size overflow"))?;
    for (name, n, v) in arrays(attributes.to_vec())? {
        if n == 0
            || v.len()
                != count
                    .checked_mul(n)
                    .ok_or_else(|| super::broken("VTK array size overflow"))?
        {
            return Err(super::broken(
                "VTK point data size does not match grid dimensions",
            ));
        }
        for c in 0..n {
            let mut m = Vec::with_capacity(count);
            for i in 0..axes[a].len() {
                for j in 0..axes[b].len() {
                    let mut idx = [0; 3];
                    idx[a] = i;
                    idx[b] = j;
                    let index = idx[0] + axes[0].len() * (idx[1] + axes[1].len() * idx[2]);
                    m.push(v[index * n + c]);
                }
            }
            let name = if n == 1 {
                format!("{prefix}{name}")
            } else if n == 3 {
                format!("{prefix}{name}_{}", ["x", "y", "z"][c])
            } else {
                format!("{prefix}{name}_{c}")
            };
            result.push(Field2D {
                name,
                x: axes[a].clone(),
                y: axes[b].clone(),
                sx: ["x", "y", "z"][a].into(),
                sy: ["x", "y", "z"][b].into(),
                m,
            });
        }
    }
    Ok(result)
}
fn grid_points(axes: &[Vec<f64>; 3]) -> Vec<f64> {
    let mut p = Vec::new();
    for &z in &axes[2] {
        for &y in &axes[1] {
            for &x in &axes[0] {
                p.extend([x, y, z]);
            }
        }
    }
    p
}
fn prefix(i: usize, n: usize) -> String {
    if n == 1 {
        String::new()
    } else {
        format!("piece{i}_")
    }
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let vtk = Vtk::import(path).map_err(super::broken)?;
    let detail = vtk.title;
    let mut frames = Vec::new();
    let mut all_fields = Vec::new();
    match vtk.data {
        DataSet::ImageData {
            extent,
            origin,
            spacing,
            pieces,
            ..
        } => {
            let ranges = extent.clone().into_ranges();
            let range_columns = (0..3)
                .map(|a| {
                    let first = origin[a] as f64 + *ranges[a].start() as f64 * spacing[a] as f64;
                    let last = origin[a] as f64 + *ranges[a].end() as f64 * spacing[a] as f64;
                    Column::float(
                        ["xRange", "yRange", "zRange"][a],
                        vec![first.min(last), first.max(last)],
                    )
                })
                .collect();
            frames.push(NamedFrame::unnamed(super::frame(range_columns)?));
            let count = pieces.len();
            for (i, p) in pieces.into_iter().enumerate() {
                let p = p
                    .into_loaded_piece_data(Some(path))
                    .map_err(super::broken)?;
                let r = p.extent.into_ranges();
                let axes: [Vec<f64>; 3] = std::array::from_fn(|a| {
                    r[a].clone()
                        .map(|k| origin[a] as f64 + k as f64 * spacing[a] as f64)
                        .collect()
                });
                all_fields.extend(fields(&p.data.point, &axes, &prefix(i, count))?);
            }
        }
        DataSet::RectilinearGrid { pieces, .. } => {
            let count = pieces.len();
            for (i, p) in pieces.into_iter().enumerate() {
                let p = p
                    .into_loaded_piece_data(Some(path))
                    .map_err(super::broken)?;
                let axes = [
                    buffer(p.coords.x)?,
                    buffer(p.coords.y)?,
                    buffer(p.coords.z)?,
                ];
                all_fields.extend(fields(&p.data.point, &axes, &prefix(i, count))?);
                table(
                    &mut frames,
                    &prefix(i, count),
                    Some(grid_points(&axes)),
                    p.data,
                )?;
            }
        }
        DataSet::StructuredGrid { pieces, .. } => {
            let n = pieces.len();
            for (i, p) in pieces.into_iter().enumerate() {
                let p = p
                    .into_loaded_piece_data(Some(path))
                    .map_err(super::broken)?;
                table(&mut frames, &prefix(i, n), Some(buffer(p.points)?), p.data)?;
            }
        }
        DataSet::UnstructuredGrid { pieces, .. } => {
            let n = pieces.len();
            for (i, p) in pieces.into_iter().enumerate() {
                let p = p
                    .into_loaded_piece_data(Some(path))
                    .map_err(super::broken)?;
                table(&mut frames, &prefix(i, n), Some(buffer(p.points)?), p.data)?;
            }
        }
        DataSet::PolyData { pieces, .. } => {
            let n = pieces.len();
            for (i, p) in pieces.into_iter().enumerate() {
                let p = p
                    .into_loaded_piece_data(Some(path))
                    .map_err(super::broken)?;
                table(&mut frames, &prefix(i, n), Some(buffer(p.points)?), p.data)?;
            }
        }
        DataSet::Field { data_array, .. } => {
            for a in data_array {
                let n = a.num_comp();
                let v = buffer(a.data)?;
                frames.push(NamedFrame::new(
                    &a.name,
                    super::frame(components(&a.name, n, &v)?)?,
                ));
            }
        }
    }
    Ok(super::object(
        path,
        "VTK file",
        frames,
        if all_fields.is_empty() {
            None
        } else {
            Some(all_fields)
        },
        detail,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vtk_x_fast_order() {
        let attrs =
            vec![Attribute::scalars("a", 1).with_data(vec![1.0f64, 2.0, 3.0, 4.0, 5.0, 6.0])];
        let f = fields(&attrs, &[vec![0., 1.], vec![0., 1., 2.], vec![0.]], "").unwrap();
        assert_eq!(f[0].m, vec![1., 3., 5., 2., 4., 6.]);
    }
    #[test]
    fn lzma_image_data_preserves_coordinates_and_values() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/field_lzma.vti");
        let object = read(&path).unwrap();
        let fields = object.to_2d_fields().unwrap().unwrap();
        let field = fields.iter().find(|f| f.name == "Signal").unwrap();
        assert_eq!(field.x, [10., 20., 30.]);
        assert_eq!(field.y, [5., 9.]);
        assert_eq!(field.m, [1., 4., 2., 5., 3., 6.]);
    }
}
