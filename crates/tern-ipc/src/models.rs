// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Typed views of the model JSON the engine stores and sends (`query` rows, the read
//! methods, `delta` models). tern-store keeps each model as a `data: Value`, so unknown keys
//! survive a save; these structs only describe it. Each has a flattened `extra` map, so a
//! key a struct doesn't name still deserialises and serialises back, and the JSON Schema
//! (and the TS types generated from it) list the known keys only.
//!
//! `crates/tern-sync/tests/view_structs.rs` syncs mail through tern-sync and tern-store and
//! checks every row they write or send against its struct's schema, unknown keys included,
//! so a key tern-store starts writing has to be added here.
//!
//! An `Option` with `skip_serializing_if` says the engine leaves the key out rather than
//! writing `null` (the TS type is `?: T`); one without it may be `null` (`?: T | null`).
//!
//! Field names are the JSON keys (`aid`, `v`, `lmrt`), not the desktop's model keys
//! (`accountId`, `version`, `lastMessageReceivedTimestamp`); [`QUERY_CLASSES`] maps the
//! queryable model keys to them. Timestamps are unix seconds.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Keys a struct doesn't name.
pub type Extra = Map<String, Value>;

/// One plugin's metadata on a model (`ModelWithMetadata.pluginMetadata`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginMetadataJson {
    pub plugin_id: String,
    pub v: i64,
    pub value: Value,
}

/// A contact as embedded in messages and threads (`from`, `to`, `participants`, ...).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContactRef {
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A folder's or label's role: one role detection assigns (tern-core `ROLES`), or empty for
/// a user folder or label. The engine doesn't check the role `ChangeRoleMappingTask` sets.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub enum CategoryRole {
    #[serde(rename = "")]
    None,
    #[serde(rename = "inbox")]
    Inbox,
    #[serde(rename = "important")]
    Important,
    #[serde(rename = "snoozed")]
    Snoozed,
    #[serde(rename = "sent")]
    Sent,
    #[serde(rename = "drafts")]
    Drafts,
    #[serde(rename = "all")]
    All,
    #[serde(rename = "spam")]
    Spam,
    #[serde(rename = "archive")]
    Archive,
    #[serde(rename = "trash")]
    Trash,
    #[serde(rename = "starred")]
    Starred,
}

/// `Folder` and `Label`, which have the same keys.
macro_rules! category_json {
    ($(#[doc = $doc:literal])* $name:ident, $cls:literal) => {
        $(#[doc = $doc])*
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "camelCase")]
        pub struct $name {
            pub id: String,
            pub aid: String,
            pub v: i64,
            #[serde(rename = "__cls")]
            #[schemars(extend("const" = $cls))]
            pub cls: String,
            /// The IMAP path, in modified UTF-7.
            pub path: String,
            #[schemars(with = "CategoryRole")]
            pub role: String,
            /// Sync state (`uidvalidity`, `highestmodseq`, ...); left out of the copies a
            /// Thread embeds.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub local_status: Option<Map<String, Value>>,
            #[serde(flatten)]
            #[schemars(skip)]
            pub extra: Extra,
        }
    };
}

category_json!(
    /// An IMAP mailbox.
    FolderJson,
    "Folder"
);
category_json!(
    /// A Gmail label.
    LabelJson,
    "Label"
);

/// A folder or label as a Thread embeds it in `folders` and `labels`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ThreadCategoryRef {
    pub id: String,
    pub aid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v: Option<i64>,
    /// `Folder` or `Label`.
    #[serde(rename = "__cls")]
    #[schemars(extend("enum" = ["Folder", "Label"]))]
    pub cls: String,
    pub path: String,
    #[schemars(with = "CategoryRole")]
    pub role: String,
    /// The thread's messages in the category.
    #[serde(rename = "_refs")]
    pub refs: i64,
    /// The thread's unread messages in the category.
    #[serde(rename = "_u")]
    pub unread: i64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "Thread"))]
    pub cls: String,
    pub subject: String,
    /// Messages that are unread (a count; the desktop reads it as a boolean).
    pub unread: i64,
    /// Messages that are starred.
    pub starred: i64,
    /// lastMessageTimestamp: any message.
    pub lmt: i64,
    /// firstMessageTimestamp.
    pub fmt: i64,
    /// lastMessageSentTimestamp.
    pub lmst: i64,
    /// lastMessageReceivedTimestamp.
    pub lmrt: i64,
    /// The Gmail thread id (X-GM-THRID) as a string; empty on IMAP accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g_thr_id: Option<String>,
    pub in_all_mail: bool,
    pub attachment_count: i64,
    pub folders: Vec<ThreadCategoryRef>,
    pub labels: Vec<ThreadCategoryRef>,
    pub participants: Vec<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Vec<PluginMetadataJson>>,
    /// The thread's `ThreadSearch` row; engine bookkeeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_row_id: Option<i64>,
    /// `lmrt` came from a sent message because none was received; engine bookkeeping.
    #[serde(
        rename = "lmrt_is_fallback",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub lmrt_is_fallback: Option<bool>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// `folders` on a Message: folder id -> [`placement_flag`] bits, one key per copy.
pub mod placement_flag {
    named_consts! { i64;
        UNREAD = 1;
        STARRED = 2;
        DRAFT = 4;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "Message"))]
    pub cls: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub date: i64,
    pub thread_id: String,
    /// headerMessageId.
    pub h_msg_id: String,
    /// replyToHeaderMessageId.
    #[serde(default)]
    pub rth_msg_id: Option<String>,
    /// forwardedHeaderMessageId.
    #[serde(default)]
    pub fwd_msg_id: Option<String>,
    /// The Gmail message id as a string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub g_msg_id: Option<String>,
    pub unread: bool,
    pub starred: bool,
    pub draft: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pristine: Option<bool>,
    /// `body` is plain text rather than HTML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plaintext: Option<bool>,
    /// From MessageBody: on `query` with `include: ["body"]`, and on deltas once fetched.
    #[serde(default)]
    pub body: Option<String>,
    /// Folder id -> [`placement_flag`] bits, one key per copy on the server. The engine
    /// always writes it; a draft a client sends may leave it out.
    #[serde(default)]
    pub folders: BTreeMap<String, i64>,
    /// X-GM-LABELS values (`\\Inbox`, `Work`); as `folders`, absent on a client's draft.
    #[serde(default)]
    pub labels: Vec<String>,
    pub from: Vec<ContactRef>,
    pub to: Vec<ContactRef>,
    pub cc: Vec<ContactRef>,
    pub bcc: Vec<ContactRef>,
    pub reply_to: Vec<ContactRef>,
    pub files: Vec<FileJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<BTreeMap<String, Value>>,
    /// List-Unsubscribe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_list_unsub: Option<String>,
    /// List-Unsubscribe-Post.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h_list_unsub_post: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Vec<PluginMetadataJson>>,
    /// `Message.isHidden()`, on the rows of `query` and the read methods
    /// ([`feature::MESSAGE_HIDDEN`](crate::feature::MESSAGE_HIDDEN)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// On `messages.get` and `threads.get` ([`feature::MESSAGE_WARNINGS`](crate::feature::MESSAGE_WARNINGS)),
    /// left out when there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<crate::MessageWarning>>,
    /// Deltas only: the body has been fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_sync_complete: Option<bool>,
    /// Deltas only: the first save of a message's headers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers_sync_complete: Option<bool>,
    /// Deltas only: the mail rules have run on the message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_ready: Option<bool>,
    /// The sync timestamp of the headers (syncedAt); engine bookkeeping.
    #[serde(rename = "_sa", default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<i64>,
    /// The sync's unchanged count (syncUnsavedChanges); engine bookkeeping.
    #[serde(rename = "_suc", default, skip_serializing_if = "Option::is_none")]
    pub sync_unsaved_changes: Option<i64>,
    /// Whether the mail rules still wait for the body; engine bookkeeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rr: Option<i64>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "File"))]
    pub cls: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_id: Option<String>,
    /// Set for inline parts, matching `cid:` references in the HTML body.
    pub content_id: Option<String>,
    pub content_type: String,
    pub filename: String,
    pub size: i64,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "Contact"))]
    pub cls: String,
    pub name: String,
    pub email: String,
    /// source: `mail`, `carddav` or `gpeople`.
    pub s: String,
    /// hidden.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<bool>,
    /// How many messages name the contact.
    pub refs: i64,
    /// The ContactGroup ids.
    pub gis: Vec<String>,
    /// The provider's record: `{vcf}` for CardDAV, the person JSON for Google.
    pub info: Value,
    /// The ContactBook id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid: Option<String>,
    /// The provider's resource name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grn: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The contact's `ContactSearch` row; engine bookkeeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_row_id: Option<i64>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContactGroupJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "ContactGroup"))]
    pub cls: String,
    pub name: String,
    /// The ContactBook id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bid: Option<String>,
    /// The provider's resource name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grn: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContactBookJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "ContactBook"))]
    pub cls: String,
    /// `carddav` or `gpeople`.
    pub source: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A CalDAV collection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CalendarJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "Calendar"))]
    pub cls: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// `#rrggbb`, lower case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "read_only", default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A stored VEVENT (one per master or recurrence exception).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EventJson {
    pub id: String,
    pub aid: String,
    pub v: i64,
    #[serde(rename = "__cls")]
    #[schemars(extend("const" = "Event"))]
    pub cls: String,
    /// calendarId.
    pub cid: String,
    pub ics: String,
    pub icsuid: String,
    /// recurrenceId; empty on a master.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// recurrenceStart: the first occurrence's start.
    pub rs: i64,
    /// recurrenceEnd: the last occurrence's end.
    pub re: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The event's `EventSearch` row; engine bookkeeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_row_id: Option<i64>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A `ThreadCounts` delta's model: `persist` when a count changes or a category is added,
/// `unpersist` (no counts) when it is removed ([`feature::COUNTS`](crate::feature::COUNTS)).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCountsJson {
    pub id: String,
    pub category_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A `ProcessState` delta's model: not stored, the account's online state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStateJson {
    /// The account id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub account_id: String,
    pub connection_error: bool,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A `ProcessIdentityRefreshNeeded` delta: the Tern server refused the engine's token.
/// Carries no models.
pub const PROCESS_IDENTITY_REFRESH_NEEDED: &str = "ProcessIdentityRefreshNeeded";

/// The account JSON without secrets, as `account.list`, `account.changed` and the account
/// methods return it. On a relayed connection only the fields
/// [`public_account_json`](crate::policy::public_account_json) keeps.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountJson {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub v: Option<i64>,
    #[serde(rename = "__cls", default, skip_serializing_if = "Option::is_none")]
    pub cls: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `gmail`, `office365`, `outlook`, `imap`, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    pub email_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// IMAP and SMTP settings; never the passwords or the refresh token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<AccountSettingsJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Autoaddress>")]
    pub autoaddress: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(default)]
    pub default_alias: Option<String>,
    /// `ok`, `invalid` or `sync_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_error: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authed_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Vec<PluginMetadataJson>>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// An account's automatic Cc or Bcc (`AccountJson.autoaddress`).
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct Autoaddress {
    /// Addresses separated by `,` or `;`.
    pub value: String,
    #[schemars(extend("enum" = ["cc", "bcc"]))]
    pub r#type: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountSettingsJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_username: Option<String>,
    /// `SSL / TLS`, `STARTTLS` or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_security: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_allow_insecure_ssl: Option<bool>,
    /// The host the user confirmed `none` for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_allow_cleartext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_security: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_allow_insecure_ssl: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_allow_cleartext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_folder: Option<String>,
    /// Only in what a client sends (`account.add`, `account.test`); never returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_password: Option<String>,
    /// As `imap_password`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_password: Option<String>,
    /// As `imap_password`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// `account.discover`'s account: the settings found for an address, before it has an id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredAccountJson {
    pub email_address: String,
    pub provider: String,
    /// Servers, ports, usernames, security (never `none`), `*_allow_insecure_ssl` and
    /// `container_folder`. An empty `imap_host`: nothing found.
    pub settings: AccountSettingsJson,
    #[serde(flatten)]
    #[schemars(skip)]
    pub extra: Extra,
}

/// A `metadata.list` model: a Thread or a Message, by `__cls`.
#[derive(JsonSchema)]
#[serde(untagged)]
// Schema only, never built, so the variants' sizes don't matter.
#[allow(dead_code, clippy::large_enum_variant)]
pub enum MetadataModelJson {
    Thread(ThreadJson),
    Message(MessageJson),
}

/// Any model a `query`, read method or delta carries, by `__cls`.
#[derive(JsonSchema)]
#[serde(untagged)]
// Schema only, never built, so the variants' sizes don't matter.
#[allow(dead_code, clippy::large_enum_variant)]
pub enum AnyModelJson {
    Thread(ThreadJson),
    Message(MessageJson),
    Folder(FolderJson),
    Label(LabelJson),
    Contact(ContactJson),
    ContactGroup(ContactGroupJson),
    ContactBook(ContactBookJson),
    Calendar(CalendarJson),
    Event(EventJson),
    File(FileJson),
    Task(crate::tasks::TaskJson),
}
