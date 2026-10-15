//! MATLAB level-5/v7.3 tables using the shared safe nested MAT decoder.
use crate::misc_formats::mat::{read_mat, MatValue};
use dv_core::{Column, Field2D, FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};

fn project(
    name: &str,
    value: MatValue,
    frames: &mut Vec<NamedFrame>,
    fields: &mut Vec<Field2D>,
) -> Result<(), IoError> {
    match value {
        MatValue::Numeric {
            dimensions,
            real,
            imaginary,
        } => {
            if dimensions.len() > 2 {
                return Err(IoError::Other(format!("MATLAB array `{name}` is {}D; tabular conversion supports at most two dimensions",dimensions.len())));
            }
            let rows = dimensions.first().copied().unwrap_or(1);
            let cols = dimensions.get(1).copied().unwrap_or(1);
            let mut columns = Vec::new();
            for j in 0..cols {
                let begin = j
                    .checked_mul(rows)
                    .ok_or_else(|| super::broken("MAT shape overflow"))?;
                let end = begin
                    .checked_add(rows)
                    .ok_or_else(|| super::broken("MAT shape overflow"))?;
                let values = real
                    .get(begin..end)
                    .ok_or_else(|| super::broken("MAT shape does not match array data"))?
                    .to_vec();
                columns.push(Column::float(
                    if imaginary.is_some() {
                        format!("C{j}_real")
                    } else {
                        format!("C{j}")
                    },
                    values,
                ));
                if let Some(imag) = &imaginary {
                    columns.push(Column::float(
                        format!("C{j}_imag"),
                        imag.get(begin..end)
                            .ok_or_else(|| super::broken("MAT imaginary shape mismatch"))?
                            .to_vec(),
                    ));
                }
            }
            frames.push(NamedFrame::new(name, super::frame(columns)?));
            for (suffix, data) in
                std::iter::once((if imaginary.is_some() { "_real" } else { "" }, &real))
                    .chain(imaginary.as_ref().map(|v| ("_imag", v)))
            {
                let count = rows
                    .checked_mul(cols)
                    .ok_or_else(|| super::broken("MAT field size overflow"))?;
                if data.len() != count {
                    return Err(super::broken("MAT component shape mismatch"));
                }
                let mut m = Vec::with_capacity(count);
                for i in 0..rows {
                    for j in 0..cols {
                        m.push(data[j * rows + i]);
                    }
                }
                fields.push(Field2D {
                    name: format!("{name}{suffix}"),
                    x: (0..rows).map(|i| i as f64).collect(),
                    y: (0..cols).map(|i| i as f64).collect(),
                    sx: "rows".into(),
                    sy: "columns".into(),
                    m,
                });
            }
        }
        MatValue::Text(text) => frames.push(NamedFrame::new(
            name,
            super::frame(vec![Column::string(name, vec![text])])?,
        )),
        MatValue::Cell(values) => {
            for (i, value) in values.into_iter().enumerate() {
                project(&format!("{name}[{i}]"), value, frames, fields)?;
            }
        }
        MatValue::Struct {
            fields: names,
            elements,
        } => {
            let single = elements.len() == 1;
            for (i, element) in elements.into_iter().enumerate() {
                if names.len() != element.len() {
                    return Err(super::broken("MAT structure field count mismatch"));
                }
                for (field, value) in names.iter().zip(element) {
                    project(
                        &if single {
                            format!("{name}.{field}")
                        } else {
                            format!("{name}[{i}].{field}")
                        },
                        value,
                        frames,
                        fields,
                    )?;
                }
            }
        }
    }
    Ok(())
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let mat = read_mat(path)?;
    if mat.contains_key("DataCell") {
        return Err(IoError::WrongFormat(
            "RAAW DataCell requires the specialized RAAW reader".into(),
        ));
    }
    let mut frames = Vec::new();
    let mut fields = Vec::new();
    for (name, value) in mat {
        project(&name, value, &mut frames, &mut fields)?;
    }
    if frames.is_empty() {
        return Err(IoError::Other("MAT file contains no tabular values".into()));
    }
    Ok(super::object(path,"Matlab mat file",frames,if fields.is_empty() {None} else {Some(fields)},"Level-5/v7.3 numeric/text arrays and nested cells/structs; complex components exposed separately".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matlab73_userblock_preserves_reference_cells_and_structure() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mat73_refs.mat");
        let object = read(&path).unwrap();
        let frames = object.to_dataframes().unwrap();
        let numbers = frames.iter().find(|f| f.name == "cells[0]").unwrap();
        assert_eq!(numbers.df.columns()[0].floats().unwrap(), [2., 4., 6.]);
        let text = frames.iter().find(|f| f.name == "cells[1]").unwrap();
        assert_eq!(
            text.df.columns()[0].data,
            dv_core::ColumnData::Str(vec!["Rust".into()])
        );
        let structure = frames
            .iter()
            .find(|f| f.name == "structure.signal")
            .unwrap();
        assert_eq!(structure.df.columns()[0].floats().unwrap(), [2., 4., 6.]);
    }
}
