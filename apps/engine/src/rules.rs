// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Mail rules in `serve` (`feature::RULES`): each
//! account's `mailRules` thread applies the enabled rules of the `mailRules` setting
//! ([`tern_tasks::mail_rules`]) to the account's arriving mail and runs its `rules.reprocess`
//! jobs.
//!
//! - Arriving mail is the messages whose delta carries `rulesReady` (`Message::
//!   update_rules_ready`: once per message, on the save that has both its body and a copy the
//!   user received), read from the account's delta stream ([`Feed::tap`]) as the desktop's
//!   `MailRulesStore` read it. Drafts, mail dated before `mailRulesAutoSince` and mail the
//!   account sent to others are left alone.
//! - The rules run one at a time over the whole batch, so a later rule can undo an earlier
//!   one, and act once per thread. Their tasks run their local phase on this thread before the
//!   next rule reads the thread back. A forward is `draft.create {kind: "forward"}` then
//!   `draft.send` without an undo window, through [`crate::drafts_api`].
//! - A rule that fails (an invalid expression, a folder that's gone) is disabled: `disabled`
//!   and `disabledReason` are written into `mailRules`, and every subscriber gets
//!   `settings.changed` and `rules.disabled` ([`on_disable`]).
//! - `rules.reprocess` walks the account's inbox threads, newest first, 50 at a time,
//!   reporting `rules.progress` to its caller after each batch. Mail that arrives meanwhile
//!   goes between batches. `mailRulesAutoSince` doesn't apply to it. A job stops (its last
//!   progress says `stopped`) on `rules.cancel`, when its connection closes, when the account
//!   stops and on a database error.
//!
//! What a delta announced but the thread hadn't handled yet when the engine stopped is not
//! looked at again, as with the desktop closed; `rules.reprocess` covers it.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use serde_json::{json, Map, Value};
use tern_compose::settings::local_tz;
use tern_compose::{ComposeSettings, Identities};
use tern_core::constants::thread;
use tern_ipc::{
    code, draft_kind, DraftCreateParams, DraftSendParams, ErrorObject, RulesCancelParams,
    RulesCancelResult, RulesProgressParams, RulesReprocessParams, RulesReprocessResult,
    MAIL_RULES_AUTO_SINCE_KEY, MAIL_RULES_KEY,
};
use tern_store::read::ReadStore;
use tern_store::{DeltaSink, Message, Model, Query, Store, Task, Thread};
use tern_sync::expiration_executor::stored_body;
use tern_sync::{EngineSettings, WorkerContext};
use tern_tasks::mail_rules::{self, Categories, Matcher, Planned, Rule};
use tracing::{error, info, warn};

use crate::dispatch::Reply;
use crate::drafts_api::{CreateCall, Here, QueueTask};
use crate::socket::Accounts;

/// Message ids an account's queue holds; later ones are dropped until it drains.
pub const QUEUE_LIMIT: usize = 10_000;
/// The messages one batch of arriving mail takes, and the threads one reprocess batch takes.
const BATCH: usize = 50;
/// How often a waiting worker looks at the account's stop flag.
const STOP_POLL: Duration = Duration::from_millis(500);

// -- disabling ------------------------------------------------------------------------------

/// Disables the rule in the settings store and tells the subscribers; true when it changed.
pub type DisableHook = Box<dyn Fn(&str, &str) -> bool + Send + Sync>;

static ON_DISABLE: OnceLock<DisableHook> = OnceLock::new();

/// `serve` writes a failing rule's `disabled` and `disabledReason` and sends
/// `settings.changed` and `rules.disabled`.
pub fn on_disable(f: DisableHook) {
    let _ = ON_DISABLE.set(f);
}

fn disable(rule: &Rule, error: &str) {
    warn!(
        "-X Disabling mail rule {:?} ({}): {error}",
        rule.name, rule.id
    );
    if let Some(f) = ON_DISABLE.get() {
        f(&rule.id, error);
    }
}

/// `mailRules` with `rule_id` disabled for `error`, for [`on_disable`]: the values to store,
/// or `None` when there is nothing to change.
pub fn disabled_values(
    current: Option<&Value>,
    rule_id: &str,
    error: &str,
) -> Option<Map<String, Value>> {
    let rules = mail_rules::with_rule_disabled(current?, rule_id, error)?;
    let mut values = Map::new();
    values.insert(MAIL_RULES_KEY.to_owned(), rules);
    Some(values)
}

/// The `mailRulesAutoSince` to store when the store has none: now, so mail that is already
/// there when the first rules are stored is left alone (`MailRulesStore` set it at its first
/// launch).
pub fn auto_since_default(current: Option<&Value>, now_ms: i64) -> Option<Map<String, Value>> {
    if current.is_some_and(|v| v.as_u64().is_some()) {
        return None;
    }
    let mut values = Map::new();
    values.insert(MAIL_RULES_AUTO_SINCE_KEY.to_owned(), json!(now_ms.max(0)));
    Some(values)
}

/// After a `settings.set` that stored rules: the default `mailRulesAutoSince`, stored when the
/// store has none, so rules apply to mail that arrives from then on. The values it stored.
pub fn set_auto_since(
    guard: &mut crate::settings::SettingsGuard<'_>,
    changed: &Map<String, Value>,
    now_ms: i64,
) -> Map<String, Value> {
    if changed.get(MAIL_RULES_KEY).is_none_or(Value::is_null) {
        return Map::new();
    }
    let key = [MAIL_RULES_AUTO_SINCE_KEY.to_owned()];
    let Ok(mut current) = guard.get(Some(&key)) else {
        return Map::new();
    };
    let Some(values) = auto_since_default(current.remove(&key[0]).as_ref(), now_ms) else {
        return Map::new();
    };
    guard.set(values).unwrap_or_else(|e| {
        warn!("-X Could not store {}: {}", key[0], e.message);
        Map::new()
    })
}

// -- jobs -----------------------------------------------------------------------------------

/// The progress callback of a job: `rules.progress` to the connection that started it.
pub type Progress = Box<dyn Fn(&RulesProgressParams) + Send + Sync>;

/// One `rules.reprocess`.
pub struct Job {
    id: String,
    account_id: String,
    thread_ids: Vec<String>,
    cancelled: AtomicBool,
    progress: Progress,
    /// Threads done so far, and whether the last progress went out.
    reported: Mutex<(usize, bool)>,
}

impl Job {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    fn report(&self, done: usize) {
        self.send(Some(done));
    }

    /// The job ended before its last thread.
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
        (self.progress)(&RulesProgressParams {
            job_id: self.id.clone(),
            account_id: self.account_id.clone(),
            done,
            total,
            stopped,
        });
    }
}

/// The `rules.reprocess` jobs a connection started. They are cancelled when it closes: only
/// that connection knows their ids, so nobody else could stop them.
#[derive(Default)]
pub struct ConnectionJobs(Mutex<Vec<Weak<Job>>>);

impl ConnectionJobs {
    fn lock(&self) -> MutexGuard<'_, Vec<Weak<Job>>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn add(&self, job: &Arc<Job>) {
        let mut jobs = self.lock();
        jobs.retain(|j| j.strong_count() > 0);
        jobs.push(Arc::downgrade(job));
    }

    /// The connection closed.
    pub fn close(&self) {
        for job in self.lock().drain(..).filter_map(|j| j.upgrade()) {
            job.cancel();
        }
    }
}

// -- feed -----------------------------------------------------------------------------------

#[derive(Default)]
struct FeedState {
    /// Ids of messages whose delta carried `rulesReady`, oldest first.
    arrived: VecDeque<String>,
    /// The `rules.reprocess` waiting or running; one per account.
    job: Option<Arc<Job>>,
    /// The worker took `job`.
    job_started: bool,
    closed: bool,
    /// The queue overflowed since it last had room; logged once.
    overflowed: bool,
}

/// One account's rules queue, fed by its delta stream ([`Feed::tap`]) and `rules.reprocess`.
pub struct Feed {
    account_id: String,
    state: Mutex<FeedState>,
    changed: Condvar,
}

static FEEDS: Mutex<Vec<Arc<Feed>>> = Mutex::new(Vec::new());

fn feeds() -> MutexGuard<'static, Vec<Arc<Feed>>> {
    FEEDS.lock().unwrap_or_else(|e| e.into_inner())
}

impl Feed {
    /// A new feed for `account_id`, found by [`Feed::find`] until it is closed.
    pub fn register(account_id: &str) -> Arc<Feed> {
        let feed = Arc::new(Feed {
            account_id: account_id.to_owned(),
            state: Mutex::default(),
            changed: Condvar::new(),
        });
        let mut all = feeds();
        all.retain(|f| f.account_id != account_id);
        all.push(feed.clone());
        feed
    }

    pub fn find(account_id: &str) -> Option<Arc<Feed>> {
        feeds().iter().find(|f| f.account_id == account_id).cloned()
    }

    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `inner`, with the account's rules-ready messages queued here first.
    pub fn tap(self: &Arc<Self>, inner: Box<dyn DeltaSink>) -> Box<dyn DeltaSink> {
        Box::new(Tap {
            feed: self.clone(),
            inner,
        })
    }

    /// The account stopped: the worker returns, a job ends with a `stopped` progress and
    /// [`Feed::find`] no longer names the feed.
    pub fn close(self: &Arc<Self>) {
        let job = {
            let mut st = self.lock();
            st.closed = true;
            st.arrived.clear();
            st.job.take()
        };
        self.changed.notify_all();
        if let Some(job) = job {
            job.cancel();
            job.stop();
        }
        feeds().retain(|f| !Arc::ptr_eq(f, self));
    }

    fn arrived(&self, line: &str) {
        // Most Message deltas carry no rulesReady; don't parse their bodies for nothing.
        if !line.contains("\"rulesReady\":true") {
            return;
        }
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if item["type"] != "persist" {
            return;
        }
        let ids: Vec<String> = item["modelJSONs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["rulesReady"] == true && m["draft"] != true)
            .filter_map(|m| m["id"].as_str().map(str::to_owned))
            .collect();
        if ids.is_empty() {
            return;
        }
        let mut st = self.lock();
        if st.closed {
            return;
        }
        for id in ids {
            if st.arrived.len() >= QUEUE_LIMIT {
                if !st.overflowed {
                    warn!(
                        "-X The mail rules queue is full; arriving mail is skipped until it drains"
                    );
                    st.overflowed = true;
                }
                break;
            }
            if !st.arrived.contains(&id) {
                st.arrived.push_back(id);
            }
        }
        drop(st);
        self.changed.notify_all();
    }

    /// Queues `job`, running `queued` before the worker can take it so the response precedes
    /// the job's progress; `ENGINE_BUSY` while another one runs for the account.
    fn push_job(&self, job: Arc<Job>, queued: impl FnOnce()) -> Result<(), ErrorObject> {
        let mut st = self.lock();
        if st.closed {
            return Err(stopped(&self.account_id));
        }
        if st.job.is_some() {
            return Err(ErrorObject::new(
                code::ENGINE_BUSY,
                format!(
                    "the account {} is reprocessing its rules already",
                    self.account_id
                ),
            ));
        }
        st.job = Some(job);
        st.job_started = false;
        queued();
        drop(st);
        self.changed.notify_all();
        Ok(())
    }

    /// Up to [`BATCH`] arrived message ids, without waiting.
    fn take_arrived(&self) -> Vec<String> {
        let mut st = self.lock();
        let n = st.arrived.len().min(BATCH);
        let ids: Vec<String> = st.arrived.drain(..n).collect();
        if st.arrived.len() < QUEUE_LIMIT {
            st.overflowed = false;
        }
        ids
    }

    /// The next thing to do, or `None` once closed or `stopping`.
    fn next(&self, stopping: &dyn Fn() -> bool) -> Option<Work> {
        let mut st = self.lock();
        loop {
            if st.closed || stopping() {
                return None;
            }
            if !st.arrived.is_empty() {
                drop(st);
                return Some(Work::Arrived(self.take_arrived()));
            }
            if let Some(job) = st.job.clone().filter(|_| !st.job_started) {
                st.job_started = true;
                return Some(Work::Job(job));
            }
            st = self
                .changed
                .wait_timeout(st, STOP_POLL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn job_done(&self, job: &Arc<Job>) {
        let mut st = self.lock();
        if st.job.as_ref().is_some_and(|j| Arc::ptr_eq(j, job)) {
            st.job = None;
            st.job_started = false;
        }
    }

    fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

enum Work {
    Arrived(Vec<String>),
    Job(Arc<Job>),
}

struct Tap {
    feed: Arc<Feed>,
    inner: Box<dyn DeltaSink>,
}

impl DeltaSink for Tap {
    fn write_item(&mut self, model_class: &str, line: &str) {
        if model_class == Message::TABLE {
            self.feed.arrived(line);
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

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

// -- the socket's half ----------------------------------------------------------------------

/// `rules.reprocess`: queues a job over the account's inbox threads, newest first, and
/// answers through `respond` before its first progress. The job is cancelled when `jobs`
/// closes.
pub fn reprocess(
    p: RulesReprocessParams,
    reads: &ReadStore,
    accounts: &dyn Accounts,
    jobs: &ConnectionJobs,
    progress: Progress,
    respond: impl FnOnce(Value),
) -> Result<(), ErrorObject> {
    accounts.check(&p.account_id)?;
    let feed = Feed::find(&p.account_id).ok_or_else(|| stopped(&p.account_id))?;
    let inbox = reads
        .category_for_role(&p.account_id, "inbox")
        .map_err(internal)?
        .ok_or_else(|| {
            ErrorObject::new(
                code::NOT_FOUND,
                format!("account {} has no inbox to reprocess", p.account_id),
            )
        })?;
    let thread_ids = reads.category_thread_ids(&inbox).map_err(internal)?;
    let job = Arc::new(Job {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: p.account_id.clone(),
        thread_ids,
        cancelled: AtomicBool::new(false),
        progress,
        reported: Mutex::new((0, false)),
    });
    let value = serde_json::to_value(RulesReprocessResult {
        job_id: job.id.clone(),
        total: job.thread_ids.len(),
    })
    .map_err(internal)?;
    info!(
        "Reprocessing the mail rules of {} over {} inbox threads (job {})",
        p.account_id,
        job.thread_ids.len(),
        job.id
    );
    feed.push_job(job.clone(), || respond(value))?;
    jobs.add(&job);
    Ok(())
}

/// `rules.cancel`.
pub fn cancel(p: RulesCancelParams) -> Reply {
    let job = feeds()
        .iter()
        .find_map(|f| f.lock().job.clone().filter(|j| j.id == p.job_id));
    if let Some(job) = &job {
        job.cancel();
    }
    serde_json::to_value(RulesCancelResult {
        cancelled: job.is_some(),
    })
    .map_err(internal)
}

// -- the worker -----------------------------------------------------------------------------

/// Starts the account's `mailRules` thread on `feed`. `sync` mode keeps no settings and runs
/// none.
pub fn start(ctx: &WorkerContext, feed: Arc<Feed>) {
    let Some(settings) = ctx.settings.clone() else {
        return;
    };
    let c = ctx.clone();
    if let Err(e) = ctx.spawn(thread::MAIL_RULES, move || {
        let started_ms = c.clock.now().saturating_mul(1000);
        Worker {
            ctx: c,
            feed,
            settings,
            store: None,
            matcher: Matcher::default(),
            started_ms,
        }
        .run()
    }) {
        error!("-X Could not start the {} thread: {e}", thread::MAIL_RULES);
    }
}

/// A message the rules look at: its JSON and HTML body.
struct Mail {
    message: Message,
    body: String,
}

struct Worker {
    ctx: WorkerContext,
    feed: Arc<Feed>,
    settings: Arc<dyn EngineSettings>,
    /// Opened on first use, on this thread.
    store: Option<Store>,
    matcher: Matcher,
    /// `mailRulesAutoSince` when the store has none.
    started_ms: i64,
}

impl Worker {
    fn run(mut self) {
        let ctx = self.ctx.clone();
        while let Some(work) = self.feed.next(&|| ctx.is_stopping()) {
            let result = match work {
                Work::Arrived(ids) => self.arrived(&ids),
                Work::Job(job) => {
                    let feed = self.feed.clone();
                    run_job(&feed, &job, |job| self.job(job))
                }
            };
            if let Err(e) = result {
                return ctx.fail_with(&e);
            }
        }
    }

    fn store(&mut self) -> tern_core::Result<&Store> {
        if self.store.is_none() {
            self.store = Some(Store::open(&self.ctx.config_dir, self.ctx.deltas.clone())?);
        }
        Ok(self.store.as_ref().expect("opened above"))
    }

    fn account_id(&self) -> &str {
        self.ctx.account.id()
    }

    fn rules(&self) -> Vec<Rule> {
        mail_rules::enabled_rules(
            self.settings.get(MAIL_RULES_KEY).as_ref(),
            self.ctx.account.id(),
        )
    }

    fn is_mine(&self) -> impl Fn(&str) -> bool {
        let me = Identities::from_accounts_json([self.ctx.account.as_json()]);
        move |email: &str| me.is_me(email)
    }

    /// Arriving mail: the messages still there, not drafts, dated no earlier than
    /// `mailRulesAutoSince` (whose second the Date header can share) and not sent by the
    /// account to others.
    fn arrived(&mut self, ids: &[String]) -> tern_core::Result<()> {
        if self.rules().is_empty() {
            return Ok(());
        }
        let since = self
            .settings
            .get(MAIL_RULES_AUTO_SINCE_KEY)
            .and_then(|v| v.as_i64())
            .unwrap_or(self.started_ms);
        let mail = self.load(ids, Some(since))?;
        self.apply(&mail)
    }

    fn load(&mut self, ids: &[String], since_ms: Option<i64>) -> tern_core::Result<Vec<Mail>> {
        let account_id = self.account_id().to_owned();
        let is_mine = self.is_mine();
        let store = self.store()?;
        let messages = store.find_all::<Message>(
            &Query::new()
                .equal("accountId", account_id.as_str())
                .equal_any("id", ids.iter().map(String::as_str)),
        )?;
        let mut mail = Vec::with_capacity(messages.len());
        // In the order the ids came, as the deltas announced them.
        for id in ids {
            let Some(message) = messages.iter().find(|m| m.id() == id) else {
                continue;
            };
            if message.is_draft()
                || since_ms.is_some_and(|since| message.date().saturating_mul(1000) < since)
                || mail_rules::is_sent_to_others(&message.data, &is_mine)
            {
                continue;
            }
            let body = stored_body(store, message.id())?.unwrap_or_default();
            mail.push(Mail {
                message: message.clone(),
                body,
            });
        }
        Ok(mail)
    }

    /// `processMessages`: one rule at a time over every message, once per thread.
    fn apply(&mut self, mail: &[Mail]) -> tern_core::Result<()> {
        if mail.is_empty() {
            return Ok(());
        }
        let mut queued = false;
        'rules: for rule in self.rules() {
            let mut seen: HashSet<String> = HashSet::new();
            for m in mail {
                match self.matcher.matches(&rule, &m.message.data, &m.body) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(e) => {
                        disable(&rule, &e);
                        continue 'rules;
                    }
                }
                if !seen.insert(m.message.thread_id().to_owned()) {
                    continue;
                }
                match self.act(&rule, m)? {
                    Ok(any) => queued |= any,
                    Err(e) => {
                        disable(&rule, &e);
                        continue 'rules;
                    }
                }
            }
        }
        if queued {
            self.ctx.foreground.interrupt();
        }
        Ok(())
    }

    /// `_applyRuleToMessage`: every action planned against the thread as it is now, then
    /// their tasks run; nothing runs when one action can't. `Ok(Err)` is the rule's error.
    fn act(&mut self, rule: &Rule, m: &Mail) -> tern_core::Result<Result<bool, String>> {
        let account_id = self.account_id().to_owned();
        let store = self.store()?;
        let Some(thread) =
            store.find::<Thread>(&Query::new().equal("id", m.message.thread_id()))?
        else {
            warn!(
                "-X Cannot find thread {} to process mail rules",
                m.message.thread_id()
            );
            return Ok(Ok(false));
        };
        let categories = Categories::load(store, &account_id)?;
        let mut planned = Vec::with_capacity(rule.actions.len());
        for action in &rule.actions {
            match mail_rules::plan_action(&categories, &account_id, action, &thread) {
                Ok(p) => planned.push(p),
                Err(e) => return Ok(Err(e)),
            }
        }
        info!(
            "Mail rule {:?} matches message {}; {} action(s)",
            rule.name,
            m.message.id(),
            planned.len()
        );
        let mut queued = false;
        for p in planned {
            match p {
                Planned::Task(mut task) => {
                    self.perform_local(&mut task)?;
                    queued = true;
                }
                Planned::Forward(to) => {
                    if let Err(e) = self.forward(&m.message, &to)? {
                        return Ok(Err(e));
                    }
                    queued = true;
                }
                Planned::Nothing => {}
            }
        }
        Ok(Ok(queued))
    }

    fn perform_local(&mut self, task: &mut Task) -> tern_core::Result<()> {
        let ctx = self.ctx.clone();
        let store = self.store()?;
        ctx.tasks.perform_local(store, &ctx, task)?;
        if !task.error().is_null() {
            warn!(
                "-X The mail rules' {} failed its local phase: {}",
                task.class(),
                task.error()
            );
        }
        Ok(())
    }

    /// `composeAndSendForward`: the engine's forward of `message` to `to`, sent at once.
    /// Mail from this account isn't forwarded, so a rule that forwards to the account itself
    /// can't forward its own forwards forever.
    fn forward(&mut self, message: &Message, to: &str) -> tern_core::Result<Result<(), String>> {
        let from_me = {
            let is_mine = self.is_mine();
            message
                .from()
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["email"].as_str())
                .any(is_mine)
        };
        if from_me {
            info!(
                "-- Not forwarding message {}: it is from this account",
                message.id()
            );
            return Ok(Ok(()));
        }
        let settings = self.settings.clone();
        let compose = ComposeSettings::from_values(|k| settings.get(k), local_tz());
        let accounts = Identities::from_accounts_json([self.ctx.account.as_json()]);
        let mut fields = Map::new();
        fields.insert("to".into(), json!([{ "name": "", "email": to }]));
        let call = CreateCall {
            params: DraftCreateParams {
                account_id: Some(self.account_id().to_owned()),
                kind: draft_kind::FORWARD.to_owned(),
                message_id: Some(message.id().to_owned()),
                fields: Some(fields),
                save: Some(true),
                ..Default::default()
            },
            settings: compose,
            accounts,
            resurface_body: None,
            attached: Vec::new(),
        };
        let ctx = self.ctx.clone();
        let store = self.store()?;
        let queue = LocalQueue {
            ctx: &ctx,
            store,
            database_error: Default::default(),
        };
        let here = Here {
            store,
            account: &ctx.account,
            config_dir: &ctx.config_dir,
            queue: &queue,
            metadata: &*ctx.metadata,
        };
        let sent = here.create(call).and_then(|draft| {
            let hmid = draft["hMsgId"].as_str().unwrap_or("").to_owned();
            here.send(DraftSendParams {
                account_id: ctx.account.id().to_owned(),
                header_message_id: hmid,
                ..Default::default()
            })
        });
        if let Some(e) = queue.database_error.into_inner() {
            return Err(e);
        }
        Ok(match sent {
            Ok(_) => {
                info!("-- Forwarded message {} to {to}", message.id());
                Ok(())
            }
            // A body purged from the cache: the message can't be quoted now, and that is no
            // fault of the rule.
            Err(e) if e.code == code::NOT_FOUND => {
                warn!(
                    "-X Could not forward message {}: {}",
                    message.id(),
                    e.message
                );
                Ok(())
            }
            Err(e) => Err(format!("Error: {}", e.message)),
        })
    }

    /// A `rules.reprocess` job, between whose batches arriving mail goes.
    fn job(&mut self, job: &Arc<Job>) -> tern_core::Result<()> {
        let mut done = 0;
        for chunk in job.thread_ids.chunks(BATCH) {
            if job.is_cancelled() || self.ctx.is_stopping() || self.feed.is_closed() {
                job.stop();
                return Ok(());
            }
            loop {
                let ids = self.feed.take_arrived();
                if ids.is_empty() {
                    break;
                }
                self.arrived(&ids)?;
            }
            let ids: Vec<String> = {
                let store = self.store()?;
                store
                    .find_all::<Message>(
                        &Query::new()
                            .equal_any("threadId", chunk.iter().map(String::as_str))
                            .order_by("date", true),
                    )?
                    .iter()
                    .map(|m| m.id().to_owned())
                    .collect()
            };
            let mail = self.load(&ids, None)?;
            self.apply(&mail)?;
            done += chunk.len();
            job.report(done);
        }
        if job.thread_ids.is_empty() {
            job.report(0);
        }
        info!(
            "Reprocessed the mail rules over {done} threads (job {})",
            job.id
        );
        Ok(())
    }
}

/// Runs `job` through `f` and frees the account for the next one. A job that ends early (on a
/// database error too) still sends its last progress, `stopped`.
fn run_job(
    feed: &Feed,
    job: &Arc<Job>,
    f: impl FnOnce(&Arc<Job>) -> tern_core::Result<()>,
) -> tern_core::Result<()> {
    let r = f(job);
    job.stop();
    feed.job_done(job);
    r
}

/// The worker's task queue for the draft pipeline: local phases on the worker's store. A
/// database error is kept to stop the account once the pipeline call has returned.
struct LocalQueue<'a> {
    ctx: &'a WorkerContext,
    store: &'a Store,
    database_error: std::cell::RefCell<Option<tern_core::Error>>,
}

impl QueueTask for LocalQueue<'_> {
    fn queue_task(&self, mut json: Value) -> Reply {
        if let Some(obj) = json.as_object_mut() {
            obj.insert("v".into(), Value::from(0));
        }
        let mut task = Task::from_json(json)
            .map_err(|e| ErrorObject::new(code::INVALID_PARAMS, format!("invalid task: {e}")))?;
        if let Err(e) = self
            .ctx
            .tasks
            .perform_local(self.store, self.ctx, &mut task)
        {
            let reply = internal(&e);
            *self.database_error.borrow_mut() = Some(e);
            return Err(reply);
        }
        if task.error().is_null() {
            Ok(task.data)
        } else {
            Err(
                ErrorObject::new(code::TASK_REJECTED, "the task's local phase failed")
                    .with_data(task.error().clone()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_stores_the_reason_and_auto_since_is_set_once() {
        let rules = json!([{"id": "r1", "accountId": "a", "name": "x"}]);
        let values = disabled_values(Some(&rules), "r1", "Error: gone").unwrap();
        assert_eq!(
            values[MAIL_RULES_KEY][0]["disabledReason"],
            json!("Error: gone")
        );
        assert!(disabled_values(Some(&values[MAIL_RULES_KEY]), "r1", "again").is_none());
        assert!(disabled_values(None, "r1", "x").is_none());

        let set = auto_since_default(None, 1_700_000_000_000).unwrap();
        assert_eq!(set[MAIL_RULES_AUTO_SINCE_KEY], json!(1_700_000_000_000i64));
        assert!(auto_since_default(Some(&json!(5)), 9).is_none());
    }

    #[test]
    fn the_tap_queues_rules_ready_messages_once() {
        let line = |m: Value| {
            json!({"type": "persist", "modelClass": "Message", "modelJSONs": [m]}).to_string()
        };
        let feed = Feed::register("tap-acct");
        feed.arrived(&line(json!({"id": "m1", "rulesReady": true})));
        feed.arrived(&line(json!({"id": "m1", "rulesReady": true})));
        feed.arrived(&line(json!({"id": "m2"})));
        feed.arrived(&line(
            json!({"id": "m3", "rulesReady": true, "draft": true}),
        ));
        assert_eq!(feed.take_arrived(), vec!["m1".to_owned()]);
        feed.close();
        feed.arrived(&line(json!({"id": "m4", "rulesReady": true})));
        assert!(feed.take_arrived().is_empty());
        assert!(Feed::find("tap-acct").is_none());
    }

    fn job(id: &str, threads: usize, sent: &Arc<Mutex<Vec<RulesProgressParams>>>) -> Arc<Job> {
        let sent = sent.clone();
        Arc::new(Job {
            id: id.to_owned(),
            account_id: "job-acct".to_owned(),
            thread_ids: (0..threads).map(|i| format!("t{i}")).collect(),
            cancelled: AtomicBool::new(false),
            progress: Box::new(move |p| sent.lock().unwrap().push(p.clone())),
            reported: Mutex::new((0, false)),
        })
    }

    #[test]
    fn a_job_that_fails_still_reports_stopped_and_frees_the_account() {
        let feed = Feed::register("fail-acct");
        let sent = Arc::default();
        let j = job("j1", 120, &sent);
        feed.push_job(j.clone(), || {}).unwrap();
        let Some(Work::Job(taken)) = feed.next(&|| false) else {
            panic!("no job")
        };
        let r = run_job(&feed, &taken, |j| {
            j.report(50);
            Err(tern_core::Error::other("disk I/O error"))
        });
        assert!(r.is_err());
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert!(sent[1].stopped && sent[1].done == 50 && sent[1].total == 120);
        assert!(feed.push_job(job("j2", 1, &Arc::default()), || {}).is_ok());
        feed.close();
    }

    #[test]
    fn closing_the_connection_cancels_its_job() {
        let feed = Feed::register("conn-acct");
        let jobs = ConnectionJobs::default();
        let sent = Arc::default();
        let j = job("j1", 120, &sent);
        feed.push_job(j.clone(), || {}).unwrap();
        jobs.add(&j);
        drop(j);
        jobs.close();
        let Some(Work::Job(taken)) = feed.next(&|| false) else {
            panic!("no job")
        };
        assert!(taken.is_cancelled());
        run_job(&feed, &taken, |_| Ok(())).unwrap();
        assert!(sent.lock().unwrap()[0].stopped);
        // The account takes a new job at once.
        assert!(feed.push_job(job("j2", 1, &Arc::default()), || {}).is_ok());
        feed.close();
    }
}
