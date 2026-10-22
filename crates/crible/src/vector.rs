// SPDX-License-Identifier: Apache-2.0

//! Loading a query vector from a file, as `crible search --vector-file` wants.
//!
//! Two forms are accepted, and the file content decides which one:
//!
//! - a NumPy `.npy` array (`f4` or `f8`, little endian, C order), with shape
//!   `(dims,)` or `(1, dims)`;
//! - a JSON array of numbers, or an object carrying the array under `vector`.
//!
//! The parser is deliberately small: P1 only needs to read a vector the
//! reference tooling already wrote (`tools/reference_topk.py`), and no crate of
//! `docs/07-stack-et-deploiement.md` reads `.npy`. A malformed file is an
//! [`ErrorCode::InvalidRequest`] error naming the file and the fault; the
//! dimensionality and the finiteness of the values are checked later, by the
//! dense search, because only the index knows its width.

use std::fs;
use std::path::Path;

use crible_core::error::{CribleError, ErrorCode};

/// Magic bytes opening every NumPy `.npy` file.
const NPY_MAGIC: &[u8] = b"\x93NUMPY";

/// Loads a vector from `path`, an `.npy` file or a JSON file.
pub fn load(path: &Path) -> Result<Vec<f32>, CribleError> {
    let bytes = fs::read(path).map_err(|error| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!("cannot read the vector file {}: {error}", path.display()),
        )
    })?;
    if bytes.starts_with(NPY_MAGIC) {
        parse_npy(path, &bytes)
    } else {
        parse_json(path, &bytes)
    }
}

/// Parses a NumPy `.npy` array of `f4` or `f8`.
fn parse_npy(path: &Path, bytes: &[u8]) -> Result<Vec<f32>, CribleError> {
    let invalid = |message: String| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "{} is not a readable .npy vector: {message}",
                path.display()
            ),
        )
    };
    if bytes.len() < 10 {
        return Err(invalid(
            "the file is shorter than the .npy header".to_owned(),
        ));
    }
    let major = bytes[6];
    let header_start = match major {
        1 => 10,
        2 | 3 => 12,
        other => {
            return Err(invalid(format!("unsupported .npy format version {other}")));
        }
    };
    if bytes.len() < header_start {
        return Err(invalid(
            "the file is shorter than the .npy header".to_owned(),
        ));
    }
    let header_length = if major == 1 {
        usize::from(u16::from_le_bytes([bytes[8], bytes[9]]))
    } else {
        let raw: [u8; 4] = bytes[8..12]
            .try_into()
            .map_err(|_| invalid("the header length is truncated".to_owned()))?;
        usize::try_from(u32::from_le_bytes(raw))
            .map_err(|_| invalid("the header length does not fit in memory".to_owned()))?
    };
    let header_end = header_start
        .checked_add(header_length)
        .ok_or_else(|| invalid("the header length overflows".to_owned()))?;
    if bytes.len() < header_end {
        return Err(invalid(
            "the file is shorter than the .npy header".to_owned(),
        ));
    }
    let header = String::from_utf8_lossy(&bytes[header_start..header_end]);

    let descr = header_value(&header, "descr")
        .ok_or_else(|| invalid("the header has no 'descr' entry".to_owned()))?;
    let dtype = descr.trim_start_matches(['<', '|', '=']).to_owned();
    if descr.starts_with('>') {
        return Err(invalid(format!(
            "big-endian dtype '{descr}' is not supported; .npy vectors are little endian"
        )));
    }
    let item_size = match dtype.as_str() {
        "f4" => 4,
        "f8" => 8,
        other => {
            return Err(invalid(format!(
                "unsupported dtype '{other}'; expected f4 or f8"
            )));
        }
    };
    let fortran = header_bool(&header, "fortran_order")
        .ok_or_else(|| invalid("the header has no 'fortran_order' entry".to_owned()))?;
    if fortran {
        return Err(invalid(
            "Fortran-ordered arrays are not supported; write the vector in C order".to_owned(),
        ));
    }
    let shape = header_shape(&header)
        .ok_or_else(|| invalid("the header has no 'shape' entry".to_owned()))?;
    let count = match shape.as_slice() {
        [count] => *count,
        [1, count] => *count,
        other => {
            return Err(invalid(format!(
                "unsupported shape {other:?}; expected (dims,) or (1, dims)"
            )));
        }
    };

    let data = &bytes[header_end..];
    let expected = count
        .checked_mul(item_size)
        .ok_or_else(|| invalid("the vector size overflows".to_owned()))?;
    if data.len() != expected {
        return Err(invalid(format!(
            "the data holds {} bytes where the shape needs {expected}",
            data.len()
        )));
    }
    let mut vector = Vec::with_capacity(count);
    for index in 0..count {
        let start = index * item_size;
        let value = if item_size == 4 {
            let raw: [u8; 4] = data[start..start + 4]
                .try_into()
                .map_err(|_| invalid("a component is truncated".to_owned()))?;
            f32::from_le_bytes(raw)
        } else {
            let raw: [u8; 8] = data[start..start + 8]
                .try_into()
                .map_err(|_| invalid("a component is truncated".to_owned()))?;
            f64::from_le_bytes(raw) as f32
        };
        vector.push(value);
    }
    Ok(vector)
}

/// Reads a quoted string value of the Python-dict header.
///
/// The header is a Python literal, not JSON: its keys and values are single
/// quoted and a boolean is `True`/`False`. The keys of a `.npy` header are
/// constants, so a small scanner is enough and no parser dependency is added.
fn header_value(header: &str, key: &str) -> Option<String> {
    let position = header.find(key)?;
    let rest = &header[position + key.len()..];
    let colon = rest.find(':')?;
    let rest = &rest[colon + 1..];
    let quote = rest.find(['\'', '"'])?;
    let quote_character = rest[quote..].chars().next()?;
    let value = &rest[quote + 1..];
    let end = value.find(quote_character)?;
    Some(value[..end].to_owned())
}

/// Reads an unquoted boolean value of the Python-dict header.
fn header_bool(header: &str, key: &str) -> Option<bool> {
    let position = header.find(key)?;
    let rest = &header[position + key.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start_matches([' ', '\'']);
    if rest.starts_with("True") {
        Some(true)
    } else if rest.starts_with("False") {
        Some(false)
    } else {
        None
    }
}

/// Reads the shape tuple of the header, as a list of dimensions.
fn header_shape(header: &str) -> Option<Vec<usize>> {
    let position = header.find("shape")?;
    let rest = &header[position..];
    let open = rest.find('(')?;
    let rest = &rest[open + 1..];
    let close = rest.find(')')?;
    let inner = &rest[..close];
    let dimensions = inner
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.parse::<usize>().ok())
        .collect::<Option<Vec<_>>>()?;
    Some(dimensions)
}

/// Parses a JSON array of numbers, or an object whose `vector` member is one.
fn parse_json(path: &Path, bytes: &[u8]) -> Result<Vec<f32>, CribleError> {
    let invalid = |message: String| {
        CribleError::new(
            ErrorCode::InvalidRequest,
            format!(
                "{} is not a readable JSON vector: {message}",
                path.display()
            ),
        )
    };
    let text = std::str::from_utf8(bytes)
        .map_err(|error| invalid(format!("the file is not UTF-8: {error}")))?;
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| invalid(format!("invalid JSON: {error}")))?;
    let array = match &value {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(object) => object
            .get("vector")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                invalid(
                    "expected an array of numbers, or an object with a 'vector' array".to_owned(),
                )
            })?,
        _ => {
            return Err(invalid(
                "expected an array of numbers, or an object with a 'vector' array".to_owned(),
            ));
        }
    };
    array
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let number = item
                .as_f64()
                .ok_or_else(|| invalid(format!("component {index} is not a number")))?;
            Ok(number as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::load;

    fn write_npy(path: &std::path::Path, descr: &str, shape: &str, data: &[u8]) {
        let header = format!("{{'descr': '{descr}', 'fortran_order': False, 'shape': {shape}, }}");
        let mut padding = 0;
        while (10 + header.len() + padding) % 64 != 0 {
            padding += 1;
        }
        let mut padded = header;
        padded.push_str(&" ".repeat(padding));
        padded.push('\n');
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x93NUMPY");
        bytes.push(1);
        bytes.push(0);
        bytes.extend_from_slice(
            &u16::try_from(padded.len())
                .expect("the test header fits in u16")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(padded.as_bytes());
        bytes.extend_from_slice(data);
        fs::write(path, bytes).expect("the test vector is written");
    }

    #[test]
    fn reads_a_one_dimensional_f32_npy_file() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("vector.npy");
        let mut data = Vec::new();
        for value in [1.0_f32, -2.5, 0.25] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        write_npy(&path, "<f4", "(3,)", &data);
        assert_eq!(
            load(&path).expect("the vector is readable"),
            vec![1.0, -2.5, 0.25]
        );
    }

    #[test]
    fn reads_a_row_shaped_f64_npy_file() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let path = temp.path().join("vector.npy");
        let mut data = Vec::new();
        for value in [0.5_f64, 1.5] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        write_npy(&path, "<f8", "(1, 2)", &data);
        assert_eq!(load(&path).expect("the vector is readable"), vec![0.5, 1.5]);
    }

    #[test]
    fn reads_a_json_array_or_an_object_with_a_vector_member() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let array = temp.path().join("array.json");
        fs::write(&array, "[1, 2.5, -3]").expect("the test vector is written");
        assert_eq!(
            load(&array).expect("the vector is readable"),
            vec![1.0, 2.5, -3.0]
        );

        let object = temp.path().join("object.json");
        fs::write(&object, r#"{"vector": [0.25, 0.5]}"#).expect("the test vector is written");
        assert_eq!(
            load(&object).expect("the vector is readable"),
            vec![0.25, 0.5]
        );
    }

    #[test]
    fn refuses_a_big_endian_or_fortran_ordered_npy_file() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let big_endian = temp.path().join("big.npy");
        write_npy(&big_endian, ">f4", "(1,)", &[0, 0, 0, 0]);
        let error = load(&big_endian).expect_err("a big-endian vector is refused");
        assert!(
            error.message().contains("little endian"),
            "unexpected message: {}",
            error.message()
        );

        let fortran = temp.path().join("fortran.npy");
        let header = "{'descr': '<f4', 'fortran_order': True, 'shape': (1,), }";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x93NUMPY");
        bytes.push(1);
        bytes.push(0);
        bytes.extend_from_slice(
            &u16::try_from(header.len())
                .expect("the test header fits in u16")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        fs::write(&fortran, bytes).expect("the test vector is written");
        let error = load(&fortran).expect_err("a Fortran-ordered vector is refused");
        assert!(
            error.message().contains("Fortran"),
            "unexpected message: {}",
            error.message()
        );
    }
}
