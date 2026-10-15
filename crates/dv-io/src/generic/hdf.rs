//! Safe shared HDF5 decoding for NetCDF-4 and MATLAB v7.3.
use dv_core::IoError;
use hdf5_reader::{
    storage::{FileStorage, Storage, StorageBuffer},
    Hdf5File,
};
use hdf5_reader::{Dataset, Datatype, H5Type};
use std::{path::Path, sync::Arc};

struct UserblockStorage {
    file: FileStorage,
    start: u64,
}
impl Storage for UserblockStorage {
    fn len(&self) -> u64 {
        self.file.len() - self.start
    }
    fn read_range(&self, offset: u64, len: usize) -> hdf5_reader::error::Result<StorageBuffer> {
        let absolute = offset
            .checked_add(self.start)
            .ok_or(hdf5_reader::error::Error::OffsetOutOfBounds(offset))?;
        self.file.read_range(absolute, len)
    }
}

/// HDF5 offsets are relative to the base address, not to a MATLAB userblock.
/// hdf5-reader 0.9.1 locates the signature but does not relocate object addresses.
pub(crate) fn open(path: &Path) -> Result<Hdf5File, IoError> {
    let file = FileStorage::open(path).map_err(super::broken)?;
    let mut start = 0u64;
    loop {
        if start.checked_add(8).is_none_or(|end| end > file.len()) {
            return Err(super::broken("HDF5 signature not found"));
        }
        if file.read_range(start, 8).map_err(super::broken)?.as_ref() == b"\x89HDF\r\n\x1a\n" {
            break;
        }
        start = if start == 0 {
            512
        } else {
            start
                .checked_mul(2)
                .ok_or_else(|| super::broken("HDF5 userblock offset overflow"))?
        };
    }
    Hdf5File::from_storage(Arc::new(UserblockStorage { file, start })).map_err(super::broken)
}

pub(crate) fn number(bytes: &[u8], dtype: &Datatype) -> Result<f64, IoError> {
    macro_rules! value {
        ($t:ty) => {
            <$t as H5Type>::from_bytes(bytes, dtype)
                .map(|v| v as f64)
                .map_err(super::broken)
        };
    }
    match dtype {
        Datatype::FixedPoint {
            size: 1,
            signed: true,
            ..
        } => value!(i8),
        Datatype::FixedPoint {
            size: 1,
            signed: false,
            ..
        } => value!(u8),
        Datatype::FixedPoint {
            size: 2,
            signed: true,
            ..
        } => value!(i16),
        Datatype::FixedPoint {
            size: 2,
            signed: false,
            ..
        } => value!(u16),
        Datatype::FixedPoint {
            size: 4,
            signed: true,
            ..
        } => value!(i32),
        Datatype::FixedPoint {
            size: 4,
            signed: false,
            ..
        } => value!(u32),
        Datatype::FixedPoint {
            size: 8,
            signed: true,
            ..
        } => value!(i64),
        Datatype::FixedPoint {
            size: 8,
            signed: false,
            ..
        } => value!(u64),
        Datatype::FloatingPoint { size: 4, .. } => value!(f32),
        Datatype::FloatingPoint { size: 8, .. } => value!(f64),
        Datatype::Enum { base, .. } => number(bytes, base),
        other => Err(IoError::Other(format!(
            "HDF5 datatype {other:?} cannot be projected to a numeric column"
        ))),
    }
}

pub(crate) fn numbers(dataset: &Dataset) -> Result<Vec<f64>, IoError> {
    let width = hdf5_reader::dtype_element_size(dataset.dtype()).map_err(super::broken)?;
    let bytes = dataset.read_raw_bytes().map_err(super::broken)?;
    if width == 0 || bytes.len() % width != 0 {
        return Err(super::broken("invalid HDF5 numeric payload size"));
    }
    bytes
        .chunks_exact(width)
        .map(|b| number(b, dataset.dtype()))
        .collect()
}

pub(crate) fn shape(dataset: &Dataset) -> Result<Vec<usize>, IoError> {
    dataset
        .shape()
        .iter()
        .map(|&v| usize::try_from(v).map_err(super::broken))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hdf5_reader::ByteOrder;
    #[test]
    fn numeric_byte_order_and_signedness() {
        let signed = Datatype::FixedPoint {
            size: 2,
            signed: true,
            byte_order: ByteOrder::BigEndian,
        };
        assert_eq!(number(&(-123i16).to_be_bytes(), &signed).unwrap(), -123.);
        let float = Datatype::FloatingPoint {
            size: 8,
            byte_order: ByteOrder::LittleEndian,
        };
        assert_eq!(number(&1.25f64.to_le_bytes(), &float).unwrap(), 1.25);
        assert!(number(&[0], &float).is_err());
    }
}
