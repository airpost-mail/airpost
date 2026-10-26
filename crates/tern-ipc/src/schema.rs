// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The protocol as one JSON Schema document (draft 2020-12): every method's params and
//! result, the notifications, the model and task JSON, and in `x-tern` the constants
//! (method names, features, error codes, settings keys, ...) with their doc comments.
//!
//! `cargo run -p tern-ipc --bin tern-ipc-schema` writes it to
//! `packages/client/schema/tern-ipc.json`, and the query allowlist ([`crate::query_attrs`])
//! to `crates/tern-store/src/model_query_allowlist.json`. `packages/client/scripts/generate.mjs`
//! turns the schema into `packages/client/src/protocol/*.ts`. All three outputs are checked
//! in; the tests below and CI fail when they drift from these types.
//!
//! `$defs.RpcMethods` maps each method to `{params, result}`, `$defs.RpcNotifications` each
//! notification to its params, `$defs.ModelJsonByClass` each `modelClass` to its JSON. A
//! method without params has `params: {"type": "null", "tsType": "undefined"}`.

use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::JsonSchema;
use serde_json::{json, Map, Value};

use crate::policy::{method_class, MethodClass};
use crate::*;

/// One method: its name, params (`None`: it takes none) and result.
struct MethodSchema {
    name: &'static str,
    params: Option<Value>,
    result: Value,
}

fn sub<T: JsonSchema>(g: &mut SchemaGenerator) -> Value {
    g.subschema_for::<T>().to_value()
}

fn call<P: JsonSchema, R: JsonSchema>(g: &mut SchemaGenerator, name: &'static str) -> MethodSchema {
    MethodSchema {
        name,
        params: Some(sub::<P>(g)),
        result: sub::<R>(g),
    }
}

fn call0<R: JsonSchema>(g: &mut SchemaGenerator, name: &'static str) -> MethodSchema {
    MethodSchema {
        name,
        params: None,
        result: sub::<R>(g),
    }
}

/// Every method a client can call, with its params and result types. Results the engine
/// builds without a type here are `Value` (`unknown` in TS).
fn methods(g: &mut SchemaGenerator) -> Vec<MethodSchema> {
    use method::*;
    vec![
        call::<HelloParams, HelloResult>(g, HELLO),
        call::<SubscribeParams, SubscribeResult>(g, SUBSCRIBE),
        call::<TaskQueueParams, TaskJson>(g, TASK_QUEUE),
        call::<TaskCancelParams, Empty>(g, TASK_CANCEL),
        call::<SyncWakeParams, Empty>(g, SYNC_WAKE),
        call::<BodiesFetchParams, Empty>(g, BODIES_FETCH),
        call::<CalendarSyncParams, CalendarSyncResult>(g, CALENDAR_SYNC),
        call0::<EngineStatusResult>(g, ENGINE_STATUS),
        call::<CategoriesListParams, FoldersListResult>(g, FOLDERS_LIST),
        call::<CategoriesListParams, LabelsListResult>(g, LABELS_LIST),
        call::<ThreadsListParams, ThreadsListResult>(g, THREADS_LIST),
        call::<ThreadsGetParams, ThreadsGetResult>(g, THREADS_GET),
        call::<MessagesGetParams, MessagesGetResult>(g, MESSAGES_GET),
        call::<SearchParams, SearchResult>(g, SEARCH),
        call::<ContactsSearchParams, ContactsSearchResult>(g, CONTACTS_SEARCH),
        call::<TasksGetParams, TasksGetResult>(g, TASKS_GET),
        call::<CountsGetParams, CountsGetResult>(g, COUNTS_GET),
        call::<TasksListParams, TasksListResult>(g, TASKS_LIST),
        call::<MetadataListParams, MetadataListResult>(g, METADATA_LIST),
        call::<QueryParams, QueryResult>(g, QUERY),
        call::<QuerySubscribeParams, QuerySubscribeResult>(g, QUERY_SUBSCRIBE),
        call::<QuerySetWindowParams, QueryWindowResult>(g, QUERY_SET_WINDOW),
        call::<SubIdParams, Empty>(g, QUERY_UNSUBSCRIBE),
        call::<FilesGetParams, FilesGetResult>(g, FILES_GET),
        call::<FilesReadParams, FilesReadResult>(g, FILES_READ),
        call::<FilesPutParams, FilesPutResult>(g, FILES_PUT),
        call::<ThreadsSnoozeParams, ThreadsSnoozeResult>(g, THREADS_SNOOZE),
        call::<ThreadsUnsnoozeParams, ThreadsSnoozeResult>(g, THREADS_UNSNOOZE),
        call::<ThreadsActionParams, IntentResult>(g, THREADS_ARCHIVE),
        call::<ThreadsActionParams, IntentResult>(g, THREADS_TRASH),
        call::<ThreadsActionParams, IntentResult>(g, THREADS_SPAM),
        call::<ThreadsActionParams, IntentResult>(g, THREADS_NOT_SPAM),
        call::<ThreadsMoveParams, IntentResult>(g, THREADS_MOVE),
        call::<ThreadsLabelParams, IntentResult>(g, THREADS_LABEL),
        call::<ThreadsMarkReadParams, IntentResult>(g, THREADS_MARK_READ),
        call::<ThreadsStarParams, IntentResult>(g, THREADS_STAR),
        call::<ThreadsRemoveFromViewParams, IntentResult>(g, THREADS_REMOVE_FROM_VIEW),
        call::<CategoryIdParams, IntentResult>(g, CATEGORIES_MARK_READ),
        call::<CategoriesCreateParams, IntentResult>(g, CATEGORIES_CREATE),
        call::<CategoriesRenameParams, IntentResult>(g, CATEGORIES_RENAME),
        call::<CategoryIdParams, IntentResult>(g, CATEGORIES_DELETE),
        call::<CategoryIdParams, IntentResult>(g, CATEGORIES_EMPTY),
        call::<UndoApplyParams, UndoApplyResult>(g, UNDO_APPLY),
        call::<UndoApplyParams, UndoApplyResult>(g, UNDO_REDO),
        call::<UndoListParams, UndoListResult>(g, UNDO_LIST),
        call::<UndoRecordParams, UndoRecordResult>(g, UNDO_RECORD),
        call::<MessagesUnsubscribeParams, MessagesUnsubscribeResult>(g, MESSAGES_UNSUBSCRIBE),
        call::<DraftCreateParams, MessageJson>(g, DRAFT_CREATE),
        call::<DraftSendParams, DraftSendResult>(g, DRAFT_SEND),
        call::<DraftCancelSendParams, DraftCancelSendResult>(g, DRAFT_CANCEL_SEND),
        call::<DraftValidateParams, DraftValidateResult>(g, DRAFT_VALIDATE),
        call::<DraftUpdateParams, MessageJson>(g, DRAFT_UPDATE),
        call::<DraftDeleteParams, DraftDeleteResult>(g, DRAFT_DELETE),
        call0::<TemplatesListResult>(g, TEMPLATES_LIST),
        call::<TemplateNameParams, TemplateResult>(g, TEMPLATES_GET),
        call::<TemplateSaveParams, TemplateResult>(g, TEMPLATES_SAVE),
        call::<TemplateNameParams, Empty>(g, TEMPLATES_DELETE),
        call::<EventsExpandParams, EventsExpandResult>(g, EVENTS_EXPAND),
        call::<EventsSubscribeParams, EventsSubscribeResult>(g, EVENTS_SUBSCRIBE),
        call::<SubIdParams, Empty>(g, EVENTS_UNSUBSCRIBE),
        call::<EventsCreateParams, EventsWriteResult>(g, EVENTS_CREATE),
        call::<EventsUpdateParams, EventsWriteResult>(g, EVENTS_UPDATE),
        call::<EventsRsvpParams, EventsRsvpResult>(g, EVENTS_RSVP),
        call::<ContactsGetParams, ContactDetails>(g, CONTACTS_GET),
        call::<ContactsCreateParams, ContactDetails>(g, CONTACTS_CREATE),
        call::<ContactsUpdateParams, ContactDetails>(g, CONTACTS_UPDATE),
        call::<ContactsImportParams, ContactsImportResult>(g, CONTACTS_IMPORT),
        call::<ContactsExportParams, ContactsExportResult>(g, CONTACTS_EXPORT),
        call0::<AccountListResult>(g, ACCOUNT_LIST),
        call::<AccountAddParams, AccountAddResult>(g, ACCOUNT_ADD),
        call::<AccountRemoveParams, AccountRemoveResult>(g, ACCOUNT_REMOVE),
        call::<AccountDiscoverParams, AccountDiscoverResult>(g, ACCOUNT_DISCOVER),
        call::<OAuthBeginParams, OAuthBeginResult>(g, ACCOUNT_OAUTH_BEGIN),
        call::<OAuthCompleteParams, OAuthCompleteResult>(g, ACCOUNT_OAUTH_COMPLETE),
        call::<AccountSetSecretsParams, AccountAddResult>(g, ACCOUNT_SET_SECRETS),
        call::<AccountUpdateParams, AccountUpdateResult>(g, ACCOUNT_UPDATE),
        call::<AccountReorderParams, Empty>(g, ACCOUNT_REORDER),
        call::<SecretsSetParams, Empty>(g, SECRETS_SET),
        call::<SecretsDeleteParams, Empty>(g, SECRETS_DELETE),
        call::<SecretsHasParams, SecretsHasResult>(g, SECRETS_HAS),
        call::<ServerSetParams, ServerSetResult>(g, SERVER_SET),
        call0::<ServerSetResult>(g, SERVER_CLEAR),
        call0::<Empty>(g, ENGINE_SHUTDOWN),
        call::<CredentialsImportParams, CredentialsImportResult>(g, CREDENTIALS_IMPORT),
        call::<AccountTestParams, AccountTestResult>(g, ACCOUNT_TEST),
        call::<AccountResetParams, Empty>(g, ACCOUNT_RESET),
        call::<ResetDatabaseParams, ResetDatabaseResult>(g, ENGINE_RESET_DATABASE),
        call::<SetLogLevelParams, SetLogLevelResult>(g, ENGINE_SET_LOG_LEVEL),
        call::<LogsTailParams, LogsTailResult>(g, ENGINE_LOGS_TAIL),
        call::<SettingsGetParams, SettingsValues>(g, SETTINGS_GET),
        call::<SettingsValues, Empty>(g, SETTINGS_SET),
        call::<AccountIdParams, Empty>(g, TEST_CRASH),
        call::<AccountIdParams, Empty>(g, TEST_PANIC),
        call::<IntegrationParams, Value>(g, INTEGRATIONS_TRANSLATE),
        call::<IntegrationParams, Value>(g, INTEGRATIONS_DETECT),
        call::<IntegrationParams, Value>(g, INTEGRATIONS_LANGUAGES),
        call::<IntegrationParams, Value>(g, INTEGRATIONS_GRAMMAR),
        call::<IntegrationParams, Value>(g, INTEGRATIONS_ENRICH),
        call::<IntegrationParams, IntegrationTestReport>(g, INTEGRATIONS_TEST),
        call::<AiModelsParams, Vec<AiModelInfo>>(g, AI_MODELS),
        call::<AiProviderParams, AiTestReport>(g, AI_TEST),
        call::<AiSummarizeParams, AiCompletion>(g, AI_SUMMARIZE),
        call::<AiSuggestRepliesParams, Vec<AiSuggestion>>(g, AI_SUGGEST_REPLIES),
        call::<AiDraftReplyParams, AiCompletion>(g, AI_DRAFT_REPLY),
        call::<AiCancelParams, Empty>(g, AI_CANCEL),
        call::<AiUsageParams, Vec<AiUsageRow>>(g, AI_USAGE),
        call::<AiTriageRunParams, AiTriageRunResult>(g, AI_TRIAGE_RUN),
        call::<AiTriageListParams, AiTriageListResult>(g, AI_TRIAGE_LIST),
        call::<AiTriageSubscribeParams, AiTriageSubscribeResult>(g, AI_TRIAGE_SUBSCRIBE),
        call::<SubIdParams, Empty>(g, AI_TRIAGE_UNSUBSCRIBE),
        call::<RulesReprocessParams, RulesReprocessResult>(g, RULES_REPROCESS),
        call::<RulesCancelParams, RulesCancelResult>(g, RULES_CANCEL),
    ]
}

/// Every notification the engine sends, with its params.
fn notifications(g: &mut SchemaGenerator) -> Vec<(&'static str, Value)> {
    use method::*;
    vec![
        (DELTA, sub::<DeltaParams>(g)),
        (QUERY_CHANGED, sub::<QueryChangedParams>(g)),
        (EVENTS_CHANGED, sub::<EventsChangedParams>(g)),
        (TASK_DONE, sub::<TaskDoneParams>(g)),
        (UNDO_CHANGED, sub::<UndoChangedParams>(g)),
        (AI_TRIAGE_CHANGED, sub::<AiTriageChangedParams>(g)),
        (DISCONNECT, sub::<DisconnectParams>(g)),
        (ACCOUNT_STATE, sub::<AccountStateParams>(g)),
        (ACCOUNT_CHANGED, sub::<AccountChangedParams>(g)),
        (MAIL_NEW, sub::<MailNewParams>(g)),
        (SETTINGS_CHANGED, sub::<SettingsValues>(g)),
        (CAPABILITIES_CHANGED, sub::<Capabilities>(g)),
        (AI_DELTA, sub::<AiDeltaParams>(g)),
        (AI_TRIAGE_PROGRESS, sub::<AiTriageProgressParams>(g)),
        (RULES_PROGRESS, sub::<RulesProgressParams>(g)),
        (RULES_DISABLED, sub::<RulesDisabledParams>(g)),
    ]
}

/// The JSON of each `delta.modelClass`.
fn model_classes(g: &mut SchemaGenerator) -> Vec<(&'static str, Value)> {
    vec![
        ("Thread", sub::<ThreadJson>(g)),
        ("Message", sub::<MessageJson>(g)),
        ("Folder", sub::<FolderJson>(g)),
        ("Label", sub::<LabelJson>(g)),
        ("Contact", sub::<ContactJson>(g)),
        ("ContactGroup", sub::<ContactGroupJson>(g)),
        ("ContactBook", sub::<ContactBookJson>(g)),
        ("Calendar", sub::<CalendarJson>(g)),
        ("Event", sub::<EventJson>(g)),
        ("File", sub::<FileJson>(g)),
        ("Task", sub::<TaskJson>(g)),
        ("ThreadCounts", sub::<ThreadCountsJson>(g)),
        ("ProcessState", sub::<ProcessStateJson>(g)),
        // Carries no models.
        (
            PROCESS_IDENTITY_REFRESH_NEEDED,
            json!({"not": {}, "tsType": "never"}),
        ),
    ]
}

/// Types no method names but clients read: the `data` of `AiFailed` and `IntegrationFailed`.
fn error_data(g: &mut SchemaGenerator) {
    sub::<ProviderErrorJson>(g);
}

fn named<T: Clone + Into<Value>>(items: &[(&str, T, &str)]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|(name, value, doc)| {
                let mut o = Map::new();
                o.insert("name".into(), json!(name));
                o.insert("value".into(), value.clone().into());
                let doc = doc_text(doc);
                if !doc.is_empty() {
                    o.insert("description".into(), json!(doc));
                }
                Value::Object(o)
            })
            .collect(),
    )
}

/// The constant groups clients mirror, by the module they're in.
fn groups() -> Value {
    json!({
        "method": named(method::NAMED),
        "feature": named(feature::NAMED),
        "code": named(code::NAMED),
        "account_state": named(account_state::NAMED),
        "capability_reason": named(capability_reason::NAMED),
        "integration_kind": named(integration_kind::NAMED),
        "server_feature": named(server_feature::NAMED),
        "ai_feature": named(ai_feature::NAMED),
        "unsubscribe_method": named(unsubscribe_method::NAMED),
        "draft_kind": named(draft_kind::NAMED),
        "task_status": named(task_status::NAMED),
        "placement_flag": named(placement_flag::NAMED),
        "undo_state": named(undo_state::NAMED),
        "undo_action": named(undo_action::NAMED),
    })
}

fn constants() -> Value {
    json!({
        "PROTOCOL_VERSION": PROTOCOL_VERSION,
        "MAX_LINE_BYTES": MAX_LINE_BYTES,
        "READ_DEFAULT_LIMIT": READ_DEFAULT_LIMIT,
        "READ_MAX_LIMIT": READ_MAX_LIMIT,
        "REPLY_MAX_BYTES": REPLY_MAX_BYTES,
        "READ_TIMEOUT_MS": READ_TIMEOUT_MS,
        "LOGS_TAIL_MAX_BYTES": LOGS_TAIL_MAX_BYTES,
        "LOGS_TAIL_DEFAULT_BYTES": LOGS_TAIL_DEFAULT_BYTES,
        "FILES_READ_MAX_BYTES": FILES_READ_MAX_BYTES,
        "DRAFT_ATTACHMENTS_MAX_BYTES": DRAFT_ATTACHMENTS_MAX_BYTES,
        "AI_TRIAGE_RUN_MAX": AI_TRIAGE_RUN_MAX,
        "EVENTS_EXPAND_MAX_OCCURRENCES": EVENTS_EXPAND_MAX_OCCURRENCES,
        "UNDO_LOG_MAX_ENTRIES": UNDO_LOG_MAX_ENTRIES,
        "UNDO_LOG_MAX_SECS": UNDO_LOG_MAX_SECS,
        "THREAD_COUNTS_CLASS": "ThreadCounts",
        "CREDENTIALS_PER_REQUEST": CREDENTIALS_PER_REQUEST,
        "SCOPE_ADMIN": policy::SCOPE_ADMIN,
        "MAIL_RULES_KEY": MAIL_RULES_KEY,
        "MAIL_RULES_AUTO_SINCE_KEY": MAIL_RULES_AUTO_SINCE_KEY,
    })
}

fn settings_keys() -> Value {
    let kind = |k: SettingKind| match k {
        SettingKind::Bool => "boolean",
        SettingKind::Count => "count",
        SettingKind::String => "string",
        SettingKind::Object => "object",
        SettingKind::Array => "array",
    };
    Value::Object(
        SETTINGS_KEYS
            .iter()
            .map(|(key, k)| ((*key).to_owned(), json!(kind(*k))))
            .collect(),
    )
}

fn class_name(c: MethodClass) -> &'static str {
    match c {
        MethodClass::Open => "open",
        MethodClass::LocalOnly => "local-only",
        MethodClass::ParentOnly => "parent-only",
        MethodClass::Debug => "debug",
    }
}

fn doc_of(name: &str) -> Option<String> {
    method::NAMED
        .iter()
        .find(|(_, value, _)| *value == name)
        .map(|(_, _, doc)| doc_text(doc))
        .filter(|d| !d.is_empty())
}

/// The whole document.
pub fn protocol() -> Value {
    let mut g = SchemaSettings::draft2020_12().into_generator();

    let mut rpc_methods = Map::new();
    let mut classes = Map::new();
    let mut params_defs = Vec::new();
    for m in methods(&mut g) {
        if let Some(r) = m.params.as_ref().and_then(|p| p["$ref"].as_str()) {
            params_defs.push(r.trim_start_matches("#/$defs/").to_owned());
        }
        let params = m
            .params
            .unwrap_or_else(|| json!({"type": "null", "tsType": "undefined"}));
        let mut entry = json!({
            "type": "object",
            "properties": {"params": params, "result": m.result},
            "required": ["params", "result"],
            "additionalProperties": false,
        });
        if let Some(doc) = doc_of(m.name) {
            entry["description"] = json!(doc);
        }
        rpc_methods.insert(m.name.into(), entry);
        classes.insert(m.name.into(), json!(class_name(method_class(m.name))));
    }
    let mut rpc_notifications = Map::new();
    for (name, mut params) in notifications(&mut g) {
        if let (Some(doc), Some(o)) = (doc_of(name), params.as_object_mut()) {
            // A `$ref` with a sibling description keeps the referenced type's name.
            o.insert("description".into(), json!(doc));
        }
        rpc_notifications.insert(name.into(), params);
    }
    let mut by_class = Map::new();
    for (name, schema) in model_classes(&mut g) {
        by_class.insert(name.into(), schema);
    }
    error_data(&mut g);

    let mut defs = g.take_definitions(true);
    // The same types as the engine writes them: a field it leaves out when `None`
    // (`skip_serializing_if`) is optional there, one it writes as `null` is required.
    let mut sg = SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator();
    methods(&mut sg);
    notifications(&mut sg);
    model_classes(&mut sg);
    error_data(&mut sg);
    let written = sg.take_definitions(true);
    for (name, def) in defs.iter_mut() {
        // A method's params keep `null`, which the engine reads as absent: `accounts: null`.
        let written = written.get(name).filter(|_| !params_defs.contains(name));
        absent_rather_than_null(def, written);
    }
    let object = |title: &str, description: &str, properties: Map<String, Value>| {
        let required: Vec<&String> = properties.keys().collect();
        json!({
            "title": title,
            "description": description,
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    };
    defs.insert(
        "RpcMethods".into(),
        object(
            "RpcMethods",
            "Each method's params and result.",
            rpc_methods,
        ),
    );
    defs.insert(
        "RpcNotifications".into(),
        object(
            "RpcNotifications",
            "Each notification's params.",
            rpc_notifications,
        ),
    );
    defs.insert(
        "ModelJsonByClass".into(),
        object(
            "ModelJsonByClass",
            "The JSON of each `modelClass` a delta carries.",
            by_class,
        ),
    );
    for (name, def) in defs.iter_mut() {
        if let Some(o) = def.as_object_mut() {
            o.entry("title").or_insert_with(|| json!(name));
        }
    }
    let defs: Map<String, Value> = {
        let mut sorted: Vec<_> = defs.into_iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        sorted.into_iter().collect()
    };

    let mut doc = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "tern-ipc",
        "description": "The Tern engine's socket protocol, generated from crates/tern-ipc by `cargo run -p tern-ipc --bin tern-ipc-schema`. Do not edit.",
        "x-tern": {
            "protocolVersion": PROTOCOL_VERSION,
            "constants": constants(),
            "groups": groups(),
            "settingsKeys": settings_keys(),
            "methodClasses": classes,
        },
        "$defs": defs,
    });
    clean_descriptions(&mut doc);
    doc
}

fn required(def: &Value) -> Vec<&str> {
    def["required"]
        .as_array()
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// An `Option` field the engine leaves out when `None` is typed as absent only: `?: T`, not
/// `?: T | null`. A field the engine writes as `null` keeps it.
/// `written` is the def as the engine writes it; `None` leaves every `null` in place.
fn absent_rather_than_null(def: &mut Value, written: Option<&Value>) {
    let read_req: Vec<String> = required(def).into_iter().map(String::from).collect();
    let written_req = written.map(required);
    let Some(props) = def.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    for (key, prop) in props.iter_mut() {
        // `x-null`: `null` means something there (a `double_option`, a patch's "remove").
        if prop
            .as_object_mut()
            .and_then(|o| o.remove("x-null"))
            .is_some()
        {
            continue;
        }
        let Some(written_req) = &written_req else {
            continue;
        };
        if !read_req.iter().any(|r| r == key) && !written_req.contains(&key.as_str()) {
            strip_null(prop);
        }
    }
}

/// `T | null` as `T`: `type: [t, "null"]` (and a `null` in its `enum`) or
/// `anyOf: [T, {type: null}]`.
fn strip_null(prop: &mut Value) {
    let Some(o) = prop.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(types)) = o.get_mut("type") {
        types.retain(|t| t != "null");
        if types.len() == 1 {
            let t = types[0].clone();
            o.insert("type".into(), t);
        }
        if let Some(Value::Array(e)) = o.get_mut("enum") {
            e.retain(|v| !v.is_null());
        }
        return;
    }
    let Some(Value::Array(any)) = o.get("anyOf") else {
        return;
    };
    let rest: Vec<Value> = any
        .iter()
        .filter(|s| s.get("type") != Some(&json!("null")))
        .cloned()
        .collect();
    if rest.len() == any.len() {
        return;
    }
    o.remove("anyOf");
    match <[Value; 1]>::try_from(rest) {
        Ok([Value::Object(only)]) => {
            for (k, v) in only {
                o.entry(k).or_insert(v);
            }
        }
        Ok([other]) => {
            o.insert("allOf".into(), json!([other]));
        }
        Err(rest) => {
            o.insert("anyOf".into(), Value::Array(rest));
        }
    }
}

/// The document as the checked-in file holds it.
pub fn protocol_text() -> String {
    let mut text = serde_json::to_string_pretty(&protocol()).expect("the schema serialises");
    text.push('\n');
    text
}

/// A doc comment as `named_consts!` collects it (a leading space on each line) as text.
fn doc_text(doc: &str) -> String {
    let lines: Vec<&str> = doc
        .trim_end()
        .lines()
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    clean_doc(&lines.join("\n"))
}

/// Rustdoc links as plain text: ``[`x`](path)`` and ``[`x`]`` become `` `x` ``.
fn clean_doc(doc: &str) -> String {
    let mut out = String::with_capacity(doc.len());
    let mut rest = doc;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(close) = after.find(']') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let text = &after[..close];
        let tail = &after[close + 1..];
        // Only link-shaped brackets: `[`x`]` or `[text](path)`.
        if let Some(path_tail) = tail.strip_prefix('(') {
            if let Some(end) = path_tail.find(')') {
                out.push_str(text);
                rest = &path_tail[end + 1..];
                continue;
            }
        }
        if text.starts_with('`') && text.ends_with('`') && text.len() > 1 {
            out.push_str(text);
        } else {
            out.push('[');
            out.push_str(text);
            out.push(']');
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

fn clean_descriptions(v: &mut Value) {
    match v {
        Value::Object(o) => {
            for (k, child) in o.iter_mut() {
                match child {
                    Value::String(s) if k == "description" => *s = clean_doc(s),
                    _ => clean_descriptions(child),
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(clean_descriptions),
        _ => {}
    }
}

/// Where `value` breaks the schema of `$defs[def]`: keys the schema doesn't list, missing
/// required keys, wrong JSON types, consts and enums. Covers the subset of JSON Schema this
/// document uses (`$ref`, `properties`, `required`, `additionalProperties`, `items`, `anyOf`,
/// `oneOf`, `type`, `const`, `enum`), for tests that check real engine output against it.
pub fn violations(value: &Value, def: &str) -> Vec<String> {
    static DOC: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let doc = DOC.get_or_init(protocol);
    let mut out = Vec::new();
    check(
        &doc["$defs"],
        &json!({"$ref": format!("#/$defs/{def}")}),
        value,
        def,
        &mut out,
    );
    out
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "null" => v.is_null(),
        "boolean" => v.is_boolean(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "string" => v.is_string(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        _ => true,
    }
}

fn check(defs: &Value, schema: &Value, v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(s) = schema.as_object() else {
        // `true`: anything.
        return;
    };
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        let name = r.trim_start_matches("#/$defs/");
        check(defs, &defs[name], v, path, out);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(options) = s.get(key).and_then(Value::as_array) {
            let fits = options.iter().any(|o| {
                let mut errs = Vec::new();
                check(defs, o, v, path, &mut errs);
                errs.is_empty()
            });
            if !fits {
                // Report against the option whose `__cls` matches, else the first.
                let pick = options
                    .iter()
                    .find(|o| {
                        let target = o.get("$ref").and_then(Value::as_str).unwrap_or("");
                        let def = &defs[target.trim_start_matches("#/$defs/")];
                        def["properties"]["__cls"]["const"] == v["__cls"]
                    })
                    .or(options.first());
                match pick {
                    Some(o) => check(defs, o, v, path, out),
                    None => out.push(format!("{path}: fits none of {key}")),
                }
            }
        }
    }
    if let Some(t) = s.get("type") {
        let ok = match t {
            Value::String(t) => type_matches(t, v),
            Value::Array(ts) => ts
                .iter()
                .filter_map(Value::as_str)
                .any(|t| type_matches(t, v)),
            _ => true,
        };
        if !ok {
            out.push(format!("{path}: {v} is not {t}"));
            return;
        }
    }
    if let Some(c) = s.get("const") {
        if c != v {
            out.push(format!("{path}: {v} is not {c}"));
        }
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        if !e.contains(v) {
            out.push(format!(
                "{path}: {v} is not one of {}",
                Value::Array(e.clone())
            ));
        }
    }
    if let (Some(items), Some(a)) = (s.get("items"), v.as_array()) {
        for (i, item) in a.iter().enumerate() {
            check(defs, items, item, &format!("{path}[{i}]"), out);
        }
    }
    if let Some(o) = v.as_object() {
        let props = s.get("properties").and_then(Value::as_object);
        if let Some(required) = s.get("required").and_then(Value::as_array) {
            for k in required.iter().filter_map(Value::as_str) {
                if !o.contains_key(k) {
                    out.push(format!("{path}: missing {k}"));
                }
            }
        }
        let extra = s.get("additionalProperties");
        for (k, child) in o {
            let child_path = format!("{path}.{k}");
            match props.and_then(|p| p.get(k)) {
                Some(p) => check(defs, p, child, &child_path, out),
                None => match extra {
                    Some(Value::Bool(true)) => {}
                    Some(extra @ Value::Object(_)) => check(defs, extra, child, &child_path, out),
                    // Unlisted keys: a key the struct doesn't name. Objects whose schema
                    // lists nothing (a `Map`, `Value`) take any.
                    _ if props.is_some() => out.push(format!("{child_path}: not in the schema")),
                    _ => {}
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_a_method_or_a_notification() {
        let mut g = SchemaSettings::draft2020_12().into_generator();
        let mut seen: Vec<&str> = methods(&mut g).iter().map(|m| m.name).collect();
        seen.extend(notifications(&mut g).iter().map(|(n, _)| *n));
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), seen.len(), "a name is listed twice");
        let mut names: Vec<&str> = method::NAMED.iter().map(|(_, v, _)| *v).collect();
        names.sort();
        assert_eq!(sorted, names);
    }

    #[test]
    fn violations_name_unknown_keys_and_wrong_types() {
        let folder = json!({"id": "f", "aid": "a", "v": 1, "__cls": "Folder", "path": "INBOX",
                            "role": "inbox"});
        assert_eq!(violations(&folder, "FolderJson"), Vec::<String>::new());
        let mut odd = folder.clone();
        odd["zzz"] = json!(1);
        odd["v"] = json!("1");
        odd["__cls"] = json!("Label");
        odd.as_object_mut().unwrap().remove("path");
        let mut found = violations(&odd, "FolderJson");
        found.sort();
        assert_eq!(
            found,
            [
                "FolderJson.__cls: \"Label\" is not \"Folder\"",
                "FolderJson.v: \"1\" is not \"integer\"",
                "FolderJson.zzz: not in the schema",
                "FolderJson: missing path",
            ]
        );
        // Nested, through a `$ref` and an array.
        let thread = json!({"id": "t", "aid": "a", "v": 1, "__cls": "Thread", "subject": "",
            "unread": 0, "starred": 0, "lmt": 0, "fmt": 0, "lmst": 0, "lmrt": 0,
            "inAllMail": true, "attachmentCount": 0, "labels": [], "participants": [],
            "folders": [{"id": "f", "aid": "a", "__cls": "Folder", "path": "", "role": "",
                         "_refs": 1, "_u": 0, "extra": true}]});
        assert_eq!(
            violations(&thread, "ThreadJson"),
            ["ThreadJson.folders[0].extra: not in the schema"]
        );
    }

    #[test]
    fn task_status_lists_the_constants() {
        let statuses: Vec<&str> = task_status::NAMED.iter().map(|(_, v, _)| *v).collect();
        assert_eq!(
            protocol()["$defs"]["ChangeUnreadTaskJson"]["properties"]["status"]["enum"],
            json!(statuses)
        );
    }

    #[test]
    fn a_skipped_none_is_absent_and_a_written_one_null() {
        let doc = protocol();
        let props = |def: &str| doc["$defs"][def]["properties"].clone();
        // `skip_serializing_if`: absent, never null.
        assert_eq!(props("AccountJson")["name"]["type"], "string");
        // Written as null.
        assert_eq!(
            props("ThreadsListResult")["next"]["type"],
            json!(["string", "null"])
        );
        // `x-null`.
        assert_eq!(
            props("QueryResult")["nextCursor"]["type"],
            json!(["string", "null"])
        );
        assert_eq!(
            props("EventPatch")["recurrenceRule"]["type"],
            json!(["string", "null"])
        );
        // A method's params keep null.
        assert_eq!(
            props("QueryParams")["accounts"]["type"],
            json!(["array", "null"])
        );
        assert!(!protocol_text().contains("x-null"));
    }

    #[test]
    fn docs_lose_their_links() {
        assert_eq!(
            clean_doc("see [`feature::COUNTS`](crate::feature::COUNTS) and [`Client`], [x]"),
            "see `feature::COUNTS` and `Client`, [x]"
        );
    }

    #[test]
    fn the_checked_in_schema_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../packages/client/schema/tern-ipc.json"
        );
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == protocol_text(),
            "packages/client/schema/tern-ipc.json is out of date: run \
             `cargo run -p tern-ipc --bin tern-ipc-schema`, then \
             `npm run generate` in packages/client"
        );
    }
}
