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
        | method::QUERY
        | method::ACCOUNT_LIST
        // settings.set is open for the keys in RELAYED_SETTINGS_KEYS only
        // (relayed_settings_allowed).
        | method::SETTINGS_GET
        | method::SETTINGS_SET => MethodClass::Open,
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

/// The settings a relayed device without [`SCOPE_ADMIN`] may write. The rest carry HTML the
/// desktop renders in a window with Node (`signatures`, `templates`, the resurfacing bodies)
/// or act on mail by themselves (`mailRules`), so they stay local until those clients treat
/// them as untrusted.
pub const RELAYED_SETTINGS_KEYS: &[&str] = &[
    "core.sending.undoSend",
    "core.sending.defaultAccountIdForSend",
    "core.notifications.unsnoozeToTop",
    "core.composing.html",
    "defaultSignatures",
];

/// Whether a relayed device with `scopes` may write every key of `values` (the
/// `settings.set` params' `values`).
pub fn relayed_settings_allowed(values: &Map<String, Value>, scopes: &[String]) -> bool {
    is_admin(scopes)
        || values
            .keys()
            .all(|k| RELAYED_SETTINGS_KEYS.contains(&k.as_str()))
}

/// Whether a relayed device with `scopes` may make the request `name` with `params`: the
/// method's class, and for `task.queue` and `settings.set` what the params ask for.
pub fn relayed_request_allowed(name: &str, params: &Value, scopes: &[String]) -> bool {
    if !relayed_method_allowed(name, scopes) {
        return false;
    }
    match name {
        method::TASK_QUEUE => params
            .get("task")
            .is_some_and(|t| relayed_task_allowed(t, scopes)),
        method::SETTINGS_SET => params
            .get("values")
            .and_then(Value::as_object)
            .is_some_and(|v| relayed_settings_allowed(v, scopes)),
        _ => true,
    }
}

/// Bounds a relayed `query`: rows and ids default to and stop at
/// [`READ_MAX_LIMIT`](crate::READ_MAX_LIMIT), as every other relayed read does, so one
/// request can't load a whole mailbox into the engine and one reply line. Counts and
/// params that aren't an object, or whose limit isn't a non-negative integer, pass as they
/// are for the engine to answer or refuse. Local clients keep the desktop's unbounded
/// reads.
pub fn cap_relayed_query(params: &mut Value) {
    let Some(p) = params.as_object_mut() else {
        return;
    };
    if p.get("count").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let max = crate::READ_MAX_LIMIT as u64;
    match p.get("limit") {
        None | Some(Value::Null) => {
            p.insert("limit".into(), max.into());
        }
        Some(Value::Number(n)) if n.as_u64().is_some_and(|l| l > max) => {
            p.insert("limit".into(), max.into());
        }
        Some(_) => {}
    }
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
    fn relayed_query_is_capped() {
        let capped = |mut v: Value| {
            cap_relayed_query(&mut v);
            v
        };
        let max = crate::READ_MAX_LIMIT;
        assert_eq!(
            capped(json!({"class": "Message", "include": ["body"]}))["limit"],
            max
        );
        assert_eq!(
            capped(json!({"class": "Thread", "limit": null}))["limit"],
            max
        );
        assert_eq!(
            capped(json!({"class": "Thread", "limit": 100_000}))["limit"],
            max
        );
        assert_eq!(
            capped(json!({"class": "Thread", "idsOnly": true}))["limit"],
            max
        );
        assert_eq!(capped(json!({"class": "Thread", "limit": 20}))["limit"], 20);
        assert_eq!(capped(json!({"class": "Thread", "limit": -1}))["limit"], -1);
        assert_eq!(
            capped(json!({"class": "Thread", "limit": "x"}))["limit"],
            "x"
        );
        assert!(capped(json!({"class": "Thread", "count": true}))
            .get("limit")
            .is_none());
        assert_eq!(capped(json!(null)), json!(null));
    }

    #[test]
    fn classes() {
        assert_eq!(method_class("threads.list"), MethodClass::Open);
        assert_eq!(method_class("account.add"), MethodClass::ParentOnly);
        assert_eq!(method_class("test.panic"), MethodClass::Debug);
        assert_eq!(method_class("secrets.set"), MethodClass::LocalOnly);
        assert_eq!(method_class("Threads.List"), MethodClass::LocalOnly);
        let admin = vec![SCOPE_ADMIN.to_owned()];
        assert!(relayed_method_allowed("threads.list", &[]));
        assert!(relayed_method_allowed("settings.set", &[]));
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
    fn settings() {
        let admin = vec![SCOPE_ADMIN.to_owned()];
        let set = |values: Value| json!({ "values": values });
        let sig = set(json!({"signatures": {"s1": {"body": "<img src=x onerror=alert(1)>"}}}));
        assert!(!relayed_request_allowed("settings.set", &sig, &[]));
        assert!(relayed_request_allowed("settings.set", &sig, &admin));
        for key in ["templates", "mailRules", "resurfacing.snoozeBody"] {
            let p = set(json!({ key: null }));
            assert!(!relayed_request_allowed("settings.set", &p, &[]), "{key}");
        }
        // One refused key refuses the call.
        let mixed = set(json!({"core.sending.undoSend": 0, "templates": []}));
        assert!(!relayed_request_allowed("settings.set", &mixed, &[]));
        let undo = set(json!({"core.sending.undoSend": 0}));
        assert!(relayed_request_allowed("settings.set", &undo, &[]));
        assert!(!relayed_request_allowed("settings.set", &json!({}), &[]));
        assert!(relayed_request_allowed("settings.get", &json!({}), &[]));
        let task = json!({"task": {"__cls": "GetManyRFC2822Task"}});
        assert!(!relayed_request_allowed("task.queue", &task, &[]));
        assert!(!relayed_request_allowed("secrets.set", &json!({}), &[]));
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
