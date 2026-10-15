use calamine::{open_workbook_auto, Data, DataType, Reader};
use dv_core::{Column, FileObject, IoError, NamedFrame};
use std::{path::Path, sync::Arc};

fn empty(v: &Data) -> bool {
    matches!(v, Data::Empty | Data::Error(_))
}
fn cell_text(v: &Data) -> String {
    match v {
        Data::Empty | Data::Error(_) => String::new(),
        Data::Bool(b) => {
            if *b {
                "True".into()
            } else {
                "False".into()
            }
        }
        _ => v.to_string(),
    }
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let mut book = open_workbook_auto(path).map_err(super::broken)?;
    let mut frames = Vec::new();
    for name in book.sheet_names().to_vec() {
        let range = book.worksheet_range(&name).map_err(super::broken)?;
        let rows: Vec<&[Data]> = range
            .rows()
            .filter(|r| r.iter().any(|v| !empty(v)))
            .collect();
        if rows.is_empty() {
            continue;
        }
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let indices: Vec<usize> = (0..width)
            .filter(|&i| rows.iter().any(|r| r.get(i).is_some_and(|v| !empty(v))))
            .collect();
        let mut columns = Vec::with_capacity(indices.len());
        for i in indices {
            let title = rows[0]
                .get(i)
                .map(cell_text)
                .unwrap_or_else(|| "nan".into());
            let values: Vec<&Data> = rows[1..]
                .iter()
                .map(|r| r.get(i).unwrap_or(&Data::Empty))
                .collect();
            if values
                .iter()
                .any(|v| v.is_datetime() || v.is_datetime_iso())
                && values.iter().all(|v| empty(v) || v.as_datetime().is_some())
            {
                columns.push(Column::datetime(
                    title,
                    values
                        .iter()
                        .map(|v| {
                            v.as_datetime()
                                .map(|dt| dv_core::dataframe::datetime_to_seconds(&dt))
                                .unwrap_or(f64::NAN)
                        })
                        .collect(),
                ));
            } else if values
                .iter()
                .all(|v| empty(v) || matches!(v, Data::Int(_) | Data::Float(_) | Data::Bool(_)))
            {
                columns.push(Column::float(
                    title,
                    values
                        .iter()
                        .map(|v| match v {
                            Data::Bool(b) => f64::from(u8::from(*b)),
                            _ => v.as_f64().unwrap_or(f64::NAN),
                        })
                        .collect(),
                ));
            } else {
                columns.push(Column::string(
                    title,
                    values.iter().map(|v| cell_text(v)).collect(),
                ));
            }
        }
        frames.push(NamedFrame::new(name, super::frame(columns)?));
    }
    if frames.len() == 1 {
        frames[0].name.clear();
    }
    Ok(super::object(
        path,
        "Excel file",
        frames,
        None,
        "All nonempty worksheets; first nonempty row supplies column names".into(),
    ))
}
