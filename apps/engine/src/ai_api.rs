// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `ai.*` methods on `serve`'s socket (docs/ai-plan.md section 3), over [`ms_ai`], and
//! `capabilities.ai`. Triage (section 4) is not served yet.
//!
//! A thread call is checked and its [`MailContext`] read on the connection's reader
//! thread, from its read-only database connection; the model call then runs on the AI
//! workers ([`crate::workers::AI`]). The account must be opted in and the feature on before
//! any mail is read. Until the engine keeps the AI settings and the provider keys
//! (desktop-client-plan stages 3 and 11), both come with each call from a local client
//! (`config`, `settings`); the relay policy keeps them out of relayed calls, which so far
//! find no provider. Nothing here keeps or logs the key, the prompt or the answer.
//!
//! Every call holds a [`CancelToken`] registered with its connection ([`Streams`]):
//! `ai.cancel {streamId}` and the connection closing cancel it, which drops the provider
//! request.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{Local, TimeZone};
use ms_ai::{
    Ai, AiError, CancelToken, ContextMessage, DraftOpts, Feature, MailContext, Options, Person,
    ProviderConfig, ReplyOpts, Sink, SummaryOpts, UsageRecord,
};
use ms_ipc::{
    ai_feature, code, method, AccountStatus, AiCancelParams, AiDraftReplyParams, AiModelsParams,
    AiProviderParams, AiSettings, AiSuggestRepliesParams, AiSummarizeParams, AiUsageParams,
    AiUsageRow, Capability, ErrorObject, Request as RpcRequest, CREDENTIALS_PER_REQUEST,
};
use ms_store::read::{CategoryTable, ReadStore};
use serde_json::{json, Value};

use crate::dispatch::Reply;
use crate::read_api::ReadApi;
use crate::socket::Accounts;

/// The methods that call a model, on the AI workers. `ai.cancel` and `ai.usage` are
/// answered at once.
pub fn is_ai_method(name: &str) -> bool {
    matches!(
        name,
        method::AI_MODELS
            | method::AI_TEST
            | method::AI_SUMMARIZE
            | method::AI_SUGGEST_REPLIES
            | method::AI_DRAFT_REPLY
    )
}

/// Body text read per call, in characters per token of the model's window: twice what
/// the budget could send at four characters a token. Older bodies past it aren't read.
const BODY_CHARS_PER_TOKEN: usize = 8;
/// `ai.usage` keeps this many days.
const USAGE_DAYS: i64 = 90;
const DEFAULT_USAGE_DAYS: u32 = 30;

/// `-32011` with the serialized error as `data`. Its text never holds a key, an address
/// or mail text.
fn failed(e: AiError) -> ErrorObject {
    let data = serde_json::to_value(&e).unwrap_or(Value::Null);
    ErrorObject::new(code::AI_FAILED, format!("AI request failed: {e}")).with_data(data)
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

fn invalid(why: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, format!("invalid params: {why}"))
}

fn answer<T: serde::Serialize>(r: ms_ai::Result<T>) -> Reply {
    serde_json::to_value(r.map_err(failed)?).map_err(internal)
}

static SHARED: Mutex<Option<Arc<Ai>>> = Mutex::new(None);

/// The process's [`Ai`], made on the first call: its caches and the usage counters are
/// shared by every connection.
fn ai() -> Result<Arc<Ai>, ErrorObject> {
    let mut shared = SHARED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ai) = &*shared {
        return Ok(ai.clone());
    }
    let options = Options {
        usage_observer: Some(Arc::new(record_usage)),
        ..Options::default()
    };
    let ai = Arc::new(Ai::with_options(options).map_err(failed)?);
    *shared = Some(ai.clone());
    Ok(ai)
}

/// Drops the cached summaries and suggestions (`account.reset`, `engine.resetDatabase`).
pub fn clear_cache() {
    let shared = SHARED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(ai) = shared {
        ai.clear_cache();
    }
}

// -- Calls ----------------------------------------------------------------------------------

/// A checked call, its mail already read, ready for a worker.
pub struct Prepared {
    stream_id: Option<String>,
    job: Job,
}

enum Job {
    Models(ProviderConfig),
    Test(ProviderConfig),
    Summarize(ProviderConfig, MailContext, SummaryOpts),
    Suggest(ProviderConfig, MailContext, ReplyOpts),
    Draft(ProviderConfig, MailContext, DraftOpts),
}

impl Prepared {
    /// The client's `streamId`, which `ai.delta` and `ai.cancel` name (suggestions send no
    /// `ai.delta`).
    pub fn stream_id(&self) -> Option<&str> {
        self.stream_id.as_deref()
    }

    /// Calls the model. Blocking: run it on a worker. Streamed text goes to `sink`, whose
    /// token stops the call.
    pub fn run(self, sink: &mut dyn Sink) -> Reply {
        let ai = ai()?;
        match self.job {
            Job::Models(config) => answer(ai.models(&config)),
            Job::Test(config) => answer(ai.test(&config)),
            Job::Summarize(config, context, opts) => {
                answer(ai.summarize(&config, &context, &opts, sink))
            }
            Job::Suggest(config, context, opts) => {
                answer(ai.suggest_replies_until(&config, &context, &opts, sink.cancel_token()))
            }
            Job::Draft(config, context, opts) => {
                answer(ai.draft_reply(&config, &context, &opts, sink))
            }
        }
    }
}

/// Checks a call [`is_ai_method`] accepts and reads the mail it needs from `reads`.
pub fn prepare(
    req: &RpcRequest,
    reads: &mut ReadApi,
    accounts: &dyn Accounts,
) -> Result<Prepared, ErrorObject> {
    let now = |job| Prepared {
        stream_id: None,
        job,
    };
    match req.method.as_str() {
        method::AI_MODELS => {
            let p: AiModelsParams = req.params_as()?;
            Ok(now(Job::Models(provider_config(&p)?)))
        }
        method::AI_TEST => {
            let p: AiModelsParams = req.params_as()?;
            Ok(now(Job::Test(provider_config(&p)?)))
        }
        method::AI_SUMMARIZE => {
            let p: AiSummarizeParams = req.params_as()?;
            let stream_id = checked_stream_id(p.stream_id)?;
            let thread = ThreadCall::check(
                accounts,
                &p.account_id,
                &p.thread_id,
                ai_feature::SUMMARY,
                p.settings.as_ref(),
                &p.provider,
            )?;
            let context = thread.context(reads, accounts)?;
            let opts = SummaryOpts { refresh: p.refresh };
            Ok(Prepared {
                stream_id,
                job: Job::Summarize(thread.config, context, opts),
            })
        }
        method::AI_SUGGEST_REPLIES => {
            let p: AiSuggestRepliesParams = req.params_as()?;
            let stream_id = checked_stream_id(p.stream_id)?;
            let thread = ThreadCall::check(
                accounts,
                &p.account_id,
                &p.thread_id,
                ai_feature::REPLY,
                p.settings.as_ref(),
                &p.provider,
            )?;
            let context = thread.context(reads, accounts)?;
            let opts = match p.count {
                Some(count) => ReplyOpts { count },
                None => ReplyOpts::default(),
            };
            Ok(Prepared {
                stream_id,
                job: Job::Suggest(thread.config, context, opts),
            })
        }
        method::AI_DRAFT_REPLY => {
            let p: AiDraftReplyParams = req.params_as()?;
            let stream_id = checked_stream_id(p.stream_id)?;
            let thread = ThreadCall::check(
                accounts,
                &p.account_id,
                &p.thread_id,
                ai_feature::REPLY,
                p.settings.as_ref(),
                &p.provider,
            )?;
            let context = thread.context(reads, accounts)?;
            let opts = DraftOpts {
                instruction: p.instruction,
                suggestion: p.suggestion,
            };
            Ok(Prepared {
                stream_id,
                job: Job::Draft(thread.config, context, opts),
            })
        }
        other => Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            format!("unknown method {other}"),
        )),
    }
}

fn checked_stream_id(id: Option<String>) -> Result<Option<String>, ErrorObject> {
    match id {
        Some(id) if id.is_empty() || id.len() > 128 => {
            Err(invalid("streamId must be 1 to 128 characters"))
        }
        id => Ok(id),
    }
}

/// The call's provider: the `config` it carries. The engine keeps no provider configs yet,
/// so a `provider` id alone (a relayed call) names none.
fn provider_config(p: &AiProviderParams) -> Result<ProviderConfig, ErrorObject> {
    match &p.config {
        Some(c) if !c.is_null() => {
            serde_json::from_value(c.clone()).map_err(|e| invalid(format!("config: {e}")))
        }
        _ => Err(failed(AiError::NotConfigured {
            what: "the model provider",
        })),
    }
}

/// A thread call that passed the account, feature and provider checks.
struct ThreadCall<'a> {
    account_id: &'a str,
    thread_id: &'a str,
    config: ProviderConfig,
    include_addresses: bool,
}

impl<'a> ThreadCall<'a> {
    fn check(
        accounts: &dyn Accounts,
        account_id: &'a str,
        thread_id: &'a str,
        feature: &'static str,
        settings: Option<&AiSettings>,
        provider: &AiProviderParams,
    ) -> Result<Self, ErrorObject> {
        accounts.check(account_id)?;
        let settings = settings
            .filter(|s| s.account_enabled(account_id))
            .ok_or_else(|| {
                failed(AiError::NotEnabled {
                    what: "AI for this account",
                })
            })?;
        if !settings.feature_enabled(feature) {
            return Err(failed(AiError::NotEnabled {
                what: feature_name(feature),
            }));
        }
        Ok(ThreadCall {
            account_id,
            thread_id,
            config: provider_config(provider)?,
            include_addresses: settings.include_addresses,
        })
    }

    fn context(
        &self,
        reads: &mut ReadApi,
        accounts: &dyn Accounts,
    ) -> Result<MailContext, ErrorObject> {
        let owner = owner(accounts.statuses(), self.account_id);
        let body_chars =
            (self.config.context_tokens() as usize).saturating_mul(BODY_CHARS_PER_TOKEN);
        let store = reads.store()?;
        let mut context = mail_context(store, self.account_id, self.thread_id, body_chars)?;
        context.owner = owner;
        context.include_addresses = self.include_addresses;
        Ok(context)
    }
}

fn feature_name(feature: &str) -> &'static str {
    match feature {
        ai_feature::SUMMARY => "the summary feature",
        ai_feature::REPLY => "the reply feature",
        _ => "the triage feature",
    }
}

fn owner(statuses: Vec<AccountStatus>, account_id: &str) -> Person {
    statuses
        .into_iter()
        .find(|s| s.account_id == account_id)
        .map(|s| {
            let name = s
                .account
                .as_ref()
                .and_then(|a| a.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            Person::new(name, s.email)
        })
        .unwrap_or_default()
}

// -- Mail context ---------------------------------------------------------------------------

/// The thread as the model may see it (docs/ai-plan.md section 5): drafts, hidden
/// messages, messages only in Spam or Trash and encrypted ones left out; bodies as plain
/// text without quoted replies, read newest first until `body_chars`, older messages
/// with their headers and `body_omitted`; the participants of the messages kept.
fn mail_context(
    store: &ReadStore,
    account_id: &str,
    thread_id: &str,
    body_chars: usize,
) -> Result<MailContext, ErrorObject> {
    let thread = store
        .thread(account_id, thread_id)
        .map_err(internal)?
        .ok_or_else(|| ErrorObject::new(code::NOT_FOUND, format!("no thread {thread_id}")))?;
    let mut roles: HashMap<String, String> = HashMap::new();
    let mut messages = Vec::new();
    for m in store
        .thread_messages(account_id, thread_id)
        .map_err(internal)?
    {
        if m["draft"] == json!(true) || m["hidden"] == json!(true) || has_encrypted_part(&m) {
            continue;
        }
        if only_in_spam_or_trash(store, account_id, &m, &mut roles)? {
            continue;
        }
        messages.push(m);
    }

    // Every body is read for the inline-PGP check, budget or not, so what is left out
    // doesn't depend on the budget; only those within it are turned into text.
    let mut left = body_chars;
    let mut kept: Vec<&Value> = Vec::with_capacity(messages.len());
    let mut out: Vec<ContextMessage> = Vec::with_capacity(messages.len());
    for m in messages.iter().rev() {
        let id = m["id"].as_str().unwrap_or("").to_owned();
        let stored = store.message_body(&id).map_err(internal)?;
        let snippet = m["snippet"].as_str();
        let armored = match &stored {
            Some(stored) => is_pgp_armored(stored),
            None => snippet.is_some_and(is_pgp_armored),
        };
        if armored {
            continue;
        }
        let has_text = stored
            .as_deref()
            .or(snippet)
            .is_some_and(|b| !b.trim().is_empty());
        let body_omitted = left == 0 && has_text;
        let body = if left > 0 {
            match stored {
                Some(stored) => Some(body_text(&stored, m["plaintext"] == json!(true))),
                None => snippet.map(str::to_owned),
            }
            .filter(|b| !b.trim().is_empty())
        } else {
            None
        };
        // Counted as ms-ai sends it, without the signature.
        if let Some(b) = &body {
            left = left.saturating_sub(ms_ai::strip_signature(b).chars().count());
        }
        kept.push(m);
        out.push(ContextMessage {
            id,
            date: format_date(m["date"].as_i64().unwrap_or(0)),
            from: m["from"].get(0).map(person).unwrap_or_default(),
            subject: m["subject"].as_str().map(str::to_owned),
            body,
            body_omitted,
            attachments: attachments(m),
        });
    }
    out.reverse();
    kept.reverse();
    if out.is_empty() {
        return Err(failed(AiError::Unsupported {
            what: "a thread with only drafts, Spam, Trash or encrypted mail",
        }));
    }
    Ok(MailContext {
        subject: thread["subject"].as_str().unwrap_or("").to_owned(),
        participants: participants(&kept),
        messages: out,
        ..MailContext::default()
    })
}

/// The senders and recipients of the messages sent, oldest first. Not the thread's own
/// list: the store adds the people of every message to it, drafts and Spam included.
fn participants(messages: &[&Value]) -> Vec<Person> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in messages {
        for field in ["from", "to", "cc"] {
            for contact in m[field].as_array().into_iter().flatten() {
                let p = person(contact);
                let key = if p.address.is_empty() {
                    p.name.clone()
                } else {
                    p.address.to_lowercase()
                };
                if !key.is_empty() && seen.insert(key) {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn person(contact: &Value) -> Person {
    Person::new(
        contact["name"].as_str().unwrap_or(""),
        contact["email"].as_str().unwrap_or(""),
    )
}

/// Whether every copy of `m` is in Spam or Trash. A message with no copies (in transit)
/// counts as elsewhere.
fn only_in_spam_or_trash(
    store: &ReadStore,
    account_id: &str,
    m: &Value,
    roles: &mut HashMap<String, String>,
) -> Result<bool, ErrorObject> {
    let Some(folders) = m["folders"].as_object().filter(|f| !f.is_empty()) else {
        return Ok(false);
    };
    for id in folders.keys() {
        if !roles.contains_key(id) {
            let role = store
                .category_role(CategoryTable::Folder, account_id, id)
                .map_err(internal)?
                .unwrap_or_default();
            roles.insert(id.clone(), role);
        }
        if !matches!(roles[id].as_str(), "spam" | "trash") {
            return Ok(false);
        }
    }
    Ok(true)
}

/// S/MIME (`smime.p7m`) or PGP/MIME (`application/pgp-encrypted`) parts, which the
/// engine keeps as files.
fn has_encrypted_part(m: &Value) -> bool {
    m["files"].as_array().into_iter().flatten().any(|f| {
        let ty = f["contentType"].as_str().unwrap_or("").to_ascii_lowercase();
        let name = f["filename"].as_str().unwrap_or("").to_ascii_lowercase();
        matches!(
            ty.as_str(),
            "application/pkcs7-mime" | "application/x-pkcs7-mime" | "application/pgp-encrypted"
        ) || name.ends_with(".p7m")
            || name.ends_with(".pgp")
            || name == "encrypted.asc"
    })
}

/// Inline PGP. A snippet may hold only the start of the armor line.
fn is_pgp_armored(body: &str) -> bool {
    body.contains("-----BEGIN PGP MESSAGE")
}

/// Visible files only: inline images carry a `contentId`.
fn attachments(m: &Value) -> Vec<String> {
    m["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["contentId"].as_str().is_none_or(str::is_empty))
        .filter_map(|f| f["filename"].as_str())
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .collect()
}

fn format_date(secs: i64) -> String {
    Local
        .timestamp_opt(secs, 0)
        .single()
        .map(|d| d.format("%a %-d %b %Y %H:%M").to_string())
        .unwrap_or_default()
}

/// A stored body as plain text without what it quotes: cited blockquotes dropped by the
/// flattener, then `>` lines, the attribution line before them, and everything after an
/// Outlook "Original Message" separator.
fn body_text(stored: &str, plaintext: bool) -> String {
    let text = if plaintext {
        stored.to_owned()
    } else {
        ms_mime::flatten_html_with(stored, false, false)
    };
    strip_quoted(&text)
}

fn strip_quoted(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let all: Vec<&str> = text.lines().collect();
    for (i, line) in all.iter().enumerate() {
        let t = line.trim();
        let outlook_rule = t.len() >= 20 && t.chars().all(|c| c == '_');
        let next_is_from = all
            .get(i + 1)
            .is_some_and(|n| n.trim_start().starts_with("From:"));
        if t.eq_ignore_ascii_case("-----Original Message-----") || (outlook_rule && next_is_from) {
            break;
        }
        if t.starts_with('>') {
            if lines.last().is_some_and(|l| is_attribution(l)) {
                lines.pop();
            }
            continue;
        }
        lines.push(line);
    }
    while lines
        .last()
        .is_some_and(|l| l.trim().is_empty() || is_attribution(l))
    {
        lines.pop();
    }
    lines.join("\n").trim().to_owned()
}

/// "On Tue, 4 Mar 2025 at 10:00, Ann <ann@acme.io> wrote:" and its common translations.
fn is_attribution(line: &str) -> bool {
    let t = line.trim_end();
    [
        "wrote:",
        "a écrit :",
        "a écrit:",
        "schrieb:",
        "escribió:",
        "scritto:",
        "escreveu:",
    ]
    .iter()
    .any(|end| t.ends_with(end))
}

// -- Streams --------------------------------------------------------------------------------

/// A connection's AI calls in flight and their tokens.
#[derive(Default)]
pub struct Streams {
    state: Mutex<StreamsState>,
}

#[derive(Default)]
struct StreamsState {
    next: u64,
    calls: Vec<(u64, Option<String>, CancelToken)>,
    /// The connection is gone: later registrations start cancelled.
    closed: bool,
}

/// One registered call; dropping it forgets the call.
pub struct Registration {
    streams: Arc<Streams>,
    key: u64,
    pub token: CancelToken,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.streams.lock().calls.retain(|(k, _, _)| *k != self.key);
    }
}

impl Streams {
    fn lock(&self) -> MutexGuard<'_, StreamsState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registers a call; a `stream_id` already streaming on this connection is refused, so
    /// `ai.delta` and `ai.cancel` name one call.
    pub fn register(
        self: &Arc<Self>,
        stream_id: Option<&str>,
    ) -> Result<Registration, ErrorObject> {
        let mut st = self.lock();
        if let Some(id) = stream_id {
            if st.calls.iter().any(|(_, s, _)| s.as_deref() == Some(id)) {
                return Err(invalid(format!(
                    "streamId {id:?} is already streaming on this connection"
                )));
            }
        }
        st.next += 1;
        let key = st.next;
        let token = CancelToken::new();
        if st.closed {
            token.cancel();
        }
        st.calls
            .push((key, stream_id.map(str::to_owned), token.clone()));
        Ok(Registration {
            streams: self.clone(),
            key,
            token,
        })
    }

    /// `ai.cancel`. An id that isn't streaming (it just finished) is not an error.
    pub fn cancel(&self, req: &RpcRequest) -> Reply {
        let p: AiCancelParams = req.params_as()?;
        match (p.stream_id, p.job_id) {
            (Some(id), _) => {
                for (_, s, token) in &self.lock().calls {
                    if s.as_deref() == Some(id.as_str()) {
                        token.cancel();
                    }
                }
            }
            // No triage jobs yet.
            (None, Some(_)) => {}
            (None, None) => return Err(invalid("ai.cancel takes streamId or jobId")),
        }
        Ok(json!({}))
    }

    /// The connection closed: every call stops.
    pub fn close(&self) {
        let mut st = self.lock();
        st.closed = true;
        for (_, _, token) in &st.calls {
            token.cancel();
        }
    }
}

// -- Usage ----------------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
struct Totals {
    input: u64,
    output: u64,
    requests: u64,
}

/// By day, provider id and feature, since the engine started.
static USAGE: Mutex<BTreeMap<(String, String, &'static str), Totals>> = Mutex::new(BTreeMap::new());

fn day_key(days_ago: i64) -> String {
    (Local::now().date_naive() - chrono::Duration::days(days_ago))
        .format("%Y-%m-%d")
        .to_string()
}

fn record_usage(r: &UsageRecord) {
    let mut usage = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    let oldest = day_key(USAGE_DAYS - 1);
    usage.retain(|(day, _, _), _| *day >= oldest);
    let feature = match r.feature {
        Feature::Summary => ai_feature::SUMMARY,
        Feature::Reply => ai_feature::REPLY,
        Feature::Triage => ai_feature::TRIAGE,
        Feature::Test => "test",
    };
    let t = usage
        .entry((day_key(0), r.provider_id.clone(), feature))
        .or_default();
    t.input += r.usage.input_tokens;
    t.output += r.usage.output_tokens;
    t.requests += 1;
}

/// `ai.usage`: the last `days` days, newest first.
pub fn usage(req: &RpcRequest) -> Reply {
    let p: AiUsageParams = req.params_as()?;
    let days = i64::from(
        p.days
            .unwrap_or(DEFAULT_USAGE_DAYS)
            .clamp(1, USAGE_DAYS as u32),
    );
    let oldest = day_key(days - 1);
    let usage = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    let mut rows: Vec<AiUsageRow> = usage
        .iter()
        .filter(|((day, _, _), _)| *day >= oldest)
        .map(|((day, provider, feature), t)| AiUsageRow {
            day: day.clone(),
            provider: provider.clone(),
            feature: (*feature).to_owned(),
            input_tokens: t.input,
            output_tokens: t.output,
            requests: t.requests,
        })
        .collect();
    rows.sort_by(|a, b| b.day.cmp(&a.day));
    serde_json::to_value(rows).map_err(internal)
}

// -- Capabilities ---------------------------------------------------------------------------

/// `capabilities.ai`: the features this engine serves. The client sends the provider,
/// key and switches with each call, so it decides availability from its own settings
/// ([`CREDENTIALS_PER_REQUEST`]).
pub fn capabilities() -> BTreeMap<String, Capability> {
    [ai_feature::SUMMARY, ai_feature::REPLY]
        .into_iter()
        .map(|f| {
            (
                f.to_owned(),
                Capability {
                    available: true,
                    credentials: Some(CREDENTIALS_PER_REQUEST.to_owned()),
                    ..Capability::default()
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_text_is_dropped() {
        let html = "<p>Sounds good.</p><p>See you then.</p>\
            <div class=\"gmail_quote\"><div>On Tue, 4 Mar 2025, Ann &lt;ann@acme.io&gt; wrote:<br></div>\
            <blockquote class=\"gmail_quote\">Lunch on Friday?</blockquote></div>";
        let text = body_text(html, false);
        assert!(text.starts_with("Sounds good."), "{text}");
        assert!(text.contains("See you then."), "{text}");
        assert!(!text.contains("Lunch"), "{text}");
        assert!(!text.contains("wrote:"), "{text}");

        let cited = "<p>Yes.</p><blockquote type=\"cite\">Can you?</blockquote>";
        assert_eq!(body_text(cited, false), "Yes.");

        let outlook = "Fine by me.\n\n-----Original Message-----\nFrom: Bob\nOld text";
        assert_eq!(body_text(outlook, true), "Fine by me.");
        let rule = format!("Ok.\n{}\nFrom: Bob\nSent: Monday", "_".repeat(32));
        assert_eq!(body_text(&rule, true), "Ok.");
        let interleaved = "Answers inline.\n> Q1?\nA1.\n> Q2?\nA2.";
        assert_eq!(body_text(interleaved, true), "Answers inline.\nA1.\nA2.");
    }

    #[test]
    fn encrypted_parts_are_found() {
        let with = |files: Value| has_encrypted_part(&json!({ "files": files }));
        assert!(with(
            json!([{"filename": "smime.p7m", "contentType": "application/octet-stream"}])
        ));
        assert!(with(
            json!([{"filename": "x", "contentType": "Application/PGP-Encrypted"}])
        ));
        assert!(with(
            json!([{"filename": "encrypted.asc", "contentType": "application/octet-stream"}])
        ));
        assert!(!with(
            json!([{"filename": "smime.p7s", "contentType": "application/pkcs7-signature"}])
        ));
        assert!(!with(json!([])));
        assert!(is_pgp_armored("hi\n-----BEGIN PGP MESSAGE-----\nabc"));
    }

    #[test]
    fn streams_cancel_by_id_and_on_close() {
        let streams = Arc::new(Streams::default());
        let a = streams.register(Some("s1")).unwrap();
        let b = streams.register(None).unwrap();
        assert_eq!(
            streams.register(Some("s1")).err().unwrap().code,
            code::INVALID_PARAMS
        );
        let cancel = |p: Value| streams.cancel(&RpcRequest::new(1, "ai.cancel", p));
        cancel(json!({"streamId": "other"})).unwrap();
        assert!(!a.token.is_cancelled());
        cancel(json!({"streamId": "s1"})).unwrap();
        assert!(a.token.is_cancelled() && !b.token.is_cancelled());
        assert_eq!(cancel(json!({})).unwrap_err().code, code::INVALID_PARAMS);
        drop(a);
        // Free again once the call ended.
        let c = streams.register(Some("s1")).unwrap();
        streams.close();
        assert!(b.token.is_cancelled() && c.token.is_cancelled());
        assert!(streams.register(None).unwrap().token.is_cancelled());
    }

    #[test]
    fn usage_is_counted_per_day_provider_and_feature() {
        let record = |provider: &str, feature| {
            record_usage(&UsageRecord {
                provider_id: provider.to_owned(),
                kind: ms_ai::ProviderKind::Ollama,
                model: "m".into(),
                feature,
                usage: ms_ai::TokenUsage {
                    input_tokens: 10,
                    output_tokens: 2,
                    estimated: false,
                },
            })
        };
        record("usage-test", Feature::Summary);
        record("usage-test", Feature::Summary);
        record("usage-test", Feature::Reply);
        let rows = usage(&RpcRequest::new(1, "ai.usage", json!({"days": 1}))).unwrap();
        let mine: Vec<&Value> = rows
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["provider"] == "usage-test")
            .collect();
        assert_eq!(mine.len(), 2);
        let summary = mine.iter().find(|r| r["feature"] == "summary").unwrap();
        assert_eq!(summary["inputTokens"], 20);
        assert_eq!(summary["outputTokens"], 4);
        assert_eq!(summary["requests"], 2);
        assert_eq!(summary["day"], json!(day_key(0)));
    }

    #[test]
    fn a_relayed_call_finds_no_provider() {
        let e = provider_config(&AiProviderParams {
            config: None,
            provider: Some("p".into()),
        })
        .unwrap_err();
        assert_eq!(e.code, code::AI_FAILED);
        assert_eq!(e.data.unwrap()["key"], "not-configured");
        let e = provider_config(&AiProviderParams {
            config: Some(json!({"id": "p", "kind": "nope"})),
            provider: None,
        })
        .unwrap_err();
        assert_eq!(e.code, code::INVALID_PARAMS);
    }
}
