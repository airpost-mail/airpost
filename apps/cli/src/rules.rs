// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern rules list|show|add|update|remove|enable|disable|reprocess`: the `mailRules` setting,
//! which the desktop's Preferences > Mail Rules edits and the engine runs on arriving mail
//! (`tern_tasks::mail_rules`), and `rules.reprocess` with its `rules.progress`.
//!
//! A rule is stored in the desktop's `MailRule` shape: `{id, accountId, name, conditionMode,
//! conditions: [{templateKey, comparatorKey, value}], actions: [{templateKey, value?,
//! valueName?}], disabled, disabledReason?}`. Enum conditions (`hasAttachment`, `starred`)
//! carry no `comparatorKey`, as the scenario editor's `coerceInstance` leaves them; folder and
//! label actions carry the category's id and its display name, which the engine falls back to
//! when the id changes. Every rule written is checked against the templates
//! `mail-rules-templates.ts` offers the account, with expressions compiled as the engine
//! compiles them, so nothing is saved that the engine would disable on first use.

use std::io::{IsTerminal, Read};
use std::time::{Duration, Instant};

use regex::RegexBuilder;
use serde_json::{json, Map, Value};
use tern_client::tasks::{is_final, TASK_CLASS};
use tern_ipc::{
    feature, method, DeltaParams, RulesProgressParams, RulesReprocessResult, SubscribeParams,
};

use crate::app::Ctx;
use crate::cli::{RuleArgs, RulesCommand};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::organize::{by_role, find_named, Categories};
use crate::output::{self, Table};
use crate::settings;
use crate::tasks::describe_error;

const RULES_KEY: &str = "mailRules";
/// The `source` of the tasks the engine queues for rules.
const RULES_SOURCE: &str = "Mail Rules";
/// `MailRulesStore._onAddMailRule`'s default name.
const DEFAULT_NAME: &str = "Untitled Rule";
/// As `tern_tasks::mail_rules`: a bigger compiled expression is refused.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// With `reprocess --wait`, how long after the run's end (or the last task delta) the
/// command still listens for the tasks the rules queued.
const TASK_GRACE: Duration = Duration::from_millis(1500);

/// `ConditionTemplates` of type String: key, the word `--if` takes, other words for it.
const STRING_FIELDS: &[(&str, &str, &[&str])] = &[
    ("from", "from", &[]),
    ("to", "to", &[]),
    ("cc", "cc", &[]),
    ("bcc", "bcc", &[]),
    ("anyRecipient", "recipient", &["anyrecipient"]),
    ("replyTo", "reply-to", &[]),
    (
        "anyAttachmentName",
        "attachment-name",
        &["attachment", "anyattachmentname"],
    ),
    ("subject", "subject", &[]),
    ("body", "body", &[]),
];
/// `ConditionTemplates` of type Enum, whose values are `true` and `false`.
const ENUM_FIELDS: &[(&str, &str, &[&str])] = &[
    ("hasAttachment", "has-attachment", &["hasattachments"]),
    ("starred", "starred", &["isstarred"]),
];
/// `Comparators.String`.
const COMPARATORS: &[(&str, &str, &[&str])] = &[
    ("contains", "contains", &["has"]),
    (
        "doesNotContain",
        "not-contains",
        &["doesnotcontain", "notcontain"],
    ),
    ("beginsWith", "begins-with", &["startswith"]),
    ("endsWith", "ends-with", &[]),
    ("equals", "equals", &["is"]),
    (
        "matchesExpression",
        "matches",
        &["matchesexpression", "regex"],
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionValue {
    None,
    Address,
    Folder,
    Label,
}

/// Which accounts `ActionTemplatesForAccount` offers an action to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Offered {
    All,
    Gmail,
    Folders,
}

/// `ActionTemplates`: key, value, accounts, the word `--then` takes, other words for it.
const ACTIONS: &[(&str, ActionValue, Offered, &str, &[&str])] = &[
    (
        "markAsRead",
        ActionValue::None,
        Offered::All,
        "mark-read",
        &["read"],
    ),
    ("moveToTrash", ActionValue::None, Offered::All, "trash", &[]),
    ("star", ActionValue::None, Offered::All, "star", &[]),
    (
        "forward",
        ActionValue::Address,
        Offered::All,
        "forward",
        &[],
    ),
    (
        "changeFolder",
        ActionValue::Folder,
        Offered::Folders,
        "move",
        &[],
    ),
    (
        "moveToLabel",
        ActionValue::Label,
        Offered::Gmail,
        "move",
        &[],
    ),
    (
        "applyLabel",
        ActionValue::Label,
        Offered::Gmail,
        "label",
        &[],
    ),
    ("archive", ActionValue::None, Offered::Gmail, "archive", &[]),
    (
        "markAsImportant",
        ActionValue::None,
        Offered::Gmail,
        "important",
        &["markimportant"],
    ),
];

const RULE_KEYS: &[&str] = &[
    "id",
    "accountId",
    "name",
    "conditionMode",
    "conditions",
    "actions",
    "disabled",
    "disabledReason",
];
const CONDITION_KEYS: &[&str] = &["templateKey", "comparatorKey", "value", "valueName"];
const ACTION_KEYS: &[&str] = &["templateKey", "comparatorKey", "value", "valueName"];

/// A word as the tables compare it: lowercase, without `-`, `_` and spaces.
fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn lookup(table: &[(&'static str, &'static str, &[&str])], word: &str) -> Option<&'static str> {
    let w = norm(word);
    table
        .iter()
        .find(|(key, flag, more)| norm(key) == w || norm(flag) == w || more.contains(&w.as_str()))
        .map(|(key, _, _)| *key)
}

fn flag_of(table: &[(&'static str, &'static str, &[&str])], key: &str) -> String {
    table
        .iter()
        .find(|(k, _, _)| *k == key)
        .map_or_else(|| key.to_owned(), |(_, f, _)| (*f).to_owned())
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

/// The first word of `s` and the rest after it.
fn first_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim_start()),
        None => (s, ""),
    }
}

/// A value without one pair of surrounding quotes.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    for q in ['\'', '"'] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

fn enum_value(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "" | "true" | "yes" => Some("true"),
        "false" | "no" => Some("false"),
        _ => None,
    }
}

/// `--if FIELD COMPARATOR VALUE` (or `has-attachment|starred [true|false]`) as a condition.
fn parse_condition(text: &str) -> Result<Value> {
    let bad = |why: String| CliError::usage(format!("--if {text:?}: {why}"));
    let (field, rest) = first_word(text);
    if let Some(key) = lookup(ENUM_FIELDS, field) {
        let rest = rest.strip_prefix("is ").unwrap_or(rest);
        let value = enum_value(unquote(rest))
            .ok_or_else(|| bad(format!("{field} takes true or false, not {rest:?}")))?;
        return Ok(json!({ "templateKey": key, "value": value }));
    }
    let key = lookup(STRING_FIELDS, field).ok_or_else(|| {
        bad(format!(
            "no field {field:?}; the fields are {}, {}",
            STRING_FIELDS
                .iter()
                .map(|(_, f, _)| *f)
                .collect::<Vec<_>>()
                .join(", "),
            ENUM_FIELDS
                .iter()
                .map(|(_, f, _)| *f)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    // `does not contain`, `begins with`: the longest run of words that is a comparator.
    let mut words: Vec<(&str, &str)> = Vec::new();
    let mut rest_of = rest;
    for _ in 0..3 {
        let (w, r) = first_word(rest_of);
        if w.is_empty() {
            break;
        }
        words.push((w, r));
        rest_of = r;
    }
    let found = (1..=words.len()).rev().find_map(|n| {
        let joined: String = words[..n].iter().map(|(w, _)| *w).collect();
        lookup(COMPARATORS, &joined).map(|c| (c, words[n - 1].1))
    });
    let (comparator, value) = found.ok_or_else(|| {
        bad(format!(
            "give a comparator after {field}: {}",
            COMPARATORS
                .iter()
                .map(|(_, f, _)| *f)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    Ok(json!({ "templateKey": key, "comparatorKey": comparator, "value": unquote(value) }))
}

/// `--then ACTION [VALUE]` as an action, its folder or label still by name. `move` is
/// `changeFolder`, or `moveToLabel` on Gmail.
fn parse_action(text: &str, gmail: bool) -> Result<Value> {
    let (verb, rest) = first_word(text);
    let rest = unquote(rest);
    let w = norm(verb);
    let key = if w == "move" {
        if gmail {
            "moveToLabel"
        } else {
            "changeFolder"
        }
    } else {
        ACTIONS
            .iter()
            .find(|(key, _, _, flag, more)| {
                norm(key) == w || norm(flag) == w || more.contains(&w.as_str())
            })
            .map(|(key, ..)| *key)
            .ok_or_else(|| {
                CliError::usage(format!(
                    "--then {text:?}: no action {verb:?}; the actions are mark-read, star, \
                     trash, forward ADDRESS, move FOLDER, and on Gmail label LABEL, archive, \
                     important"
                ))
            })?
    };
    let value = rest
        .strip_prefix("to ")
        .filter(|_| matches!(key, "forward" | "changeFolder" | "moveToLabel"))
        .unwrap_or(rest);
    let mut action = json!({ "templateKey": key });
    if !value.is_empty() {
        action["value"] = json!(unquote(value));
    }
    Ok(action)
}

/// `Category.displayName` as the engine computes it (`tern_tasks::mail_rules::display_name`):
/// the decoded path without an INBOX, `[Gmail]` or `[Mailspring]` prefix (or `Tern/`,
/// `Mailspring/`).
fn category_display_name(path: &str) -> String {
    let decoded = tern_client::utf7::decode_or_raw(path);
    for prefix in ["INBOX", "[Gmail]", "[Mailspring]"] {
        if decoded.starts_with(prefix) && decoded.chars().count() > prefix.chars().count() + 1 {
            return decoded.chars().skip(prefix.chars().count() + 1).collect();
        }
    }
    for prefix in ["Tern/", "Tern.", "Mailspring/", "Mailspring."] {
        if let Some(rest) = decoded.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    if decoded == "INBOX" {
        return "Inbox".into();
    }
    decoded
}

/// The folder or label `want` names among `list`: by id, path, display name or role. Sent and
/// Drafts are left out, as the rule editor leaves them out.
fn find_category<'a>(list: &'a [Value], want: &str) -> Option<&'a Value> {
    let lower = want.to_lowercase();
    let open: Vec<Value> = list
        .iter()
        .filter(|c| !matches!(str_of(c, "role"), "sent" | "drafts"))
        .cloned()
        .collect();
    let id = find_named(&open, want)
        .or_else(|| {
            open.iter()
                .find(|c| category_display_name(str_of(c, "path")).to_lowercase() == lower)
        })
        .or_else(|| by_role(&open, &lower))
        .map(|c| str_of(c, "id").to_owned())?;
    list.iter().find(|c| str_of(c, "id") == id)
}

fn unknown_keys(v: &Value, allowed: &[&str], what: &str) -> Result<()> {
    if let Some(k) = v
        .as_object()
        .into_iter()
        .flat_map(|o| o.keys())
        .find(|k| !allowed.contains(&k.as_str()))
    {
        return Err(CliError::usage(format!(
            "{what} has a field {k:?}; it takes {}",
            allowed.join(", ")
        )));
    }
    Ok(())
}

fn check_condition(c: &Value, n: usize) -> Result<()> {
    let what = format!("condition {n}");
    if !c.is_object() {
        return Err(CliError::usage(format!("{what} is not an object")));
    }
    unknown_keys(c, CONDITION_KEYS, &what)?;
    let key = str_of(c, "templateKey");
    if ENUM_FIELDS.iter().any(|(k, ..)| *k == key) {
        if !c["comparatorKey"].is_null() {
            return Err(CliError::usage(format!(
                "{what}: {key} takes no comparatorKey"
            )));
        }
        if !matches!(c["value"].as_str(), Some("true" | "false")) {
            return Err(CliError::usage(format!(
                "{what}: {key} takes the value \"true\" or \"false\""
            )));
        }
        return Ok(());
    }
    if !STRING_FIELDS.iter().any(|(k, ..)| *k == key) {
        return Err(CliError::usage(format!(
            "{what}: no condition {key:?}; the conditions are {}",
            STRING_FIELDS
                .iter()
                .chain(ENUM_FIELDS)
                .map(|(k, ..)| *k)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let comparator = str_of(c, "comparatorKey");
    if !COMPARATORS.iter().any(|(k, ..)| *k == comparator) {
        return Err(CliError::usage(format!(
            "{what}: {key} needs a comparatorKey: {}",
            COMPARATORS
                .iter()
                .map(|(k, ..)| *k)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let value = match &c["value"] {
        Value::String(s) if !s.is_empty() => s,
        _ => {
            return Err(CliError::usage(format!(
                "{what}: {key} {comparator} needs a value to compare with"
            )))
        }
    };
    if comparator == "matchesExpression" {
        RegexBuilder::new(value)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map_err(|e| {
                CliError::usage(format!(
                    "{what}: {value:?} is not an expression the engine can run: {e}"
                ))
            })?;
    }
    Ok(())
}

fn check_action(a: &mut Value, n: usize, cats: &Categories, email: &str) -> Result<()> {
    let what = format!("action {n}");
    if !a.is_object() {
        return Err(CliError::usage(format!("{what} is not an object")));
    }
    unknown_keys(a, ACTION_KEYS, &what)?;
    let gmail = cats.uses_labels();
    let key = str_of(a, "templateKey").to_owned();
    let Some((_, value_kind, offered, flag, _)) = ACTIONS.iter().find(|(k, ..)| *k == key) else {
        return Err(CliError::usage(format!(
            "{what}: no action {key:?}; the actions are {}",
            ACTIONS
                .iter()
                .map(|(k, ..)| *k)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    };
    match (offered, gmail) {
        (Offered::Gmail, false) => {
            return Err(CliError::usage(format!(
                "{what}: {flag} ({key}) is for Gmail accounts, and {email} has folders; use \
                 move FOLDER"
            )))
        }
        (Offered::Folders, true) => {
            return Err(CliError::usage(format!(
                "{what}: {key} is for accounts with folders, and {email} is a Gmail account; \
                 use move LABEL or label LABEL"
            )))
        }
        _ => {}
    }
    if !a["comparatorKey"].is_null() {
        return Err(CliError::usage(format!(
            "{what}: {key} takes no comparatorKey"
        )));
    }
    let obj = a.as_object_mut().expect("an object");
    obj.remove("comparatorKey");
    let value = obj
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    match value_kind {
        ActionValue::None => {
            if !obj.get("value").is_none_or(Value::is_null) {
                return Err(CliError::usage(format!("{what}: {flag} takes no value")));
            }
            obj.remove("value");
            obj.remove("valueName");
        }
        ActionValue::Address => {
            if !value.contains('@') || value.contains(char::is_whitespace) {
                return Err(CliError::usage(format!(
                    "{what}: forward needs an address to forward to, not {value:?}"
                )));
            }
            obj.insert("value".into(), json!(value));
            obj.remove("valueName");
        }
        ActionValue::Folder | ActionValue::Label => {
            let (list, noun, lists) = if *value_kind == ActionValue::Folder {
                (&cats.folders, "folder", "folders")
            } else {
                (&cats.labels, "label", "labels")
            };
            if value.is_empty() {
                return Err(CliError::usage(format!("{what}: {flag} needs a {noun}")));
            }
            // A category whose id is gone is still found by the name saved with the rule.
            let saved_name = obj
                .get("valueName")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let found = find_category(list, &value)
                .or_else(|| {
                    if saved_name.is_empty() {
                        None
                    } else {
                        find_category(list, &saved_name)
                    }
                })
                .ok_or_else(|| {
                    CliError::failed(format!(
                        "{what}: {email} has no {noun} {value}; `tern {lists}` lists them, `tern \
                         {noun} create` makes one"
                    ))
                })?;
            obj.insert("value".into(), found["id"].clone());
            obj.insert(
                "valueName".into(),
                json!(category_display_name(str_of(found, "path"))),
            );
        }
    }
    Ok(())
}

/// Checks `rule` for `cats`'s account and writes its folders and labels as ids with their
/// names. Every error names what is wrong.
fn check_rule(rule: &mut Value, cats: &Categories, email: &str) -> Result<()> {
    let Some(obj) = rule.as_object_mut() else {
        return Err(CliError::usage("the rule is not a JSON object"));
    };
    // Rules saved before the desktop wrote `disabled` are on.
    obj.entry("disabled").or_insert(json!(false));
    unknown_keys(rule, RULE_KEYS, "the rule")?;
    for key in ["id", "accountId", "name"] {
        if !rule[key].is_string() || str_of(rule, key).trim().is_empty() {
            return Err(CliError::usage(format!("the rule needs a {key}")));
        }
    }
    if !matches!(rule["conditionMode"].as_str(), Some("any" | "all")) {
        return Err(CliError::usage(
            "the rule's conditionMode is \"any\" or \"all\"",
        ));
    }
    if !rule["disabled"].is_boolean() {
        return Err(CliError::usage("the rule's disabled is true or false"));
    }
    if !(rule["disabledReason"].is_null() || rule["disabledReason"].is_string()) {
        return Err(CliError::usage("the rule's disabledReason is text or null"));
    }
    let conditions = rule["conditions"]
        .as_array()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| CliError::usage("the rule needs a condition (--if)"))?;
    for (i, c) in conditions.iter().enumerate() {
        check_condition(c, i + 1)?;
    }
    let actions = rule["actions"]
        .as_array_mut()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| CliError::usage("the rule needs an action (--then)"))?;
    for (i, a) in actions.iter_mut().enumerate() {
        check_action(a, i + 1, cats, email)?;
    }
    Ok(())
}

// -- the setting ------------------------------------------------------------------------------

struct Rules {
    engine: usize,
    list: Vec<Value>,
}

impl Rules {
    fn load(engines: &mut [Engine]) -> Result<Rules> {
        let engine = settings::engine_with(engines, feature::RULES, "run mail rules")?;
        let mut values = settings::read(&mut engines[engine], &[RULES_KEY])?;
        let list = match values.remove(RULES_KEY) {
            Some(Value::Array(a)) => a,
            _ => Vec::new(),
        };
        Ok(Rules { engine, list })
    }

    fn save(&self, engines: &mut [Engine]) -> Result<()> {
        let mut values = Map::new();
        values.insert(RULES_KEY.into(), Value::Array(self.list.clone()));
        settings::write(&mut engines[self.engine], values)
    }

    /// The index of the rule `want` names: its id, else its name in any case, among the
    /// rules of `account` when given.
    fn find(&self, want: &str, account: Option<&str>) -> Result<usize> {
        if let Some(i) = self.list.iter().position(|r| str_of(r, "id") == want) {
            return Ok(i);
        }
        let named: Vec<usize> = (0..self.list.len())
            .filter(|&i| {
                let r = &self.list[i];
                str_of(r, "name").eq_ignore_ascii_case(want.trim())
                    && account.is_none_or(|a| str_of(r, "accountId") == a)
            })
            .collect();
        match named.as_slice() {
            [one] => Ok(*one),
            [] => Err(CliError::failed(format!(
                "no rule {want}; `tern rules list` lists them"
            ))),
            many => Err(CliError::failed(format!(
                "several rules are named {want} ({}); give an id",
                many.iter()
                    .map(|&i| str_of(&self.list[i], "id"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }
}

/// The running account a rule is for.
fn rule_target(engines: &[Engine], rules_engine: usize, account_id: &str) -> Result<Target> {
    discovery::targets(engines, None)?
        .into_iter()
        .find(|t| t.engine == rules_engine && t.account_id == account_id)
        .ok_or_else(|| {
            CliError::failed(format!(
                "the rule is for account {account_id}, which the engine doesn't run; `tern \
                 accounts` lists them"
            ))
        })
}

fn email_of(engines: &[Engine], account_id: &str) -> String {
    engines
        .iter()
        .flat_map(|e| &e.hello.accounts)
        .find(|a| a.account_id == account_id)
        .map_or_else(|| account_id.to_owned(), |a| a.email.clone())
}

fn read_json_arg(arg: &str) -> Result<Value> {
    let text = if arg == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| CliError::failed(format!("could not read the rule from stdin: {e}")))?;
        s
    } else {
        arg.to_owned()
    };
    serde_json::from_str(&text).map_err(|e| CliError::usage(format!("--rule: not JSON: {e}")))
}

// -- describing rules -------------------------------------------------------------------------

fn describe_condition(c: &Value) -> String {
    let key = str_of(c, "templateKey");
    if ENUM_FIELDS.iter().any(|(k, ..)| *k == key) {
        let field = flag_of(ENUM_FIELDS, key);
        return if str_of(c, "value") == "false" {
            format!("not {field}")
        } else {
            field
        };
    }
    format!(
        "{} {} {:?}",
        flag_of(STRING_FIELDS, key),
        flag_of(COMPARATORS, str_of(c, "comparatorKey")),
        str_of(c, "value")
    )
}

fn describe_action(a: &Value) -> String {
    let key = str_of(a, "templateKey");
    let named = match str_of(a, "valueName") {
        "" => str_of(a, "value"),
        n => n,
    };
    match key {
        "markAsRead" => "mark read".into(),
        "moveToTrash" => "move to the trash".into(),
        "star" => "star".into(),
        "archive" => "archive".into(),
        "markAsImportant" => "mark important".into(),
        "forward" => format!("forward to {}", str_of(a, "value")),
        "changeFolder" => format!("move to {named}"),
        "moveToLabel" => format!("move to label {named}"),
        "applyLabel" => format!("label {named}"),
        other => other.to_owned(),
    }
}

fn state_of(rule: &Value) -> String {
    match (
        rule["disabled"].as_bool() == Some(true),
        str_of(rule, "disabledReason"),
    ) {
        (false, _) => "on".into(),
        (true, "") => "off".into(),
        (true, why) => format!("off: {}", output::one_line(why)),
    }
}

fn list_of(rule: &Value, key: &str, describe: fn(&Value) -> String) -> Vec<String> {
    rule[key]
        .as_array()
        .into_iter()
        .flatten()
        .map(describe)
        .collect()
}

fn print_rule(ctx: &Ctx, engines: &[Engine], rule: &Value) {
    let label = |l: &str| ctx.style.bold(&format!("{l:<9}"));
    output::line(&format!("{} {}", label("Name:"), str_of(rule, "name")));
    output::line(&format!("{} {}", label("Id:"), str_of(rule, "id")));
    output::line(&format!(
        "{} {}",
        label("Account:"),
        email_of(engines, str_of(rule, "accountId"))
    ));
    output::line(&format!("{} {}", label("State:"), state_of(rule)));
    let mode = if str_of(rule, "conditionMode") == "any" {
        "any condition"
    } else {
        "all conditions"
    };
    output::line(&format!("{} {mode}", label("Match:")));
    for (title, lines) in [
        ("If:", list_of(rule, "conditions", describe_condition)),
        ("Then:", list_of(rule, "actions", describe_action)),
    ] {
        for (i, l) in lines.iter().enumerate() {
            let head = if i == 0 { label(title) } else { " ".repeat(9) };
            output::line(&format!("{head} {}", output::one_line(l)));
        }
    }
}

// -- commands ---------------------------------------------------------------------------------

pub fn run(ctx: &Ctx, engines: &mut [Engine], command: RulesCommand) -> Result<()> {
    match command {
        RulesCommand::List => list(ctx, engines),
        RulesCommand::Show { rule } => show(ctx, engines, &rule),
        RulesCommand::Add { args, disabled } => add(ctx, engines, &args, disabled),
        RulesCommand::Update { rule, args } => update(ctx, engines, &rule, &args),
        RulesCommand::Remove { rule } => remove(ctx, engines, &rule),
        RulesCommand::Enable { rule } => set_enabled(ctx, engines, &rule, true),
        RulesCommand::Disable { rule } => set_enabled(ctx, engines, &rule, false),
        RulesCommand::Reprocess { wait, timeout } => reprocess(ctx, engines, wait, timeout),
    }
}

/// `--account`'s id, when given.
fn account_filter(ctx: &Ctx, engines: &[Engine]) -> Result<Option<String>> {
    match ctx.account() {
        Some(a) => Ok(Some(discovery::one_target(engines, Some(a))?.account_id)),
        None => Ok(None),
    }
}

fn list(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let rules = Rules::load(engines)?;
    let account = account_filter(ctx, engines)?;
    let shown: Vec<&Value> = rules
        .list
        .iter()
        .filter(|r| {
            account
                .as_deref()
                .is_none_or(|a| str_of(r, "accountId") == a)
        })
        .collect();
    if ctx.json {
        output::json(&json!({ "rules": shown }));
        return Ok(());
    }
    if shown.is_empty() {
        output::line("No mail rules; `tern rules add` makes one");
        return Ok(());
    }
    let mut table = Table::new(vec!["NAME", "ACCOUNT", "STATE", "IF", "THEN", "ID"], 3);
    for r in shown {
        let join = if str_of(r, "conditionMode") == "any" {
            " or "
        } else {
            " and "
        };
        table.row(
            vec![
                output::one_line(str_of(r, "name")),
                email_of(engines, str_of(r, "accountId")),
                state_of(r),
                output::one_line(&list_of(r, "conditions", describe_condition).join(join)),
                output::one_line(&list_of(r, "actions", describe_action).join(", ")),
                str_of(r, "id").to_owned(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

fn show(ctx: &Ctx, engines: &mut [Engine], want: &str) -> Result<()> {
    let rules = Rules::load(engines)?;
    let account = account_filter(ctx, engines)?;
    let rule = &rules.list[rules.find(want, account.as_deref())?];
    if ctx.json {
        output::json(&json!({ "rule": rule }));
    } else {
        print_rule(ctx, engines, rule);
    }
    Ok(())
}

/// `--rule`'s object, read once (it may be stdin).
fn rule_patch(args: &RuleArgs) -> Result<Option<Map<String, Value>>> {
    match &args.rule {
        None => Ok(None),
        Some(r) => match read_json_arg(r)? {
            Value::Object(m) => Ok(Some(m)),
            _ => Err(CliError::usage("--rule: the rule is a JSON object")),
        },
    }
}

/// The rule `args` (and `patch`, `--rule`'s object) describe on top of `base`: a new rule's
/// defaults, or the rule updated.
fn apply_args(
    base: &mut Value,
    patch: Option<Map<String, Value>>,
    args: &RuleArgs,
    gmail: bool,
) -> Result<()> {
    if let Some(patch) = patch {
        for (k, v) in patch {
            if (k == "id" || k == "accountId")
                && base.get(&k).is_some_and(|old| !old.is_null() && *old != v)
            {
                return Err(CliError::usage(format!(
                    "--rule: a rule's {k} can't change"
                )));
            }
            base[k] = v;
        }
    }
    if let Some(n) = &args.name {
        base["name"] = json!(n.trim());
    }
    if let Some(m) = &args.mode {
        base["conditionMode"] = json!(m);
    }
    if !args.conditions.is_empty() {
        base["conditions"] = args
            .conditions
            .iter()
            .map(|c| parse_condition(c))
            .collect::<Result<Vec<_>>>()?
            .into();
    }
    if !args.actions.is_empty() {
        base["actions"] = args
            .actions
            .iter()
            .map(|a| parse_action(a, gmail))
            .collect::<Result<Vec<_>>>()?
            .into();
    }
    Ok(())
}

fn add(ctx: &Ctx, engines: &mut [Engine], args: &RuleArgs, disabled: bool) -> Result<()> {
    let mut rules = Rules::load(engines)?;
    let patch = rule_patch(args)?;
    // The account: the rule's own, else --account, else the only one.
    let given = patch
        .as_ref()
        .and_then(|p| p.get("accountId"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let account_id = match given {
        Some(id) => id,
        None => discovery::one_target(engines, ctx.account())?.account_id,
    };
    let t = rule_target(engines, rules.engine, &account_id)?;
    let cats = Categories::load(engines, &t)?;
    let mut rule = json!({
        "id": tern_client::tasks::temp_id(),
        "name": DEFAULT_NAME,
        "conditionMode": "all",
        "conditions": [],
        "actions": [],
        "disabled": disabled,
        "accountId": t.account_id,
    });
    apply_args(&mut rule, patch, args, cats.uses_labels())?;
    check_rule(&mut rule, &cats, &t.email)?;
    if rules
        .list
        .iter()
        .any(|r| str_of(r, "id") == str_of(&rule, "id"))
    {
        return Err(CliError::usage(format!(
            "a rule with id {} exists; `tern rules update` changes it",
            str_of(&rule, "id")
        )));
    }
    rules.list.push(rule.clone());
    rules.save(engines)?;
    report(ctx, engines, &rule, "Added")
}

fn report(ctx: &Ctx, engines: &[Engine], rule: &Value, verb: &str) -> Result<()> {
    if ctx.json {
        output::json(&json!({ "rule": rule }));
    } else {
        output::line(&format!(
            "{verb} rule {:?} ({})",
            str_of(rule, "name"),
            str_of(rule, "id")
        ));
        print_rule(ctx, engines, rule);
    }
    Ok(())
}

fn update(ctx: &Ctx, engines: &mut [Engine], want: &str, args: &RuleArgs) -> Result<()> {
    if args.rule.is_none()
        && args.name.is_none()
        && args.mode.is_none()
        && args.conditions.is_empty()
        && args.actions.is_empty()
    {
        return Err(CliError::usage(
            "nothing to change: give --name, --if, --then, --match or --rule",
        ));
    }
    let patch = rule_patch(args)?;
    let mut rules = Rules::load(engines)?;
    let account = account_filter(ctx, engines)?;
    let i = rules.find(want, account.as_deref())?;
    let mut rule = rules.list[i].clone();
    let t = rule_target(engines, rules.engine, str_of(&rule, "accountId"))?;
    let cats = Categories::load(engines, &t)?;
    apply_args(&mut rule, patch, args, cats.uses_labels())?;
    check_rule(&mut rule, &cats, &t.email)?;
    rules.list[i] = rule.clone();
    rules.save(engines)?;
    report(ctx, engines, &rule, "Updated")
}

fn remove(ctx: &Ctx, engines: &mut [Engine], want: &str) -> Result<()> {
    let mut rules = Rules::load(engines)?;
    let account = account_filter(ctx, engines)?;
    let i = rules.find(want, account.as_deref())?;
    let rule = rules.list.remove(i);
    rules.save(engines)?;
    if ctx.json {
        output::json(&json!({ "id": rule["id"], "deleted": true }));
    } else {
        output::line(&format!(
            "Deleted rule {:?} ({})",
            str_of(&rule, "name"),
            str_of(&rule, "id")
        ));
    }
    Ok(())
}

fn set_enabled(ctx: &Ctx, engines: &mut [Engine], want: &str, on: bool) -> Result<()> {
    let mut rules = Rules::load(engines)?;
    let account = account_filter(ctx, engines)?;
    let i = rules.find(want, account.as_deref())?;
    let rule = &mut rules.list[i];
    // The preferences' switch: on clears the engine's reason, off keeps it.
    rule["disabled"] = json!(!on);
    if on {
        rule["disabledReason"] = Value::Null;
    }
    let rule = rule.clone();
    rules.save(engines)?;
    if ctx.json {
        output::json(&json!({ "rule": rule }));
    } else {
        output::line(&format!(
            "Turned rule {:?} ({}) {}",
            str_of(&rule, "name"),
            str_of(&rule, "id"),
            if on { "on" } else { "off" }
        ));
    }
    Ok(())
}

// -- reprocess --------------------------------------------------------------------------------

/// One account's `rules.reprocess` job, as it ended.
struct JobRun {
    job_id: String,
    total: usize,
    done: usize,
    stopped: bool,
    /// The tasks the rules queued, by id, as last seen.
    tasks: Vec<Value>,
}

fn reprocess(ctx: &Ctx, engines: &mut [Engine], wait: bool, timeout: Option<u64>) -> Result<()> {
    let rules = Rules::load(engines)?;
    let enabled = |aid: &str| {
        rules
            .list
            .iter()
            .any(|r| str_of(r, "accountId") == aid && r["disabled"].as_bool() != Some(true))
    };
    let targets: Vec<Target> = discovery::targets(engines, ctx.account())?
        .into_iter()
        .filter(|t| t.engine == rules.engine)
        .collect();
    let picked: Vec<Target> = targets
        .iter()
        .filter(|t| enabled(&t.account_id))
        .cloned()
        .collect();
    if picked.is_empty() {
        return Err(CliError::failed(match targets.as_slice() {
            [one] => format!(
                "{} has no rule that is on; `tern rules list` lists them",
                one.email
            ),
            _ => "no account has a rule that is on; `tern rules list` lists them".to_owned(),
        }));
    }
    let deadline = timeout.map(|s| Instant::now() + Duration::from_secs(s));
    let live = !ctx.json && std::io::stdout().is_terminal();
    let engine = &mut engines[rules.engine];
    if wait {
        let params = SubscribeParams {
            accounts: Some(picked.iter().map(|t| t.account_id.clone()).collect()),
            classes: Some(vec![TASK_CLASS.to_owned()]),
        };
        engine
            .client
            .subscribe(&params)
            .map_err(|e| CliError::call(&picked[0].email, method::SUBSCRIBE, e))?;
    }
    let mut runs: Vec<(Target, JobRun)> = Vec::new();
    for t in &picked {
        let run = run_job(engine, t, wait, deadline, |done, total| {
            if live {
                output::text(&format!("\r{}: {done}/{total} threads", t.email));
            }
        })?;
        if live && run.total > 0 {
            output::text("\r");
        }
        runs.push((t.clone(), run));
    }
    // A rule the engine turned off while it ran shows in the setting.
    let after = Rules::load(engines)?;
    let turned_off: Vec<&Value> = after
        .list
        .iter()
        .filter(|r| r["disabled"].as_bool() == Some(true))
        .filter(|r| {
            rules
                .list
                .iter()
                .any(|b| b["id"] == r["id"] && b["disabled"].as_bool() != Some(true))
        })
        .collect();
    let failed: Vec<String> = runs
        .iter()
        .flat_map(|(t, r)| {
            r.tasks.iter().filter(|x| !x["error"].is_null()).map(|x| {
                format!(
                    "{}: {} {} failed: {}",
                    t.email,
                    str_of(x, "__cls"),
                    str_of(x, "id"),
                    describe_error(&x["error"])
                )
            })
        })
        .collect();
    if ctx.json {
        let accounts: Vec<Value> = runs
            .iter()
            .map(|(t, r)| {
                let mut v = json!({
                    "accountId": t.account_id, "email": t.email, "jobId": r.job_id,
                    "total": r.total, "done": r.done, "stopped": r.stopped,
                });
                if wait {
                    v["tasks"] = json!(r.tasks);
                }
                v
            })
            .collect();
        output::json(&json!({ "accounts": accounts, "disabledRules": turned_off }));
    } else {
        for (t, r) in &runs {
            let n = r.total;
            let threads = format!("{n} inbox thread{}", if n == 1 { "" } else { "s" });
            output::line(&if r.stopped {
                format!("{}: stopped after {} of {threads}", t.email, r.done)
            } else if wait {
                format!(
                    "{}: went through {threads}; the server has the {} change{}",
                    t.email,
                    r.tasks.len(),
                    if r.tasks.len() == 1 { "" } else { "s" }
                )
            } else {
                format!("{}: went through {threads}", t.email)
            });
        }
        for r in &turned_off {
            output::warn(&format!(
                "the engine turned rule {:?} ({}) off: {}",
                str_of(r, "name"),
                str_of(r, "id"),
                str_of(r, "disabledReason")
            ));
        }
    }
    if let Some((t, r)) = runs.iter().find(|(_, r)| r.stopped) {
        return Err(CliError::failed(format!(
            "{}: the run stopped after {} of {} threads: it was cancelled, or the account \
             stopped",
            t.email, r.done, r.total
        )));
    }
    if !failed.is_empty() {
        return Err(CliError::failed(failed.join("\n")));
    }
    Ok(())
}

/// Starts `rules.reprocess` for `t` and reads until its last `rules.progress` (and, with
/// `wait`, until every task the rules queued is final). The engine stops the job when this
/// connection closes, so the command stays until then; at `deadline` it cancels the job.
fn run_job(
    engine: &mut Engine,
    t: &Target,
    wait: bool,
    deadline: Option<Instant>,
    mut progress: impl FnMut(usize, usize),
) -> Result<JobRun> {
    let err = |m: &str, e| CliError::call(&t.email, m, e);
    let r: RulesReprocessResult = engine
        .client
        .call_as(
            method::RULES_REPROCESS,
            json!({ "accountId": t.account_id }),
        )
        .map_err(|e| err(method::RULES_REPROCESS, e))?;
    let mut run = JobRun {
        job_id: r.job_id,
        total: r.total,
        done: 0,
        stopped: false,
        tasks: Vec::new(),
    };
    let mut job_over = run.total == 0;
    // The Task deltas of the last batch may come after its progress.
    let mut settled_at: Option<Instant> = job_over.then(|| Instant::now() + TASK_GRACE);
    let pending = |run: &JobRun| run.tasks.iter().any(|x| !is_final(x));
    let outcome = (|| -> Result<()> {
        loop {
            let settling = settled_at.filter(|_| !pending(&run));
            if job_over && (!wait || settling.is_some_and(|s| Instant::now() >= s)) {
                return Ok(());
            }
            let left = match deadline {
                Some(d) => {
                    let left = d.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(CliError::failed(format!(
                            "{}: not done within the --timeout ({} of {} threads)",
                            t.email, run.done, run.total
                        )));
                    }
                    Some(left)
                }
                None => None,
            };
            let quiet = settling.map(|s| s.saturating_duration_since(Instant::now()));
            let read_for = match (left, quiet) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            }
            .map(|d| d.max(Duration::from_millis(10)));
            engine
                .client
                .set_read_timeout(read_for)
                .map_err(|e| err(method::RULES_REPROCESS, e))?;
            let n = match engine.client.next_notification() {
                Ok(n) => n,
                Err(e) if e.is_timeout() => continue,
                Err(e) => return Err(err(method::RULES_REPROCESS, e)),
            };
            if n.method == method::RULES_PROGRESS {
                let Ok(p) = serde_json::from_value::<RulesProgressParams>(n.params) else {
                    continue;
                };
                if p.job_id != run.job_id {
                    continue;
                }
                run.done = p.done;
                run.total = p.total;
                if p.stopped {
                    run.stopped = true;
                    return Ok(());
                }
                progress(p.done, p.total);
                if p.done >= p.total {
                    job_over = true;
                    settled_at = Some(Instant::now() + TASK_GRACE);
                }
            } else if n.method == method::DELTA {
                let Ok(d) = serde_json::from_value::<DeltaParams>(n.params) else {
                    continue;
                };
                if d.model_class != TASK_CLASS || d.account_id != t.account_id {
                    continue;
                }
                for task in d.model_jsons {
                    if str_of(&task, "source") != RULES_SOURCE {
                        continue;
                    }
                    if d.kind == "unpersist" {
                        run.tasks.retain(|x| x["id"] != task["id"] || is_final(x));
                        continue;
                    }
                    match run.tasks.iter_mut().find(|x| x["id"] == task["id"]) {
                        Some(seen) => *seen = task,
                        None => run.tasks.push(task),
                    }
                }
                if job_over {
                    settled_at = Some(Instant::now() + TASK_GRACE);
                }
            }
        }
    })();
    let _ = engine
        .client
        .set_read_timeout(Some(tern_client::CALL_TIMEOUT));
    if outcome.is_err() && !job_over {
        let _ = engine
            .client
            .call(method::RULES_CANCEL, json!({ "jobId": run.job_id }));
    }
    outcome.map(|()| run)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cats(gmail: bool) -> Categories {
        let c = |id: &str, path: &str, role: &str, cls: &str| json!({"id": id, "aid": "a", "path": path, "role": role, "__cls": cls});
        if gmail {
            Categories {
                folders: vec![
                    c("f-all", "[Gmail]/All Mail", "all", "Folder"),
                    c("f-trash", "[Gmail]/Trash", "trash", "Folder"),
                ],
                labels: vec![
                    c("l-inbox", "INBOX", "inbox", "Label"),
                    c("l-sent", "[Gmail]/Sent Mail", "sent", "Label"),
                    c("l-rec", "Receipts", "", "Label"),
                ],
                counts: Map::new(),
            }
        } else {
            Categories {
                folders: vec![
                    c("f-in", "INBOX", "inbox", "Folder"),
                    c("f-sent", "INBOX.Sent", "sent", "Folder"),
                    c("f-rec", "INBOX.Receipts", "", "Folder"),
                ],
                labels: vec![],
                counts: Map::new(),
            }
        }
    }

    fn rule(conditions: Value, actions: Value) -> Value {
        json!({
            "id": "local-1", "accountId": "a", "name": "R", "conditionMode": "all",
            "conditions": conditions, "actions": actions, "disabled": false,
        })
    }

    #[test]
    fn conditions_parse_into_the_desktop_shape() {
        assert_eq!(
            parse_condition("from contains billing@shop.example").unwrap(),
            json!({"templateKey": "from", "comparatorKey": "contains", "value": "billing@shop.example"})
        );
        assert_eq!(
            parse_condition("subject does not contain 'weekly digest'").unwrap(),
            json!({"templateKey": "subject", "comparatorKey": "doesNotContain", "value": "weekly digest"})
        );
        assert_eq!(
            parse_condition("reply-to begins-with  noreply").unwrap()["comparatorKey"],
            "beginsWith"
        );
        assert_eq!(
            parse_condition("attachment-name matches \\.pdf$").unwrap(),
            json!({"templateKey": "anyAttachmentName", "comparatorKey": "matchesExpression", "value": "\\.pdf$"})
        );
        assert_eq!(
            parse_condition("has-attachment").unwrap(),
            json!({"templateKey": "hasAttachment", "value": "true"})
        );
        assert_eq!(
            parse_condition("starred is false").unwrap()["value"],
            "false"
        );
        for bad in ["sender contains x", "from x@y", "starred maybe", "subject"] {
            assert_eq!(parse_condition(bad).unwrap_err().exit_code(), 2, "{bad}");
        }
    }

    #[test]
    fn actions_parse_by_account_kind() {
        assert_eq!(
            parse_action("move Receipts", false).unwrap(),
            json!({"templateKey": "changeFolder", "value": "Receipts"})
        );
        assert_eq!(
            parse_action("move to Receipts", true).unwrap()["templateKey"],
            "moveToLabel"
        );
        assert_eq!(
            parse_action("forward to a@x.test", false).unwrap(),
            json!({"templateKey": "forward", "value": "a@x.test"})
        );
        assert_eq!(
            parse_action("mark-read", false).unwrap(),
            json!({"templateKey": "markAsRead"})
        );
        assert_eq!(
            parse_action("markAsImportant", true).unwrap()["templateKey"],
            "markAsImportant"
        );
        assert!(parse_action("explode", false).is_err());
    }

    #[test]
    fn rules_are_checked_and_categories_resolved() {
        let mut r = rule(
            json!([parse_condition("from contains shop").unwrap()]),
            json!([parse_action("move receipts", false).unwrap(), {"templateKey": "star"}]),
        );
        check_rule(&mut r, &cats(false), "me@x.test").unwrap();
        assert_eq!(
            r["actions"][0],
            json!({"templateKey": "changeFolder", "value": "f-rec", "valueName": "Receipts"})
        );
        assert_eq!(r["actions"][1], json!({"templateKey": "star"}));

        let mut g = rule(
            json!([{"templateKey": "starred", "value": "true"}]),
            json!([{"templateKey": "applyLabel", "value": "gone", "valueName": "Receipts"}]),
        );
        check_rule(&mut g, &cats(true), "me@gmail.test").unwrap();
        assert_eq!(g["actions"][0]["value"], "l-rec");

        let err = |r: Value, gmail: bool| {
            let mut r = r;
            check_rule(&mut r, &cats(gmail), "me@x.test")
                .unwrap_err()
                .to_string()
        };
        let from = json!([{"templateKey": "from", "comparatorKey": "contains", "value": "x"}]);
        let star = json!([{"templateKey": "star"}]);
        for (r, gmail, want) in [
            (rule(json!([]), star.clone()), false, "needs a condition"),
            (rule(from.clone(), json!([])), false, "needs an action"),
            (
                rule(
                    json!([{"templateKey": "from", "comparatorKey": "matchesExpression", "value": "(x"}]),
                    star.clone(),
                ),
                false,
                "not an expression",
            ),
            (
                rule(
                    json!([{"templateKey": "from", "comparatorKey": "like", "value": "x"}]),
                    star.clone(),
                ),
                false,
                "needs a comparatorKey",
            ),
            (
                rule(
                    json!([{"templateKey": "from", "comparatorKey": "contains", "value": ""}]),
                    star.clone(),
                ),
                false,
                "needs a value",
            ),
            (
                rule(
                    json!([{"templateKey": "starred", "comparatorKey": "contains", "value": "true"}]),
                    star.clone(),
                ),
                false,
                "takes no comparatorKey",
            ),
            (
                rule(
                    from.clone(),
                    json!([{"templateKey": "applyLabel", "value": "x"}]),
                ),
                false,
                "is for Gmail",
            ),
            (
                rule(
                    from.clone(),
                    json!([{"templateKey": "changeFolder", "value": "Receipts"}]),
                ),
                true,
                "is a Gmail account",
            ),
            (
                rule(
                    from.clone(),
                    json!([{"templateKey": "changeFolder", "value": "Sent"}]),
                ),
                false,
                "has no folder Sent",
            ),
            (
                rule(
                    from.clone(),
                    json!([{"templateKey": "forward", "value": "nobody"}]),
                ),
                false,
                "needs an address",
            ),
            (
                rule(from.clone(), json!([{"templateKey": "star", "value": "x"}])),
                false,
                "takes no value",
            ),
        ] {
            let e = err(r, gmail);
            assert!(e.contains(want), "{want}: {e}");
        }
        let mut extra = rule(from.clone(), star.clone());
        extra["priority"] = json!(1);
        let e = check_rule(&mut extra, &cats(false), "me@x.test")
            .unwrap_err()
            .to_string();
        assert!(e.contains("\"priority\""), "{e}");
        let mut mode = rule(from, star);
        mode["conditionMode"] = json!("some");
        assert!(check_rule(&mut mode, &cats(false), "me@x.test").is_err());
    }

    #[test]
    fn display_names_match_the_engine() {
        assert_eq!(category_display_name("INBOX.Receipts"), "Receipts");
        assert_eq!(category_display_name("[Gmail]/All Mail"), "All Mail");
        assert_eq!(category_display_name("INBOX"), "Inbox");
        assert_eq!(category_display_name("Tern/Snoozed"), "Snoozed");
        assert_eq!(category_display_name("Re&AOc-us"), "Reçus");
    }

    #[test]
    fn rules_are_described() {
        let r = rule(
            json!([
                {"templateKey": "from", "comparatorKey": "doesNotContain", "value": "x"},
                {"templateKey": "hasAttachment", "value": "false"},
            ]),
            json!([{"templateKey": "changeFolder", "value": "f", "valueName": "Receipts"}]),
        );
        assert_eq!(
            list_of(&r, "conditions", describe_condition),
            ["from not-contains \"x\"", "not has-attachment"]
        );
        assert_eq!(
            list_of(&r, "actions", describe_action),
            ["move to Receipts"]
        );
        let mut off = r.clone();
        off["disabled"] = json!(true);
        off["disabledReason"] = json!("Error: The folder could not be found.");
        assert_eq!(state_of(&off), "off: Error: The folder could not be found.");
        let rules = Rules {
            engine: 0,
            list: vec![r.clone(), {
                let mut b = r;
                b["id"] = json!("local-2");
                b["name"] = json!("Other");
                b
            }],
        };
        assert_eq!(rules.find("other", None).unwrap(), 1);
        assert_eq!(rules.find("local-1", None).unwrap(), 0);
        assert!(rules.find("other", Some("b")).is_err());
    }
}
