// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Mail rules: the rules of the `mailRules` setting
//! checked against a message and turned into the tasks the desktop queued for them. Ported
//! from the desktop's `mail-rules-processor.ts`, `mail-rules-templates.ts` and the
//! comparators of `scenario-editor-models.ts`.
//!
//! - A rule applies to messages of its account. `conditionMode` `all` needs every condition,
//!   anything else one of them.
//! - Conditions read the message's addresses (emails, then names), attachment names, starred
//!   flag, subject and HTML body. String comparators ignore case; one that takes a list
//!   matches when some item does, except `doesNotContain`, which needs every item. The enum
//!   conditions (`hasAttachment`, `starred`) and an unknown comparator compare exactly.
//! - Actions act on the message's thread, as the desktop's did, and can't be undone. Folder
//!   and label ids are hashes of the IMAP path, so a rule whose category id is gone falls back
//!   to `valueName`, the display name saved with the rule, when exactly one category has it.
//! - An error (an invalid expression, a category that can't be found) is the rule's: the
//!   caller disables it with the message, which reads as the desktop's `err.toString()`.

use std::collections::HashMap;

use regex::{Regex, RegexBuilder};
use serde_json::{json, Value};
use tern_core::Result;
use tern_store::model::json_str;
use tern_store::{Folder, Label, Model, Query, Store, Task, Thread};

/// The `source` the tasks carry.
pub const SOURCE: &str = "Mail Rules";

/// Compiled expressions above this size are refused, so a rule can't make matching slow.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// One entry of `mailRules`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub disabled: bool,
    /// `conditionMode: "all"`.
    pub all: bool,
    pub conditions: Vec<Condition>,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Condition {
    pub template_key: String,
    pub comparator_key: String,
    pub value: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub template_key: String,
    pub value: String,
    /// The display name of the folder or label chosen for `value`.
    pub value_name: String,
}

fn str_field(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

impl Rule {
    /// The rule in `v`, or `None` when it has no id or account.
    pub fn from_json(v: &Value) -> Option<Rule> {
        let id = str_field(v, "id");
        let account_id = str_field(v, "accountId");
        if id.is_empty() || account_id.is_empty() {
            return None;
        }
        let list = |key: &str| v.get(key).and_then(Value::as_array).cloned();
        let conditions = list("conditions")
            .unwrap_or_default()
            .iter()
            .map(|c| Condition {
                template_key: str_field(c, "templateKey"),
                comparator_key: str_field(c, "comparatorKey"),
                value: c.get("value").cloned().unwrap_or(Value::Null),
            })
            .collect();
        let actions = list("actions")
            .unwrap_or_default()
            .iter()
            .map(|a| Action {
                template_key: str_field(a, "templateKey"),
                value: str_field(a, "value"),
                value_name: str_field(a, "valueName"),
            })
            .collect();
        Some(Rule {
            id,
            account_id,
            name: str_field(v, "name"),
            disabled: v.get("disabled").and_then(Value::as_bool).unwrap_or(false),
            all: json_str(v, "conditionMode") == "all",
            conditions,
            actions,
        })
    }
}

/// The enabled rules of `account_id` in `mailRules` order.
pub fn enabled_rules(setting: Option<&Value>, account_id: &str) -> Vec<Rule> {
    setting
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Rule::from_json)
        .filter(|r| !r.disabled && r.account_id == account_id)
        .collect()
}

/// `setting` with the rule `rule_id` disabled for `reason`; `None` when there is no such rule
/// or it is disabled already.
pub fn with_rule_disabled(setting: &Value, rule_id: &str, reason: &str) -> Option<Value> {
    let mut rules = setting.as_array()?.clone();
    let rule = rules
        .iter_mut()
        .find(|r| r.get("id").and_then(Value::as_str) == Some(rule_id))?;
    if rule.get("disabled").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let obj = rule.as_object_mut()?;
    obj.insert("disabled".into(), json!(true));
    obj.insert("disabledReason".into(), json!(reason));
    Some(Value::Array(rules))
}

// -- conditions -----------------------------------------------------------------------------

/// What a condition template reads off a message.
enum Actual {
    One(Option<String>),
    List(Vec<Option<String>>),
}

fn contacts(message: &Value, keys: &[&str]) -> Vec<Option<String>> {
    let all: Vec<&Value> = keys
        .iter()
        .flat_map(|k| {
            message
                .get(*k)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .collect();
    let field = |c: &&Value, f: &str| c.get(f).and_then(Value::as_str).map(str::to_owned);
    all.iter()
        .map(|c| field(c, "email"))
        .chain(all.iter().map(|c| field(c, "name")))
        .collect()
}

fn files(message: &Value) -> &[Value] {
    message
        .get("files")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn bool_text(b: bool) -> Option<String> {
    Some(if b { "true" } else { "false" }.to_owned())
}

/// `ConditionTemplates[].valueForMessage`; `None` for a template the engine doesn't know.
fn actual(template_key: &str, message: &Value, body: &str) -> Option<(Actual, bool)> {
    let string = |a| Some((a, true));
    match template_key {
        "from" | "to" | "cc" | "bcc" | "replyTo" => {
            string(Actual::List(contacts(message, &[template_key])))
        }
        "anyRecipient" => string(Actual::List(contacts(message, &["to", "cc", "bcc"]))),
        "anyAttachmentName" => string(Actual::List(
            files(message)
                .iter()
                .map(|f| f.get("filename").and_then(Value::as_str).map(str::to_owned))
                .collect(),
        )),
        "subject" => string(Actual::One(
            message
                .get("subject")
                .and_then(Value::as_str)
                .map(str::to_owned),
        )),
        "body" => string(Actual::One(Some(body.to_owned()))),
        "hasAttachment" => Some((Actual::One(bool_text(!files(message).is_empty())), false)),
        "starred" => Some((
            Actual::One(bool_text(
                message
                    .get("starred")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )),
            false,
        )),
        _ => None,
    }
}

/// Checks rules against messages, keeping the expressions it compiled.
#[derive(Default)]
pub struct Matcher {
    expressions: HashMap<String, std::result::Result<Regex, String>>,
}

impl Matcher {
    /// `_checkRuleForMessage`: whether `rule` matches the message (`message` its JSON,
    /// `body` its HTML). `Err` is the rule's error, e.g. an invalid expression.
    pub fn matches(
        &mut self,
        rule: &Rule,
        message: &Value,
        body: &str,
    ) -> std::result::Result<bool, String> {
        if json_str(message, "aid") != rule.account_id {
            return Ok(false);
        }
        for condition in &rule.conditions {
            let hit = self.condition(condition, message, body)?;
            if rule.all && !hit {
                return Ok(false);
            }
            if !rule.all && hit {
                return Ok(true);
            }
        }
        Ok(rule.all)
    }

    fn condition(
        &mut self,
        condition: &Condition,
        message: &Value,
        body: &str,
    ) -> std::result::Result<bool, String> {
        let Some((actual, string_template)) = actual(&condition.template_key, message, body) else {
            return Ok(false);
        };
        let desired = match &condition.value {
            Value::Null => None,
            Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        };
        let comparator = if string_template {
            condition.comparator_key.as_str()
        } else {
            ""
        };
        let every = comparator == "doesNotContain";
        let one = |m: &mut Self, a: &Option<String>| m.compare(comparator, a, &desired);
        match actual {
            Actual::One(a) => one(self, &a),
            Actual::List(items) => {
                for item in &items {
                    let hit = one(self, item)?;
                    if every && !hit {
                        return Ok(false);
                    }
                    if !every && hit {
                        return Ok(true);
                    }
                }
                Ok(every)
            }
        }
    }

    /// `Comparators.String` by key; any other key is `Comparator.Default` (`_.isEqual`).
    fn compare(
        &mut self,
        comparator: &str,
        actual: &Option<String>,
        desired: &Option<String>,
    ) -> std::result::Result<bool, String> {
        let a = actual.as_deref().filter(|s| !s.is_empty());
        let d = desired.as_deref().filter(|s| !s.is_empty());
        let lower = |s: &str| s.to_lowercase();
        Ok(match comparator {
            "contains" => matches!((a, d), (Some(a), Some(d)) if lower(a).contains(&lower(d))),
            "doesNotContain" => match (a, d) {
                (_, None) => false,
                (None, _) => true,
                (Some(a), Some(d)) => !lower(a).contains(&lower(d)),
            },
            "beginsWith" => {
                matches!((a, d), (Some(a), Some(d)) if lower(a).starts_with(&lower(d)))
            }
            "endsWith" => matches!((a, d), (Some(a), Some(d)) if lower(a).ends_with(&lower(d))),
            "equals" => match (a, d) {
                (Some(a), Some(d)) => lower(a) == lower(d),
                _ => actual == desired,
            },
            "matchesExpression" => match (a, d) {
                (Some(a), Some(d)) => self.expression(d)?.is_match(a),
                _ => false,
            },
            _ => actual.is_some() && actual == desired,
        })
    }

    fn expression(&mut self, pattern: &str) -> std::result::Result<&Regex, String> {
        let compiled = self
            .expressions
            .entry(pattern.to_owned())
            .or_insert_with(|| {
                RegexBuilder::new(pattern)
                    .case_insensitive(true)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| {
                        format!("SyntaxError: Invalid regular expression: /{pattern}/: {e}")
                    })
            });
        compiled.as_ref().map_err(Clone::clone)
    }
}

/// Mail this account sent to other people (`MailRulesStore._isSentToOthers`): from one of its
/// addresses and to none of them. Rules apply to mail that arrives, never to the user's own
/// sent mail; mail from another of the user's accounts still arrives at this one.
pub fn is_sent_to_others(message: &Value, is_mine: &dyn Fn(&str) -> bool) -> bool {
    let emails = |key: &str| -> Vec<String> {
        message
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| c.get("email").and_then(Value::as_str).map(str::to_owned))
            .collect()
    };
    let from_me = emails("from").iter().any(|e| is_mine(e));
    let to_me = ["to", "cc", "bcc"]
        .iter()
        .flat_map(|k| emails(k))
        .any(|e| is_mine(&e));
    from_me && !to_me
}

// -- actions --------------------------------------------------------------------------------

/// What one action does to a thread.
#[derive(Debug)]
pub enum Planned {
    Task(Task),
    /// `composeAndSendForward` of the message to this address.
    Forward(String),
    /// Nothing to do (the thread is read or starred already).
    Nothing,
}

#[derive(Clone, Debug)]
struct Category {
    json: Value,
    is_label: bool,
}

impl Category {
    fn id(&self) -> &str {
        json_str(&self.json, "id")
    }
    fn role(&self) -> &str {
        json_str(&self.json, "role")
    }
    fn path(&self) -> &str {
        json_str(&self.json, "path")
    }
}

/// `Category.displayName`: the decoded path without its INBOX, `[Gmail]` or `[Mailspring]`
/// prefix (or a `Mailspring/` one).
pub fn display_name(path: &str) -> String {
    let decoded = tern_core::utf7::decode_or_raw(path);
    for prefix in ["INBOX", "[Gmail]", "[Mailspring]"] {
        if decoded.starts_with(prefix) && decoded.chars().count() > prefix.chars().count() + 1 {
            return decoded.chars().skip(prefix.chars().count() + 1).collect();
        }
    }
    if decoded.starts_with("Mailspring/") || decoded.starts_with("Mailspring.") {
        return decoded.chars().skip(11).collect();
    }
    if decoded == "INBOX" {
        return "Inbox".into();
    }
    decoded
}

/// `Category.isLockedCategory`: Sent and Drafts.
fn is_locked(category: &Value) -> bool {
    ["sent", "drafts"]
        .iter()
        .any(|r| json_str(category, "role") == *r || json_str(category, "path") == *r)
}

/// An account's folders and labels, read once per message the rules act on.
pub struct Categories {
    all: Vec<Category>,
}

impl Categories {
    pub fn load(store: &Store, account_id: &str) -> Result<Categories> {
        let q = || Query::new().equal("accountId", account_id);
        let mut all: Vec<Category> = store
            .find_all::<Folder>(&q())?
            .into_iter()
            .map(|f| Category {
                json: f.to_json(),
                is_label: false,
            })
            .collect();
        all.extend(
            store
                .find_all::<Label>(&q())?
                .into_iter()
                .map(|l| Category {
                    json: l.to_json(),
                    is_label: true,
                }),
        );
        Ok(Categories { all })
    }

    fn by_role(&self, role: &str) -> Option<&Category> {
        self.all.iter().find(|c| c.role() == role)
    }

    /// `resolveCategory`: by id, else by the saved display name when exactly one category
    /// has it, since moving mail to the wrong one of two is worse than a disabled rule.
    fn resolve(&self, action: &Action) -> Option<&Category> {
        if let Some(c) = self.all.iter().find(|c| c.id() == action.value) {
            return Some(c);
        }
        if action.value_name.is_empty() {
            return None;
        }
        let mut named = self
            .all
            .iter()
            .filter(|c| display_name(c.path()) == action.value_name);
        match (named.next(), named.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    }
}

fn task(class: &str, account_id: &str, thread: &Thread, extra: Value) -> Planned {
    let mut data = json!({
        "threadIds": [thread.id()], "source": SOURCE, "canBeUndone": false,
    });
    if let (Some(obj), Some(extra)) = (data.as_object_mut(), extra.as_object()) {
        obj.extend(extra.clone());
    }
    Planned::Task(Task::create(class, account_id, &data))
}

fn rule_error(message: &str) -> std::result::Result<Planned, String> {
    Err(format!("Error: {message}"))
}

/// `MailRulesActions[action.templateKey]` for `thread`, read from the store just now.
pub fn plan_action(
    categories: &Categories,
    account_id: &str,
    action: &Action,
    thread: &Thread,
) -> std::result::Result<Planned, String> {
    let labels = |add: Vec<Value>, remove: Vec<Value>| {
        task(
            "ChangeLabelsTask",
            account_id,
            thread,
            json!({ "labelsToAdd": add, "labelsToRemove": remove }),
        )
    };
    let move_to = |folder: &Category| {
        task(
            "ChangeFolderTask",
            account_id,
            thread,
            json!({ "folder": folder.json }),
        )
    };
    match action.template_key.as_str() {
        "markAsImportant" => match categories.by_role("important") {
            Some(c) => Ok(labels(vec![c.json.clone()], vec![])),
            None => rule_error("Could not find `important` label"),
        },
        "moveToTrash" => match categories.by_role("trash") {
            Some(c) => Ok(move_to(c)),
            None => rule_error("Could not find `trash` folder"),
        },
        "markAsRead" if thread.unread() == 0 => Ok(Planned::Nothing),
        "markAsRead" => Ok(task(
            "ChangeUnreadTask",
            account_id,
            thread,
            json!({ "unread": false }),
        )),
        "star" if thread.starred() > 0 => Ok(Planned::Nothing),
        "star" => Ok(task(
            "ChangeStarredTask",
            account_id,
            thread,
            json!({ "starred": true }),
        )),
        "forward" if action.value.trim().is_empty() => {
            rule_error("An address to forward to is required.")
        }
        "forward" => Ok(Planned::Forward(action.value.trim().to_owned())),
        "changeFolder" if action.value.is_empty() => rule_error("A folder is required."),
        "changeFolder" => match categories.resolve(action).filter(|c| !c.is_label) {
            Some(c) => Ok(move_to(c)),
            None => rule_error("The folder could not be found."),
        },
        "applyLabel" if action.value.is_empty() => rule_error("A label is required."),
        "applyLabel" => match categories.resolve(action).filter(|c| c.is_label) {
            Some(c) => Ok(labels(vec![c.json.clone()], vec![])),
            None => rule_error("The label could not be found."),
        },
        "archive" => match categories.by_role("inbox") {
            Some(c) => Ok(labels(vec![], vec![c.json.clone()])),
            None => rule_error("Could not find `inbox` label"),
        },
        "moveToLabel" if action.value.is_empty() => rule_error("A label is required."),
        "moveToLabel" => {
            let label = categories
                .by_role(&action.value)
                .or_else(|| categories.resolve(action))
                .filter(|c| c.is_label);
            let Some(label) = label else {
                return rule_error("The label could not be found.");
            };
            let remove: Vec<Value> = thread
                .labels()
                .as_array()
                .into_iter()
                .flatten()
                .filter(|l| !is_locked(l) && json_str(l, "id") != label.id())
                .cloned()
                .collect();
            Ok(labels(vec![label.json.clone()], remove))
        }
        other => rule_error(&format!("{other} is not a supported action.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AID: &str = "b5djvgcuhj6i3x8nm53d0vnjm";

    fn rule(v: Value) -> Rule {
        let mut v = v;
        v["id"] = json!("local-ac7f1671-ba03");
        v["accountId"] = json!(AID);
        Rule::from_json(&v).unwrap()
    }

    fn contact(email: &str) -> Value {
        json!({ "email": email })
    }

    fn message(v: Value) -> Value {
        let mut m = json!({ "aid": AID, "from": [], "to": [], "cc": [], "bcc": [], "files": [] });
        for (k, val) in v.as_object().unwrap() {
            m[k] = val.clone();
        }
        m
    }

    /// The cases of `spec/mail-rules-processor-spec.ts` `_checkRuleForMessage`.
    fn spec_cases() -> Vec<(Rule, Vec<Value>, Vec<Value>)> {
        let from = |e: &str| message(json!({ "from": [contact(e)] }));
        let cc_subject = |emails: &[&str], subject: &str| {
            message(json!({
                "cc": emails.iter().map(|e| contact(e)).collect::<Vec<_>>(),
                "subject": subject,
            }))
        };
        let files_to = |file: Option<&str>, to: &str| {
            let files: Vec<Value> = file.map(|f| json!({ "filename": f })).into_iter().collect();
            message(json!({ "files": files, "to": [contact(to)] }))
        };
        vec![
            (
                rule(json!({
                    "name": "conditionMode Any, contains, equals",
                    "conditions": [
                        {"templateKey": "from", "comparatorKey": "contains", "value": "@mailspring.com"},
                        {"templateKey": "from", "comparatorKey": "equals", "value": "oldschool@nilas.com"},
                    ],
                    "conditionMode": "any",
                    "actions": [{"templateKey": "markAsRead"}],
                })),
                vec![
                    from("ben@mailspring.com"),
                    from("ben@mailspring.com.jp"),
                    from("oldschool@nilas.com"),
                ],
                vec![
                    from("ben@other.com"),
                    from("ben@nilas.com"),
                    from("twooldschool@nilas.com"),
                ],
            ),
            (
                rule(json!({
                    "name": "conditionMode all, ends with, begins with",
                    "conditions": [
                        {"templateKey": "cc", "comparatorKey": "endsWith", "value": ".com"},
                        {"templateKey": "subject", "comparatorKey": "beginsWith", "value": "[TEST] "},
                    ],
                    "conditionMode": "any",
                    "actions": [{"templateKey": "applyLabel", "value": "51a0hb8d6l78mmhy19ffx4txs"}],
                })),
                vec![
                    cc_subject(&["ben@mailspring.org"], "[TEST] ABCD"),
                    cc_subject(&["ben@mailspring.org"], "[test] ABCD"),
                    cc_subject(&["ben@mailspring.com"], "Whatever"),
                    cc_subject(&["a@test.com"], "Whatever"),
                    cc_subject(&["a@hasacom.com"], "[test] Whatever"),
                    cc_subject(&["a@hasacom.org", "b@mailspring.com"], "Whatever"),
                ],
                vec![
                    cc_subject(&["a@hasacom.org"], "Whatever"),
                    cc_subject(&["a@hasacom.org"], "[test]Whatever"),
                    cc_subject(&["a.com@hasacom.org"], "Whatever [test] "),
                ],
            ),
            (
                rule(json!({
                    "name": "Any attachment name endsWith, anyRecipient equals",
                    "conditions": [
                        {"templateKey": "anyAttachmentName", "comparatorKey": "endsWith", "value": ".pdf"},
                        {"templateKey": "anyRecipient", "comparatorKey": "equals", "value": "files@mailspring.com"},
                    ],
                    "conditionMode": "any",
                    "actions": [{"templateKey": "changeFolder", "value": "51a0hb8d6l78mmhy19ffx4txs"}],
                })),
                vec![
                    files_to(Some("bengotow.pdf"), "ben@mailspring.org"),
                    files_to(None, "files@mailspring.com"),
                    message(json!({
                        "to": [contact("ben@mailspring.com")],
                        "cc": [contact("ben@test.com"), contact("files@mailspring.com")],
                    })),
                ],
                vec![
                    files_to(None, "ben@mailspring.org"),
                    files_to(Some("bengotow.pdfz"), "ben@mailspring.org"),
                    files_to(Some("bengotowpdf"), "ben@mailspring.org"),
                    files_to(None, "afiles@mailspring.com"),
                    files_to(None, "files@mailspring.coma"),
                ],
            ),
        ]
    }

    #[test]
    fn filters_the_spec_messages() {
        let mut m = Matcher::default();
        for (rule, good, bad) in spec_cases() {
            for (i, msg) in good.iter().enumerate() {
                assert_eq!(m.matches(&rule, msg, ""), Ok(true), "{i} ({})", rule.name);
            }
            for (i, msg) in bad.iter().enumerate() {
                assert_eq!(m.matches(&rule, msg, ""), Ok(false), "{i} ({})", rule.name);
            }
        }
    }

    #[test]
    fn checks_the_account() {
        let (rule, good, _) = spec_cases().remove(0);
        let mut msg = good[0].clone();
        msg["aid"] = json!("not the same!");
        assert_eq!(Matcher::default().matches(&rule, &msg, ""), Ok(false));
    }

    #[test]
    fn all_mode_needs_every_condition_and_does_not_contain_needs_every_item() {
        let r = rule(json!({
            "conditions": [
                {"templateKey": "from", "comparatorKey": "doesNotContain", "value": "spam"},
                {"templateKey": "body", "comparatorKey": "contains", "value": "INVOICE"},
            ],
            "conditionMode": "all",
        }));
        let mut m = Matcher::default();
        let msg = message(json!({ "from": [{"email": "a@b.test", "name": "Ann"}] }));
        assert_eq!(m.matches(&r, &msg, "<p>your invoice</p>"), Ok(true));
        assert_eq!(m.matches(&r, &msg, "<p>hello</p>"), Ok(false));
        let spammy = message(json!({ "from": [{"email": "a@b.test", "name": "Spam Co"}] }));
        assert_eq!(m.matches(&r, &spammy, "invoice"), Ok(false));
        // No conditions: `every` holds, `some` doesn't.
        let empty_all = rule(json!({ "conditions": [], "conditionMode": "all" }));
        let empty_any = rule(json!({ "conditions": [], "conditionMode": "any" }));
        assert_eq!(m.matches(&empty_all, &msg, ""), Ok(true));
        assert_eq!(m.matches(&empty_any, &msg, ""), Ok(false));
    }

    #[test]
    fn enum_conditions_compare_exactly() {
        let r = rule(json!({
            "conditions": [
                {"templateKey": "hasAttachment", "comparatorKey": null, "value": "true"},
                {"templateKey": "starred", "value": "false"},
            ],
            "conditionMode": "all",
        }));
        let mut m = Matcher::default();
        let with_file = message(json!({ "files": [{"filename": "a.txt"}], "starred": false }));
        assert_eq!(m.matches(&r, &with_file, ""), Ok(true));
        let starred = message(json!({ "files": [{"filename": "a.txt"}], "starred": true }));
        assert_eq!(m.matches(&r, &starred, ""), Ok(false));
        assert_eq!(m.matches(&r, &message(json!({})), ""), Ok(false));
    }

    #[test]
    fn expressions_ignore_case_and_an_invalid_one_is_the_rules_error() {
        let r = |pattern: &str| {
            rule(json!({
                "conditions": [{"templateKey": "subject", "comparatorKey": "matchesExpression", "value": pattern}],
                "conditionMode": "any",
            }))
        };
        let mut m = Matcher::default();
        let msg = message(json!({ "subject": "Order #1234 shipped" }));
        assert_eq!(m.matches(&r(r"order #\d+"), &msg, ""), Ok(true));
        assert_eq!(m.matches(&r("^shipped"), &msg, ""), Ok(false));
        let err = m.matches(&r("(unclosed"), &msg, "").unwrap_err();
        assert!(
            err.starts_with("SyntaxError: Invalid regular expression"),
            "{err}"
        );
        // Not reached when the subject is empty, as in the desktop.
        let blank = message(json!({ "subject": "" }));
        assert_eq!(m.matches(&r("(unclosed"), &blank, ""), Ok(false));
    }

    #[test]
    fn unknown_templates_never_match() {
        let r = rule(json!({
            "conditions": [{"templateKey": "nope", "comparatorKey": "contains", "value": "x"}],
            "conditionMode": "any",
        }));
        assert_eq!(
            Matcher::default().matches(&r, &message(json!({})), "x"),
            Ok(false)
        );
    }

    #[test]
    fn sent_to_others() {
        let mine = |e: &str| e == "me@work.test";
        let m = |from: &str, to: &[&str], bcc: &[&str]| {
            message(json!({
                "from": [contact(from)],
                "to": to.iter().map(|e| contact(e)).collect::<Vec<_>>(),
                "bcc": bcc.iter().map(|e| contact(e)).collect::<Vec<_>>(),
            }))
        };
        assert!(is_sent_to_others(
            &m("me@work.test", &["bob@x.test"], &[]),
            &mine
        ));
        assert!(!is_sent_to_others(
            &m("me@work.test", &["bob@x.test"], &["me@work.test"]),
            &mine
        ));
        assert!(!is_sent_to_others(
            &m("me@home.test", &["me@work.test"], &[]),
            &mine
        ));
    }

    #[test]
    fn disabling_writes_the_reason_once() {
        let setting = json!([{"id": "r1", "accountId": AID}, {"id": "r2", "accountId": AID}]);
        let out = with_rule_disabled(&setting, "r2", "Error: nope").unwrap();
        assert_eq!(out[1]["disabled"], json!(true));
        assert_eq!(out[1]["disabledReason"], json!("Error: nope"));
        assert_eq!(out[0], setting[0]);
        assert!(with_rule_disabled(&out, "r2", "again").is_none());
        assert!(with_rule_disabled(&setting, "r3", "x").is_none());
        assert_eq!(enabled_rules(Some(&out), AID).len(), 1);
        assert!(enabled_rules(Some(&out), "other").is_empty());
    }

    #[test]
    fn display_names_drop_the_server_prefixes() {
        assert_eq!(display_name("INBOX/Receipts"), "Receipts");
        assert_eq!(display_name("INBOX.Receipts"), "Receipts");
        assert_eq!(display_name("[Gmail]/Trash"), "Trash");
        assert_eq!(display_name("INBOX"), "Inbox");
        assert_eq!(display_name("Receipts"), "Receipts");
        assert_eq!(display_name("Mailspring/Snoozed"), "Snoozed");
        assert_eq!(display_name("Entw&APw-rfe"), "Entwürfe");
    }

    fn categories(list: Vec<(Value, bool)>) -> Categories {
        Categories {
            all: list
                .into_iter()
                .map(|(json, is_label)| Category { json, is_label })
                .collect(),
        }
    }

    fn thread(v: Value) -> Thread {
        let mut data = json!({ "id": "t1", "aid": AID, "v": 1, "unread": 1, "starred": 0, "labels": [], "folders": [] });
        for (k, val) in v.as_object().unwrap() {
            data[k] = val.clone();
        }
        Thread::new(data)
    }

    fn planned_task(p: std::result::Result<Planned, String>) -> Task {
        match p {
            Ok(Planned::Task(t)) => t,
            other => panic!("expected a task, got {other:?}"),
        }
    }

    /// `spec/mail-rules-processor-spec.ts` "category resolution".
    #[test]
    fn falls_back_to_the_display_name_only_when_it_is_unambiguous() {
        let folder = json!({ "id": "new-id", "path": "INBOX/Receipts", "role": "" });
        let action = |name: &str| Action {
            template_key: "changeFolder".into(),
            value: "stale-id".into(),
            value_name: name.into(),
        };
        let cats = categories(vec![(folder.clone(), false)]);
        let t = planned_task(plan_action(
            &cats,
            AID,
            &action("Receipts"),
            &thread(json!({})),
        ));
        assert_eq!(t.class(), "ChangeFolderTask");
        assert_eq!(t.data["folder"]["id"], json!("new-id"));
        assert_eq!(t.data["threadIds"], json!(["t1"]));
        assert_eq!(t.data["canBeUndone"], json!(false));
        assert_eq!(t.data["source"], json!(SOURCE));

        let err = plan_action(&cats, AID, &action("Nope"), &thread(json!({}))).unwrap_err();
        assert_eq!(err, "Error: The folder could not be found.");

        let twin = json!({ "id": "twin-id", "path": "Receipts", "role": "" });
        let cats = categories(vec![(folder, false), (twin, false)]);
        assert!(plan_action(&cats, AID, &action("Receipts"), &thread(json!({}))).is_err());
    }

    #[test]
    fn actions_build_the_desktops_tasks() {
        let inbox = json!({ "id": "l-inbox", "path": "\\Inbox", "role": "inbox" });
        let important = json!({ "id": "l-imp", "path": "\\Important", "role": "important" });
        let work = json!({ "id": "l-work", "path": "Work", "role": "" });
        let sent = json!({ "id": "l-sent", "path": "\\Sent", "role": "sent" });
        let trash = json!({ "id": "f-trash", "path": "[Gmail]/Trash", "role": "trash" });
        let cats = categories(vec![
            (inbox.clone(), true),
            (important.clone(), true),
            (work.clone(), true),
            (trash.clone(), false),
        ]);
        let act = |key: &str, value: &str| Action {
            template_key: key.into(),
            value: value.into(),
            value_name: String::new(),
        };
        let t = thread(json!({ "labels": [inbox.clone(), sent.clone()] }));

        let read = planned_task(plan_action(&cats, AID, &act("markAsRead", ""), &t));
        assert_eq!(
            (read.class(), &read.data["unread"]),
            ("ChangeUnreadTask", &json!(false))
        );
        let already_read = thread(json!({ "unread": 0 }));
        assert!(matches!(
            plan_action(&cats, AID, &act("markAsRead", ""), &already_read),
            Ok(Planned::Nothing)
        ));
        let star = planned_task(plan_action(&cats, AID, &act("star", ""), &t));
        assert_eq!(
            (star.class(), &star.data["starred"]),
            ("ChangeStarredTask", &json!(true))
        );

        let imp = planned_task(plan_action(&cats, AID, &act("markAsImportant", ""), &t));
        assert_eq!(imp.data["labelsToAdd"], json!([important]));
        let archive = planned_task(plan_action(&cats, AID, &act("archive", ""), &t));
        assert_eq!(archive.data["labelsToRemove"], json!([inbox]));
        let trash_task = planned_task(plan_action(&cats, AID, &act("moveToTrash", ""), &t));
        assert_eq!(trash_task.data["folder"], trash);
        let apply = planned_task(plan_action(&cats, AID, &act("applyLabel", "l-work"), &t));
        assert_eq!(apply.data["labelsToAdd"], json!([work]));

        // Every unlocked label goes but the target; Sent stays.
        let moved = planned_task(plan_action(&cats, AID, &act("moveToLabel", "l-work"), &t));
        assert_eq!(moved.data["labelsToAdd"], json!([work]));
        assert_eq!(moved.data["labelsToRemove"], json!([inbox]));
        // By role first.
        let by_role = planned_task(plan_action(
            &cats,
            AID,
            &act("moveToLabel", "important"),
            &t,
        ));
        assert_eq!(by_role.data["labelsToAdd"], json!([important]));

        // A label is no folder and a folder no label.
        assert!(plan_action(&cats, AID, &act("changeFolder", "l-work"), &t).is_err());
        assert!(plan_action(&cats, AID, &act("applyLabel", "f-trash"), &t).is_err());
        assert_eq!(
            plan_action(&cats, AID, &act("applyLabel", ""), &t).unwrap_err(),
            "Error: A label is required."
        );

        match plan_action(&cats, AID, &act("forward", " fwd@x.test "), &t) {
            Ok(Planned::Forward(to)) => assert_eq!(to, "fwd@x.test"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            plan_action(&cats, AID, &act("explode", ""), &t).unwrap_err(),
            "Error: explode is not a supported action."
        );
        let none = categories(vec![]);
        assert_eq!(
            plan_action(&none, AID, &act("moveToTrash", ""), &t).unwrap_err(),
            "Error: Could not find `trash` folder"
        );
    }
}
