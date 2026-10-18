// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! AI triage in `serve` (docs/ai-plan.md section 4): each account's `aiTriage` thread takes
//! its new inbox mail (the `mail.new` items of its delta stream) and the `ai.triage.run` jobs
//! into a bounded queue, asks the triage provider about each thread's latest inbound message
//! and stores the answer as `tern-ai-triage` thread metadata through `SyncbackMetadataTask`'s
//! local phase, as the expiration executor writes its metadata. The plugin id is local-only
//! (`ms_net::LOCAL_ONLY_PLUGINS`), so no Tern server receives it.
//!
//! - Only for accounts opted in (`core.ai.accounts`) with triage on (`core.ai.features`),
//!   with the stored provider; one that needs a key waits for the engine to hold keys
//!   (desktop-client-plan stage 11), so new mail is triaged by keyless providers only.
//! - One request in flight per provider across accounts; at most `core.ai.limits.triagePerHour`
//!   (200) per account per hour; a `rate-limited` or `quota-exceeded` answer pauses every
//!   account on that provider until `retryAfter` (15 minutes without one; 1 s to 24 h), and
//!   the daily token cap holds them until midnight or until the cap is raised.
//!   `capabilities.ai.triage` is unavailable meanwhile.
//! - The switches and the provider are read again before each request, so a thread that
//!   waited goes to the provider triage uses then, or nowhere once triage is off for the
//!   account.
//! - A job's last `ai.triage.progress` has `done == total`, or `stopped` when it ended early.
//! - A value the user set (`source: "user"`) is never replaced; a thread already triaged for
//!   its latest inbound message with this prompt version is not asked about again.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use ms_ai::{AiError, CancelToken, Category, Person, ProviderConfig, TriageAnswer, PROMPT_VERSION};
use ms_core::constants::{thread, AI_TRIAGE_PLUGIN};
use ms_ipc::{
    ai_feature, code, AiProviderParams, AiTriageListParams, AiTriageListResult,
    AiTriageProgressParams, AiTriageRunParams, AiTriageRunResult, ErrorObject,
    Request as RpcRequest, AI_TRIAGE_RUN_MAX, READ_MAX_LIMIT,
};
use ms_store::model::metadata_value;
use ms_store::read::ReadStore;
use ms_store::{DeltaSink, Model, Query, Store, Task, Thread, MAIL_NEW};
use ms_sync::{EngineSettings, WorkerContext};
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::ai_api::{
    self, check_daily_cap, check_switches, failed, internal, invalid, is_own, mail_context,
    resolve_provider, Bodies, StoredAi,
};
use crate::dispatch::Reply;
use crate::read_api::ReadApi;
use crate::socket::Accounts;

/// New-mail items an account's queue holds; later ones are dropped until it drains.
pub const QUEUE_LIMIT: usize = 1000;
/// `ai.triage.run` jobs an account's queue holds.
const JOBS_LIMIT: usize = 8;
/// How long a `rate-limited` or `quota-exceeded` answer without `Retry-After` pauses triage.
const DEFAULT_PAUSE: Duration = Duration::from_secs(15 * 60);
/// The bounds of a provider's `Retry-After`: 0 would have triage ask again at once, and a
/// huge value would overflow the deadline. The daily cap runs to midnight, under a day.
const MIN_PAUSE: Duration = Duration::from_secs(1);
const MAX_PAUSE: Duration = Duration::from_secs(24 * 3600);
const HOUR: Duration = Duration::from_secs(3600);
/// How often a waiting worker looks at the account's stop flag.
const STOP_POLL: Duration = Duration::from_millis(500);
/// How long a worker waits on a pause, the daily cap or the hourly limit before it reads the
/// settings again: a cap raised or triage turned off meanwhile applies within this.
const RECHECK: Duration = Duration::from_secs(1);
/// `ai.triage.run` without `threadIds` or `sinceDays`.
const DEFAULT_SINCE_DAYS: u32 = 7;
/// What a value set by the user carries (`source`), which triage never replaces.
const USER_SOURCE: &str = "user";

// -- Jobs -----------------------------------------------------------------------------------

/// The progress callback of a job: `ai.triage.progress` to the connection that started it.
pub type Progress = Box<dyn Fn(&AiTriageProgressParams) + Send + Sync>;

/// The provider triage asks, looked up again before each request.
enum Source {
    /// The one the triage feature names (new mail, and `ai.triage.run` without a provider).
    Feature,
    /// The stored provider `ai.triage.run` named (`provider`).
    Stored(String),
    /// The config a local client sent with `ai.triage.run`.
    Given(ProviderConfig),
}

impl Source {
    /// The provider as the store has it now; `None` once it is gone or can't run.
    fn config(&self, stored: &StoredAi) -> Option<ProviderConfig> {
        let config = match self {
            Source::Given(config) => return Some(config.clone()),
            Source::Stored(id) => stored.provider(id)?.ok()?,
            Source::Feature => resolve_provider(
                &AiProviderParams::default(),
                Some(ai_feature::TRIAGE),
                stored,
            )
            .ok()?,
        };
        config.check().is_ok().then_some(config)
    }
}

/// One `ai.triage.run`: its threads, its provider and its progress callback.
pub struct Job {
    id: String,
    thread_ids: Vec<String>,
    source: Source,
    token: CancelToken,
    progress: Progress,
    /// Woken by a cancel, so a worker waiting on the job's behalf sees it at once.
    feed: Weak<Feed>,
    /// Threads done so far, and whether the last progress went out.
    reported: Mutex<(usize, bool)>,
}

impl Job {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `ai.cancel {jobId}`: the job stops before its next thread, and the request in flight
    /// is dropped.
    pub fn cancel(&self) {
        self.token.cancel();
        if let Some(feed) = self.feed.upgrade() {
            feed.changed.notify_all();
        }
    }

    fn report(&self, done: usize) {
        self.send(Some(done));
    }

    /// The job ended before its last thread: one last progress with `stopped`, unless the
    /// last one went out already.
    fn stop(&self) {
        self.send(None);
    }

    fn send(&self, done: Option<usize>) {
        let mut reported = self.reported.lock().unwrap_or_else(|e| e.into_inner());
        if reported.1 {
            return;
        }
        let total = self.thread_ids.len();
        let stopped = done.is_none();
        let done = done.unwrap_or(reported.0);
        *reported = (done, stopped || done >= total);
        (self.progress)(&AiTriageProgressParams {
            job_id: self.id.clone(),
            done,
            total,
            stopped,
        });
    }
}

// -- Feed -----------------------------------------------------------------------------------

enum Item {
    New { thread_id: String },
    Job(Arc<Job>),
}

#[derive(Default)]
struct FeedState {
    items: VecDeque<Item>,
    /// The job being worked on, cancelled with the feed.
    running: Option<Arc<Job>>,
    closed: bool,
    /// The queue overflowed since it last had room; logged once.
    overflowed: bool,
}

/// One account's triage queue, fed by its delta stream ([`Feed::tap`]) and `ai.triage.run`.
pub struct Feed {
    account_id: String,
    state: Mutex<FeedState>,
    changed: Condvar,
    /// The new-mail request in flight; cancelled when the account stops.
    cancel: CancelToken,
}

static FEEDS: Mutex<Vec<Arc<Feed>>> = Mutex::new(Vec::new());

impl Feed {
    /// A new feed for `account_id`, found by [`Feed::find`] until it is closed.
    pub fn register(account_id: &str) -> Arc<Feed> {
        let feed = Arc::new(Feed {
            account_id: account_id.to_owned(),
            state: Mutex::default(),
            changed: Condvar::new(),
            cancel: CancelToken::new(),
        });
        let mut feeds = FEEDS.lock().unwrap_or_else(|e| e.into_inner());
        feeds.retain(|f| f.account_id != account_id);
        feeds.push(feed.clone());
        feed
    }

    pub fn find(account_id: &str) -> Option<Arc<Feed>> {
        FEEDS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|f| f.account_id == account_id)
            .cloned()
    }

    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `inner`, with the account's `mail.new` items queued here first.
    pub fn tap(self: &Arc<Self>, inner: Box<dyn DeltaSink>) -> Box<dyn DeltaSink> {
        Box::new(Tap {
            feed: self.clone(),
            inner,
        })
    }

    /// The account stopped: the worker returns, the request in flight is dropped, the jobs
    /// running and queued end with a `stopped` progress and [`Feed::find`] no longer names
    /// the feed.
    pub fn close(self: &Arc<Self>) {
        let jobs: Vec<Arc<Job>> = {
            let mut st = self.lock();
            st.closed = true;
            let queued = st.items.drain(..).filter_map(|item| match item {
                Item::Job(job) => Some(job),
                Item::New { .. } => None,
            });
            let queued: Vec<Arc<Job>> = queued.collect();
            st.running.take().into_iter().chain(queued).collect()
        };
        self.cancel.cancel();
        self.changed.notify_all();
        for job in jobs {
            job.cancel();
            job.stop();
        }
        FEEDS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|f| !Arc::ptr_eq(f, self));
    }

    fn mail_new(&self, line: &str) {
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let mut st = self.lock();
        if st.closed {
            return;
        }
        for m in item["modelJSONs"].as_array().into_iter().flatten() {
            let Some(thread_id) = m["threadId"].as_str().filter(|t| !t.is_empty()) else {
                continue;
            };
            let queued = st
                .items
                .iter()
                .any(|i| matches!(i, Item::New { thread_id: t } if t == thread_id));
            if queued {
                continue;
            }
            let new_mail = st
                .items
                .iter()
                .filter(|i| matches!(i, Item::New { .. }))
                .count();
            if new_mail >= QUEUE_LIMIT {
                if !st.overflowed {
                    warn!("-X AI triage queue is full; new mail is not triaged until it drains");
                    st.overflowed = true;
                }
                continue;
            }
            st.items.push_back(Item::New {
                thread_id: thread_id.to_owned(),
            });
        }
        drop(st);
        self.changed.notify_all();
    }

    /// Queues `job` and runs `queued` before the worker can take it, so a response sent there
    /// precedes the job's progress; `ENGINE_BUSY` when the account already has
    /// [`JOBS_LIMIT`] waiting.
    fn push_job(&self, job: Arc<Job>, queued: impl FnOnce()) -> Result<(), ErrorObject> {
        let mut st = self.lock();
        if st.closed {
            return Err(stopped(&self.account_id));
        }
        let jobs = st
            .items
            .iter()
            .filter(|i| matches!(i, Item::Job(_)))
            .count();
        if jobs >= JOBS_LIMIT {
            return Err(ErrorObject::new(
                code::ENGINE_BUSY,
                format!("{jobs} triage jobs are waiting for this account; try again later"),
            ));
        }
        st.items.push_back(Item::Job(job));
        queued();
        drop(st);
        self.changed.notify_all();
        Ok(())
    }

    /// The next item, or `None` once closed or `stopping`.
    fn next(&self, stopping: &dyn Fn() -> bool) -> Option<Item> {
        let mut st = self.lock();
        loop {
            if st.closed || stopping() {
                return None;
            }
            if let Some(item) = st.items.pop_front() {
                if st.items.len() < QUEUE_LIMIT {
                    st.overflowed = false;
                }
                st.running = match &item {
                    Item::Job(job) => Some(job.clone()),
                    Item::New { .. } => None,
                };
                return Some(item);
            }
            st = self
                .changed
                .wait_timeout(st, STOP_POLL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Sleeps until `deadline`; true when the feed closed or `stopping` meanwhile.
    fn wait_until(&self, deadline: Instant, stopping: &dyn Fn() -> bool) -> bool {
        let mut st = self.lock();
        loop {
            if st.closed || stopping() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self
                .changed
                .wait_timeout(st, (deadline - now).min(STOP_POLL))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

struct Tap {
    feed: Arc<Feed>,
    inner: Box<dyn DeltaSink>,
}

impl DeltaSink for Tap {
    fn write_item(&mut self, model_class: &str, line: &str) {
        if model_class == MAIL_NEW {
            self.feed.mail_new(line);
        }
        self.inner.write_item(model_class, line);
    }
}

fn stopped(account_id: &str) -> ErrorObject {
    ErrorObject::new(
        code::ACCOUNT_STOPPED,
        format!("account {account_id} is not running"),
    )
}

// -- Provider gates -------------------------------------------------------------------------

#[derive(Default)]
struct Gate {
    /// Held for the request in flight.
    in_flight: Mutex<()>,
    /// Until when, and the error key that paused it.
    pause: Mutex<Option<(Instant, &'static str)>>,
}

static GATES: Mutex<Vec<(String, Arc<Gate>)>> = Mutex::new(Vec::new());

fn gate(provider_id: &str) -> Arc<Gate> {
    let mut gates = GATES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, g)) = gates.iter().find(|(id, _)| id == provider_id) {
        return g.clone();
    }
    let g = Arc::new(Gate::default());
    gates.push((provider_id.to_owned(), g.clone()));
    g
}

/// The error key (`rate-limited`, `quota-exceeded`) of the provider's pause, while it lasts.
pub fn paused(provider_id: &str) -> Option<&'static str> {
    let g = gate(provider_id);
    let pause = g.pause.lock().unwrap_or_else(|e| e.into_inner());
    pause
        .filter(|(until, _)| *until > Instant::now())
        .map(|(_, key)| key)
}

static ON_CAPABILITIES: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Called when a pause begins or ends: `serve` sends `capabilities.changed`.
pub fn on_capabilities_changed(f: Box<dyn Fn() + Send + Sync>) {
    let _ = ON_CAPABILITIES.set(f);
}

fn capabilities_changed() {
    if let Some(f) = ON_CAPABILITIES.get() {
        f();
    }
}

// -- Worker ---------------------------------------------------------------------------------

/// Starts the account's `aiTriage` thread on `feed`. `sync` mode keeps no settings and runs
/// none.
pub fn start(ctx: &WorkerContext, feed: Arc<Feed>) {
    let Some(settings) = ctx.settings.clone() else {
        return;
    };
    let c = ctx.clone();
    if let Err(e) = ctx.spawn(thread::AI_TRIAGE, move || {
        Worker {
            ctx: c,
            feed,
            settings,
            store: None,
            reads: None,
            recent: VecDeque::new(),
            capped: None,
        }
        .run()
    }) {
        error!("-X Could not start the {} thread: {e}", thread::AI_TRIAGE);
    }
}

/// How one thread's triage ended.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Next,
    /// The account is stopping, or the job was cancelled or can't go on.
    Stop,
}

/// What [`Worker::gates`] found.
enum Gates {
    /// The request may go.
    Open,
    /// It waited; the caller reads the settings again and asks once more.
    Waited,
    /// The account is stopping or the job was cancelled.
    Stopping,
}

struct Worker {
    ctx: WorkerContext,
    feed: Arc<Feed>,
    settings: Arc<dyn EngineSettings>,
    /// Opened on first use, on this thread: an account with triage off opens nothing.
    store: Option<Store>,
    reads: Option<ReadStore>,
    /// When this account's triage requests in the last hour went out.
    recent: VecDeque<Instant>,
    /// The provider whose daily cap holds this account's triage, while it does.
    capped: Option<String>,
}

impl Worker {
    fn run(mut self) {
        let ctx = self.ctx.clone();
        while let Some(item) = self.feed.next(&|| ctx.is_stopping()) {
            let flow = match item {
                Item::New { thread_id } => self.new_mail(&thread_id),
                Item::Job(job) => self.job(&job),
            };
            if flow == Flow::Stop && (ctx.is_stopping() || self.feed.lock().closed) {
                return;
            }
        }
    }

    fn stored(&self) -> StoredAi {
        StoredAi::read(&|k| self.settings.get(k))
    }

    fn account_id(&self) -> &str {
        self.ctx.account.id()
    }

    fn new_mail(&mut self, thread_id: &str) -> Flow {
        let token = self.feed.cancel.clone();
        self.triage(thread_id, &Source::Feature, &token, Flow::Next)
    }

    fn job(&mut self, job: &Arc<Job>) -> Flow {
        for (i, thread_id) in job.thread_ids.iter().enumerate() {
            if job.token.is_cancelled()
                || self.triage(thread_id, &job.source, &job.token, Flow::Stop) == Flow::Stop
            {
                job.stop();
                return Flow::Stop;
            }
            job.report(i + 1);
        }
        if job.thread_ids.is_empty() {
            job.report(0);
        }
        Flow::Next
    }

    fn reads(&mut self) -> Option<&ReadStore> {
        if self.reads.is_none() {
            match ReadStore::open(&self.ctx.config_dir) {
                Ok(r) => self.reads = Some(r),
                Err(e) => {
                    error!("-X AI triage could not open the database: {e}");
                    return None;
                }
            }
        }
        self.reads.as_ref()
    }

    /// Triages `thread_id` on the provider `source` names, once the provider's pause, its
    /// daily cap and the account's hourly limit let it. The switches and the provider are
    /// read again after every wait (A5); `off` is the flow once triage is off for the
    /// account or the provider is gone.
    fn triage(&mut self, thread_id: &str, source: &Source, token: &CancelToken, off: Flow) -> Flow {
        loop {
            if token.is_cancelled() {
                return Flow::Stop;
            }
            let stored = self.stored();
            if check_switches(stored.switches(), self.account_id(), ai_feature::TRIAGE).is_err() {
                return off;
            }
            let Some(config) = source.config(&stored) else {
                return off;
            };
            match self.gates(&config.id, &stored, token) {
                Gates::Open => {}
                Gates::Waited => continue,
                Gates::Stopping => return Flow::Stop,
            }
            let Some((context, latest)) = self.context(thread_id, &stored) else {
                return Flow::Next;
            };
            let answer = {
                let gate = gate(&config.id);
                let _in_flight = gate.in_flight.lock().unwrap_or_else(|e| e.into_inner());
                // Another account's answer may have paused the provider or used up its daily
                // cap meanwhile; the next round waits it out.
                if paused(&config.id).is_some()
                    || check_daily_cap(&config.id, stored.daily_tokens(&config.id)).is_err()
                {
                    continue;
                }
                self.recent.push_back(Instant::now());
                let ai = match ai_api::ai() {
                    Ok(ai) => ai,
                    Err(e) => {
                        error!("-X AI triage is unavailable: {}", e.message);
                        return Flow::Stop;
                    }
                };
                ai.triage_until(&config, &context, Some(token))
            };
            return match answer {
                Ok(answer) => self.save(thread_id, &latest, &answer),
                Err(e @ (AiError::RateLimited { .. } | AiError::QuotaExceeded { .. })) => {
                    pause(&config.id, &e);
                    continue;
                }
                Err(AiError::Cancelled) => Flow::Stop,
                Err(e) => {
                    warn!("-X AI triage of thread {thread_id} failed: {}", e.key());
                    Flow::Next
                }
            };
        }
    }

    /// The thread as triage sends it and the id of its latest inbound message, or `None`
    /// when there is nothing to ask: no such thread, nothing the model may read, no inbound
    /// message, a value the user set, or one already given for that message.
    fn context(
        &mut self,
        thread_id: &str,
        stored: &StoredAi,
    ) -> Option<(ms_ai::MailContext, String)> {
        let account = self.ctx.account.clone();
        let owners: Vec<String> = account.own_addresses().map(str::to_owned).collect();
        let reads = self.reads()?;
        let thread = reads.thread(account.id(), thread_id).ok()??;
        let existing = metadata_value(&thread, AI_TRIAGE_PLUGIN);
        if existing.is_some_and(is_users) {
            return None;
        }
        let mut context = mail_context(
            reads,
            account.id(),
            thread_id,
            Bodies::LatestInbound { owners: &owners },
        )
        .ok()?;
        let latest = context
            .messages
            .iter()
            .rev()
            .find(|m| !is_own(&owners, &m.from.address))?
            .id
            .clone();
        let current = existing.is_some_and(|v| {
            v["messageId"].as_str() == Some(latest.as_str())
                && v["v"].as_u64() == Some(u64::from(PROMPT_VERSION))
        });
        if current {
            return None;
        }
        let name = account.as_json()["name"].as_str().unwrap_or("").to_owned();
        context.owner = Person::new(name, account.email_address().to_owned());
        context.include_addresses = stored.switches().include_addresses;
        Some((context, latest))
    }

    /// Whether a request to `provider_id` may go: not while the provider is paused or used
    /// its daily cap, nor past this account's hourly limit. One wait lasts [`RECHECK`] at
    /// most.
    fn gates(&mut self, provider_id: &str, stored: &StoredAi, token: &CancelToken) -> Gates {
        let g = gate(provider_id);
        let until = g
            .pause
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|p| p.0);
        match until {
            Some(until) if until > Instant::now() => return self.wait(until, token),
            Some(_) => {
                let mut pause = g.pause.lock().unwrap_or_else(|e| e.into_inner());
                if pause.is_some_and(|(until, _)| until <= Instant::now()) {
                    *pause = None;
                    drop(pause);
                    info!("{}", ms_core::logmsg::ai_triage_resumed());
                    capabilities_changed();
                }
            }
            None => {}
        }
        // Not a pause of the gate: the cap is read from the settings each time, so raising
        // it lets triage go on at once.
        let held = self.capped.as_deref() == Some(provider_id);
        match check_daily_cap(provider_id, stored.daily_tokens(provider_id)) {
            Err(e) => {
                if !held {
                    self.capped = Some(provider_id.to_owned());
                    let wait = e.retry_after().unwrap_or_default();
                    warn!(
                        "{}",
                        ms_core::logmsg::ai_triage_paused(wait.as_secs(), e.key())
                    );
                    capabilities_changed();
                }
                return self.wait(Instant::now() + RECHECK, token);
            }
            Ok(()) if held => {
                self.capped = None;
                info!("{}", ms_core::logmsg::ai_triage_resumed());
                capabilities_changed();
            }
            Ok(()) => {}
        }
        while self.recent.front().is_some_and(|t| t.elapsed() >= HOUR) {
            self.recent.pop_front();
        }
        let per_hour = stored.triage_per_hour().max(1) as usize;
        if self.recent.len() >= per_hour {
            let until = self.recent[0] + HOUR;
            return self.wait(until, token);
        }
        Gates::Open
    }

    /// Sleeps until `until`, [`RECHECK`] at most.
    fn wait(&self, until: Instant, token: &CancelToken) -> Gates {
        let ctx = &self.ctx;
        let stopping = || ctx.is_stopping() || token.is_cancelled();
        if self
            .feed
            .wait_until(until.min(Instant::now() + RECHECK), &stopping)
        {
            Gates::Stopping
        } else {
            Gates::Waited
        }
    }

    /// Stores the answer as the thread's `tern-ai-triage` value, unless the user set one
    /// since it was read.
    fn save(&mut self, thread_id: &str, message_id: &str, answer: &TriageAnswer) -> Flow {
        if self.store.is_none() {
            match Store::open(&self.ctx.config_dir, self.ctx.deltas.clone()) {
                Ok(s) => self.store = Some(s),
                Err(e) => {
                    self.ctx.fail_with(&e);
                    return Flow::Stop;
                }
            }
        }
        let store = self.store.as_ref().expect("opened above");
        let thread = match store.find::<Thread>(&Query::new().equal("id", thread_id)) {
            Ok(Some(t)) => t,
            Ok(None) => return Flow::Next,
            Err(e) => {
                self.ctx.fail_with(&e);
                return Flow::Stop;
            }
        };
        if metadata_value(thread.json(), AI_TRIAGE_PLUGIN).is_some_and(is_users) {
            return Flow::Next;
        }
        let value = json!({
            "category": answer.triage.category.as_str(),
            "needsReply": answer.triage.needs_reply,
            "reason": answer.triage.reason,
            "messageId": message_id,
            "model": answer.model,
            "v": PROMPT_VERSION,
            "at": ms_core::Clock::now(&*self.ctx.clock),
        });
        let mut task = Task::create(
            "SyncbackMetadataTask",
            self.ctx.account.id(),
            &json!({
                "modelId": thread_id,
                "modelClassName": "thread",
                "pluginId": AI_TRIAGE_PLUGIN,
                "value": value,
            }),
        );
        if let Err(e) = self.ctx.tasks.perform_local(store, &self.ctx, &mut task) {
            self.ctx.fail_with(&e);
            return Flow::Stop;
        }
        self.ctx.foreground.interrupt();
        info!("{}", ms_core::logmsg::ai_triage_stored(thread_id));
        Flow::Next
    }
}

fn is_users(value: &Value) -> bool {
    value["source"].as_str() == Some(USER_SOURCE)
}

/// A `rate-limited` or `quota-exceeded` answer pauses every account on the provider.
fn pause(provider_id: &str, e: &AiError) {
    let wait = pause_for(e);
    *gate(provider_id)
        .pause
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some((Instant::now() + wait, e.key()));
    warn!(
        "{}",
        ms_core::logmsg::ai_triage_paused(wait.as_secs(), e.key())
    );
    capabilities_changed();
}

/// The provider's `Retry-After` within [`MIN_PAUSE`] and [`MAX_PAUSE`], [`DEFAULT_PAUSE`]
/// without one.
fn pause_for(e: &AiError) -> Duration {
    e.retry_after()
        .unwrap_or(DEFAULT_PAUSE)
        .clamp(MIN_PAUSE, MAX_PAUSE)
}

// -- Socket methods -------------------------------------------------------------------------

/// `ai.triage.run`: checks the account and provider, picks the threads and queues the job
/// on the account's feed. `respond` gets `{jobId, queued}` before any progress is sent.
pub fn run(
    req: &RpcRequest,
    reads: &mut ReadApi,
    accounts: &dyn Accounts,
    stored: &StoredAi,
    progress: Progress,
    respond: impl FnOnce(Value),
) -> Result<Arc<Job>, ErrorObject> {
    let p: AiTriageRunParams = req.params_as()?;
    accounts.check(&p.account_id)?;
    check_switches(stored.switches(), &p.account_id, ai_feature::TRIAGE)?;
    let config = resolve_provider(&p.provider, Some(ai_feature::TRIAGE), stored)?;
    config.check().map_err(failed)?;
    let source = match (&p.provider.config, &p.provider.provider) {
        (Some(c), _) if !c.is_null() => Source::Given(config),
        (_, Some(id)) => Source::Stored(id.clone()),
        _ => Source::Feature,
    };
    let thread_ids = match p.thread_ids {
        Some(ids) => {
            let mut unique: Vec<String> = Vec::new();
            for id in ids {
                if !unique.contains(&id) {
                    unique.push(id);
                }
            }
            unique.truncate(AI_TRIAGE_RUN_MAX);
            unique
        }
        None => {
            let days = p.since_days.unwrap_or(DEFAULT_SINCE_DAYS).clamp(1, 365);
            let store = reads.store()?;
            let inbox = store
                .category_for_role(&p.account_id, "inbox")
                .map_err(internal)?;
            let since = ms_core::Clock::now(&ms_core::SystemClock) - i64::from(days) * 86_400;
            store
                .category_threads_since(
                    &inbox.into_iter().collect::<Vec<_>>(),
                    since,
                    AI_TRIAGE_RUN_MAX,
                )
                .map_err(internal)?
        }
    };
    let feed = Feed::find(&p.account_id).ok_or_else(|| stopped(&p.account_id))?;
    let job = Arc::new(Job {
        id: uuid::Uuid::new_v4().to_string(),
        thread_ids,
        source,
        token: CancelToken::new(),
        progress,
        feed: Arc::downgrade(&feed),
        reported: Mutex::new((0, false)),
    });
    let value = serde_json::to_value(AiTriageRunResult {
        job_id: job.id.clone(),
        queued: job.thread_ids.len(),
    })
    .map_err(internal)?;
    feed.push_job(job.clone(), || respond(value))?;
    Ok(job)
}

/// `ai.triage.list`: the opted-in accounts' inbox threads with a triage value.
pub fn list(
    req: &RpcRequest,
    reads: &mut ReadApi,
    accounts: &dyn Accounts,
    stored: &StoredAi,
) -> Reply {
    let p: AiTriageListParams = req.params_as()?;
    if let Some(c) = &p.category {
        if Category::parse(c).is_none() {
            return Err(invalid(format!("unknown category {c:?}")));
        }
    }
    let named = match p.account_ids {
        Some(ids) => {
            for a in &ids {
                accounts.check(a)?;
            }
            ids
        }
        None => accounts
            .statuses()
            .into_iter()
            .map(|a| a.account_id)
            .collect(),
    };
    let store = reads.store()?;
    let mut inboxes = Vec::new();
    for a in named
        .iter()
        .filter(|a| stored.switches().account_enabled(a))
    {
        if let Some(id) = store.category_for_role(a, "inbox").map_err(internal)? {
            inboxes.push(id);
        }
    }
    let (thread_ids, total) = store
        .triaged_threads(
            &inboxes,
            AI_TRIAGE_PLUGIN,
            p.category.as_deref(),
            p.limit.min(READ_MAX_LIMIT),
            p.offset.unwrap_or(0),
        )
        .map_err(internal)?;
    serde_json::to_value(AiTriageListResult { thread_ids, total }).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_new_items_queue_once_per_thread_and_stop_at_the_bound() {
        let feed = Feed::register("feed-test");
        let line = |threads: &[&str]| {
            let models: Vec<Value> = threads
                .iter()
                .enumerate()
                .map(|(i, t)| json!({"id": format!("m{i}"), "threadId": t}))
                .collect();
            json!({"modelJSONs": models}).to_string()
        };
        feed.mail_new(&line(&["t1", "t1", "t2"]));
        feed.mail_new(&line(&["t1"]));
        let next = |feed: &Feed| match feed.next(&|| false) {
            Some(Item::New { thread_id }) => thread_id,
            _ => panic!("new mail"),
        };
        assert_eq!(next(&feed), "t1");
        assert_eq!(next(&feed), "t2");

        let many: Vec<String> = (0..QUEUE_LIMIT + 5).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        feed.mail_new(&line(&refs));
        assert_eq!(feed.lock().items.len(), QUEUE_LIMIT);
        assert!(Feed::find("feed-test").is_some());
        feed.close();
        assert!(feed.lock().items.is_empty());
        assert!(feed.next(&|| false).is_none());
        assert!(Feed::find("feed-test").is_none());
        assert!(feed.cancel.is_cancelled());
    }

    type Sent = Arc<Mutex<Vec<AiTriageProgressParams>>>;

    fn job_on(feed: &Arc<Feed>, id: &str, threads: usize, sent: &Sent) -> Arc<Job> {
        let sent = sent.clone();
        Arc::new(Job {
            id: id.to_owned(),
            thread_ids: (0..threads).map(|i| format!("t{i}")).collect(),
            source: Source::Feature,
            token: CancelToken::new(),
            progress: Box::new(move |p| sent.lock().unwrap().push(p.clone())),
            feed: Arc::downgrade(feed),
            reported: Mutex::new((0, false)),
        })
    }

    #[test]
    fn jobs_are_bounded_and_cancelled_with_the_feed() {
        let feed = Feed::register("jobs-test");
        let sent = Sent::default();
        let job = |id: &str| job_on(&feed, id, 1, &sent);
        for i in 0..JOBS_LIMIT {
            feed.push_job(job(&format!("j{i}")), || {}).unwrap();
        }
        let e = feed
            .push_job(job("extra"), || panic!("not queued"))
            .unwrap_err();
        assert_eq!(e.code, code::ENGINE_BUSY);
        let Some(Item::Job(running)) = feed.next(&|| false) else {
            panic!("a job")
        };
        assert_eq!(running.id(), "j0");
        feed.close();
        assert!(running.token.is_cancelled());
        assert_eq!(
            feed.push_job(job("late"), || {}).unwrap_err().code,
            code::ACCOUNT_STOPPED
        );
    }

    #[test]
    fn a_closing_feed_ends_its_jobs_with_a_stopped_progress() {
        let feed = Feed::register("stop-test");
        let sent = Sent::default();
        feed.push_job(job_on(&feed, "running", 3, &sent), || {})
            .unwrap();
        feed.push_job(job_on(&feed, "queued", 2, &sent), || {})
            .unwrap();
        let Some(Item::Job(running)) = feed.next(&|| false) else {
            panic!("a job")
        };
        running.report(1);
        feed.close();
        // The worker's own reports come too late.
        running.report(2);
        running.stop();
        let progress = |id: &str, done, total, stopped| AiTriageProgressParams {
            job_id: id.into(),
            done,
            total,
            stopped,
        };
        assert_eq!(
            *sent.lock().unwrap(),
            [
                progress("running", 1, 3, false),
                progress("running", 1, 3, true),
                progress("queued", 0, 2, true),
            ]
        );
    }

    #[test]
    fn a_finished_job_sends_no_stopped_progress() {
        let feed = Feed::register("finish-test");
        let sent = Sent::default();
        let job = job_on(&feed, "j", 1, &sent);
        job.report(1);
        job.stop();
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert!(!sent.lock().unwrap()[0].stopped);
        feed.close();
    }

    #[test]
    fn a_cancel_wakes_a_waiting_worker() {
        let feed = Feed::register("wake-test");
        let job = job_on(&feed, "j", 1, &Sent::default());
        let cancelling = job.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            cancelling.cancel();
        });
        let started = Instant::now();
        let token = job.token.clone();
        assert!(feed.wait_until(started + Duration::from_secs(10), &|| token.is_cancelled()));
        assert!(started.elapsed() < STOP_POLL, "{:?}", started.elapsed());
        canceller.join().unwrap();
        feed.close();
    }

    #[test]
    fn retry_after_is_clamped() {
        let limited = |retry_after| AiError::RateLimited { retry_after };
        assert_eq!(pause_for(&limited(Some(Duration::ZERO))), MIN_PAUSE);
        assert_eq!(pause_for(&limited(Some(Duration::MAX))), MAX_PAUSE);
        assert_eq!(
            pause_for(&limited(Some(Duration::from_secs(30)))),
            Duration::from_secs(30)
        );
        assert_eq!(pause_for(&limited(None)), DEFAULT_PAUSE);
        let quota = AiError::QuotaExceeded {
            status: Some(402),
            retry_after: None,
        };
        assert_eq!(pause_for(&quota), DEFAULT_PAUSE);
        // A deadline far off doesn't overflow.
        pause("clamp-test", &limited(Some(Duration::MAX)));
        assert_eq!(paused("clamp-test"), Some("rate-limited"));
    }

    #[test]
    fn a_pause_lasts_until_its_deadline() {
        let g = gate("pause-test");
        assert_eq!(paused("pause-test"), None);
        *g.pause.lock().unwrap() = Some((Instant::now() + Duration::from_secs(60), "rate-limited"));
        assert_eq!(paused("pause-test"), Some("rate-limited"));
        *g.pause.lock().unwrap() = Some((Instant::now() - Duration::from_secs(1), "rate-limited"));
        assert_eq!(paused("pause-test"), None);
    }

    #[test]
    fn user_values_are_recognised() {
        assert!(is_users(
            &json!({"category": "important", "source": "user"})
        ));
        assert!(!is_users(&json!({"category": "important"})));
    }
}
