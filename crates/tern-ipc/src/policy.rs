// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! What a relayed connection may do. `tern-gateway`
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
    /// `serve` mode's stdin only, and the socket of a `--detached` engine for local clients
    /// (`credentials.import` stays on stdin): methods that stop or restart the whole engine.
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
        // The tasks these queue are open too; unsubscribe acts only on URIs from the
        // message's own header.
        | method::THREADS_SNOOZE
        | method::THREADS_UNSNOOZE
        | method::MESSAGES_UNSUBSCRIBE
        // Drafts and their sends, as task.queue's SyncbackDraftTask and SendDraftTask.
        | method::DRAFT_CREATE
        | method::DRAFT_SEND
        | method::DRAFT_CANCEL_SEND
        | method::DRAFT_VALIDATE
        | method::DRAFT_UPDATE
        | method::DRAFT_DELETE
        // Names only; saving and deleting write files, and stay local.
        | method::TEMPLATES_LIST
        | method::TEMPLATES_GET
        // Reads. The calendar and contact writes queue event and contact tasks, which stay
        // local until they have been reviewed for the web (RELAYED_TASK_CLASSES).
        | method::EVENTS_EXPAND
        | method::CONTACTS_GET
        | method::CONTACTS_EXPORT
        // settings.set is open for the keys in RELAYED_SETTINGS_KEYS only
        // (relayed_settings_allowed).
        | method::SETTINGS_GET
        | method::SETTINGS_SET
        // Writes no account or secret; account.update is open for ACCOUNT_DISPLAY_FIELDS
        // only (relayed_request_allowed).
        | method::ACCOUNT_DISCOVER
        | method::ACCOUNT_REORDER
        | method::ACCOUNT_UPDATE => MethodClass::Open,
        // T13's local-only class: accounts, their secrets and the engine's server.
        method::ACCOUNT_ADD
        | method::ACCOUNT_REMOVE
        | method::ACCOUNT_TEST
        | method::ACCOUNT_RESET
        | method::ACCOUNT_OAUTH_BEGIN
        | method::ACCOUNT_OAUTH_COMPLETE
        | method::ACCOUNT_SET_SECRETS
        | method::SECRETS_SET
        | method::SECRETS_DELETE
        | method::SECRETS_HAS
        | method::SERVER_SET
        | method::SERVER_CLEAR
        | method::ENGINE_LOGS_TAIL => MethodClass::LocalOnly,
        method::ENGINE_RESET_DATABASE
        | method::ENGINE_SET_LOG_LEVEL
        | method::ENGINE_SHUTDOWN
        | method::CREDENTIALS_IMPORT => MethodClass::ParentOnly,
        method::TEST_CRASH | method::TEST_PANIC => MethodClass::Debug,
        // Paths on the engine's machine; browsers use the gateway's file routes.
        method::FILES_GET | method::FILES_READ | method::FILES_PUT => MethodClass::LocalOnly,
        // Runs the rules (forwards included) over the whole inbox; the rules themselves can
        // only be set locally (RELAYED_SETTINGS_KEYS).
        method::RULES_REPROCESS | method::RULES_CANCEL => MethodClass::LocalOnly,
        // The engine makes HTTP requests to the URL in the params' `config`, from the
        // user's machine and network.
        method::INTEGRATIONS_TRANSLATE
        | method::INTEGRATIONS_DETECT
        | method::INTEGRATIONS_LANGUAGES
        | method::INTEGRATIONS_GRAMMAR
        | method::INTEGRATIONS_ENRICH
        | method::INTEGRATIONS_TEST => MethodClass::LocalOnly,
        // A relayed request may not name the provider URL or the switches
        // (relayed_request_allowed); the engine uses its own.
        method::AI_MODELS
        | method::AI_TEST
        | method::AI_SUMMARIZE
        | method::AI_SUGGEST_REPLIES
        | method::AI_DRAFT_REPLY
        | method::AI_CANCEL
        | method::AI_USAGE
        | method::AI_TRIAGE_RUN
        | method::AI_TRIAGE_LIST => MethodClass::Open,
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
/// out, except GetMessageRFC2822Task without a `filepath` (file-id mode, see
/// [`relayed_task_allowed`]); so do the event and contact tasks until they have been
/// reviewed for the web.
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
    match task.get("__cls").and_then(Value::as_str) {
        Some("GetMessageRFC2822Task") => task
            .get("filepath")
            .is_none_or(|p| p.is_null() || p.as_str() == Some("")),
        Some(cls) => RELAYED_TASK_CLASSES.contains(&cls),
        None => false,
    }
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

/// Settings no relayed device may write, admin scope or not: the AI providers would make the
/// engine send mail to any URL a remote device names, and the switches would opt accounts in
/// (as for the `config` and `settings` params).
pub const LOCAL_ONLY_SETTINGS_PREFIX: &str = "core.ai.";

/// Whether a relayed device with `scopes` may write every key of `values` (the
/// `settings.set` params' `values`).
pub fn relayed_settings_allowed(values: &Map<String, Value>, scopes: &[String]) -> bool {
    if values
        .keys()
        .any(|k| k.starts_with(LOCAL_ONLY_SETTINGS_PREFIX))
    {
        return false;
    }
    is_admin(scopes)
        || values
            .keys()
            .all(|k| RELAYED_SETTINGS_KEYS.contains(&k.as_str()))
}

/// The `ai.*` params only a local client may send, admin scope or not: `config` would make
/// the engine send mail, and the key, to any URL a remote device names, hosts on the
/// user's network included, and `settings` would let it opt
/// accounts in.
const AI_LOCAL_PARAMS: &[&str] = &["config", "settings"];

/// Whether a relayed device with `scopes` may make the request `name` with `params`: the
/// method's class, and for `task.queue`, `settings.set`, `files.put` and `ai.*` what the
/// params ask for.
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
        method::ACCOUNT_UPDATE => {
            is_admin(scopes)
                || params
                    .get("patch")
                    .and_then(Value::as_object)
                    .is_some_and(|p| {
                        p.keys()
                            .all(|k| crate::ACCOUNT_DISPLAY_FIELDS.contains(&k.as_str()))
                    })
        }
        // Admin or not, a remote device never names a path for the engine to read. Params
        // that aren't an object are refused: positional params fill `path` too.
        method::FILES_PUT => params
            .as_object()
            .is_some_and(|p| p.get("path").is_none_or(Value::is_null)),
        // `attach` names paths too.
        method::DRAFT_CREATE | method::DRAFT_UPDATE => params.as_object().is_some_and(|p| {
            p.get("attach")
                .is_none_or(|a| a.is_null() || a.as_array().is_some_and(Vec::is_empty))
        }),
        // Positional params are refused for the same reason. Absent params read as `{}`,
        // as `Request::params_as` reads them.
        ai if ai.starts_with("ai.") => {
            params.is_null()
                || params.as_object().is_some_and(|p| {
                    AI_LOCAL_PARAMS
                        .iter()
                        .all(|k| p.get(*k).is_none_or(Value::is_null))
                })
        }
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
        for m in ["threads.snooze", "threads.unsnooze", "messages.unsubscribe"] {
            assert!(relayed_method_allowed(m, &[]), "{m}");
        }
        assert_eq!(method_class("account.add"), MethodClass::LocalOnly);
        assert_eq!(method_class("credentials.import"), MethodClass::ParentOnly);
        assert_eq!(method_class("test.panic"), MethodClass::Debug);
        assert_eq!(method_class("secrets.set"), MethodClass::LocalOnly);
        assert_eq!(method_class("Threads.List"), MethodClass::LocalOnly);
        let admin = vec![SCOPE_ADMIN.to_owned()];
        assert!(relayed_method_allowed("threads.list", &[]));
        assert!(relayed_method_allowed("settings.set", &[]));
        assert!(!relayed_method_allowed("secrets.set", &[]));
        assert!(relayed_method_allowed("secrets.set", &admin));
        assert!(relayed_method_allowed("account.add", &admin));
        assert!(!relayed_method_allowed("account.add", &[]));
        assert!(!relayed_method_allowed("credentials.import", &admin));
        assert!(!relayed_method_allowed("engine.shutdown", &admin));
        assert!(!relayed_method_allowed("test.crash", &admin));
        for m in ["files.get", "files.read", "files.put"] {
            assert_eq!(method_class(m), MethodClass::LocalOnly);
            assert!(!relayed_method_allowed(m, &[]));
            assert!(relayed_method_allowed(m, &admin));
        }
        for m in ["integrations.translate", "integrations.test"] {
            assert_eq!(method_class(m), MethodClass::LocalOnly);
            assert!(!relayed_method_allowed(m, &[]));
        }
        for m in ["ai.models", "ai.summarize", "ai.cancel", "ai.triage.list"] {
            assert_eq!(method_class(m), MethodClass::Open, "{m}");
        }
        assert_eq!(method_class("ai.other"), MethodClass::LocalOnly);
        for m in ["events.expand", "contacts.get", "contacts.export"] {
            assert_eq!(method_class(m), MethodClass::Open, "{m}");
        }
        for m in [
            "events.create",
            "events.update",
            "events.rsvp",
            "contacts.create",
            "contacts.update",
            "contacts.import",
        ] {
            assert!(!relayed_method_allowed(m, &[]), "{m}");
            assert!(relayed_method_allowed(m, &admin), "{m}");
        }
        let put = |p: Value| relayed_request_allowed("files.put", &p, &admin);
        assert!(put(json!({"accountId": "a", "name": "x", "data": ""})));
        assert!(put(json!({"accountId": "a", "name": "x", "path": null})));
        assert!(!put(
            json!({"accountId": "a", "name": "x", "path": "/etc/passwd"})
        ));
        assert!(!put(json!(["a", "x", null, null, "/etc/passwd"])));
        assert!(!put(json!(["a", "x", null, "aGk="])));
        assert!(!put(json!(null)));
        for m in ["draft.create", "draft.update"] {
            let ok = |p: Value| relayed_request_allowed(m, &p, &admin);
            assert!(ok(json!({"accountId": "a", "headerMessageId": "h"})), "{m}");
            assert!(ok(json!({"accountId": "a", "attach": []})), "{m}");
            assert!(
                !ok(json!({"accountId": "a", "attach": ["/etc/passwd"]})),
                "{m}"
            );
            assert!(
                !ok(json!({"accountId": "a", "attach": "/etc/passwd"})),
                "{m}"
            );
            assert!(!ok(json!(["a", "h", null, ["/etc/passwd"]])), "{m}");
        }
        assert!(relayed_request_allowed(
            "draft.delete",
            &json!({"accountId": "a", "headerMessageId": "h"}),
            &[]
        ));
    }

    #[test]
    fn relayed_ai_requests_never_carry_config_or_settings() {
        let admin = [SCOPE_ADMIN.to_owned()];
        for scopes in [&[][..], &admin[..]] {
            let ok = |p: Value| relayed_request_allowed("ai.summarize", &p, scopes);
            assert!(ok(
                json!({"accountId": "a", "threadId": "t", "provider": "p"})
            ));
            assert!(ok(
                json!({"accountId": "a", "threadId": "t", "config": null})
            ));
            assert!(!ok(json!({"accountId": "a", "threadId": "t",
                "config": {"id": "p", "kind": "openai", "url": "http://192.168.1.10/v1"}})));
            assert!(!ok(json!({"accountId": "a", "threadId": "t",
                "settings": {"accounts": ["a"]}})));
            assert!(!ok(json!(["a", "t"])));
            assert!(!relayed_request_allowed(
                "ai.test",
                &json!({"config": {"id": "p", "kind": "ollama"}}),
                scopes
            ));
            assert!(relayed_request_allowed(
                "ai.cancel",
                &json!({"streamId": "s"}),
                scopes
            ));
            assert!(relayed_request_allowed("ai.usage", &Value::Null, scopes));
        }
    }

    #[test]
    fn tasks() {
        let t = |cls: &str| json!({"__cls": cls, "filepath": "/x"});
        assert!(relayed_task_allowed(&t("ChangeStarredTask"), &[]));
        assert!(!relayed_task_allowed(&t("GetMessageRFC2822Task"), &[]));
        assert!(!relayed_task_allowed(&t("GetManyRFC2822Task"), &[]));
        let export = |extra: Value| {
            let mut v = json!({"__cls": "GetMessageRFC2822Task", "messageId": "m1"});
            v.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            v
        };
        assert!(relayed_task_allowed(&export(json!({})), &[]));
        assert!(relayed_task_allowed(
            &export(json!({"filepath": null})),
            &[]
        ));
        assert!(relayed_task_allowed(&export(json!({"filepath": ""})), &[]));
        assert!(!relayed_task_allowed(&export(json!({"filepath": 1})), &[]));
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
        for key in ["core.ai.providers", "core.ai.accounts", "core.ai.features"] {
            let p = set(json!({ key: [] }));
            assert!(
                !relayed_request_allowed("settings.set", &p, &admin),
                "{key}"
            );
        }
        let task = json!({"task": {"__cls": "GetManyRFC2822Task"}});
        assert!(!relayed_request_allowed("task.queue", &task, &[]));
        assert!(!relayed_request_allowed("secrets.set", &json!({}), &[]));
    }

    #[test]
    fn the_local_only_class() {
        let admin = vec![SCOPE_ADMIN.to_owned()];
        for m in [
            method::ACCOUNT_ADD,
            method::ACCOUNT_TEST,
            method::ACCOUNT_REMOVE,
            method::ACCOUNT_RESET,
            method::ACCOUNT_OAUTH_BEGIN,
            method::ACCOUNT_OAUTH_COMPLETE,
            method::ACCOUNT_SET_SECRETS,
            method::SECRETS_SET,
            method::SECRETS_DELETE,
            method::SECRETS_HAS,
            method::SERVER_SET,
            method::SERVER_CLEAR,
            method::ENGINE_LOGS_TAIL,
        ] {
            assert_eq!(method_class(m), MethodClass::LocalOnly, "{m}");
            assert!(!relayed_request_allowed(m, &json!({}), &[]), "{m}");
            assert!(relayed_request_allowed(m, &json!({}), &admin), "{m}");
        }
        assert!(relayed_request_allowed(
            method::ACCOUNT_DISCOVER,
            &json!({"email": "a@b.test"}),
            &[]
        ));
        let update = |patch: Value| json!({"accountId": "a", "patch": patch});
        assert!(relayed_request_allowed(
            method::ACCOUNT_UPDATE,
            &update(json!({"label": "Work", "aliases": []})),
            &[]
        ));
        for patch in [
            json!({"settings": {"imap_host": "evil.test"}}),
            json!({"label": "x", "provider": "imap"}),
            json!(null),
        ] {
            assert!(!relayed_request_allowed(
                method::ACCOUNT_UPDATE,
                &update(patch.clone()),
                &[]
            ));
            assert!(relayed_request_allowed(
                method::ACCOUNT_UPDATE,
                &update(patch),
                &admin
            ));
        }
        assert!(relayed_request_allowed(
            method::ACCOUNT_REORDER,
            &json!({"ids": []}),
            &[]
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
