// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Typed reads of task JSON with nlohmann's failure behaviour, as Mailspring-Sync's
//! `TaskProcessor.cpp` saw it (Copyright © 2017 Foundry 376, GPL-3.0; see NOTICE.md).
//!
//! The C++ read task fields with `data["key"].get<T>()`, which throws `json::type_error` on
//! a missing or mistyped field; `performLocal` / `performRemote` turned that into an
//! `unhandled-exception` error on the task (`errorJSONForUnexpectedException`). These
//! helpers return the same `what` text as an [`Error::Other`], which `Error::to_json` maps
//! to that shape.

use ms_core::{Error, Result};
use serde_json::Value;

static NULL: Value = Value::Null;

/// nlohmann's `type_name()`.
fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_error(expected: &str, v: &Value) -> Error {
    Error::other(format!(
        "[json.exception.type_error.302] type must be {expected}, but is {}",
        type_name(v)
    ))
}

/// `data[key]` for reading: null when absent or when `data` is not an object, as a
/// non-const nlohmann `operator[]` on null yields.
pub(crate) fn at<'a>(data: &'a Value, key: &str) -> &'a Value {
    data.get(key).unwrap_or(&NULL)
}

/// `data.count(key)`.
pub(crate) fn has(data: &Value, key: &str) -> bool {
    data.get(key).is_some()
}

pub(crate) fn get_str(v: &Value) -> Result<String> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| type_error("string", v))
}

pub(crate) fn get_bool(v: &Value) -> Result<bool> {
    v.as_bool().ok_or_else(|| type_error("boolean", v))
}

/// `get<int>()` and friends: any number (truncated) or a boolean.
pub(crate) fn get_i64(v: &Value) -> Result<i64> {
    match v {
        Value::Number(n) => Ok(n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u as i64))
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0)),
        Value::Bool(b) => Ok(*b as i64),
        other => Err(type_error("number", other)),
    }
}

/// `v.at(0)`: the first element of an array.
pub(crate) fn first_element(v: &Value) -> Result<&Value> {
    match v {
        Value::Array(a) => a.first().ok_or_else(|| {
            Error::other("[json.exception.out_of_range.401] array index 0 is out of range")
        }),
        other => Err(Error::other(format!(
            "[json.exception.type_error.304] cannot use at() with {}",
            type_name(other)
        ))),
    }
}

/// What a range-for over a nlohmann value visits: array elements, object values, nothing
/// for null, and a scalar once.
pub(crate) fn elements(v: &Value) -> Vec<&Value> {
    match v {
        Value::Null => Vec::new(),
        Value::Array(a) => a.iter().collect(),
        Value::Object(o) => o.values().collect(),
        scalar => vec![scalar],
    }
}

/// `for (auto & m : v) ids.push_back(m.get<string>())`.
pub(crate) fn str_elements(v: &Value) -> Result<Vec<String>> {
    elements(v).into_iter().map(get_str).collect()
}

/// `vector<string> ids = v`: an array of strings.
pub(crate) fn str_vec(v: &Value) -> Result<Vec<String>> {
    match v {
        Value::Array(a) => a.iter().map(get_str).collect(),
        other => Err(type_error("array", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_read_like_nlohmann() {
        let data = json!({"unread": null, "n": 2.0, "ids": ["a", "b"]});
        assert_eq!(
            get_bool(at(&data, "unread")).unwrap_err().to_string(),
            "[json.exception.type_error.302] type must be boolean, but is null"
        );
        assert_eq!(
            get_str(at(&data, "missing")).unwrap_err().to_string(),
            "[json.exception.type_error.302] type must be string, but is null"
        );
        assert_eq!(get_i64(at(&data, "n")).unwrap(), 2);
        assert_eq!(str_vec(at(&data, "ids")).unwrap(), ["a", "b"]);
        assert!(str_vec(at(&data, "missing")).is_err());
        assert!(str_elements(at(&data, "missing")).unwrap().is_empty());
        let e = Error::other("x");
        assert_eq!(e.to_json()["key"], "unhandled-exception");
    }
}
