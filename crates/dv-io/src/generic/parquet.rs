//! Arrow-backed Parquet IO. Nested, binary and decimal columns are rejected.
use ::parquet::arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter};
use arrow_array::*;
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use dv_core::{Column, ColumnData, DataFrame, FileObject, IoError, NamedFrame};
use std::{fs::File, path::Path, sync::Arc};

fn column(name: &str, a: &dyn Array) -> Result<Column, IoError> {
    macro_rules! number {
        ($ty:ty) => {{
            let a = a
                .as_any()
                .downcast_ref::<$ty>()
                .ok_or_else(|| super::broken("Arrow type mismatch"))?;
            Column::float(
                name,
                (0..a.len())
                    .map(|i| {
                        if a.is_null(i) {
                            f64::NAN
                        } else {
                            a.value(i) as f64
                        }
                    })
                    .collect(),
            )
        }};
    }
    macro_rules! dates {
        ($ty:ty, $scale:expr) => {{
            let a = a
                .as_any()
                .downcast_ref::<$ty>()
                .ok_or_else(|| super::broken("Arrow timestamp type mismatch"))?;
            Column::datetime(
                name,
                (0..a.len())
                    .map(|i| {
                        if a.is_null(i) {
                            f64::NAN
                        } else {
                            a.value(i) as f64 * $scale
                        }
                    })
                    .collect(),
            )
        }};
    }
    Ok(match a.data_type() {
        DataType::Null => Column::float(name, vec![f64::NAN; a.len()]),
        DataType::Int8 => number!(Int8Array), DataType::Int16 => number!(Int16Array), DataType::Int32 => number!(Int32Array), DataType::Int64 => number!(Int64Array),
        DataType::UInt8 => number!(UInt8Array), DataType::UInt16 => number!(UInt16Array), DataType::UInt32 => number!(UInt32Array), DataType::UInt64 => number!(UInt64Array),
        DataType::Float32 => number!(Float32Array), DataType::Float64 => number!(Float64Array),
        DataType::Boolean => {
            let a = a.as_any().downcast_ref::<BooleanArray>().ok_or_else(|| super::broken("Arrow bool mismatch"))?;
            Column::float(name, (0..a.len()).map(|i| if a.is_null(i) { f64::NAN } else { f64::from(u8::from(a.value(i))) }).collect())
        },
        DataType::Utf8 => {
            let a = a.as_any().downcast_ref::<StringArray>().ok_or_else(|| super::broken("Arrow string mismatch"))?;
            Column::string(name, (0..a.len()).map(|i| if a.is_null(i) { String::new() } else { a.value(i).into() }).collect())
        },
        DataType::LargeUtf8 => {
            let a = a.as_any().downcast_ref::<LargeStringArray>().ok_or_else(|| super::broken("Arrow string mismatch"))?;
            Column::string(name, (0..a.len()).map(|i| if a.is_null(i) { String::new() } else { a.value(i).into() }).collect())
        },
        DataType::Timestamp(TimeUnit::Second, _) => dates!(TimestampSecondArray, 1.0),
        DataType::Timestamp(TimeUnit::Millisecond, _) => dates!(TimestampMillisecondArray, 1e-3),
        DataType::Timestamp(TimeUnit::Microsecond, _) => dates!(TimestampMicrosecondArray, 1e-6),
        DataType::Timestamp(TimeUnit::Nanosecond, _) => dates!(TimestampNanosecondArray, 1e-9),
        DataType::Date32 => dates!(Date32Array, 86400.0), DataType::Date64 => dates!(Date64Array, 1e-3),
        other => return Err(IoError::Other(format!("Parquet column `{name}` has unsupported Arrow type {other:?}; dv-core supports numeric, string and datetime columns"))),
    })
}

pub fn read(path: &Path) -> Result<Arc<dyn FileObject>, IoError> {
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(File::open(path)?).map_err(super::broken)?;
    let schema = builder.schema().clone();
    let empty = RecordBatch::new_empty(schema.clone());
    let mut columns = empty
        .columns()
        .iter()
        .zip(schema.fields())
        .map(|(a, f)| column(f.name(), a.as_ref()))
        .collect::<Result<Vec<_>, _>>()?;
    let reader = builder.build().map_err(super::broken)?;
    for batch in reader {
        let batch = batch.map_err(super::broken)?;
        for (c, a) in columns.iter_mut().zip(batch.columns()) {
            c.data.extend_from(&column(&c.name, a.as_ref())?.data);
        }
    }
    Ok(super::object(
        path,
        "Parquet file",
        vec![NamedFrame::unnamed(super::frame(columns)?)],
        None,
        "Arrow schema retained for supported scalar columns".into(),
    ))
}

pub fn write(df: &DataFrame, path: &Path) -> Result<(), IoError> {
    let mut fields = Vec::with_capacity(df.ncols());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(df.ncols());
    for c in df.columns() {
        let (kind, array): (DataType, ArrayRef) = match &c.data {
            ColumnData::Float(v) => (
                DataType::Float64,
                Arc::new(Float64Array::from_iter(v.iter().map(|&x| {
                    if x.is_nan() {
                        None
                    } else {
                        Some(x)
                    }
                }))),
            ),
            ColumnData::Str(v) => (
                DataType::Utf8,
                Arc::new(StringArray::from_iter_values(v.iter().map(String::as_str))),
            ),
            ColumnData::DateTime(v) => {
                let values = v
                    .iter()
                    .map(|&x| {
                        if !x.is_finite() {
                            return Ok(None);
                        }
                        let t = x * 1e6;
                        if t < i64::MIN as f64 || t >= i64::MAX as f64 {
                            return Err(IoError::Other(format!(
                                "datetime in column {} is outside Parquet microsecond range",
                                c.name
                            )));
                        }
                        Ok(Some(t.round() as i64))
                    })
                    .collect::<Result<Vec<_>, IoError>>()?;
                (
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    Arc::new(TimestampMicrosecondArray::from(values)),
                )
            }
        };
        fields.push(Field::new(&c.name, kind, true));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays).map_err(super::broken)?;
    let mut writer =
        ArrowWriter::try_new(File::create(path)?, schema, None).map_err(super::broken)?;
    writer.write(&batch).map_err(super::broken)?;
    writer.close().map_err(super::broken)?;
    Ok(())
}
