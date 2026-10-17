// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! What a relayed connection may do (docs/web-gateway-plan.md G6, G10). `tern-gateway`
//! relays remote devices to the engine and checks these before it forwards a request; the
//! engine checks them again on every connection whose `hello` says `relayed`.

use serde_json::{Map, Value};

use crate::method;

/// Lets a relayed device call [`MethodClass::LocalOnly`] methods and queue any task. Granted
/// at pairing only.
pub const SCOPE_ADMIN: &str = "admin";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodClass {
    /// Any connection, relayed or not.
    Open,
    /// Local clients, and relayed devices with [`SCOPE_ADMIN`]: methods that write accounts,
    /// secrets or the engine itself, or hand out local paths. Every method this table
    /// doesn't name is in this class.
    LocalOnly,
    /// `serve` mode's stdin only (`account.add` and the rest of [`crate::method`]'s
    /// parent methods).
    ParentOnly,
    /// Debug builds' crash and panic triggers.
    Debug,
}

pub fn method_class(name: &str) -> MethodClass {
    match name {
        method::HELLO
        | method::SUBSCRIBE
        | method::TASK_QUEUE
        | method::TASK_CANCEL
        | method::SYNC_WAKE
        | method::BODIES_FETCH
        | method::CALENDAR_SYNC
        | method::ENGINE_STATUS
        | method::FOLDERS_LIST
        | method::LABELS_LIST
        | method::THREADS_LIST
        | method::THREADS_GET
        | method::MESSAGES_GET
        | method::SEARCH
        | method::CONTACTS_SEARCH
        | method::TASKS_GET
        | method::COUNTS_GET
        | method::TASKS_LIST
        | method::METADATA_LIST
        | method::ACCOUNT_LIST => MethodClass::Open,
        method::ACCOUNT_ADD
        | method::ACCOUNT_REMOVE
        | method::ACCOUNT_TEST
        | method::ACCOUNT_RESET
        | method::ENGINE_RESET_DATABASE
        | method::ENGINE_SET_LOG_LEVEL
        | method::ENGINE_SHUTDOWN => MethodClass::ParentOnly,
        method::TEST_CRASH | method::TEST_PANIC => MethodClass::Debug,
        _ => MethodClass::LocalOnly,
    }
}

/// Whether a relayed device with `scopes` may call `name`.
pub fn relayed_method_allowed(name: &str, scopes: &[String]) -> bool {
    match method_class(name) {
        MethodClass::Open => true,
        MethodClass::LocalOnly => is_admin(scopes),
        MethodClass::ParentOnly | MethodClass::Debug => false,
    }
}

pub fn is_admin(scopes: &[String]) -> bool {
    scopes.iter().any(|s| s == SCOPE_ADMIN)
}

/// The task classes a relayed device without [`SCOPE_ADMIN`] may queue: the ones the web
/// client sends. The RFC 2822 export tasks write to a path the client names, so they stay
/// out; so do the event and contact tasks until they have been reviewed for the web.
pub const RELAYED_TASK_CLASSES: &[&str] = &[
    "ChangeUnreadTask",
    "ChangeStarredTask",
    "ChangeFolderTask",
    "ChangeLabelsTask",
    "SyncbackDraftTask",
    "SendDraftTask",
    "DestroyDraftTask",
    "SyncbackMetadataTask",
    "ExpungeAllInFolderTask",
    "SyncbackCategoryTask",
    "DestroyCategoryTask",
];

/// Whether a relayed device with `scopes` may queue `task` (the `task.queue` params'
/// task JSON).
pub fn relayed_task_allowed(task: &Value, scopes: &[String]) -> bool {
    if is_admin(scopes) {
        return true;
    }
    task.get("__cls")
        .and_then(Value::as_str)
        .is_some_and(|cls| RELAYED_TASK_CLASSES.contains(&cls))
}

const ACCOUNT_FIELDS: &[&str] = &[
    "id",
    "name",
    "emailAddress",
    "provider",
    "label",
    "aliases",
    "defaultAlias",
    "color",
];

const SETTINGS_FIELDS: &[&str] = &[
    "imap_host",
    "imap_port",
    "imap_username",
    "imap_security",
    "smtp_host",
    "smtp_port",
    "smtp_username",
    "smtp_security",
];

/// The account JSON reduced to the fields listed here, never to "everything but the
/// secrets": a field added to the account JSON later stays out until it is listed.
pub fn public_account_json(account: &Value) -> Value {
    let pick = |from: &Value, keys: &[&str]| -> Map<String, Value> {
        keys.iter()
            .filter_map(|k| from.get(*k).map(|v| ((*k).to_owned(), v.clone())))
            .collect()
    };
    let mut out = pick(account, ACCOUNT_FIELDS);
    if let Some(settings) = account.get("settings") {
        out.insert(
            "settings".into(),
            Value::Object(pick(settings, SETTINGS_FIELDS)),
        );
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classes() {
        assert_eq!(method_class("threads.list"), MethodClass::Open);
        assert_eq!(method_class("account.add"), MethodClass::ParentOnly);
        assert_eq!(method_class("test.panic"), MethodClass::Debug);
        assert_eq!(method_class("secrets.set"), MethodClass::LocalOnly);
        assert_eq!(method_class("Threads.List"), MethodClass::LocalOnly);
        let admin = vec![SCOPE_ADMIN.to_owned()];
        assert!(relayed_method_allowed("threads.list", &[]));
        assert!(!relayed_method_allowed("secrets.set", &[]));
        assert!(relayed_method_allowed("secrets.set", &admin));
        assert!(!relayed_method_allowed("account.add", &admin));
        assert!(!relayed_method_allowed("test.crash", &admin));
    }

    #[test]
    fn tasks() {
        let t = |cls: &str| json!({"__cls": cls, "filepath": "/x"});
        assert!(relayed_task_allowed(&t("ChangeStarredTask"), &[]));
        assert!(!relayed_task_allowed(&t("GetMessageRFC2822Task"), &[]));
        assert!(!relayed_task_allowed(&t("GetManyRFC2822Task"), &[]));
        assert!(!relayed_task_allowed(&json!({}), &[]));
        assert!(relayed_task_allowed(
            &t("GetMessageRFC2822Task"),
            &[SCOPE_ADMIN.to_owned()]
        ));
    }

    #[test]
    fn account_fields_are_picked() {
        let a = json!({
            "id": "a1", "emailAddress": "me@example.com", "provider": "imap",
            "aliases": ["Me <me@example.com>"], "syncState": "ok", "future": "x",
            "settings": {
                "imap_host": "imap.example.com", "imap_port": 993, "imap_username": "me",
                "imap_password": "s3cret", "smtp_password": "s3cret", "refresh_token": "s3cret",
                "xoauth_refresh_token": "s3cret", "smtp_host": "smtp.example.com",
            },
        });
        let p = public_account_json(&a);
        assert!(!p.to_string().contains("s3cret"));
        assert_eq!(p["settings"]["imap_port"], 993);
        assert_eq!(p["aliases"][0], "Me <me@example.com>");
        assert!(p.get("future").is_none());
        assert!(p.get("syncState").is_none());
    }
}
