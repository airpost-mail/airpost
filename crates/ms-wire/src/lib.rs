// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! Wire types shared by `tern-backend` and the engine's HTTP metadata backend
//! (docs/backend-plan.md 3.1, 3.4, 3.5, 4.1).
//!
//! MIT, written from scratch: depends on serde and serde_json only, and on no engine crate.
//! The metadata protocol keeps the field names the C++ engine speaks: push bodies are
//! camelCase, page and stream records snake_case except `headerMessageId`. The Tern
//! endpoints (`/tern/v1/...`) and the engine's identity line are camelCase.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The stream sends this many `\n` bytes every [`HEARTBEAT_INTERVAL_SECS`].
pub const HEARTBEAT_BYTES: usize = 16;
pub const HEARTBEAT_INTERVAL_SECS: u64 = 5;
/// Clients drop a stream that delivers less than [`LOW_SPEED_LIMIT_BYTES_PER_SEC`] over this
/// window (curl's low-speed abort in C++, the per-read timeout in Rust).
pub const LOW_SPEED_WINDOW_SECS: u64 = 30;
pub const LOW_SPEED_LIMIT_BYTES_PER_SEC: u64 = 1;
/// Page size for `GET /metadata/<aid>`, stream replay batches and backfill bodies.
pub const PAGE_SIZE: usize = 500;
pub const BACKFILL_MAX_RECORDS: usize = PAGE_SIZE;
/// C++ reads `v` with `get<uint32_t>()`, so the server clamps to this.
pub const V_MAX: u32 = u32::MAX;
/// Largest serialized `value` a push may carry; larger is 413.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
/// Open streams per `(user, aid)` before the server answers 429.
pub const MAX_STREAMS_PER_ACCOUNT: usize = 4;
/// Server-side limit on accounts per user.
pub const MAX_ACCOUNTS_PER_USER: usize = 50;
/// The server ends a stream cleanly after this long; clients reconnect with their cursor.
pub const STREAM_MAX_SECS: u64 = 3600;
/// Body limit for `POST /tern/v1/backfill/<aid>`. Each record's `value` is still capped at
/// [`MAX_VALUE_BYTES`]; a client splits batches that would exceed this.
pub const BACKFILL_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
/// Claims are kept this long, then purged (3.8).
pub const CLAIM_RETENTION_DAYS: i64 = 30;

/// Model classes that carry plugin metadata. Anything else is rejected on push, because the
/// C++ engine aborts on an unknown class in the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectType {
    Message,
    Thread,
    Contact,
}

impl ObjectType {
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectType::Message => "message",
            ObjectType::Thread => "thread",
            ObjectType::Contact => "contact",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "message" => Some(ObjectType::Message),
            "thread" => Some(ObjectType::Thread),
            "contact" => Some(ObjectType::Contact),
            _ => None,
        }
    }
}

/// Body of `POST /metadata/<aid>/<objectId>/<pluginId>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPush {
    pub object_type: ObjectType,
    /// `null` (C++) and `0` (Rust) both mean the pushing device does not know the version.
    #[serde(default)]
    pub version: Option<u64>,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_message_id: Option<String>,
}

/// Response to a push. Both engines ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResponse {
    pub v: u32,
}

/// One element of the `POST /tern/v1/backfill/<aid>` body (at most [`BACKFILL_MAX_RECORDS`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillRecord {
    pub object_type: ObjectType,
    pub object_id: String,
    pub plugin_id: String,
    #[serde(default)]
    pub version: Option<u64>,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_message_id: Option<String>,
}

/// Response to a backfill: how many records changed a row (inserted, or `v` raised so the
/// device adopts the server's value from the stream), and the indices of the records skipped
/// as invalid (a value over [`MAX_VALUE_BYTES`], bad ids, an unknown object type). A client
/// drops rejected records instead of retrying them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillResponse {
    pub written: usize,
    #[serde(default)]
    pub rejected: Vec<usize>,
}

/// A metadata row as sent in pages and in stream lines (`attributes`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataRecord {
    pub object_type: ObjectType,
    pub object_id: String,
    pub aid: String,
    pub plugin_id: String,
    pub v: u32,
    pub value: Value,
    #[serde(
        rename = "headerMessageId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub header_message_id: Option<String>,
}

/// `GET /deltas/<aid>/head`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadResponse {
    /// Decimal seq. Always a JSON string.
    pub cursor: String,
}

/// Events the server sends on the stream besides metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendEvent {
    #[serde(rename = "identity-refresh-needed")]
    IdentityRefreshNeeded,
}

/// One NDJSON line of `GET /deltas/<aid>/streaming`. Heartbeat lines (bare `\n`) are not
/// represented; readers skip lines of 0 or 1 bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "object")]
pub enum StreamLine {
    #[serde(rename = "metadata")]
    Metadata {
        attributes: MetadataRecord,
        /// Must be a JSON string: C++ throws on a number.
        cursor: String,
    },
    #[serde(rename = "backend-event")]
    BackendEvent { event: BackendEvent },
}

/// Server features, advertised by `/tern/v1/server` and `/tern/v1/me`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Features {
    #[serde(default)]
    pub tracking: bool,
    #[serde(default)]
    pub sharing: bool,
}

/// `signup` in [`ServerInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignupMode {
    Open,
    Closed,
}

/// `GET /tern/v1/server`, no auth. The desktop checks a URL is a Tern server with it before
/// opening the login window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    /// Random per server instance; rotated by `backup` and `rotate-server-id`.
    pub server_id: String,
    pub version: String,
    #[serde(rename = "publicURL")]
    pub public_url: String,
    pub signup: SignupMode,
    #[serde(default)]
    pub features: Features,
}

/// `GET /tern/v1/me`. The engine binds its cursors to `(server_id, user_id)` from here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub user_id: String,
    pub email: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    /// Scopes the user's tracking URLs (3.6).
    #[serde(rename = "trackingNS")]
    pub tracking_ns: String,
    pub server_id: String,
    #[serde(rename = "publicURL")]
    pub public_url: String,
    #[serde(default)]
    pub features: Features,
}

impl Me {
    /// What the engine stores as `metadata-binding-<aid>`.
    pub fn binding(&self) -> String {
        binding(&self.server_id, &self.user_id)
    }
}

/// `"<serverId>:<userId>"`, the pair an engine's cursors are bound to.
pub fn binding(server_id: &str, user_id: &str) -> String {
    format!("{server_id}:{user_id}")
}

pub const LOGIN_RESULT_TYPE: &str = "tern-login";
pub const LOGIN_RESULT_VERSION: u32 = 1;
/// Id of the hidden element on the login result page that holds base64url([`LoginResult`]).
pub const LOGIN_RESULT_ELEMENT_ID: &str = "tern-login-result";

/// The JSON in `#tern-login-result` after a successful `POST /tern/v1/login`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: u32,
    pub token: String,
    pub user_id: String,
    pub server_id: String,
    pub email: String,
}

impl fmt::Debug for LoginResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginResult")
            .field("kind", &self.kind)
            .field("version", &self.version)
            .field("token", &REDACTED)
            .field("user_id", &self.user_id)
            .field("server_id", &self.server_id)
            .field("email", &self.email)
            .finish()
    }
}

pub const TERN_SERVER_TYPE: &str = "tern-server";
pub const TERN_SERVER_VERSION: u32 = 1;
const REDACTED: &str = "<redacted>";

/// The Rust engine's identity line (stdin after the account line, or `--identity`):
/// `{"type":"tern-server","version":1,"url","token","userId","serverId"}` (4.1). `Debug`
/// never shows the token.
#[derive(Clone, PartialEq, Eq)]
pub struct TernServer {
    /// Scheme, host, optional port and path prefix; no trailing slash (see [`check_url`]).
    pub url: String,
    pub token: String,
    pub user_id: String,
    pub server_id: String,
}

impl fmt::Debug for TernServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TernServer")
            .field("url", &self.url)
            .field("token", &REDACTED)
            .field("user_id", &self.user_id)
            .field("server_id", &self.server_id)
            .finish()
    }
}

/// Why an identity line with `"type":"tern-server"` was refused. Names the field, never its
/// value, so it can be logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TernServerError {
    pub field: &'static str,
    pub reason: &'static str,
}

impl fmt::Display for TernServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.reason)
    }
}

impl std::error::Error for TernServerError {}

impl TernServer {
    /// True when `v` claims to be a Tern server object, valid or not.
    pub fn is_tern_server(v: &Value) -> bool {
        v.get("type").and_then(Value::as_str) == Some(TERN_SERVER_TYPE)
    }

    /// Parses and validates a Tern server object.
    pub fn from_json(v: &Value) -> Result<TernServer, TernServerError> {
        let err = |field, reason| TernServerError { field, reason };
        if !Self::is_tern_server(v) {
            return Err(err("type", "must be \"tern-server\""));
        }
        if v.get("version").and_then(Value::as_u64) != Some(u64::from(TERN_SERVER_VERSION)) {
            return Err(err("version", "unsupported"));
        }
        let text = |field: &'static str| -> Result<String, TernServerError> {
            match v.get(field).and_then(Value::as_str) {
                Some(s) if !s.trim().is_empty() => Ok(s.to_owned()),
                Some(_) => Err(err(field, "empty")),
                None => Err(err(field, "missing or not a string")),
            }
        };
        let server = TernServer {
            url: text("url")?,
            token: text("token")?,
            user_id: text("userId")?,
            server_id: text("serverId")?,
        };
        check_url(&server.url).map_err(|reason| err("url", reason))?;
        if server
            .token
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ')
        {
            return Err(err("token", "contains whitespace or control characters"));
        }
        Ok(server)
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "type": TERN_SERVER_TYPE,
            "version": TERN_SERVER_VERSION,
            "url": self.url,
            "token": self.token,
            "userId": self.user_id,
            "serverId": self.server_id,
        })
    }

    /// The host (and port) of `url`, for logs.
    pub fn host(&self) -> &str {
        split_url(&self.url).map(|(_, a, _)| a).unwrap_or("")
    }

    pub fn binding(&self) -> String {
        binding(&self.server_id, &self.user_id)
    }
}

/// `scheme://authority[/path]` → `(scheme, authority, path)`.
fn split_url(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    Some((scheme, authority, path))
}

/// A Tern server base URL: `https`, or `http` only for a loopback host; scheme, host,
/// optional port and path prefix; no trailing slash, query, fragment or userinfo; never a
/// Mailspring host.
pub fn check_url(url: &str) -> Result<(), &'static str> {
    let (scheme, authority, path) = split_url(url).ok_or("not an absolute URL")?;
    if url.contains(['?', '#']) {
        return Err("has a query or fragment");
    }
    if url
        .bytes()
        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err("contains whitespace");
    }
    if authority.contains('@') {
        return Err("has userinfo");
    }
    if url.ends_with('/') {
        return Err("has a trailing slash");
    }
    let (host, port) = split_host_port(authority).ok_or("bad host")?;
    if host.is_empty() {
        return Err("bad host");
    }
    if let Some(p) = port {
        if p.is_empty() || p.parse::<u16>().is_err() {
            return Err("bad port");
        }
    }
    // A trailing dot names the same host.
    let host_lc = host.trim_end_matches('.').to_ascii_lowercase();
    if host_lc == "getmailspring.com" || host_lc.ends_with(".getmailspring.com") {
        return Err("is a Mailspring host");
    }
    match scheme {
        "https" => {}
        "http" if is_loopback(&host_lc) => {}
        "http" => return Err("http is only allowed for loopback hosts"),
        _ => return Err("scheme must be https"),
    }
    if path.split('/').skip(1).any(|seg| seg.is_empty()) {
        return Err("bad path");
    }
    Ok(())
}

fn split_host_port(authority: &str) -> Option<(&str, Option<&str>)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        host.parse::<std::net::Ipv6Addr>().ok()?;
        return match after {
            "" => Some((host, None)),
            _ => Some((host, Some(after.strip_prefix(':')?))),
        };
    }
    match authority.rsplit_once(':') {
        Some((h, p)) => {
            if h.contains(':') {
                return None;
            }
            Some((h, Some(p)))
        }
        None => Some((authority, None)),
    }
    .filter(|(h, _)| {
        h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    })
}

fn is_loopback(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") || host == "::1" {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_accepts_null_and_zero_version() {
        let p: MetadataPush =
            serde_json::from_value(json!({"objectType":"thread","version":null,"value":{}}))
                .unwrap();
        assert_eq!(p.version, None);
        let p: MetadataPush =
            serde_json::from_value(json!({"objectType":"message","version":0,"value":[]})).unwrap();
        assert_eq!(p.version, Some(0));
        let p: MetadataPush =
            serde_json::from_value(json!({"objectType":"contact","value":{"a":1}})).unwrap();
        assert_eq!(p.version, None);
        assert_eq!(p.header_message_id, None);
    }

    #[test]
    fn push_rejects_unknown_or_capitalised_object_type() {
        for t in ["Thread", "event", ""] {
            let r = serde_json::from_value::<MetadataPush>(
                json!({"objectType": t, "version": 1, "value": {}}),
            );
            assert!(r.is_err(), "{t} accepted");
        }
    }

    #[test]
    fn stream_metadata_line_shape() {
        let line = StreamLine::Metadata {
            attributes: MetadataRecord {
                object_type: ObjectType::Thread,
                object_id: "t:abc".into(),
                aid: "0123abcd".into(),
                plugin_id: "thread-snooze".into(),
                v: 3,
                value: json!({"expiration": 1}),
                header_message_id: Some("<m@x>".into()),
            },
            cursor: "42".into(),
        };
        let v = serde_json::to_value(&line).unwrap();
        assert_eq!(
            v,
            json!({
                "object": "metadata",
                "attributes": {
                    "object_type": "thread", "object_id": "t:abc", "aid": "0123abcd",
                    "plugin_id": "thread-snooze", "v": 3, "value": {"expiration": 1},
                    "headerMessageId": "<m@x>"
                },
                "cursor": "42"
            })
        );
        assert_eq!(serde_json::from_value::<StreamLine>(v).unwrap(), line);
    }

    #[test]
    fn record_omits_missing_header_message_id() {
        let r = MetadataRecord {
            object_type: ObjectType::Message,
            object_id: "m".into(),
            aid: "a".into(),
            plugin_id: "p".into(),
            v: 1,
            value: json!({}),
            header_message_id: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert!(v.get("headerMessageId").is_none());
    }

    #[test]
    fn backend_event_line_shape() {
        let line = StreamLine::BackendEvent {
            event: BackendEvent::IdentityRefreshNeeded,
        };
        assert_eq!(
            serde_json::to_string(&line).unwrap(),
            r#"{"object":"backend-event","event":"identity-refresh-needed"}"#
        );
    }

    fn server_json() -> Value {
        json!({"type": "tern-server", "version": 1, "url": "https://tern.example.com/sub",
               "token": "tern_secret", "userId": "u1", "serverId": "s1"})
    }

    #[test]
    fn tern_server_parses_and_round_trips() {
        let s = TernServer::from_json(&server_json()).unwrap();
        assert_eq!(s.url, "https://tern.example.com/sub");
        assert_eq!(s.host(), "tern.example.com");
        assert_eq!(s.binding(), "s1:u1");
        assert_eq!(s.to_json(), server_json());
        let debug = format!("{s:?}");
        assert!(!debug.contains("tern_secret"), "{debug}");
    }

    #[test]
    fn tern_server_names_the_bad_field() {
        let with = |k: &str, v: Value| {
            let mut j = server_json();
            j[k] = v;
            TernServer::from_json(&j).unwrap_err().field
        };
        assert_eq!(with("version", json!(2)), "version");
        assert_eq!(with("token", json!("")), "token");
        assert_eq!(with("token", json!(5)), "token");
        assert_eq!(with("userId", json!(" ")), "userId");
        assert_eq!(with("serverId", Value::Null), "serverId");
        assert_eq!(with("url", json!("http://tern.example.com")), "url");
        let mut j = server_json();
        j.as_object_mut().unwrap().remove("serverId");
        assert_eq!(TernServer::from_json(&j).unwrap_err().field, "serverId");
        assert!(!TernServer::is_tern_server(&json!({"emailAddress": "a"})));
    }

    #[test]
    fn server_urls() {
        for ok in [
            "https://tern.example.com",
            "https://tern.example.com:8443/a/b",
            "http://127.0.0.1:5101",
            "http://localhost:5101/tern",
            "http://[::1]:5101",
            "https://[2001:db8::1]",
        ] {
            assert_eq!(check_url(ok), Ok(()), "{ok}");
        }
        for bad in [
            "tern.example.com",
            "ftp://tern.example.com",
            "http://tern.example.com",
            "http://10.0.0.1",
            "https://tern.example.com/",
            "https://tern.example.com/a/",
            "https://tern.example.com//a",
            "https://tern.example.com?x=1",
            "https://tern.example.com#f",
            "https://user:pw@tern.example.com",
            "https://id.getmailspring.com",
            "https://GetMailspring.com",
            "https://id.getmailspring.com.",
            "https://[not-an-ip]",
            "https://tern.example.com:99999",
            "https://tern.example.com:",
            "https://",
            "https://tern example.com",
        ] {
            assert!(check_url(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn server_info_and_me_shapes() {
        let info = ServerInfo {
            server_id: "s".into(),
            version: "0.1.0".into(),
            public_url: "https://t.example".into(),
            signup: SignupMode::Closed,
            features: Features::default(),
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({"serverId": "s", "version": "0.1.0", "publicURL": "https://t.example",
                   "signup": "closed", "features": {"tracking": false, "sharing": false}})
        );
        let me: Me = serde_json::from_value(json!({
            "userId": "u", "email": "e@x", "firstName": "F", "lastName": "L",
            "trackingNS": "n", "serverId": "s", "publicURL": "https://t.example",
            "features": {"tracking": true, "sharing": false}
        }))
        .unwrap();
        assert_eq!(me.binding(), "s:u");
        assert_eq!(serde_json::to_value(&me).unwrap()["trackingNS"], "n");
    }

    #[test]
    fn login_result_shape() {
        let r = LoginResult {
            kind: LOGIN_RESULT_TYPE.into(),
            version: LOGIN_RESULT_VERSION,
            token: "tern_x".into(),
            user_id: "u".into(),
            server_id: "s".into(),
            email: "e@x".into(),
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"type": "tern-login", "version": 1, "token": "tern_x", "userId": "u",
                   "serverId": "s", "email": "e@x"})
        );
        assert!(!format!("{r:?}").contains("tern_x"));
    }

    #[test]
    fn head_cursor_is_a_string() {
        assert_eq!(
            serde_json::to_string(&HeadResponse { cursor: "0".into() }).unwrap(),
            r#"{"cursor":"0"}"#
        );
    }
}
