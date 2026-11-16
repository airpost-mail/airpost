// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Mail rules in the engine ([`feature::RULES`](crate::feature::RULES), `serve` only). The rules
//! are the `mailRules` setting, in the desktop's `MailRule` shape (`{id, accountId, name,
//! conditions, conditionMode, actions, disabled?, disabledReason?}`); the engine applies the
//! enabled ones to mail that arrives after `mailRulesAutoSince` and disables a rule that fails,
//! writing `disabled` and `disabledReason` into the setting.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The setting holding the rules, in order.
pub const MAIL_RULES_KEY: &str = "mailRules";
/// Unix milliseconds: rules apply to mail dated from it on. The engine sets it to the time of
/// the first `settings.set` that stores rules, when there is none yet.
pub const MAIL_RULES_AUTO_SINCE_KEY: &str = "mailRulesAutoSince";

/// `rules.reprocess`: applies the account's enabled rules to the mail in its inbox, newest
/// threads first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesReprocessParams {
    pub account_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesReprocessResult {
    pub job_id: String,
    /// The inbox threads the job goes through.
    pub total: usize,
}

/// `rules.cancel`: stops a `rules.reprocess` job before its next batch of threads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesCancelParams {
    pub job_id: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesCancelResult {
    /// False when no such job was running.
    pub cancelled: bool,
}

/// Notification [`method::RULES_PROGRESS`](crate::method::RULES_PROGRESS), to the connection
/// that called `rules.reprocess`: `done` of `total` inbox threads gone through. The last one
/// has `done == total`, or `stopped`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesProgressParams {
    pub job_id: String,
    pub account_id: String,
    pub done: usize,
    pub total: usize,
    /// The job ended after `done` threads: it was cancelled or the account stopped.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
}

/// Notification [`method::RULES_DISABLED`](crate::method::RULES_DISABLED), to every subscribed
/// connection: the engine disabled a rule that failed. The rule's `disabled` and
/// `disabledReason` are in `mailRules` too, so a client that missed this reads them there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RulesDisabledParams {
    pub rule_id: String,
    pub error: String,
}
