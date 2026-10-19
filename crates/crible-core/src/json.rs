// SPDX-License-Identifier: Apache-2.0

//! Small helpers for reading JSON values and reporting `invalid_request`.
//!
//! Every message names the JSON path of the faulty value, so a client can
//! locate the mistake without reading the engine source.

use serde_json::{Map, Value};

use crate::error::{CribleError, ErrorCode};

/// Builds an `invalid_request` error carrying `message`.
pub(crate) fn invalid(message: impl Into<String>) -> CribleError {
    CribleError::new(ErrorCode::InvalidRequest, message)
}

/// Name of the JSON type of `value`, for error messages.
pub(crate) fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Borrows `value` as an object, or fails naming `path`.
pub(crate) fn object<'a>(
    value: &'a Value,
    path: &str,
) -> Result<&'a Map<String, Value>, CribleError> {
    value.as_object().ok_or_else(|| {
        invalid(format!(
            "expected an object at {path}, got {}",
            json_type(value)
        ))
    })
}

/// Returns the required member `key` of `object`, or fails naming `path`.
pub(crate) fn member<'a>(
    object: &'a Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<&'a Value, CribleError> {
    object
        .get(key)
        .ok_or_else(|| invalid(format!("missing required member '{key}' at {path}")))
}

/// Reads a required string member.
pub(crate) fn string_member(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<String, CribleError> {
    string(member(object, path, key)?, &format!("{path}.{key}"))
}

/// Reads a string value.
pub(crate) fn string(value: &Value, path: &str) -> Result<String, CribleError> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        invalid(format!(
            "expected a string at {path}, got {}",
            json_type(value)
        ))
    })
}

/// Reads a required boolean member.
pub(crate) fn boolean_member(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<bool, CribleError> {
    boolean(member(object, path, key)?, &format!("{path}.{key}"))
}

/// Reads a boolean value.
pub(crate) fn boolean(value: &Value, path: &str) -> Result<bool, CribleError> {
    value.as_bool().ok_or_else(|| {
        invalid(format!(
            "expected a boolean at {path}, got {}",
            json_type(value)
        ))
    })
}

/// Reads a non-negative integer value.
pub(crate) fn unsigned(value: &Value, path: &str) -> Result<u64, CribleError> {
    value.as_u64().ok_or_else(|| {
        invalid(format!(
            "expected a non-negative integer at {path}, got {}",
            json_type(value)
        ))
    })
}

/// Reads a `u32` value.
pub(crate) fn u32_value(value: &Value, path: &str) -> Result<u32, CribleError> {
    let number = unsigned(value, path)?;
    u32::try_from(number).map_err(|_| {
        invalid(format!(
            "value at {path} is above the maximum of {}",
            u32::MAX
        ))
    })
}

/// Reads a required `u32` member.
pub(crate) fn u32_member(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<u32, CribleError> {
    u32_value(member(object, path, key)?, &format!("{path}.{key}"))
}

/// Reads a required number member.
pub(crate) fn float_member(
    object: &Map<String, Value>,
    path: &str,
    key: &str,
) -> Result<f64, CribleError> {
    float(member(object, path, key)?, &format!("{path}.{key}"))
}

/// Reads a number value.
pub(crate) fn float(value: &Value, path: &str) -> Result<f64, CribleError> {
    value.as_f64().ok_or_else(|| {
        invalid(format!(
            "expected a number at {path}, got {}",
            json_type(value)
        ))
    })
}

/// Rejects any member of `object` whose key is not in `allowed`.
pub(crate) fn reject_unknown_keys(
    object: &Map<String, Value>,
    path: &str,
    allowed: &[&str],
) -> Result<(), CribleError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(format!("unknown setting '{key}' at {path}.{key}")));
        }
    }
    Ok(())
}
