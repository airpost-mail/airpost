// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The draft pipeline (docs/desktop-client-plan.md 3.3, stage 13,
//! [`feature::DRAFT_PIPELINE`](crate::feature::DRAFT_PIPELINE)): drafts built by the engine as
//! the desktop builds them, the send checks, and sends with an undo window.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `draft.create` kinds.
pub mod draft_kind {
    pub const NEW: &str = "new";
    pub const REPLY: &str = "reply";
    pub const REPLY_ALL: &str = "reply-all";
    pub const FORWARD: &str = "forward";
    pub const MAILTO: &str = "mailto";
    /// The self-addressed message that brings a thread to the top of the inbox (snooze,
    /// reminders); `threadId`, and `messageId` for the message it answers.
    pub const RESURFACE: &str = "resurface";
    pub const ALL: [&str; 6] = [NEW, REPLY, REPLY_ALL, FORWARD, MAILTO, RESURFACE];
}

/// `draft.create`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCreateParams {
    /// The account to write from: for `new`, `mailto` and `resurface`; a reply or forward is
    /// written from the message's account. Without it, `core.sending.defaultAccountIdForSend`
    /// or else the first account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// One of [`draft_kind`].
    pub kind: String,
    /// `resurface`: the thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// `reply`, `reply-all`, `forward`: the message (its id); `resurface`: the message it
    /// answers, else the thread's newest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// `mailto`: the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailto: Option<String>,
    /// Message JSON keys set over what the kind builds: `to`, `cc`, `bcc`, `from` (contact
    /// lists), `subject`, `body`, `files`, `threadId`, `hMsgId`, `rthMsgId`, `fwdMsgId`,
    /// `plaintext`, `pristine`; `aid` for `new`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Value>>,
    /// `reply`, `reply-all`: `prefer-existing` answers with the newest draft already replying
    /// to the message (its recipients switched to the kind), `prefer-existing-if-pristine`
    /// only when that draft is untouched. Without it a new draft is made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    /// HTML or plaintext; `core.composing.html` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<bool>,
    /// The IANA zone for dates in quotes; the engine's zone by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tz: Option<String>,
    /// False: answer with the draft without saving it (a client that saves it itself after
    /// its own hooks). True by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save: Option<bool>,
    /// `resurface`: the body; the `resurfacing.snoozeBody` setting's by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// What a client's send hooks made of the body (`applyTransformsForSending`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftBodies {
    /// The body that goes out, and is saved to Sent.
    #[serde(rename = "self", default, skip_serializing_if = "Option::is_none")]
    pub self_body: Option<String>,
    /// Recipient address to the body that recipient gets.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub per_recipient: BTreeMap<String, String>,
}

impl DraftBodies {
    pub fn is_empty(&self) -> bool {
        self.self_body.is_none() && self.per_recipient.is_empty()
    }
}

/// `draft.send`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSendParams {
    pub account_id: String,
    pub header_message_id: String,
    /// The undo window. Without it (or 0) the draft is sent now; with it the engine writes
    /// send-later metadata (`isUndoSend`) and sends when it is due, also after a restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    /// `send` (default) or `send-and-archive`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bodies: Option<DraftBodies>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSendResult {
    /// The `SendDraftTask`, when sent now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Unix seconds, when the send waits out the undo window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<i64>,
}

/// `draft.cancelSend`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCancelSendParams {
    pub account_id: String,
    pub header_message_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftCancelSendResult {
    /// False when there was nothing to cancel: no pending send, or it has started.
    pub cancelled: bool,
}

/// `draft.validate`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftValidateParams {
    pub account_id: String,
    pub header_message_id: String,
    /// Recipients whose names aren't checked against the salutation (the desktop's "Send &
    /// Ignore Warnings For This Email").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_salutation_for: Vec<String>,
}

/// One `draft.validate` finding.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftProblem {
    /// `no-recipients`, `invalid-recipient`, `salutation-mismatch`, `unsendable-from`,
    /// `blank-subject`, `attachment-mentioned`, `empty-template`.
    pub code: String,
    /// The desktop's English text.
    pub message: String,
    /// `recipients` or `misc`.
    pub group: String,
}

/// Errors stop the send; warnings ask first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftValidateResult {
    pub errors: Vec<DraftProblem>,
    pub warnings: Vec<DraftProblem>,
}
