// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The draft pipeline ([`feature::DRAFT_PIPELINE`](crate::feature::DRAFT_PIPELINE)): drafts built
//! by the engine as the desktop builds them, the send checks, and sends with an undo window;
//! and ([`feature::DRAFT_EDITING`](crate::feature::DRAFT_EDITING)) edits, deletes and
//! attachments by path.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `draft.create` kinds.
pub mod draft_kind {
    named_consts! { &str;
        NEW = "new";
        REPLY = "reply";
        REPLY_ALL = "reply-all";
        FORWARD = "forward";
        MAILTO = "mailto";
        /// The self-addressed message that brings a thread to the top of the inbox (snooze,
        /// reminders); `threadId`, and `messageId` for the message it answers.
        RESURFACE = "resurface";
    }
    pub const ALL: [&str; 6] = [NEW, REPLY, REPLY_ALL, FORWARD, MAILTO, RESURFACE];
}

/// `draft.create`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    /// `plaintext`, `pristine`; `aid` for `new`. A draft the engine saves (`save` not false)
    /// is not pristine when these set any recipients, subject, body or files, unless they
    /// set `pristine` themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Value>>,
    /// `reply`, `reply-all`: `prefer-existing` answers with the newest draft already replying
    /// to the message (its recipients switched to the kind, then `fields` over it, `hMsgId`
    /// aside), `prefer-existing-if-pristine` only when that draft is untouched. Without it a
    /// new draft is made.
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
    /// Files to attach, as [`DraftUpdateParams::attach`]. With `save: false` too: the engine
    /// keeps the copies, and the draft's `files` carries them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attach: Vec<String>,
    /// A template's name ([`feature::TEMPLATES`](crate::feature::TEMPLATES)), applied after
    /// `fields` as the desktop inserts one: its body replaces the text above the signature
    /// and quote, and its subject is the draft's unless `fields` has a `subject` or the draft
    /// is a reply or forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// The most bytes of attachments a draft carries, each file and all together (the
/// desktop's `AttachmentStore` limit).
pub const DRAFT_ATTACHMENTS_MAX_BYTES: u64 = 25_000_000;

/// `draft.update`: an edit to a stored draft, refused while it is being sent or has a
/// send-later (an undo window or a schedule) pending. Answers with the saved draft's Message
/// JSON and its `body`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftUpdateParams {
    pub account_id: String,
    pub header_message_id: String,
    /// Message JSON keys set over the stored draft, as [`DraftCreateParams::fields`] are over
    /// a built one; `aid` and `hMsgId` stay. The draft is no longer pristine when these, or
    /// `attach` or `detach`, write recipients, subject, body or files, unless they set
    /// `pristine` themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Value>>,
    /// Absolute paths of files on the engine's machine to attach, after `fields`. The engine
    /// copies each into its files directory as a new File, named after the path and typed by
    /// its extension; a file the draft already carries with the same name and bytes isn't
    /// attached twice. Local clients only ([`policy`](crate::policy)).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attach: Vec<String>,
    /// Files to remove, after `attach`: a File id, or a filename that names one of the
    /// draft's files. An upload's copy (a File with no `messageId`) is deleted with it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detach: Vec<String>,
}

/// `draft.delete`: queues a `DestroyDraftTask`. A send-later pending on the draft (an undo
/// window or a schedule) is cancelled in the same transaction, so the expiration executor
/// sends it wholly before or not at all; a draft whose `SendDraftTask` is queued can't be
/// deleted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftDeleteParams {
    pub account_id: String,
    pub header_message_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftDeleteResult {
    /// The `DestroyDraftTask`.
    pub task_id: String,
    /// A pending send-later was cancelled with the draft.
    pub cancelled_send: bool,
}

/// What a client's send hooks made of the body (`applyTransformsForSending`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftSendResult {
    /// The `SendDraftTask`, when sent now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Unix seconds, when the send waits out the undo window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<i64>,
    /// With `scheduledAt` on the socket: the undo log entry, whose undo is `draft.cancelSend`
    /// ([`feature::UNDO`](crate::feature::UNDO)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<String>,
}

/// `draft.cancelSend`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftCancelSendParams {
    pub account_id: String,
    pub header_message_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftCancelSendResult {
    /// False when there was nothing to cancel: no pending send, or it has started.
    pub cancelled: bool,
}

/// `draft.validate`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftProblem {
    /// `no-recipients`, `invalid-recipient`, `salutation-mismatch`, `unsendable-from`,
    /// `blank-subject`, `attachment-mentioned`, `empty-template`, `template-variable`.
    pub code: String,
    /// The desktop's English text.
    pub message: String,
    /// `recipients` or `misc`.
    pub group: String,
}

/// Errors stop the send; warnings ask first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DraftValidateResult {
    pub errors: Vec<DraftProblem>,
    pub warnings: Vec<DraftProblem>,
}
