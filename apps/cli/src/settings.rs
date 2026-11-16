// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `airpost settings get|set`: the settings the engine keeps,
//! on the one engine that announces them.

use airpost_ipc::{feature, method, SettingsGetParams, SettingsValues, SETTINGS_KEYS};
use serde_json::{json, Map, Value};

use crate::app::Ctx;
use crate::discovery::Engine;
use crate::error::{CliError, Result};
use crate::output;

fn settings_engine(engines: &mut [Engine]) -> Result<&mut Engine> {
    engines
        .iter_mut()
        .find(|e| e.has_feature(feature::SETTINGS))
        .ok_or_else(|| {
            CliError::failed(
                "the running engine doesn't keep settings; update Airpost (settings need its \
                 serve engine)",
            )
        })
}

fn call(engine: &mut Engine, method: &str, params: Value) -> Result<Value> {
    let pid = engine.entry.pid;
    engine
        .client
        .call(method, params)
        .map_err(|e| CliError::call(&format!("engine pid {pid}"), method, e))
}

/// The stored values of `keys` on the engine that keeps settings; a key without one is
/// absent.
pub(crate) fn read(engine: &mut Engine, keys: &[&str]) -> Result<Map<String, Value>> {
    let params = SettingsGetParams {
        keys: Some(keys.iter().map(|k| (*k).to_owned()).collect()),
    };
    let result = call(engine, method::SETTINGS_GET, json!(params))?;
    let values: SettingsValues = serde_json::from_value(result)
        .map_err(|e| CliError::failed(format!("odd settings.get result: {e}")))?;
    Ok(values.values)
}

/// Stores `values` (null clears a key) on the engine that keeps settings.
pub(crate) fn write(engine: &mut Engine, values: Map<String, Value>) -> Result<()> {
    call(
        engine,
        method::SETTINGS_SET,
        json!(SettingsValues {
            values,
            ..Default::default()
        }),
    )?;
    Ok(())
}

/// The engine that keeps settings and has `need`, by index.
pub(crate) fn engine_with(engines: &[Engine], need: &str, what: &str) -> Result<usize> {
    engines
        .iter()
        .position(|e| e.has_feature(feature::SETTINGS) && e.has_feature(need))
        .ok_or_else(|| {
            CliError::failed(format!(
                "the running engine can't {what} (no {need}); update Airpost (this needs its \
                 serve engine)"
            ))
        })
}

pub fn get(ctx: &Ctx, engines: &mut [Engine], keys: Vec<String>) -> Result<()> {
    for k in &keys {
        if airpost_ipc::setting_kind(k).is_none() {
            return Err(unknown_key(k));
        }
    }
    let engine = settings_engine(engines)?;
    let params = SettingsGetParams {
        keys: (!keys.is_empty()).then_some(keys),
    };
    let result = call(engine, method::SETTINGS_GET, json!(params))?;
    let values: SettingsValues = serde_json::from_value(result)
        .map_err(|e| CliError::failed(format!("odd settings.get result: {e}")))?;
    if ctx.json {
        // The values only, as `set` prints them; the store id is the engine's business.
        output::json(&json!({ "values": values.values }));
        return Ok(());
    }
    let mut keys: Vec<&String> = values.values.keys().collect();
    keys.sort();
    for k in keys {
        output::line(&format!("{k} = {}", values.values[k]));
    }
    Ok(())
}

pub fn set(ctx: &Ctx, engines: &mut [Engine], key: &str, value: &str) -> Result<()> {
    if airpost_ipc::setting_kind(key).is_none() {
        return Err(unknown_key(key));
    }
    let value = parse_value(value);
    if let Some(e) = airpost_ipc::setting_error(key, &value) {
        return Err(CliError::usage(e));
    }
    let engine = settings_engine(engines)?;
    let mut values = Map::new();
    values.insert(key.to_owned(), value);
    call(
        engine,
        method::SETTINGS_SET,
        json!(SettingsValues {
            values: values.clone(),
            ..Default::default()
        }),
    )?;
    if ctx.json {
        output::json(&json!({ "values": values }));
    } else {
        output::line(&format!("{key} = {}", values[key]));
    }
    Ok(())
}

/// JSON when it parses (`15000`, `true`, `null`, `{"a": 1}`), else the text as a string.
fn parse_value(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

fn unknown_key(key: &str) -> CliError {
    let known: Vec<&str> = SETTINGS_KEYS.iter().map(|(k, _)| *k).collect();
    CliError::usage(format!(
        "{key:?} is not a setting the engine keeps; it keeps {}",
        known.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_json_or_text() {
        assert_eq!(parse_value("15000"), json!(15000));
        assert_eq!(parse_value("false"), json!(false));
        assert_eq!(parse_value("null"), Value::Null);
        assert_eq!(parse_value("Back on top"), json!("Back on top"));
        assert_eq!(parse_value("\"5000\""), json!("5000"));
        assert_eq!(unknown_key("x").exit_code(), 2);
    }
}
