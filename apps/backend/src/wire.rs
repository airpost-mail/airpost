// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! Wire types shared by the backend and the engine's HTTP metadata backend
//! (docs/backend-plan.md 3.1, 3.4, 3.5).
//!
//! This module moves to `crates/ms-wire` in B1-E, so it depends on serde and serde_json only.
//! Field names follow what the C++ engine and the desktop already speak: push bodies and the
//! identity are camelCase, page and stream records are snake_case except `headerMessageId`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

/// Server features advertised in `tern.features`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Features {
    #[serde(default)]
    pub tracking: bool,
    #[serde(default)]
    pub sharing: bool,
}

/// The Tern block of the identity. Absent in Mailspring identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TernInfo {
    /// Random per server instance; the engine binds its cursors to it.
    pub server_id: String,
    #[serde(rename = "publicURL")]
    pub public_url: String,
    #[serde(rename = "trackingNS")]
    pub tracking_ns: String,
    #[serde(default)]
    pub features: Features,
}

/// The identity JSON: `GET /api/me` (without `token`), the `#identity-result` of
/// `/onboarding` and the engine's `--identity` / stdin input (with `token`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    pub email_address: String,
    /// `%Y-%m-%dT%H:%M:%S.000Z`. C++ throws at startup without it.
    pub created_at: String,
    pub stripe_plan: String,
    pub stripe_plan_effective: String,
    /// Always `{}` from a Tern server: every feature unlimited.
    #[serde(default)]
    pub feature_usage: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tern: Option<TernInfo>,
}

/// The plan name a Tern server reports. Anything but `Basic` makes `hasProFeatures()` true.
pub const STRIPE_PLAN: &str = "Tern";

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

    #[test]
    fn identity_round_trip_and_mailspring_identity() {
        let id = Identity {
            id: "u1".into(),
            token: Some("tern_x".into()),
            first_name: "A".into(),
            last_name: "B".into(),
            email_address: "a@b.c".into(),
            created_at: "2026-01-02T03:04:05.000Z".into(),
            stripe_plan: STRIPE_PLAN.into(),
            stripe_plan_effective: STRIPE_PLAN.into(),
            feature_usage: Map::new(),
            tern: Some(TernInfo {
                server_id: "s".into(),
                public_url: "https://t.example".into(),
                tracking_ns: "n".into(),
                features: Features {
                    tracking: true,
                    sharing: false,
                },
            }),
        };
        let v = serde_json::to_value(&id).unwrap();
        assert_eq!(v["emailAddress"], "a@b.c");
        assert_eq!(v["stripePlanEffective"], "Tern");
        assert_eq!(v["featureUsage"], json!({}));
        assert_eq!(v["tern"]["publicURL"], "https://t.example");
        assert_eq!(v["tern"]["trackingNS"], "n");
        assert_eq!(v["tern"]["serverId"], "s");
        assert_eq!(
            v["tern"]["features"],
            json!({"tracking": true, "sharing": false})
        );
        assert_eq!(serde_json::from_value::<Identity>(v).unwrap(), id);

        // A Mailspring identity (no `tern`, real feature usage) still parses.
        let ms: Identity = serde_json::from_value(json!({
            "id": "x", "token": "t", "firstName": "F", "lastName": "L",
            "emailAddress": "e@x", "createdAt": "2017-01-01T00:00:00.000Z",
            "stripePlan": "Basic", "stripePlanEffective": "Basic",
            "featureUsage": {"snooze": {"featureLimitName": "pro", "usedInPeriod": 1,
                                        "quota": 15, "period": "weekly"}}
        }))
        .unwrap();
        assert!(ms.tern.is_none());
    }

    #[test]
    fn head_cursor_is_a_string() {
        assert_eq!(
            serde_json::to_string(&HeadResponse { cursor: "0".into() }).unwrap(),
            r#"{"cursor":"0"}"#
        );
    }
}
