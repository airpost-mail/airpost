// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Model providers for the engine's AI features: thread summaries, reply suggestions,
//! reply drafts and inbox triage.
//!
//! Three wire formats: OpenAI-compatible chat completions, Anthropic messages and Ollama's
//! native chat. Transport-free and store-free: the engine exposes [`Ai`] as the `ai.*`
//! socket methods, checks the account and feature switches, and fills a [`MailContext`]
//! from its database. Every call carries its provider config (URL, key) because the
//! desktop owns the credentials until the engine does; nothing here persists them.
//!
//! Calls block the calling thread, so call them from an engine thread and never from
//! inside a tokio runtime.
//!
//! The model never acts: its output is text for the user to read or edit, and triage
//! values from a fixed vocabulary. Logs carry the provider kind, host, model, status,
//! token counts and timing; prompts, answers, keys and addresses are never logged, never
//! part of a cache key in clear, and redacted from `Debug` output.

mod cancel;
mod config;
mod context;
mod error;
mod http;
mod prompt;
mod reply;
mod triage;
mod wire;

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use airpost_integrations::{CacheKey, KeyBuilder, TtlCache};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use airpost_integrations::Secret;
pub use cancel::{CancelToken, Sink};
pub use config::{
    ProviderConfig, ProviderKind, ANTHROPIC_URL, CLOUD_CONTEXT_TOKENS, CLOUD_MAX_OUTPUT_TOKENS,
    OLLAMA_CONTEXT_TOKENS, OLLAMA_MAX_OUTPUT_TOKENS, OLLAMA_URL, OPENAI_URL,
};
pub use context::{
    estimate_tokens, strip_signature, ContextMessage, MailContext, Person, RenderedContext,
};
pub use error::{AiError, Result};
pub use prompt::{MAX_SUGGESTIONS, PROMPT_VERSION};
pub use reply::Suggestion;
pub use triage::{Category, Triage};

use cancel::Discard;
use config::Target;
use http::{ChatFailure, ChatOutput, Http, Timeouts};
use wire::ChatRequest;

/// Room left for the request's framing (roles, separators) on top of the estimated prompt.
const PROMPT_MARGIN_TOKENS: usize = 64;
/// The thread always gets at least this much, even when a small window is mostly output.
const MIN_CONTEXT_TOKENS: usize = 256;
/// The output limit is lowered to fit the window, but never below this.
const MIN_OUTPUT_TOKENS: usize = 128;
/// Enough for a one-word answer after a reasoning model's thinking.
const TEST_MAX_OUTPUT_TOKENS: u32 = 128;
/// The Anthropic model list is paged; more pages than this would be a provider bug.
const MAX_MODEL_PAGES: usize = 10;

/// Called once per provider request the provider accepted, with its token counts: also
/// for one that failed, was cut or was cancelled after that (estimated where the provider
/// sent no usage). The engine keeps the `ai.usage` counters with it.
pub type UsageObserver = Arc<dyn Fn(&UsageRecord) + Send + Sync>;

/// Timeouts, cache bounds and the usage hook. [`Options::default`] is what the engine uses.
#[derive(Clone)]
pub struct Options {
    pub connect_timeout: Duration,
    /// Until the first answer token. Local models take a while to load.
    pub first_token_timeout: Duration,
    /// The whole streamed answer.
    pub total_timeout: Duration,
    /// The model list.
    pub models_timeout: Duration,
    pub summary_ttl: Duration,
    pub max_cached_summaries: usize,
    pub suggestion_ttl: Duration,
    pub max_cached_suggestions: usize,
    /// Before the one retry of a non-streamed call after `offline` or `provider-error`.
    pub retry_delay: Duration,
    pub usage_observer: Option<UsageObserver>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            connect_timeout: Duration::from_secs(10),
            first_token_timeout: Duration::from_secs(30),
            total_timeout: Duration::from_secs(120),
            models_timeout: Duration::from_secs(30),
            summary_ttl: Duration::from_secs(24 * 3600),
            max_cached_summaries: 500,
            suggestion_ttl: Duration::from_secs(3600),
            max_cached_suggestions: 200,
            retry_delay: Duration::from_secs(1),
            usage_observer: None,
        }
    }
}

impl fmt::Debug for Options {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Options")
            .field("connect_timeout", &self.connect_timeout)
            .field("first_token_timeout", &self.first_token_timeout)
            .field("total_timeout", &self.total_timeout)
            .field("models_timeout", &self.models_timeout)
            .field("summary_ttl", &self.summary_ttl)
            .field("max_cached_summaries", &self.max_cached_summaries)
            .field("suggestion_ttl", &self.suggestion_ttl)
            .field("max_cached_suggestions", &self.max_cached_suggestions)
            .field("retry_delay", &self.retry_delay)
            .field("usage_observer", &self.usage_observer.is_some())
            .finish()
    }
}

/// What a request was for, in logs and usage counters. `reply` covers suggestions and
/// drafts, as the feature switch does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Feature {
    Summary,
    Reply,
    Triage,
    /// [`Ai::test`].
    Test,
}

impl Feature {
    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Summary => "summary",
            Feature::Reply => "reply",
            Feature::Triage => "triage",
            Feature::Test => "test",
        }
    }
}

/// A model the provider offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
}

/// What [`Ai::test`] learned: where requests go and how long a minimal one took.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub host: String,
    pub model: String,
    pub local: bool,
    pub latency_ms: u64,
}

/// Tokens of one request, from the provider's usage fields or, where it gives none,
/// estimated at four characters per token.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
}

/// What [`UsageObserver`] receives.
#[derive(Clone, Debug)]
pub struct UsageRecord {
    pub provider_id: String,
    pub kind: ProviderKind,
    pub model: String,
    pub feature: Feature,
    pub usage: TokenUsage,
}

/// A summary or a draft. The text is plain text; the client renders it without links.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    pub text: String,
    /// The model that answered, as the provider named it.
    pub model: String,
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl fmt::Debug for Completion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Completion")
            .field("text_chars", &self.text.chars().count())
            .field("model", &self.model)
            .field("cached", &self.cached)
            .field("usage", &self.usage)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryOpts {
    /// Ask the model again even when a summary is cached; the new one replaces it.
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyOpts {
    /// Capped at [`MAX_SUGGESTIONS`].
    #[serde(default = "default_suggestion_count")]
    pub count: usize,
}

impl Default for ReplyOpts {
    fn default() -> Self {
        ReplyOpts {
            count: default_suggestion_count(),
        }
    }
}

fn default_suggestion_count() -> usize {
    3
}

#[derive(Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftOpts {
    /// Free text the user typed ("say I'll call Tuesday").
    #[serde(default)]
    pub instruction: Option<String>,
    /// The text of a [`Suggestion`] the user picked.
    #[serde(default)]
    pub suggestion: Option<String>,
}

impl fmt::Debug for DraftOpts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DraftOpts")
            .field("instruction", &self.instruction.as_ref().map(|_| "***"))
            .field("suggestion", &self.suggestion.as_ref().map(|_| "***"))
            .finish()
    }
}

/// [`Ai::triage_until`]'s result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriageAnswer {
    pub triage: Triage,
    /// The model that answered, as the provider named it.
    pub model: String,
}

/// The provider clients and caches. One per engine process; `Send + Sync`, so connection
/// threads share it.
pub struct Ai {
    http: Http,
    options: Options,
    summaries: TtlCache<Completion>,
    suggestions: TtlCache<Vec<Suggestion>>,
    /// Base URLs of OpenAI-compatible servers that refused JSON mode (LM Studio accepts
    /// only `json_schema` or `text`); their JSON requests go without `response_format`.
    no_json_mode: Mutex<HashSet<String>>,
}

impl fmt::Debug for Ai {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ai")
            .field("options", &self.options)
            .field("cached_summaries", &self.summaries.len())
            .field("cached_suggestions", &self.suggestions.len())
            .finish()
    }
}

/// A generated answer before it becomes a feature's result.
struct Generated {
    text: String,
    model: String,
    usage: TokenUsage,
}

impl Ai {
    pub fn new() -> Result<Self> {
        Ai::with_options(Options::default())
    }

    pub fn with_options(options: Options) -> Result<Self> {
        Ok(Ai {
            http: Http::new(options.connect_timeout)?,
            summaries: TtlCache::new(options.max_cached_summaries),
            suggestions: TtlCache::new(options.max_cached_suggestions),
            no_json_mode: Mutex::default(),
            options,
        })
    }

    /// Drops every cached summary and suggestion (account reset, `engine.resetDatabase`).
    pub fn clear_cache(&self) {
        self.summaries.clear();
        self.suggestions.clear();
    }

    /// The models the provider offers: `GET /v1/models` (OpenAI-compatible, Anthropic) or
    /// `GET /api/tags` (Ollama). Needs no model in `config`.
    pub fn models(&self, config: &ProviderConfig) -> Result<Vec<ModelInfo>> {
        let target = config.target(false)?;
        let mut models = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let (url, req) = wire::models_request(self.http.client(), &target, after.as_deref())?;
            let body = self
                .http
                .get_json(&target, &url, req, self.options.models_timeout)?;
            let (page, next) = wire::parse_models(target.kind, &body);
            models.extend(page);
            match next {
                Some(n) => after = Some(n),
                None => break,
            }
        }
        Ok(models)
    }

    /// The cheapest request that proves the URL, key and model work: a one-word answer.
    pub fn test(&self, config: &ProviderConfig) -> Result<TestReport> {
        let target = config.target(true)?;
        let req = ChatRequest {
            system: prompt::TEST_SYSTEM.to_owned(),
            user: prompt::TEST_USER.to_owned(),
            max_output_tokens: TEST_MAX_OUTPUT_TOKENS,
            json_schema: None,
        };
        let started = Instant::now();
        let out = self.chat(config, &target, Feature::Test, &req, &mut Discard)?;
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let generated = self.finish(config, &target, Feature::Test, &req, out);
        Ok(TestReport {
            host: target.host,
            model: generated.model,
            local: config.is_local(),
            latency_ms,
        })
    }

    /// A short summary of the thread, streamed into `sink`. Cached for
    /// [`Options::summary_ttl`] by provider, model, prompt version and thread content;
    /// a cached summary is returned whole with `cached: true`, no usage (it cost nothing)
    /// and nothing streamed.
    pub fn summarize(
        &self,
        config: &ProviderConfig,
        context: &MailContext,
        opts: &SummaryOpts,
        sink: &mut dyn Sink,
    ) -> Result<Completion> {
        let target = config.target(true)?;
        let key = cache_key("summary", config, &target, context, "");
        if !opts.refresh {
            if let Some(mut hit) = self.summaries.get(&key) {
                hit.cached = true;
                hit.usage = None;
                return Ok(hit);
            }
        }
        let g = self.generate(
            config,
            &target,
            Feature::Summary,
            context,
            prompt::summary(),
            None,
            sink,
            false,
        )?;
        let completion = completion(g)?;
        self.summaries
            .insert(key, completion.clone(), self.options.summary_ttl);
        Ok(completion)
    }

    /// Up to `opts.count` one-line replies to the latest message. Cached for
    /// [`Options::suggestion_ttl`]; retried once after `offline` or `provider-error`.
    pub fn suggest_replies(
        &self,
        config: &ProviderConfig,
        context: &MailContext,
        opts: &ReplyOpts,
    ) -> Result<Vec<Suggestion>> {
        self.suggest_replies_until(config, context, opts, None)
    }

    /// [`Ai::suggest_replies`], stopped with `cancelled` once `cancel` is cancelled.
    pub fn suggest_replies_until(
        &self,
        config: &ProviderConfig,
        context: &MailContext,
        opts: &ReplyOpts,
        cancel: Option<&CancelToken>,
    ) -> Result<Vec<Suggestion>> {
        let target = config.target(true)?;
        let count = opts.count.clamp(1, MAX_SUGGESTIONS);
        let key = cache_key("suggestions", config, &target, context, &count.to_string());
        if let Some(hit) = self.suggestions.get(&key) {
            return Ok(hit);
        }
        let g = self.generate(
            config,
            &target,
            Feature::Reply,
            context,
            prompt::suggestions(count),
            Some(prompt::suggestions_schema()),
            &mut cancel::Until(cancel),
            true,
        )?;
        let suggestions = reply::parse(&g.text, count);
        if suggestions.is_empty() {
            return Err(AiError::provider("the model suggested no replies"));
        }
        self.suggestions
            .insert(key, suggestions.clone(), self.options.suggestion_ttl);
        Ok(suggestions)
    }

    /// A full reply from the owner to the latest message, as plain text streamed into
    /// `sink`. Never cached: the user asks again to get another.
    pub fn draft_reply(
        &self,
        config: &ProviderConfig,
        context: &MailContext,
        opts: &DraftOpts,
        sink: &mut dyn Sink,
    ) -> Result<Completion> {
        let target = config.target(true)?;
        let task = prompt::draft(opts.suggestion.as_deref(), opts.instruction.as_deref());
        let g = self.generate(
            config,
            &target,
            Feature::Reply,
            context,
            task,
            None,
            sink,
            false,
        )?;
        completion(g)
    }

    /// The latest message's category. An answer outside the schema is
    /// [`Triage::fallback`], not an error; a failed request is retried once after
    /// `offline` or `provider-error`.
    pub fn triage(&self, config: &ProviderConfig, context: &MailContext) -> Result<Triage> {
        Ok(self.triage_until(config, context, None)?.triage)
    }

    /// [`Ai::triage`] with the model that answered, stopped with `cancelled` once `cancel`
    /// is cancelled.
    pub fn triage_until(
        &self,
        config: &ProviderConfig,
        context: &MailContext,
        cancel: Option<&CancelToken>,
    ) -> Result<TriageAnswer> {
        let target = config.target(true)?;
        let g = self.generate(
            config,
            &target,
            Feature::Triage,
            context,
            prompt::triage(),
            Some(prompt::triage_schema()),
            &mut cancel::Until(cancel),
            true,
        )?;
        Ok(TriageAnswer {
            triage: Triage::from_answer(&g.text),
            model: g.model,
        })
    }

    fn timeouts(&self) -> Timeouts {
        Timeouts {
            first_token: self.options.first_token_timeout,
            total: self.options.total_timeout,
        }
    }

    /// Budgets the context into the model's window, sends the request and records usage.
    /// The output limit is lowered when the window couldn't hold it, the prompt and
    /// [`MIN_CONTEXT_TOKENS`] of thread.
    #[allow(clippy::too_many_arguments)]
    fn generate(
        &self,
        config: &ProviderConfig,
        target: &Target,
        feature: Feature,
        context: &MailContext,
        task: String,
        json_schema: Option<Value>,
        sink: &mut dyn Sink,
        retry: bool,
    ) -> Result<Generated> {
        let system = prompt::system(&context.owner_name());
        let reserved = estimate_tokens(&system) + estimate_tokens(&task) + PROMPT_MARGIN_TOKENS;
        let window = target.context_tokens as usize;
        let max_output = (target.max_output_tokens as usize)
            .min(window.saturating_sub(reserved + MIN_CONTEXT_TOKENS))
            .max(MIN_OUTPUT_TOKENS);
        let budget = window
            .saturating_sub(max_output)
            .saturating_sub(reserved)
            .max(MIN_CONTEXT_TOKENS);
        let rendered = context.render(budget);
        let json_mode = json_schema.is_some()
            && !(target.kind == ProviderKind::OpenAi
                && self.no_json_mode.lock().unwrap().contains(&target.base));
        let mut req = ChatRequest {
            system,
            user: format!("{}\n\n{task}", rendered.text),
            max_output_tokens: u32::try_from(max_output).unwrap_or(u32::MAX),
            json_schema: json_schema.filter(|_| json_mode),
        };
        let out = match self.chat(config, target, feature, &req, sink) {
            Err(e) if refused_json_mode(target, &req, &e) => {
                self.no_json_mode
                    .lock()
                    .unwrap()
                    .insert(target.base.clone());
                // The prompt asks for JSON too, and both parsers find it in plain text.
                req.json_schema = None;
                self.chat(config, target, feature, &req, sink)
            }
            other => other,
        };
        let out = match out {
            Err(e) if retry && matches!(e, AiError::Offline { .. } | AiError::Provider { .. }) => {
                std::thread::sleep(self.options.retry_delay);
                self.chat(config, target, feature, &req, sink)?
            }
            other => other?,
        };
        Ok(self.finish(config, target, feature, &req, out))
    }

    /// One provider request; usage is recorded for an accepted request that then failed.
    fn chat(
        &self,
        config: &ProviderConfig,
        target: &Target,
        feature: Feature,
        req: &ChatRequest,
        sink: &mut dyn Sink,
    ) -> Result<ChatOutput> {
        match self.http.chat(target, feature, req, sink, self.timeouts()) {
            Ok(out) => Ok(out),
            Err(ChatFailure { error, partial }) => {
                if let Some(out) = partial {
                    self.finish(config, target, feature, req, out);
                }
                Err(error)
            }
        }
    }

    fn finish(
        &self,
        config: &ProviderConfig,
        target: &Target,
        feature: Feature,
        req: &ChatRequest,
        out: ChatOutput,
    ) -> Generated {
        let estimated = out.input_tokens.is_none() || out.output_tokens.is_none();
        let usage = TokenUsage {
            input_tokens: out.input_tokens.unwrap_or_else(|| {
                (estimate_tokens(&req.system) + estimate_tokens(&req.user)) as u64
            }),
            output_tokens: out
                .output_tokens
                .unwrap_or_else(|| estimate_tokens(&out.text) as u64),
            estimated,
        };
        let model = out
            .model
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| target.model.clone());
        if let Some(observer) = &self.options.usage_observer {
            observer(&UsageRecord {
                provider_id: config.id.clone(),
                kind: target.kind,
                model: model.clone(),
                feature,
                usage,
            });
        }
        Generated {
            text: out.text,
            model,
            usage,
        }
    }
}

/// A compatible server's 400 for `response_format: json_object`.
fn refused_json_mode(target: &Target, req: &ChatRequest, e: &AiError) -> bool {
    let AiError::BadRequest {
        status: Some(400),
        message,
    } = e
    else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    target.kind == ProviderKind::OpenAi
        && req.json_schema.is_some()
        && (message.contains("response_format") || message.contains("json_object"))
}

fn completion(g: Generated) -> Result<Completion> {
    let text = g.text.trim();
    if text.is_empty() {
        return Err(AiError::provider("the model returned no text"));
    }
    Ok(Completion {
        text: text.to_owned(),
        model: g.model,
        cached: false,
        usage: Some(g.usage),
    })
}

fn cache_key(
    kind: &str,
    config: &ProviderConfig,
    target: &Target,
    context: &MailContext,
    extra: &str,
) -> CacheKey {
    let key = KeyBuilder::new(kind)
        .part(&config.id)
        .part(target.kind.as_str())
        .part(&target.base)
        .part(&target.model)
        .part(&PROMPT_VERSION.to_string())
        .part(&target.context_tokens.to_string())
        .part(&target.max_output_tokens.to_string())
        .part(extra);
    context.key_parts(key).finish()
}
