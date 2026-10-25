// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The `ai.*` methods on `serve`'s socket, over [`tern_ai`],
//! `capabilities.ai` and the usage counters. Triage (section 4) is [`crate::ai_triage`].
//!
//! A thread call is checked and its [`MailContext`] read on the connection's reader
//! thread, from its read-only database connection; the model call then runs on the AI
//! workers ([`crate::workers::AI`]). The account must be opted in and the feature on before
//! any mail is read. The switches and providers are the `core.ai.*` settings ([`StoredAi`]);
//! a local client may still send its own with each call (`settings`, `config`), which the
//! relay policy keeps out of relayed calls. A stored provider's key is `serve`'s secret
//! `integration:ai-<id>` ([`on_secret`]), never a setting and never sent to a client. Nothing
//! here keeps or logs the key, the prompt or the answer.
//!
//! Every call holds a [`CancelToken`] registered with its connection ([`Streams`]):
//! `ai.cancel {streamId}` and the connection closing cancel it, which drops the provider
//! request.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use chrono::{Local, TimeZone};
use serde_json::{json, Map, Value};
use tern_ai::{
    Ai, AiError, CancelToken, ContextMessage, DraftOpts, Feature, MailContext, Options, Person,
    ProviderConfig, ProviderKind, ReplyOpts, Sink, SummaryOpts, UsageRecord,
};
use tern_core::constants::thread;
use tern_integrations::Secret;
use tern_ipc::{
    ai_feature, capability_reason, code, method, AccountStatus, AiCancelParams, AiDraftReplyParams,
    AiModelsParams, AiProviderParams, AiSettings, AiSuggestRepliesParams, AiSummarizeParams,
    AiUsageParams, AiUsageRow, Capability, ErrorObject, Request as RpcRequest,
};
use tern_store::read::{CategoryTable, ReadStore};
use tracing::{info, warn};

use crate::ai_triage::Job as TriageJob;

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
/// `<config>/ai-usage.json`: the usage counters, kept across restarts.
pub const USAGE_FILE: &str = "ai-usage.json";

/// `-32011` with the serialized error as `data`. Its text never holds a key, an address
/// or mail text.
pub(crate) fn failed(e: AiError) -> ErrorObject {
    let data = serde_json::to_value(&e).unwrap_or(Value::Null);
    ErrorObject::new(code::AI_FAILED, format!("AI request failed: {e}")).with_data(data)
}

pub(crate) fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

pub(crate) fn invalid(why: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, format!("invalid params: {why}"))
}

fn answer<T: serde::Serialize>(r: tern_ai::Result<T>) -> Reply {
    serde_json::to_value(r.map_err(failed)?).map_err(internal)
}

static SHARED: Mutex<Option<Arc<Ai>>> = Mutex::new(None);

/// The process's [`Ai`], made on the first call: its caches and the usage counters are
/// shared by every connection.
pub(crate) fn ai() -> Result<Arc<Ai>, ErrorObject> {
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

// -- Stored settings ------------------------------------------------------------------------

pub const PROVIDERS_KEY: &str = "core.ai.providers";
pub const FEATURES_KEY: &str = "core.ai.features";
pub const ACCOUNTS_KEY: &str = "core.ai.accounts";
pub const LIMITS_KEY: &str = "core.ai.limits";
pub const INCLUDE_ADDRESSES_KEY: &str = "core.ai.includeAddresses";
const KEYS: [&str; 5] = [
    PROVIDERS_KEY,
    FEATURES_KEY,
    ACCOUNTS_KEY,
    LIMITS_KEY,
    INCLUDE_ADDRESSES_KEY,
];

/// Triage requests per account per hour when `core.ai.limits` names none.
pub const DEFAULT_TRIAGE_PER_HOUR: u64 = 200;

type SecretSource = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

static SECRETS: OnceLock<SecretSource> = OnceLock::new();

/// Where [`StoredAi`] reads the providers' keys: `serve`'s secret store. Without it (`sync`
/// mode) stored providers have none.
pub fn on_secret(f: SecretSource) {
    let _ = SECRETS.set(f);
}

fn stored_secret(name: &str) -> Option<String> {
    SECRETS.get().and_then(|f| f(name))
}

type SecretForget = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

static FORGET: OnceLock<SecretForget> = OnceLock::new();

/// How [`forget_unbound_keys`] deletes a key from `serve`'s secret store. Without it (`sync`
/// mode) there are none to delete.
pub fn on_forget_secret(f: SecretForget) {
    let _ = FORGET.set(f);
}

/// A stored provider's kind and base URL, `None` when it doesn't read as a provider.
fn origins(providers: Option<&Value>) -> BTreeMap<String, Option<(ProviderKind, String)>> {
    let Some(Value::Array(list)) = providers else {
        return BTreeMap::new();
    };
    list.iter()
        .filter_map(|p| {
            let id = p["id"].as_str()?.to_owned();
            let origin = serde_json::from_value::<ProviderConfig>(p.clone())
                .ok()
                .and_then(|c| Some((c.kind, c.base_url().ok()?)));
            Some((id, origin))
        })
        .collect()
}

/// The ids of the stored providers whose key no longer applies once `core.ai.providers`
/// goes from `before` to `after`: removed, or now of another kind or server. A key was
/// given for one server and must never go to another.
pub fn unbound_keys(before: Option<&Value>, after: Option<&Value>) -> Vec<String> {
    let after = origins(after);
    origins(before)
        .into_iter()
        .filter(|(id, old)| old.is_none() || after.get(id) != Some(old))
        .map(|(id, _)| id)
        .collect()
}

/// Deletes the keys of [`unbound_keys`] before `settings.set` writes `after`; an error
/// means a key that may still be read is still there, and the write must not happen.
pub fn forget_unbound_keys(before: Option<&Value>, after: Option<&Value>) -> Result<(), String> {
    let Some(forget) = FORGET.get() else {
        return Ok(());
    };
    for id in unbound_keys(before, after) {
        let name = tern_secrets::names::ai_provider(&id);
        forget(&name).map_err(|e| format!("could not delete {name}: {e}"))?;
        info!("Secret {name} deleted: its provider was removed or changed server");
    }
    Ok(())
}

/// The `core.ai.*` settings: providers, their keys from the secret store, the switches, the
/// limits. Values of the wrong shape read as unset.
#[derive(Clone, Debug, Default)]
pub struct StoredAi {
    providers: Vec<Value>,
    /// By provider id: `integration:ai-<id>`, when it holds a value.
    keys: BTreeMap<String, Secret>,
    switches: AiSettings,
    triage_per_hour: Option<u64>,
    daily_tokens: BTreeMap<String, u64>,
}

impl StoredAi {
    /// From the store's values (`settings.get` without keys).
    pub fn from_values(values: &Map<String, Value>) -> StoredAi {
        StoredAi::read(&|k| values.get(k).cloned())
    }

    pub fn read(get: &dyn Fn(&str) -> Option<Value>) -> StoredAi {
        StoredAi::read_with(get, &stored_secret)
    }

    /// [`StoredAi::read`] with the keys from `secret` (a secret name to its value).
    pub fn read_with(
        get: &dyn Fn(&str) -> Option<Value>,
        secret: &dyn Fn(&str) -> Option<String>,
    ) -> StoredAi {
        let [providers, features, accounts, limits, include] = KEYS.map(get);
        let limits = limits.unwrap_or(Value::Null);
        // `settings.set` refuses a provider with a key; one stored before that is used
        // without it.
        let providers: Vec<Value> = match providers {
            Some(Value::Array(list)) => list
                .into_iter()
                .map(|mut p| {
                    if let Some(p) = p.as_object_mut() {
                        p.remove("apiKey");
                    }
                    p
                })
                .collect(),
            _ => Vec::new(),
        };
        let keys = providers
            .iter()
            .filter_map(|p| p["id"].as_str())
            .filter_map(|id| {
                let key = Secret::new(secret(&tern_secrets::names::ai_provider(id))?);
                (!key.is_blank()).then(|| (id.to_owned(), key))
            })
            .collect();
        StoredAi {
            providers,
            keys,
            switches: AiSettings {
                accounts: accounts
                    .as_ref()
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect(),
                features: match features {
                    Some(Value::Object(map)) => map,
                    _ => Map::new(),
                },
                include_addresses: include.as_ref().and_then(Value::as_bool) == Some(true),
            },
            triage_per_hour: limits["triagePerHour"].as_u64(),
            daily_tokens: limits["dailyTokens"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| Some((k.clone(), v.as_u64()?)))
                .collect(),
        }
    }

    pub fn switches(&self) -> &AiSettings {
        &self.switches
    }

    /// The stored provider `id` with its stored key, `None` when there is none.
    pub fn provider(&self, id: &str) -> Option<Result<ProviderConfig, ErrorObject>> {
        let raw = self
            .providers
            .iter()
            .find(|p| p["id"].as_str() == Some(id))?;
        Some(
            serde_json::from_value::<ProviderConfig>(raw.clone())
                .map(|mut config| {
                    config.api_key = self.keys.get(id).cloned();
                    config
                })
                .map_err(|_| {
                    failed(AiError::NotConfigured {
                        what: "the model provider",
                    })
                }),
        )
    }

    /// The stored key of provider `config.id`, for a `config` sent without one that goes to
    /// the same server: a caller naming another host must bring its own key, or a request
    /// could carry the stored key anywhere.
    fn key_for(&self, config: &ProviderConfig) -> Option<Secret> {
        let stored = self.provider(&config.id)?.ok()?;
        let same_server = stored.kind == config.kind
            && matches!((stored.base_url(), config.base_url()), (Ok(a), Ok(b)) if a == b);
        same_server.then_some(stored.api_key).flatten()
    }

    /// The provider id `feature` uses.
    pub fn feature_provider(&self, feature: &str) -> Option<&str> {
        self.switches.features.get(feature)?["provider"].as_str()
    }

    pub fn triage_per_hour(&self) -> u64 {
        self.triage_per_hour.unwrap_or(DEFAULT_TRIAGE_PER_HOUR)
    }

    /// The provider's daily token cap, `None` for no cap.
    pub fn daily_tokens(&self, provider_id: &str) -> Option<u64> {
        self.daily_tokens.get(provider_id).copied()
    }
}

/// The provider a call uses: the `config` a local client sent (with the stored key when it
/// has none, [`StoredAi::key_for`]), else the stored provider it names (`provider`), else
/// the one the feature uses.
pub(crate) fn resolve_provider(
    p: &AiProviderParams,
    feature: Option<&str>,
    stored: &StoredAi,
) -> Result<ProviderConfig, ErrorObject> {
    if let Some(c) = p.config.as_ref().filter(|c| !c.is_null()) {
        let mut config: ProviderConfig =
            serde_json::from_value(c.clone()).map_err(|e| invalid(format!("config: {e}")))?;
        if config.api_key.as_ref().is_none_or(Secret::is_blank) {
            if let Some(key) = stored.key_for(&config) {
                config.api_key = Some(key);
            }
        }
        return Ok(config);
    }
    let not_configured = || {
        failed(AiError::NotConfigured {
            what: "the model provider",
        })
    };
    let id = p
        .provider
        .as_deref()
        .or_else(|| feature.and_then(|f| stored.feature_provider(f)))
        .ok_or_else(not_configured)?;
    stored.provider(id).ok_or_else(not_configured)?
}

/// `quota-exceeded` until local midnight once the provider used its daily cap.
pub(crate) fn check_daily_cap(provider_id: &str, cap: Option<u64>) -> Result<(), AiError> {
    match cap {
        Some(cap) if tokens_today(provider_id) >= cap => Err(AiError::QuotaExceeded {
            status: None,
            retry_after: Some(until_midnight()),
        }),
        _ => Ok(()),
    }
}

/// After a call to `provider_id` that started with `before` tokens used today: when it
/// used up the daily cap, the capabilities change now (and at midnight, through
/// [`capabilities`]).
pub(crate) fn note_usage(provider_id: &str, cap: Option<u64>, before: u64) {
    if cap.is_some_and(|cap| before < cap && tokens_today(provider_id) >= cap) {
        crate::ai_triage::capabilities_changed();
    }
}

/// The local day whose end a [`thread::AI_CAP_RESET`] thread waits for.
static CAP_RESET: Mutex<Option<String>> = Mutex::new(None);

/// How often the [`thread::AI_CAP_RESET`] thread looks at the date: a sleeping computer's
/// clock jumps past midnight.
const CAP_RESET_POLL: Duration = Duration::from_secs(60);

/// Sends `capabilities.changed` once the local day ends, while a provider is over its daily
/// cap: the cap resets then and nothing else would say so. One thread per day.
fn notify_at_midnight() {
    let today = day_key(0);
    let mut due = CAP_RESET.lock().unwrap_or_else(|e| e.into_inner());
    if due.as_deref() == Some(today.as_str()) {
        return;
    }
    let day = today.clone();
    let spawned = std::thread::Builder::new()
        .name(thread::AI_CAP_RESET.into())
        .spawn(move || {
            while day_key(0) == day {
                std::thread::sleep(until_midnight().clamp(Duration::from_secs(1), CAP_RESET_POLL));
            }
            crate::ai_triage::capabilities_changed();
        });
    match spawned {
        Ok(_) => *due = Some(today),
        Err(e) => warn!(
            "-X Could not start the {} thread: {e}",
            thread::AI_CAP_RESET
        ),
    }
}

fn until_midnight() -> Duration {
    let now = Local::now();
    let midnight = (now.date_naive() + chrono::Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(Local).earliest());
    midnight
        .and_then(|m| (m - now).to_std().ok())
        .unwrap_or(Duration::from_secs(3600))
}

// -- Calls ----------------------------------------------------------------------------------

/// A checked call, its mail already read, ready for a worker.
pub struct Prepared {
    stream_id: Option<String>,
    job: Job,
    /// The provider's daily token cap.
    cap: Option<u64>,
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
        let provider = match &self.job {
            Job::Models(_) => None,
            Job::Test(c) | Job::Summarize(c, ..) | Job::Suggest(c, ..) | Job::Draft(c, ..) => {
                Some(c.id.as_str())
            }
        };
        if let Some(id) = provider {
            check_daily_cap(id, self.cap).map_err(failed)?;
        }
        let used = provider.map(|id| (id.to_owned(), tokens_today(id)));
        let reply = match self.job {
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
        };
        if let Some((id, before)) = used {
            note_usage(&id, self.cap, before);
        }
        reply
    }
}

/// Checks a call [`is_ai_method`] accepts and reads the mail it needs from `reads`.
pub fn prepare(
    req: &RpcRequest,
    reads: &mut ReadApi,
    accounts: &dyn Accounts,
    stored: &StoredAi,
) -> Result<Prepared, ErrorObject> {
    let prepared = prepare_job(req, reads, accounts, stored)?;
    Ok(Prepared {
        cap: match &prepared.job {
            Job::Models(_) => None,
            Job::Test(c) | Job::Summarize(c, ..) | Job::Suggest(c, ..) | Job::Draft(c, ..) => {
                stored.daily_tokens(&c.id)
            }
        },
        ..prepared
    })
}

fn prepare_job(
    req: &RpcRequest,
    reads: &mut ReadApi,
    accounts: &dyn Accounts,
    stored: &StoredAi,
) -> Result<Prepared, ErrorObject> {
    let now = |job| Prepared {
        stream_id: None,
        job,
        cap: None,
    };
    match req.method.as_str() {
        method::AI_MODELS => {
            let p: AiModelsParams = req.params_as()?;
            Ok(now(Job::Models(resolve_provider(&p, None, stored)?)))
        }
        method::AI_TEST => {
            let p: AiModelsParams = req.params_as()?;
            Ok(now(Job::Test(resolve_provider(&p, None, stored)?)))
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
                stored,
            )?;
            let context = thread.context(reads, accounts)?;
            let opts = SummaryOpts { refresh: p.refresh };
            Ok(Prepared {
                stream_id,
                job: Job::Summarize(thread.config, context, opts),
                cap: None,
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
                stored,
            )?;
            let context = thread.context(reads, accounts)?;
            let opts = match p.count {
                Some(count) => ReplyOpts { count },
                None => ReplyOpts::default(),
            };
            Ok(Prepared {
                stream_id,
                job: Job::Suggest(thread.config, context, opts),
                cap: None,
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
                stored,
            )?;
            let context = thread.context(reads, accounts)?;
            let opts = DraftOpts {
                instruction: p.instruction,
                suggestion: p.suggestion,
            };
            Ok(Prepared {
                stream_id,
                job: Job::Draft(thread.config, context, opts),
                cap: None,
            })
        }
        other => Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            format!("unknown method {other}"),
        )),
    }
}

pub(crate) fn checked_stream_id(id: Option<String>) -> Result<Option<String>, ErrorObject> {
    match id {
        Some(id) if id.is_empty() || id.len() > 128 => {
            Err(invalid("streamId must be 1 to 128 characters"))
        }
        id => Ok(id),
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
    /// The switches are the call's own `settings` when a local client sent them, else the
    /// stored ones.
    fn check(
        accounts: &dyn Accounts,
        account_id: &'a str,
        thread_id: &'a str,
        feature: &'static str,
        settings: Option<&AiSettings>,
        provider: &AiProviderParams,
        stored: &StoredAi,
    ) -> Result<Self, ErrorObject> {
        accounts.check(account_id)?;
        let settings = settings.unwrap_or(stored.switches());
        check_switches(settings, account_id, feature)?;
        Ok(ThreadCall {
            account_id,
            thread_id,
            config: resolve_provider(provider, Some(feature), stored)?,
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
        let mut context = mail_context(
            store,
            self.account_id,
            self.thread_id,
            Bodies::NewestFirst(body_chars),
        )?;
        context.owner = owner;
        context.include_addresses = self.include_addresses;
        Ok(context)
    }
}

/// `not-enabled` unless the account is opted in and `feature` is on.
pub(crate) fn check_switches(
    settings: &AiSettings,
    account_id: &str,
    feature: &'static str,
) -> Result<(), ErrorObject> {
    if !settings.account_enabled(account_id) {
        return Err(failed(AiError::NotEnabled {
            what: "AI for this account",
        }));
    }
    if !settings.feature_enabled(feature) {
        return Err(failed(AiError::NotEnabled {
            what: feature_name(feature),
        }));
    }
    Ok(())
}

fn feature_name(feature: &str) -> &'static str {
    match feature {
        ai_feature::SUMMARY => "the summary feature",
        ai_feature::REPLY => "the reply feature",
        _ => "the triage feature",
    }
}

pub(crate) fn owner(statuses: Vec<AccountStatus>, account_id: &str) -> Person {
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

/// Which bodies [`mail_context`] reads.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Bodies<'a> {
    /// Newest first until this many characters; older messages go with `body_omitted`.
    NewestFirst(usize),
    /// Only the newest message not from one of `owners` (the account's address and its
    /// aliases'), the one triage judges; the others go with their headers alone.
    LatestInbound { owners: &'a [String] },
}

/// Whether `address` is one of `owners`.
pub(crate) fn is_own(owners: &[String], address: &str) -> bool {
    owners.iter().any(|o| o.eq_ignore_ascii_case(address))
}

/// The thread as the model may see it: drafts, hidden
/// messages, messages only in Spam or Trash and encrypted ones left out; bodies as plain
/// text without quoted replies, as `bodies` says; the participants of the messages kept.
pub(crate) fn mail_context(
    store: &ReadStore,
    account_id: &str,
    thread_id: &str,
    bodies: Bodies<'_>,
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
    let mut left = match bodies {
        Bodies::NewestFirst(chars) => chars,
        Bodies::LatestInbound { .. } => usize::MAX,
    };
    let mut inbound_seen = false;
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
        let take = match bodies {
            Bodies::NewestFirst(_) => left > 0,
            Bodies::LatestInbound { owners } => {
                let from = m["from"][0]["email"].as_str().unwrap_or("");
                let inbound = !inbound_seen && !is_own(owners, from);
                inbound_seen |= inbound;
                inbound
            }
        };
        let body_omitted = matches!(bodies, Bodies::NewestFirst(_)) && !take && has_text;
        let body = if take {
            match stored {
                Some(stored) => Some(body_text(&stored, m["plaintext"] == json!(true))),
                None => snippet.map(str::to_owned),
            }
            .filter(|b| !b.trim().is_empty())
        } else {
            None
        };
        // Counted as tern-ai sends it, without the signature.
        if let Some(b) = &body {
            left = left.saturating_sub(tern_ai::strip_signature(b).chars().count());
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
        tern_mime::flatten_html_with(stored, false, false)
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
    /// The connection's `ai.triage.run` jobs, for `ai.cancel {jobId}`. They outlive the
    /// connection: their results are stored, not sent.
    jobs: Vec<Weak<TriageJob>>,
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

    /// Remembers a triage job this connection started.
    pub fn add_job(&self, job: &Arc<TriageJob>) {
        let mut st = self.lock();
        st.jobs.retain(|j| j.strong_count() > 0);
        st.jobs.push(Arc::downgrade(job));
    }

    /// `ai.cancel`. An id that isn't streaming (it just finished) is not an error. A job is
    /// cancelled only by the connection that started it.
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
            (None, Some(id)) => {
                for job in self.lock().jobs.iter().filter_map(Weak::upgrade) {
                    if job.id() == id {
                        job.cancel();
                    }
                }
            }
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

type UsageKey = (String, String, &'static str);

/// By day, provider id and feature, loaded from and saved to [`USAGE_PATH`] when set.
static USAGE: Mutex<BTreeMap<UsageKey, Totals>> = Mutex::new(BTreeMap::new());
/// `<config>/ai-usage.json`, set by [`init_usage`] (`serve`).
static USAGE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn feature_key(name: &str) -> Option<&'static str> {
    match name {
        ai_feature::SUMMARY => Some(ai_feature::SUMMARY),
        ai_feature::REPLY => Some(ai_feature::REPLY),
        ai_feature::TRIAGE => Some(ai_feature::TRIAGE),
        "test" => Some("test"),
        _ => None,
    }
}

/// Loads the counters from `<config_dir>/ai-usage.json` and saves them there from now on. A
/// file that doesn't read starts the counters anew.
pub fn init_usage(config_dir: &Path) {
    let path = config_dir.join(USAGE_FILE);
    let rows: Vec<AiUsageRow> = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            warn!(
                "-X Could not read {}: {e}; AI usage starts anew",
                path.display()
            );
            Vec::new()
        }),
        Err(_) => Vec::new(),
    };
    let mut usage = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    for r in rows {
        let Some(feature) = feature_key(&r.feature) else {
            continue;
        };
        let t = usage.entry((r.day, r.provider, feature)).or_default();
        t.input += r.input_tokens;
        t.output += r.output_tokens;
        t.requests += r.requests;
    }
    *USAGE_PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

fn rows(usage: &BTreeMap<UsageKey, Totals>, oldest: &str) -> Vec<AiUsageRow> {
    let mut rows: Vec<AiUsageRow> = usage
        .iter()
        .filter(|((day, _, _), _)| day.as_str() >= oldest)
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
    rows
}

/// Writes the counters beside the file, then renames it over the file.
fn save_usage(usage: &BTreeMap<UsageKey, Totals>) {
    let Some(path) = USAGE_PATH.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
        return;
    };
    let bytes = match serde_json::to_vec(&rows(usage, "")) {
        Ok(b) => b,
        Err(_) => return,
    };
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, &path)) {
        warn!("-X Could not write {}: {e}", path.display());
    }
}

/// Input and output tokens the provider used today.
pub(crate) fn tokens_today(provider_id: &str) -> u64 {
    let today = day_key(0);
    USAGE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|((day, provider, _), _)| *day == today && provider == provider_id)
        .map(|(_, t)| t.input + t.output)
        .sum()
}

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
    save_usage(&usage);
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
    serde_json::to_value(rows(&usage, &oldest)).map_err(internal)
}

// -- Capabilities ---------------------------------------------------------------------------

/// `capabilities.ai`, from the stored settings and keys: what a call without `config` and
/// `settings` (a relayed one, `tern ai`, background triage) can do. Not per account: a
/// feature is available once it is on for some account.
pub fn capabilities(stored: &StoredAi) -> BTreeMap<String, Capability> {
    [ai_feature::SUMMARY, ai_feature::REPLY, ai_feature::TRIAGE]
        .into_iter()
        .map(|f| (f.to_owned(), feature_capability(stored, f)))
        .collect()
}

/// `needs-engine-key` for a provider that needs a key while `integration:ai-<id>` holds
/// none; `rate-limited` only for triage, the one feature that pauses.
fn feature_capability(stored: &StoredAi, feature: &str) -> Capability {
    let unavailable = |reason: &str, config: Option<&ProviderConfig>| Capability {
        available: false,
        provider: config.map(|c| c.id.clone()),
        local: config.map(ProviderConfig::is_local),
        reason: Some(reason.to_owned()),
        ..Capability::default()
    };
    let switches = stored.switches();
    if !switches.feature_enabled(feature) || switches.accounts.is_empty() {
        return unavailable(capability_reason::NOT_SET_UP, None);
    }
    let Some(Ok(config)) = stored
        .feature_provider(feature)
        .and_then(|id| stored.provider(id))
    else {
        return unavailable(capability_reason::NOT_SET_UP, None);
    };
    match config.check() {
        Err(AiError::NotConfigured {
            what: "the API key",
        }) => return unavailable(capability_reason::NEEDS_ENGINE_KEY, Some(&config)),
        Err(_) => return unavailable(capability_reason::NOT_SET_UP, Some(&config)),
        Ok(()) => {}
    }
    let capped = check_daily_cap(&config.id, stored.daily_tokens(&config.id)).is_err();
    if capped {
        notify_at_midnight();
    }
    let paused = (feature == ai_feature::TRIAGE)
        .then(|| crate::ai_triage::paused(&config.id))
        .flatten()
        .or(capped.then_some("quota-exceeded"));
    match paused {
        Some("rate-limited") => unavailable(capability_reason::RATE_LIMITED, Some(&config)),
        Some(_) => unavailable(capability_reason::QUOTA_EXCEEDED, Some(&config)),
        None => Capability {
            available: true,
            provider: Some(config.id.clone()),
            local: Some(config.is_local()),
            ..Capability::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What tern-ai's results serialise to fits the tern-ipc types the clients' TS comes from.
    #[test]
    fn results_fit_the_protocol_schema() {
        use tern_ipc::schema::violations;
        let usage = tern_ai::TokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            estimated: true,
        };
        let cases = [
            (
                "AiModelInfo",
                serde_json::to_value(tern_ai::ModelInfo {
                    id: "m".into(),
                    name: "M".into(),
                    context_tokens: Some(8192),
                }),
            ),
            (
                "AiTestReport",
                serde_json::to_value(tern_ai::TestReport {
                    host: "localhost".into(),
                    model: "m".into(),
                    local: true,
                    latency_ms: 12,
                }),
            ),
            (
                "AiCompletion",
                serde_json::to_value(tern_ai::Completion {
                    text: "t".into(),
                    model: "m".into(),
                    cached: false,
                    usage: Some(usage),
                }),
            ),
            (
                "AiSuggestion",
                serde_json::to_value(tern_ai::Suggestion {
                    label: "Yes".into(),
                    text: "Yes, see you then.".into(),
                }),
            ),
            (
                "ProviderErrorJson",
                serde_json::to_value(tern_ai::AiError::RateLimited {
                    retry_after: Some(std::time::Duration::from_secs(3)),
                }),
            ),
            (
                "IntegrationTestReport",
                serde_json::to_value(tern_integrations::TestReport {
                    host: "api-free.deepl.com".into(),
                    usage: Some(tern_integrations::Usage {
                        used: 5,
                        limit: Some(500_000),
                    }),
                }),
            ),
        ];
        for (def, value) in cases {
            let value = value.unwrap();
            assert_eq!(
                violations(&value, def),
                Vec::<String>::new(),
                "{def}: {value}"
            );
        }
    }

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
                kind: tern_ai::ProviderKind::Ollama,
                model: "m".into(),
                feature,
                usage: tern_ai::TokenUsage {
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

    fn stored(values: Value) -> StoredAi {
        StoredAi::from_values(values.as_object().unwrap())
    }

    #[test]
    fn providers_come_from_the_call_or_the_store() {
        let none = StoredAi::default();
        let by_id = AiProviderParams {
            config: None,
            provider: Some("p".into()),
        };
        let e = resolve_provider(&by_id, None, &none).unwrap_err();
        assert_eq!(e.code, code::AI_FAILED);
        assert_eq!(e.data.unwrap()["key"], "not-configured");
        let e = resolve_provider(
            &AiProviderParams {
                config: Some(json!({"id": "p", "kind": "nope"})),
                provider: None,
            },
            None,
            &none,
        )
        .unwrap_err();
        assert_eq!(e.code, code::INVALID_PARAMS);

        let s = stored(json!({
            "core.ai.providers": [
                {"id": "p", "kind": "ollama", "name": "Laptop", "model": "llama3"},
                {"id": "q", "kind": "anthropic", "model": "claude"},
            ],
            "core.ai.features": {"triage": {"enabled": true, "provider": "q"}},
        }));
        assert_eq!(resolve_provider(&by_id, None, &s).unwrap().model, "llama3");
        let default = AiProviderParams::default();
        assert_eq!(
            resolve_provider(&default, Some(ai_feature::TRIAGE), &s)
                .unwrap()
                .id,
            "q"
        );
        assert!(resolve_provider(&default, Some(ai_feature::SUMMARY), &s).is_err());
    }

    #[test]
    fn stored_switches_and_limits() {
        let s = stored(json!({
            "core.ai.accounts": ["a1", 3],
            "core.ai.features": {"summary": true, "triage": {"enabled": false}},
            "core.ai.includeAddresses": true,
            "core.ai.limits": {"triagePerHour": 10, "dailyTokens": {"p": 5000, "q": "x"}},
        }));
        assert!(s.switches().account_enabled("a1"));
        assert_eq!(s.switches().accounts.len(), 1);
        assert!(s.switches().feature_enabled("summary"));
        assert!(!s.switches().feature_enabled("triage"));
        assert!(s.switches().include_addresses);
        assert_eq!(s.triage_per_hour(), 10);
        assert_eq!(
            (s.daily_tokens("p"), s.daily_tokens("q")),
            (Some(5000), None)
        );
        assert_eq!(
            StoredAi::default().triage_per_hour(),
            DEFAULT_TRIAGE_PER_HOUR
        );
        let e = check_switches(s.switches(), "a2", ai_feature::SUMMARY).unwrap_err();
        assert_eq!(e.data.unwrap()["key"], "not-enabled");
        assert!(check_switches(s.switches(), "a1", ai_feature::TRIAGE).is_err());
        assert!(check_switches(s.switches(), "a1", ai_feature::SUMMARY).is_ok());
    }

    #[test]
    fn triage_capability_follows_the_store() {
        let reason = |v: Value| {
            let caps = capabilities(&stored(v));
            assert_eq!(
                caps[ai_feature::SUMMARY].reason.as_deref(),
                Some("not-set-up")
            );
            caps[ai_feature::TRIAGE].reason.clone()
        };
        assert_eq!(reason(json!({})).as_deref(), Some("not-set-up"));
        let on = |kind: &str| {
            json!({
                "core.ai.providers": [{"id": "cap-p", "kind": kind, "model": "m"}],
                "core.ai.features": {"triage": {"enabled": true, "provider": "cap-p"}},
                "core.ai.accounts": ["a1"],
            })
        };
        assert_eq!(reason(on("ollama")), None);
        assert_eq!(reason(on("anthropic")).as_deref(), Some("needs-engine-key"));
        // A key stored before settings.set refused one is not used, nor shown.
        let mut keyed = on("anthropic");
        keyed["core.ai.providers"][0]["apiKey"] = json!("sk-stored");
        assert_eq!(reason(keyed.clone()).as_deref(), Some("needs-engine-key"));
        let s = stored(keyed);
        assert!(s.provider("cap-p").unwrap().unwrap().api_key.is_none());
        assert!(!format!("{s:?}").contains("sk-stored"));
        let caps = capabilities(&stored(on("ollama")));
        let triage = &caps[ai_feature::TRIAGE];
        assert_eq!(
            (triage.provider.as_deref(), triage.local),
            (Some("cap-p"), Some(true))
        );
    }

    #[test]
    fn stored_providers_take_their_keys_from_the_secret_store() {
        let values = json!({
            "core.ai.providers": [
                {"id": "claude", "kind": "anthropic", "url": "https://api.anthropic.com", "model": "m"},
                {"id": "blank", "kind": "anthropic", "model": "m"},
                {"id": "other", "kind": "anthropic", "model": "m"},
            ],
            "core.ai.features": {
                "summary": {"enabled": true, "provider": "claude"},
                "reply": {"enabled": true, "provider": "blank"},
                "triage": {"enabled": true, "provider": "claude"},
            },
            "core.ai.accounts": ["a1"],
        });
        let secrets = |name: &str| match name {
            "integration:ai-claude" => Some("sk-engine".to_owned()),
            "integration:ai-blank" => Some("  ".to_owned()),
            _ => None,
        };
        let values = values.as_object().unwrap();
        let s = StoredAi::read_with(&|k| values.get(k).cloned(), &secrets);
        let key = |c: &ProviderConfig| c.api_key.as_ref().map(|k| k.expose().to_owned());
        assert_eq!(
            key(&s.provider("claude").unwrap().unwrap()).as_deref(),
            Some("sk-engine")
        );
        assert_eq!(key(&s.provider("blank").unwrap().unwrap()), None);
        assert!(!format!("{s:?}").contains("sk-engine"));

        let caps = capabilities(&s);
        for f in [ai_feature::SUMMARY, ai_feature::TRIAGE] {
            assert!(caps[f].available, "{f}: {:?}", caps[f]);
            assert_eq!(caps[f].credentials, None);
        }
        assert_eq!(
            caps[ai_feature::REPLY].reason.as_deref(),
            Some("needs-engine-key")
        );
        // Without the secret store, the same settings need the key.
        let none = StoredAi::read_with(&|k| values.get(k).cloned(), &|_| None);
        assert_eq!(
            capabilities(&none)[ai_feature::TRIAGE].reason.as_deref(),
            Some("needs-engine-key")
        );

        // A local client's config without a key gets the stored one for the same server only.
        let sent = |config: Value| {
            let p = AiProviderParams {
                config: Some(config),
                provider: None,
            };
            key(&resolve_provider(&p, None, &s).unwrap())
        };
        let draft = json!({"id": "claude", "kind": "anthropic", "model": "draft"});
        assert_eq!(sent(draft).as_deref(), Some("sk-engine"));
        let typed =
            json!({"id": "claude", "kind": "anthropic", "model": "m", "apiKey": "sk-typed"});
        assert_eq!(sent(typed).as_deref(), Some("sk-typed"));
        let elsewhere = json!({"id": "claude", "kind": "anthropic", "model": "m",
                               "url": "https://evil.example"});
        assert_eq!(sent(elsewhere), None);
        let compatible = json!({"id": "claude", "kind": "openai", "model": "m",
                                "url": "https://api.anthropic.com"});
        assert_eq!(sent(compatible), None);
        assert_eq!(
            sent(json!({"id": "other", "kind": "anthropic", "model": "m"})),
            None
        );
    }

    #[test]
    fn keys_go_with_their_provider_or_its_server() {
        let providers = json!([
            {"id": "claude", "kind": "anthropic", "model": "m"},
            {"id": "openai", "kind": "openai", "url": "https://api.openai.com/v1/", "model": "m"},
            {"id": "laptop", "kind": "ollama", "model": "m"},
            {"id": "broken", "kind": "nonsense"},
        ]);
        let unbound = |after: Value| unbound_keys(Some(&providers), Some(&after));
        // The same servers, spelled otherwise, and another model keep every key but the
        // one that doesn't read as a provider.
        assert_eq!(
            unbound(json!([
                {"id": "claude", "kind": "anthropic", "url": "https://api.anthropic.com/v1",
                 "model": "other"},
                {"id": "openai", "kind": "openai", "model": "m"},
                {"id": "laptop", "kind": "ollama", "url": "http://127.0.0.1:11434", "model": "m"},
                {"id": "broken", "kind": "nonsense"},
            ])),
            ["broken"]
        );
        assert_eq!(
            unbound(json!([
                {"id": "claude", "kind": "anthropic", "url": "http://127.0.0.1:9999",
                 "model": "m"},
                {"id": "openai", "kind": "anthropic", "url": "https://api.openai.com/v1"},
            ])),
            ["broken", "claude", "laptop", "openai"]
        );
        assert_eq!(
            unbound_keys(Some(&providers), None),
            ["broken", "claude", "laptop", "openai"]
        );
        assert!(unbound_keys(None, Some(&providers)).is_empty());
    }

    #[test]
    fn the_daily_cap_runs_to_midnight() {
        record_usage(&UsageRecord {
            provider_id: "cap-test".to_owned(),
            kind: tern_ai::ProviderKind::Ollama,
            model: "m".into(),
            feature: Feature::Summary,
            usage: tern_ai::TokenUsage {
                input_tokens: 90,
                output_tokens: 20,
                estimated: false,
            },
        });
        assert!(check_daily_cap("cap-test", None).is_ok());
        assert!(check_daily_cap("cap-test", Some(1000)).is_ok());
        let e = check_daily_cap("cap-test", Some(100)).unwrap_err();
        assert_eq!(e.key(), "quota-exceeded");
        let wait = e.retry_after().unwrap();
        assert!(wait <= Duration::from_secs(25 * 3600), "{wait:?}");
    }

    #[test]
    fn usage_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let rows = vec![AiUsageRow {
            day: day_key(0),
            provider: "file-test".into(),
            feature: "triage".into(),
            input_tokens: 7,
            output_tokens: 1,
            requests: 1,
        }];
        std::fs::write(
            dir.path().join(USAGE_FILE),
            serde_json::to_vec(&rows).unwrap(),
        )
        .unwrap();
        init_usage(dir.path());
        assert_eq!(tokens_today("file-test"), 8);
        record_usage(&UsageRecord {
            provider_id: "file-test".to_owned(),
            kind: tern_ai::ProviderKind::Ollama,
            model: "m".into(),
            feature: Feature::Triage,
            usage: tern_ai::TokenUsage {
                input_tokens: 2,
                output_tokens: 0,
                estimated: true,
            },
        });
        let saved: Vec<AiUsageRow> =
            serde_json::from_slice(&std::fs::read(dir.path().join(USAGE_FILE)).unwrap()).unwrap();
        let mine = saved.iter().find(|r| r.provider == "file-test").unwrap();
        assert_eq!((mine.input_tokens, mine.requests), (9, 2));
        *USAGE_PATH.lock().unwrap() = None;
    }
}
