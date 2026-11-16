// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The plugin metadata the desktop's built-in plugins write, for clients that write or read
//! the same values: send later (`send-later`), send reminders (`send-reminders`, and
//! `thread:send-reminders` on a draft, which the send moves to its thread), and open and
//! link tracking (`open-tracking`, `link-tracking`). Expirations are Unix seconds, as
//! `directlyAttachMetadata` stores a `Date`.

use airpost_ipc::{feature, server_feature, Capabilities};
use serde_json::{json, Value};

pub const SEND_LATER: &str = "send-later";
pub const SEND_REMINDERS: &str = "send-reminders";
/// `send-reminders` on a draft: `SendDraftTask` attaches it to the thread when it is sent.
pub const THREAD_SEND_REMINDERS: &str = "thread:send-reminders";
pub const OPEN_TRACKING: &str = "open-tracking";
pub const LINK_TRACKING: &str = "link-tracking";

/// The value the open-tracking composer toggle writes when on.
pub fn open_tracking_on() -> Value {
    json!({"open_count": 0, "open_data": []})
}

/// The value the link-tracking composer toggle writes when on.
pub fn link_tracking_on() -> Value {
    json!({"tracked": true})
}

/// The composer's reminder on a draft (`send-reminders` `composer-button.tsx`).
pub fn draft_reminder(expiration: i64, header_message_id: &str) -> Value {
    json!({"expiration": expiration, "sentHeaderMessageId": header_message_id})
}

/// The reminder set on a sent thread (`send-reminders` `popover-button.tsx`): the message
/// that gets the reminder, and the thread's last received message as the baseline a reply
/// is measured against. `None` expiration clears it, as the popover's cancel does.
pub fn thread_reminder(
    expiration: Option<i64>,
    sent_header_message_id: Option<&str>,
    last_message_received: i64,
) -> Value {
    match expiration {
        Some(at) => json!({
            "expiration": at,
            "shouldNotify": false,
            "sentHeaderMessageId": sent_header_message_id,
            "lastReplyTimestamp": last_message_received,
        }),
        None => json!({}),
    }
}

/// The value of `plugin_id` on a model's `metadata`, when it is an object.
pub fn plugin_value<'a>(model: &'a Value, plugin_id: &str) -> Option<&'a Value> {
    model
        .get("metadata")?
        .as_array()?
        .iter()
        .find(|m| m.get("pluginId").and_then(Value::as_str) == Some(plugin_id))
        .map(|m| &m["value"])
        .filter(|v| v.is_object())
}

/// The reminder's expiration on a thread or draft, when one is pending.
pub fn reminder_at(model: &Value) -> Option<i64> {
    [THREAD_SEND_REMINDERS, SEND_REMINDERS]
        .iter()
        .find_map(|p| plugin_value(model, p)?.get("expiration")?.as_f64())
        .map(|e| e as i64)
}

/// Why the engine can't send `opens` / `links` tracking, or `None` when it can: it needs the
/// `tracking` feature and a connected server offering it (`capabilities.server`).
pub fn tracking_unavailable(
    features: &[String],
    capabilities: Option<&Capabilities>,
    opens: bool,
    links: bool,
) -> Option<String> {
    if !opens && !links {
        return None;
    }
    if !features.iter().any(|f| f == feature::TRACKING) {
        return Some("this engine can't send tracked mail (no tracking); update Airpost".into());
    }
    let wanted = [
        (opens, server_feature::READ_RECEIPTS, "open tracking"),
        (links, server_feature::LINK_TRACKING, "link tracking"),
    ];
    for (asked, key, name) in wanted {
        if !asked {
            continue;
        }
        let cap = capabilities.and_then(|c| c.server.get(key));
        if !cap.is_some_and(|c| c.available) {
            let why = cap
                .and_then(|c| c.reason.as_deref())
                .unwrap_or("needs-airpost-server");
            return Some(format!(
                "{name} needs a Airpost server that offers it, and none is connected ({why})"
            ));
        }
    }
    None
}

fn events(list: &[Value]) -> Vec<Value> {
    list.iter()
        .map(|e| json!({"recipient": e.get("recipient"), "timestamp": e.get("timestamp")}))
        .collect()
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn int(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|n| n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)))
        .unwrap_or(0)
}

/// `messageTrackingSummary`: `(opens, links)` of a sent message, each null without that
/// plugin's metadata. Opens are `{count, events}`, links `{links: [{url, clickCount,
/// events}]}`, events `{recipient, timestamp}`.
pub fn tracking_summary(message: &Value) -> (Value, Value) {
    let opens = plugin_value(message, OPEN_TRACKING).map_or(
        Value::Null,
        |m| json!({"count": int(m, "open_count"), "events": events(array(m, "open_data"))}),
    );
    let links = plugin_value(message, LINK_TRACKING).map_or(Value::Null, |m| {
        let links: Vec<Value> = array(m, "links")
            .iter()
            .map(|l| {
                json!({"url": l.get("url"), "clickCount": int(l, "click_count"),
                       "events": events(array(l, "click_data"))})
            })
            .collect();
        json!({"links": links})
    });
    (opens, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_needs_the_feature_and_a_server() {
        let none: Vec<String> = vec![];
        assert!(tracking_unavailable(&none, None, false, false).is_none());
        assert!(tracking_unavailable(&none, None, true, false)
            .unwrap()
            .contains("update Airpost"));
        let features = vec![feature::TRACKING.to_owned()];
        let caps: Capabilities = serde_json::from_value(json!({
            "server": {"readReceipts": {"available": true},
                       "linkTracking": {"available": false, "reason": "needs-airpost-server"}}
        }))
        .unwrap();
        assert!(tracking_unavailable(&features, Some(&caps), true, false).is_none());
        let why = tracking_unavailable(&features, Some(&caps), true, true).unwrap();
        assert!(
            why.starts_with("link tracking needs a Airpost server"),
            "{why}"
        );
        assert!(tracking_unavailable(&features, None, true, false).is_some());
    }

    #[test]
    fn summaries_and_reminders() {
        let m = json!({"metadata": [
            {"pluginId": "open-tracking", "v": 2, "value": {"open_count": 2,
                "open_data": [{"recipient": "a@x", "timestamp": 10}]}},
            {"pluginId": "link-tracking", "v": 1, "value": {"links": [
                {"url": "https://x", "click_count": 1, "click_data": []}]}},
            {"pluginId": "thread:send-reminders", "v": 1, "value": {"expiration": 99.0}},
        ]});
        let (opens, links) = tracking_summary(&m);
        assert_eq!(opens["count"], 2);
        assert_eq!(opens["events"][0]["recipient"], "a@x");
        assert_eq!(links["links"][0]["clickCount"], 1);
        assert_eq!(reminder_at(&m), Some(99));
        assert_eq!(tracking_summary(&json!({})), (Value::Null, Value::Null));
        assert_eq!(thread_reminder(None, None, 5), json!({}));
        assert_eq!(
            thread_reminder(Some(7), Some("h@x"), 5),
            json!({"expiration": 7, "shouldNotify": false, "sentHeaderMessageId": "h@x",
                   "lastReplyTimestamp": 5})
        );
    }
}
