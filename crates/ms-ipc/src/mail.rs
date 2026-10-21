// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Mail actions that are more than one task: snooze
//! ([`feature::SNOOZE`](crate::feature::SNOOZE)) and unsubscribe
//! ([`feature::UNSUBSCRIBE`](crate::feature::UNSUBSCRIBE)).

use serde::{Deserialize, Serialize};

/// `threads.snooze`: move the threads to Snoozed and write the `thread-snooze` metadata that
/// brings them back at `until`, as the desktop's snooze popover does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsSnoozeParams {
    pub account_id: String,
    pub thread_ids: Vec<String>,
    /// Unix seconds, in the future.
    pub until: i64,
}

/// `threads.unsnooze`: clear the snooze and move back to the inbox what is still in Snoozed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsUnsnoozeParams {
    pub account_id: String,
    pub thread_ids: Vec<String>,
}

/// `threads.snooze` and `threads.unsnooze`: the tasks queued, the move first. Their local
/// phases have run; each arrives as a `Task` delta like any other.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadsSnoozeResult {
    pub task_ids: Vec<String>,
}

/// The ways to unsubscribe from a list, in the order the engine prefers them.
pub mod unsubscribe_method {
    /// RFC 8058: the engine POSTs `List-Unsubscribe=One-Click` to the https URI.
    pub const ONE_CLICK: &str = "one-click";
    /// RFC 2369 `mailto:`: the engine sends the message the URI describes.
    pub const MAILTO: &str = "mailto";
    /// An http(s) URI for the client to open.
    pub const WEB: &str = "web";
}

/// `messages.unsubscribe`: act on the message's `List-Unsubscribe` header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesUnsubscribeParams {
    pub account_id: String,
    pub message_id: String,
    /// One of [`unsubscribe_method`]; without it the engine picks one-click when the message
    /// offers it, else mailto, else web. A method the header doesn't offer is `NOT_FOUND`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesUnsubscribeResult {
    /// The [`unsubscribe_method`] used.
    pub method: String,
    /// one-click: the server answered 2xx. mailto: the message is queued. web: always true,
    /// and the client opens `url`.
    pub ok: bool,
    /// The URI acted on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Why `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// mailto: the `SendDraftTask` that sends the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}
