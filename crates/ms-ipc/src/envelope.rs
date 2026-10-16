// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The JSON-RPC 2.0 envelope: requests, responses, error objects and notifications.
//!
//! Batches are not supported: an array is answered with one invalid-request error. A request
//! without an `id` is a client notification, which the engine accepts and never answers.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{code, JSONRPC_VERSION};

/// A request id. JSON-RPC also allows fractional numbers and null; neither is accepted.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Num(i64),
    Str(String),
}

impl From<i64> for Id {
    fn from(n: i64) -> Self {
        Id::Num(n)
    }
}

impl From<&str> for Id {
    fn from(s: &str) -> Self {
        Id::Str(s.to_owned())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    /// `None` for a client notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    pub method: String,
    /// `Null` when absent.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub params: Value,
}

impl Request {
    pub fn new(id: impl Into<Id>, method: &str, params: Value) -> Self {
        Request {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id: Some(id.into()),
            method: method.to_owned(),
            params,
        }
    }

    /// The params as `T`, or an invalid-params error naming what was wrong. Absent params
    /// read as `{}`.
    pub fn params_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, ErrorObject> {
        let params = match &self.params {
            Value::Null => Value::Object(Default::default()),
            p => p.clone(),
        };
        serde_json::from_value(params)
            .map_err(|e| ErrorObject::new(code::INVALID_PARAMS, format!("invalid params: {e}")))
    }
}

/// Exactly one of `result` and `error` is set. A `null` result reads back as `None`; no v1
/// method returns null.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    /// Null for a parse error, or an invalid request whose id could not be read.
    pub id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

impl Response {
    pub fn ok(id: Option<Id>, result: Value) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Option<Id>, error: ErrorObject) -> Self {
        Response {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }

    pub fn into_result(self) -> Result<Value, ErrorObject> {
        match (self.error, self.result) {
            (Some(e), _) => Err(e),
            (None, r) => Ok(r.unwrap_or(Value::Null)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorObject {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        ErrorObject {
            code,
            message: message.into(),
            data: None,
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl fmt::Display for ErrorObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ErrorObject {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Self {
        Notification {
            jsonrpc: JSONRPC_VERSION.to_owned(),
            method: method.to_owned(),
            params,
        }
    }
}

/// Anything a peer can send.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

/// Reads one line as a request, the server's view. The error carries the id to answer
/// with (null when it could not be read) and the error object.
pub fn parse_request(line: &[u8]) -> Result<Request, (Option<Id>, ErrorObject)> {
    let value: Value = serde_json::from_slice(line).map_err(|e| {
        (
            None,
            ErrorObject::new(code::PARSE_ERROR, format!("parse error: {e}")),
        )
    })?;
    let invalid = |id: Option<Id>, why: &str| {
        (
            id,
            ErrorObject::new(code::INVALID_REQUEST, format!("invalid request: {why}")),
        )
    };
    let Value::Object(obj) = &value else {
        return Err(invalid(None, "not an object (batches are not supported)"));
    };
    let id = match obj.get("id") {
        None => None,
        Some(v) => match serde_json::from_value::<Id>(v.clone()) {
            Ok(id) => Some(id),
            Err(_) => return Err(invalid(None, "id must be an integer or a string")),
        },
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some(JSONRPC_VERSION) {
        return Err(invalid(id, "jsonrpc must be \"2.0\""));
    }
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        return Err(invalid(id, "method must be a string"));
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    if !matches!(params, Value::Null | Value::Object(_) | Value::Array(_)) {
        return Err(invalid(id, "params must be an object or an array"));
    }
    Ok(Request {
        jsonrpc: JSONRPC_VERSION.to_owned(),
        id,
        method: method.to_owned(),
        params,
    })
}

/// Reads one line as any message, the client's view: `method` with `id` is a request,
/// `method` alone a notification, anything else a response.
pub fn parse_message(line: &[u8]) -> Result<Message, serde_json::Error> {
    let value: Value = serde_json::from_slice(line)?;
    let has_method = value.get("method").is_some();
    let has_id = value.get("id").is_some_and(|v| !v.is_null());
    Ok(match (has_method, has_id) {
        (true, true) => Message::Request(serde_json::from_value(value)?),
        (true, false) => Message::Notification(serde_json::from_value(value)?),
        (false, _) => Message::Response(serde_json::from_value(value)?),
    })
}

/// The `delta` notification for one line of today's stdout delta stream, without the
/// trailing newline: `params` is the delta object with `accountId` added. Splices the text
/// instead of re-serializing, since the engine sends one of these per flushed item and per
/// subscriber. `delta_line` must be a JSON object.
pub fn delta_notification_line(account_id: &str, delta_line: &str) -> String {
    let account = serde_json::to_string(account_id).unwrap_or_else(|_| "\"\"".to_owned());
    let rest = delta_line.trim().strip_prefix('{').unwrap_or("}");
    let sep = if rest.trim_start().starts_with('}') {
        ""
    } else {
        ","
    };
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"{}\",\"params\":{{\"accountId\":{account}{sep}{rest}}}",
        crate::method::DELTA
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_round_trip() {
        let r = Request::new(1, "hello", json!({"protocol": 1}));
        let text = serde_json::to_string(&r).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"protocol":1}}"#
        );
        assert_eq!(parse_request(text.as_bytes()).unwrap(), r);
        assert_eq!(parse_message(text.as_bytes()).unwrap(), Message::Request(r));

        let r = Request::new("abc", "engine.status", Value::Null);
        let text = serde_json::to_string(&r).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","id":"abc","method":"engine.status"}"#
        );
        assert_eq!(parse_request(text.as_bytes()).unwrap(), r);
    }

    #[test]
    fn response_round_trip() {
        let ok = Response::ok(Some(Id::Num(2)), json!({}));
        let text = serde_json::to_string(&ok).unwrap();
        assert_eq!(text, r#"{"jsonrpc":"2.0","id":2,"result":{}}"#);
        assert_eq!(
            parse_message(text.as_bytes()).unwrap(),
            Message::Response(ok)
        );

        let err = Response::err(
            None,
            ErrorObject::new(code::PARSE_ERROR, "parse error").with_data(json!([1])),
        );
        let text = serde_json::to_string(&err).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error","data":[1]}}"#
        );
        let back = parse_message(text.as_bytes()).unwrap();
        let Message::Response(back) = back else {
            panic!("{back:?}")
        };
        assert_eq!(back.into_result().unwrap_err().code, code::PARSE_ERROR);
    }

    #[test]
    fn notification_round_trip() {
        let n = Notification::new("disconnect", json!({"reason": "slow"}));
        let text = serde_json::to_string(&n).unwrap();
        assert_eq!(
            text,
            r#"{"jsonrpc":"2.0","method":"disconnect","params":{"reason":"slow"}}"#
        );
        assert_eq!(
            parse_message(text.as_bytes()).unwrap(),
            Message::Notification(n)
        );
    }

    #[test]
    fn request_errors() {
        let (id, e) = parse_request(b"garbage").unwrap_err();
        assert_eq!((id, e.code), (None, code::PARSE_ERROR));
        let (id, e) = parse_request(b"[1,2]").unwrap_err();
        assert_eq!((id, e.code), (None, code::INVALID_REQUEST));
        let (id, e) = parse_request(br#"{"jsonrpc":"2.0","id":1.5,"method":"x"}"#).unwrap_err();
        assert_eq!((id, e.code), (None, code::INVALID_REQUEST));
        let (id, e) = parse_request(br#"{"jsonrpc":"1.0","id":7,"method":"x"}"#).unwrap_err();
        assert_eq!((id, e.code), (Some(Id::Num(7)), code::INVALID_REQUEST));
        let (id, e) = parse_request(br#"{"jsonrpc":"2.0","id":"a","method":3}"#).unwrap_err();
        assert_eq!((id, e.code), (Some(Id::from("a")), code::INVALID_REQUEST));
        let (_, e) =
            parse_request(br#"{"jsonrpc":"2.0","id":1,"method":"x","params":3}"#).unwrap_err();
        assert_eq!(e.code, code::INVALID_REQUEST);

        let n = parse_request(br#"{"jsonrpc":"2.0","method":"sync.wake"}"#).unwrap();
        assert_eq!(n.id, None);
    }

    #[test]
    fn params_as_reports_invalid_params() {
        #[derive(Deserialize, Debug)]
        struct P {
            #[allow(dead_code)]
            x: u32,
        }
        let r = Request::new(1, "m", json!({"x": "no"}));
        assert_eq!(r.params_as::<P>().unwrap_err().code, code::INVALID_PARAMS);
        let r = Request::new(1, "m", Value::Null);
        assert_eq!(r.params_as::<P>().unwrap_err().code, code::INVALID_PARAMS);
    }

    #[test]
    fn delta_line_splice() {
        let line = r#"{"modelClass":"Thread","modelJSONs":[{"id":"t"}],"type":"persist"}"#;
        let n = delta_notification_line("a\"1", line);
        assert_eq!(
            n,
            r#"{"jsonrpc":"2.0","method":"delta","params":{"accountId":"a\"1","modelClass":"Thread","modelJSONs":[{"id":"t"}],"type":"persist"}}"#
        );
        let Message::Notification(parsed) = parse_message(n.as_bytes()).unwrap() else {
            panic!()
        };
        let params: crate::DeltaParams = serde_json::from_value(parsed.params).unwrap();
        assert_eq!(params.account_id, "a\"1");
        assert_eq!(params.model_class, "Thread");
        assert_eq!(params.kind, "persist");
        assert_eq!(params.model_jsons, vec![json!({"id": "t"})]);

        assert_eq!(
            delta_notification_line("a", "{}"),
            r#"{"jsonrpc":"2.0","method":"delta","params":{"accountId":"a"}}"#
        );
    }
}
