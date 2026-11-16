// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Live reads: `query.subscribe` with its `query.changed` feed, `events.subscribe` with
//! `events.changed`, and `ai.triage.subscribe` with `ai.triage.changed`
//! (docs/thin-clients-plan.md 4.1).
//!
//! One `liveQueries` thread holds every connection's subscriptions and its own read-only
//! store. Each account's `DeltaStream` flush hands it the ids it persisted or removed, by
//! class ([`note_touched`]). After at most [`COALESCE`] the thread re-runs each subscription
//! whose class and accounts the changes touch: the window as ids, the count, then the full
//! rows of the ids that entered the window or changed in it. A Thread query with `summary`
//! also re-runs when a message of one of its threads changes, and updates that thread's row.
//! Membership and order come from
//! the same compiler as `query`, so no client evaluates matchers. The subscription's
//! answers and notifications are queued from this thread only, so a client gets them in
//! order.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use airpost_core::constants::{thread, DELTA_TYPE_PERSIST, DELTA_TYPE_UNPERSIST};
use airpost_ipc::{
    code, method, policy, query_attrs::QUERY_CLASSES, AiTriageChangedParams, AiTriageListParams,
    AiTriageSubscribeParams, AiTriageSubscribeResult, ErrorObject, EventsChangedParams,
    EventsExpandParams, EventsExpandResult, EventsSubscribeParams, EventsSubscribeResult, LiveOp,
    Notification, QueryChangedParams, QuerySetWindowParams, QuerySubscribeParams,
    QuerySubscribeResult, QueryWindow, QueryWindowResult, Request as RpcRequest, Response,
    SubIdParams,
};
use airpost_store::model_query::{ModelQuery, QueryOutput, SUMMARY};
use airpost_store::DeltaItem;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::warn;

use crate::read_api::{over_budget, ReadApi};
use crate::settings::Settings;
use crate::socket::{deliver, Accounts, Hub, Outbox, Server};

/// The least time between two re-runs of the subscriptions, so a burst of flushes costs one.
const COALESCE: Duration = Duration::from_millis(50);

/// The ids a flush persisted or removed, by model class, and under [`THREADS_OF_MESSAGES`]
/// the threads of the messages it persisted or removed.
pub type Touched = BTreeMap<String, HashSet<String>>;

/// The [`Touched`] key of the threads whose messages changed.
const THREADS_OF_MESSAGES: &str = "Message.threadId";

/// Settings under this prefix decide which accounts triage covers.
pub const TRIAGE_SETTINGS_PREFIX: &str = "core.ai.";

/// The classes whose deltas the subscriptions follow.
fn is_watched(model_class: &str) -> bool {
    QUERY_CLASSES.iter().any(|c| c.class == model_class)
}

/// Adds `item`'s ids to `touched` when the live queries follow its class.
pub fn note_touched(touched: &mut Touched, item: &DeltaItem) {
    if !matches!(
        item.kind.as_str(),
        DELTA_TYPE_PERSIST | DELTA_TYPE_UNPERSIST
    ) || !is_watched(&item.model_class)
    {
        return;
    }
    let ids = touched.entry(item.model_class.clone()).or_default();
    for m in item.model_jsons() {
        if let Some(id) = m.get("id").and_then(Value::as_str) {
            ids.insert(id.to_owned());
        }
    }
    if item.model_class == "Message" {
        let threads = touched.entry(THREADS_OF_MESSAGES.to_owned()).or_default();
        for m in item.model_jsons() {
            if let Some(id) = m.get("threadId").and_then(Value::as_str) {
                threads.insert(id.to_owned());
            }
        }
    }
}

/// The [`Touched`] keys a query's subscription follows.
fn followed_classes(query: &ModelQuery) -> Vec<&str> {
    let mut classes = vec![query.class.as_str()];
    let summary = query
        .include
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|i| i == SUMMARY);
    if query.class == "Thread" && summary {
        classes.push(THREADS_OF_MESSAGES);
    }
    classes
}

pub fn is_live_method(name: &str) -> bool {
    matches!(
        name,
        method::QUERY_SUBSCRIBE
            | method::QUERY_SET_WINDOW
            | method::QUERY_UNSUBSCRIBE
            | method::EVENTS_SUBSCRIBE
            | method::EVENTS_UNSUBSCRIBE
            | method::AI_TRIAGE_SUBSCRIBE
            | method::AI_TRIAGE_UNSUBSCRIBE
    )
}

enum Msg {
    Call {
        conn: u64,
        outbox: Arc<Outbox>,
        relayed: bool,
        req: RpcRequest,
    },
    Changed {
        account_id: String,
        touched: Touched,
    },
    Rerun {
        account_id: String,
    },
    TriageSettings,
    Closed {
        conn: u64,
    },
}

/// The way to the live queries' thread.
#[derive(Clone)]
pub struct Handle(Sender<Msg>);

impl Handle {
    /// A [`is_live_method`] request from connection `conn`, answered on `outbox`.
    pub fn call(&self, conn: u64, outbox: Arc<Outbox>, relayed: bool, req: RpcRequest) {
        let _ = self.0.send(Msg::Call {
            conn,
            outbox,
            relayed,
            req,
        });
    }

    /// What one flush of `account_id`'s deltas touched.
    pub fn changed(&self, account_id: &str, touched: Touched) {
        let _ = self.0.send(Msg::Changed {
            account_id: account_id.to_owned(),
            touched,
        });
    }

    /// Every subscription over `account_id` reads again: its rows changed without deltas.
    pub fn rerun(&self, account_id: &str) {
        let _ = self.0.send(Msg::Rerun {
            account_id: account_id.to_owned(),
        });
    }

    /// The AI settings changed: every triage subscription reads again.
    pub fn triage_settings_changed(&self) {
        let _ = self.0.send(Msg::TriageSettings);
    }

    /// Connection `conn` closed: its subscriptions end.
    pub fn closed(&self, conn: u64) {
        let _ = self.0.send(Msg::Closed { conn });
    }
}

/// Starts the thread, once per server, and gives its [`Handle`] to the hub.
pub fn start(server: &Arc<Server>) {
    if server.hub.live().is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    server.hub.set_live(Handle(tx));
    let live = Live {
        reads: ReadApi::new(
            server.config_dir.clone(),
            server.accounts.clone(),
            server.database.clone(),
        ),
        accounts: server.accounts.clone(),
        settings: server.settings.clone(),
        hub: server.hub.clone(),
        next_id: 0,
        queries: BTreeMap::new(),
        events: BTreeMap::new(),
        triage: BTreeMap::new(),
    };
    crate::modes::spawn(thread::LIVE_QUERIES, move || live.run(&rx));
}

/// Changes waiting for the next re-run.
#[derive(Default)]
struct Pending {
    touched: HashMap<String, Touched>,
    rerun: HashSet<String>,
    triage_settings: bool,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.touched.is_empty() && self.rerun.is_empty() && !self.triage_settings
    }
}

struct QuerySub {
    conn: u64,
    outbox: Arc<Outbox>,
    key: Option<String>,
    query: ModelQuery,
    window: QueryWindow,
    ids: Vec<String>,
    total: i64,
    version: u64,
}

struct EventsSub {
    conn: u64,
    outbox: Arc<Outbox>,
    key: Option<String>,
    params: EventsExpandParams,
    occurrences: Vec<Value>,
    truncated: bool,
}

struct TriageSub {
    conn: u64,
    outbox: Arc<Outbox>,
    key: Option<String>,
    params: AiTriageListParams,
    thread_ids: Vec<String>,
    total: usize,
}

/// What a call changes once its answer is sent; nothing when the answer is over budget.
enum Commit {
    None,
    Query(String, QuerySub),
    Window {
        sub_id: String,
        window: QueryWindow,
        ids: Vec<String>,
        total: i64,
        version: u64,
    },
    Events(String, EventsSub),
    Triage(String, TriageSub),
}

struct Live {
    reads: ReadApi,
    accounts: Arc<dyn Accounts>,
    /// `serve`'s settings, which decide the accounts triage covers; none in `sync` mode,
    /// which has no triage.
    settings: Option<Arc<Settings>>,
    hub: Arc<Hub>,
    next_id: u64,
    queries: BTreeMap<String, QuerySub>,
    events: BTreeMap<String, EventsSub>,
    triage: BTreeMap<String, TriageSub>,
}

fn invalid(why: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, format!("invalid params: {why}"))
}

fn not_found(sub_id: &str) -> ErrorObject {
    ErrorObject::new(code::NOT_FOUND, format!("no subscription {sub_id}"))
}

fn to_value(v: impl Serialize) -> Result<Value, ErrorObject> {
    serde_json::to_value(v).map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
}

fn wants_account(accounts: Option<&[String]>, account_id: &str) -> bool {
    accounts.is_none_or(|a| a.iter().any(|x| x == account_id))
}

impl Live {
    fn run(mut self, rx: &Receiver<Msg>) {
        let mut pending = Pending::default();
        let mut last_run = Instant::now()
            .checked_sub(COALESCE)
            .unwrap_or_else(Instant::now);
        loop {
            let msg = if pending.is_empty() {
                match rx.recv() {
                    Ok(m) => m,
                    Err(_) => return,
                }
            } else {
                let due = last_run + COALESCE;
                let now = Instant::now();
                if now >= due {
                    self.refresh(std::mem::take(&mut pending));
                    last_run = Instant::now();
                    continue;
                }
                match rx.recv_timeout(due - now) {
                    Ok(m) => m,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            };
            match msg {
                Msg::Changed {
                    account_id,
                    touched,
                } => {
                    let into = pending.touched.entry(account_id).or_default();
                    for (class, ids) in touched {
                        into.entry(class).or_default().extend(ids);
                    }
                }
                Msg::Rerun { account_id } => {
                    pending.rerun.insert(account_id);
                }
                Msg::TriageSettings => pending.triage_settings = true,
                Msg::Closed { conn } => {
                    self.queries.retain(|_, s| s.conn != conn);
                    self.events.retain(|_, s| s.conn != conn);
                    self.triage.retain(|_, s| s.conn != conn);
                }
                Msg::Call {
                    conn,
                    outbox,
                    relayed,
                    req,
                } => self.call(conn, &outbox, relayed, &req),
            }
        }
    }

    fn call(&mut self, conn: u64, outbox: &Arc<Outbox>, relayed: bool, req: &RpcRequest) {
        let outcome = match req.method.as_str() {
            method::QUERY_SUBSCRIBE => self.query_subscribe(conn, outbox, relayed, req),
            method::QUERY_SET_WINDOW => self.set_window(conn, relayed, req),
            method::QUERY_UNSUBSCRIBE => self.unsubscribe(conn, req),
            method::EVENTS_SUBSCRIBE => self.events_subscribe(conn, outbox, req),
            method::EVENTS_UNSUBSCRIBE => self.unsubscribe(conn, req),
            method::AI_TRIAGE_SUBSCRIBE if self.settings.is_some() => {
                self.triage_subscribe(conn, outbox, req)
            }
            method::AI_TRIAGE_UNSUBSCRIBE => self.unsubscribe(conn, req),
            other => Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("unknown method {other}"),
            )),
        };
        let (response, commit) = match outcome {
            Ok((value, commit)) => {
                let line: Arc<[u8]> =
                    airpost_ipc::encode(&Response::ok(req.id.clone(), value)).into();
                match over_budget(line.len()) {
                    Some(e) => (Err(e), Commit::None),
                    None => (Ok(line), commit),
                }
            }
            Err(e) => (Err(e), Commit::None),
        };
        let line = match (response, &req.id) {
            (Ok(line), Some(_)) => Some(line),
            (Err(e), Some(id)) => {
                Some(airpost_ipc::encode(&Response::err(Some(id.clone()), e)).into())
            }
            (_, None) => None,
        };
        if let Some(line) = line {
            if !deliver(&self.hub, conn, outbox, line) {
                return;
            }
        }
        match commit {
            Commit::None => {}
            Commit::Query(id, sub) => {
                self.queries.insert(id, sub);
            }
            Commit::Window {
                sub_id,
                window,
                ids,
                total,
                version,
            } => {
                if let Some(sub) = self.queries.get_mut(&sub_id) {
                    (sub.window, sub.ids, sub.total, sub.version) = (window, ids, total, version);
                }
            }
            Commit::Events(id, sub) => {
                self.events.insert(id, sub);
            }
            Commit::Triage(id, sub) => {
                self.triage.insert(id, sub);
            }
        }
    }

    fn new_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    /// Ends `conn`'s subscription named `key`, as a new one with the same key replaces it.
    fn drop_key(&mut self, conn: u64, key: Option<&str>) {
        let Some(key) = key else {
            return;
        };
        self.queries
            .retain(|_, s| !(s.conn == conn && s.key.as_deref() == Some(key)));
        self.events
            .retain(|_, s| !(s.conn == conn && s.key.as_deref() == Some(key)));
        self.triage
            .retain(|_, s| !(s.conn == conn && s.key.as_deref() == Some(key)));
    }

    fn query_subscribe(
        &mut self,
        conn: u64,
        outbox: &Arc<Outbox>,
        relayed: bool,
        req: &RpcRequest,
    ) -> Result<(Value, Commit), ErrorObject> {
        let p = req.params_as::<QuerySubscribeParams>()?;
        let query = live_query(&req.params)?;
        if let Some(accounts) = &query.accounts {
            for a in accounts {
                self.accounts.check(a)?;
            }
        }
        let window = if relayed {
            policy::cap_relayed_window(p.window)
        } else {
            p.window.unwrap_or_default()
        };
        let (rows, total) = self.read_window(&query, window)?;
        self.drop_key(conn, p.key.as_deref());
        let sub_id = self.new_id("q");
        let sub = QuerySub {
            conn,
            outbox: outbox.clone(),
            key: p.key,
            ids: ids_of(&rows),
            query,
            window,
            total,
            version: 0,
        };
        let result = to_value(QuerySubscribeResult {
            sub_id: sub_id.clone(),
            rows,
            total,
            version: 0,
        })?;
        Ok((result, Commit::Query(sub_id, sub)))
    }

    fn set_window(
        &mut self,
        conn: u64,
        relayed: bool,
        req: &RpcRequest,
    ) -> Result<(Value, Commit), ErrorObject> {
        let p = req.params_as::<QuerySetWindowParams>()?;
        let sub = self
            .queries
            .get(&p.sub_id)
            .filter(|s| s.conn == conn)
            .ok_or_else(|| not_found(&p.sub_id))?;
        let window = if relayed {
            policy::cap_relayed_window(Some(p.window))
        } else {
            p.window
        };
        let (query, version) = (sub.query.clone(), sub.version + 1);
        let (rows, total) = self.read_window(&query, window)?;
        let ids = ids_of(&rows);
        let result = to_value(QueryWindowResult {
            rows,
            total,
            version,
        })?;
        Ok((
            result,
            Commit::Window {
                sub_id: p.sub_id,
                window,
                ids,
                total,
                version,
            },
        ))
    }

    /// `query.unsubscribe`, `events.unsubscribe` and `ai.triage.unsubscribe`; an unknown id
    /// is already gone.
    fn unsubscribe(&mut self, conn: u64, req: &RpcRequest) -> Result<(Value, Commit), ErrorObject> {
        let p = req.params_as::<SubIdParams>()?;
        let id = p.sub_id.as_str();
        match req.method.as_str() {
            method::QUERY_UNSUBSCRIBE => {
                if self.queries.get(id).is_some_and(|s| s.conn == conn) {
                    self.queries.remove(id);
                }
            }
            method::EVENTS_UNSUBSCRIBE => {
                if self.events.get(id).is_some_and(|s| s.conn == conn) {
                    self.events.remove(id);
                }
            }
            _ => {
                if self.triage.get(id).is_some_and(|s| s.conn == conn) {
                    self.triage.remove(id);
                }
            }
        }
        Ok((json!({}), Commit::None))
    }

    fn triage_subscribe(
        &mut self,
        conn: u64,
        outbox: &Arc<Outbox>,
        req: &RpcRequest,
    ) -> Result<(Value, Commit), ErrorObject> {
        let p = req.params_as::<AiTriageSubscribeParams>()?;
        let found = self.triage_list(&p.list)?;
        self.drop_key(conn, p.key.as_deref());
        let sub_id = self.new_id("t");
        let result = to_value(AiTriageSubscribeResult {
            sub_id: sub_id.clone(),
            thread_ids: found.thread_ids.clone(),
            total: found.total,
        })?;
        let sub = TriageSub {
            conn,
            outbox: outbox.clone(),
            key: p.key,
            params: p.list,
            thread_ids: found.thread_ids,
            total: found.total,
        };
        Ok((result, Commit::Triage(sub_id, sub)))
    }

    fn triage_list(
        &mut self,
        params: &AiTriageListParams,
    ) -> Result<airpost_ipc::AiTriageListResult, ErrorObject> {
        let accounts = self.accounts.clone();
        let stored = crate::socket::stored_ai_in(self.settings.as_ref());
        self.reads.timed(|reads| {
            crate::ai_triage::list_threads(params.clone(), reads, &*accounts, &stored)
        })
    }

    fn events_subscribe(
        &mut self,
        conn: u64,
        outbox: &Arc<Outbox>,
        req: &RpcRequest,
    ) -> Result<(Value, Commit), ErrorObject> {
        let p = req.params_as::<EventsSubscribeParams>()?;
        let params = EventsExpandParams {
            account_ids: p.account_ids,
            calendar_ids: p.calendar_ids,
            start: p.start,
            end: p.end,
            tz: p.tz,
            ..Default::default()
        };
        let found = self.expand(&params)?;
        self.drop_key(conn, p.key.as_deref());
        let sub_id = self.new_id("e");
        let result = to_value(EventsSubscribeResult {
            sub_id: sub_id.clone(),
            occurrences: found.occurrences.clone(),
            truncated: found.truncated,
        })?;
        let sub = EventsSub {
            conn,
            outbox: outbox.clone(),
            key: p.key,
            params,
            occurrences: found.occurrences,
            truncated: found.truncated,
        };
        Ok((result, Commit::Events(sub_id, sub)))
    }

    fn expand(&mut self, params: &EventsExpandParams) -> Result<EventsExpandResult, ErrorObject> {
        let accounts = self.accounts.clone();
        let v = self
            .reads
            .timed(|reads| crate::pim_api::expand(&*accounts, reads, params.clone()))?;
        serde_json::from_value(v).map_err(|e| ErrorObject::new(code::INTERNAL_ERROR, e.to_string()))
    }

    // -- reads ----------------------------------------------------------------------------

    fn rows(&mut self, q: &ModelQuery) -> Result<Vec<Value>, ErrorObject> {
        match self.reads.run_query(q)? {
            QueryOutput::Rows { rows, .. } => Ok(rows),
            _ => Ok(Vec::new()),
        }
    }

    fn count(&mut self, query: &ModelQuery) -> Result<i64, ErrorObject> {
        let mut q = query.clone();
        q.count = Some(true);
        q.include = None;
        match self.reads.run_query(&q)? {
            QueryOutput::Count(n) => Ok(n),
            _ => Ok(0),
        }
    }

    fn read_window(
        &mut self,
        query: &ModelQuery,
        window: QueryWindow,
    ) -> Result<(Vec<Value>, i64), ErrorObject> {
        let rows = self.rows(&windowed(query, window))?;
        Ok((rows, self.count(query)?))
    }

    fn window_ids(
        &mut self,
        query: &ModelQuery,
        window: QueryWindow,
    ) -> Result<Vec<String>, ErrorObject> {
        let mut q = windowed(query, window);
        q.ids_only = Some(true);
        q.include = None;
        match self.reads.run_query(&q)? {
            QueryOutput::Ids(ids) => Ok(ids),
            _ => Ok(Vec::new()),
        }
    }

    /// The rows of `ids` as the query returns them (its accounts and joined data), by id.
    fn rows_by_id(
        &mut self,
        query: &ModelQuery,
        ids: &[&str],
    ) -> Result<HashMap<String, Value>, ErrorObject> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let q = ModelQuery {
            class: query.class.clone(),
            accounts: query.accounts.clone(),
            where_: Some(json!({"attr": "id", "op": "in", "value": ids})),
            include: query.include.clone(),
            ..Default::default()
        };
        Ok(self
            .rows(&q)?
            .into_iter()
            .filter_map(|r| Some((r.get("id")?.as_str()?.to_owned(), r)))
            .collect())
    }

    // -- re-runs --------------------------------------------------------------------------

    fn refresh(&mut self, pending: Pending) {
        let affects = |accounts: Option<&[String]>, classes: &[&str]| -> Option<HashSet<String>> {
            let mut hit = pending
                .rerun
                .iter()
                .any(|a| wants_account(accounts, a))
                .then(HashSet::new);
            for (account, touched) in &pending.touched {
                if !wants_account(accounts, account) {
                    continue;
                }
                for class in classes {
                    if let Some(ids) = touched.get(*class) {
                        hit.get_or_insert_with(HashSet::new)
                            .extend(ids.iter().cloned());
                    }
                }
            }
            hit
        };
        let due: Vec<(String, HashSet<String>)> = self
            .queries
            .iter()
            .filter_map(|(id, s)| {
                affects(s.query.accounts.as_deref(), &followed_classes(&s.query))
                    .map(|t| (id.clone(), t))
            })
            .collect();
        for (sub_id, touched) in due {
            self.refresh_query(&sub_id, &touched);
        }
        let due: Vec<String> = self
            .events
            .iter()
            .filter(|(_, s)| {
                affects(s.params.account_ids.as_deref(), &["Event", "Calendar"]).is_some()
            })
            .map(|(id, _)| id.clone())
            .collect();
        for sub_id in due {
            self.refresh_events(&sub_id);
        }
        let due: Vec<String> = self
            .triage
            .iter()
            .filter(|(_, s)| {
                pending.triage_settings
                    || affects(s.params.account_ids.as_deref(), &["Thread"]).is_some()
            })
            .map(|(id, _)| id.clone())
            .collect();
        for sub_id in due {
            self.refresh_triage(&sub_id);
        }
    }

    fn refresh_triage(&mut self, sub_id: &str) {
        let Some(sub) = self.triage.get(sub_id) else {
            return;
        };
        let params = sub.params.clone();
        let found = match self.triage_list(&params) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    "-X Live triage list {sub_id} could not be read again: {}",
                    e.message
                );
                return;
            }
        };
        let Some(sub) = self.triage.get_mut(sub_id) else {
            return;
        };
        if sub.thread_ids == found.thread_ids && sub.total == found.total {
            return;
        }
        sub.thread_ids = found.thread_ids.clone();
        sub.total = found.total;
        let params = AiTriageChangedParams {
            sub_id: sub_id.to_owned(),
            thread_ids: found.thread_ids,
            total: found.total,
        };
        let (conn, outbox) = (sub.conn, sub.outbox.clone());
        if !notify(&self.hub, conn, &outbox, method::AI_TRIAGE_CHANGED, params) {
            self.triage.remove(sub_id);
        }
    }

    fn refresh_query(&mut self, sub_id: &str, touched: &HashSet<String>) {
        let Some(sub) = self.queries.get(sub_id) else {
            return;
        };
        let (query, window, old) = (sub.query.clone(), sub.window, sub.ids.clone());
        let changed = match self.query_changes(&query, window, &old, touched, sub.total) {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => {
                warn!(
                    "-X Live query {sub_id} could not be read again: {}",
                    e.message
                );
                return;
            }
        };
        let Some(sub) = self.queries.get_mut(sub_id) else {
            return;
        };
        sub.version += 1;
        sub.ids = changed.ids;
        sub.total = changed.total;
        let params = QueryChangedParams {
            sub_id: sub_id.to_owned(),
            version: sub.version,
            total: changed.total,
            ops: changed.ops,
            reset: changed.reset,
        };
        let (conn, outbox) = (sub.conn, sub.outbox.clone());
        if !notify(&self.hub, conn, &outbox, method::QUERY_CHANGED, params) {
            self.queries.remove(sub_id);
        }
    }

    /// The window's change since `old`, or `None` when nothing the client holds changed.
    fn query_changes(
        &mut self,
        query: &ModelQuery,
        window: QueryWindow,
        old: &[String],
        touched: &HashSet<String>,
        old_total: i64,
    ) -> Result<Option<Changes>, ErrorObject> {
        let new = self.window_ids(query, window)?;
        let total = self.count(query)?;
        let before: HashSet<&str> = old.iter().map(String::as_str).collect();
        let updated: Vec<&str> = new
            .iter()
            .map(String::as_str)
            .filter(|id| before.contains(id) && touched.contains(*id))
            .collect();
        if new == old && updated.is_empty() && total == old_total {
            return Ok(None);
        }
        let steps = diff(old, &new);
        if steps.len() + updated.len() > new.len().max(1) {
            let rows = self.rows(&windowed(query, window))?;
            return Ok(Some(Changes {
                ids: ids_of(&rows),
                total,
                ops: None,
                reset: Some(rows),
            }));
        }
        let wanted: Vec<&str> = steps
            .iter()
            .filter_map(|s| match s {
                Step::Insert(id, _) => Some(id.as_str()),
                _ => None,
            })
            .chain(updated.iter().copied())
            .collect();
        let mut rows = self.rows_by_id(query, &wanted)?;
        let mut ops = Vec::with_capacity(steps.len() + updated.len());
        for step in steps {
            ops.push(match step {
                Step::Remove(id) => LiveOp::Remove { id },
                Step::Move(id, index) => LiveOp::Move { id, index },
                Step::Insert(id, index) => match rows.remove(&id) {
                    Some(row) => LiveOp::Insert { index, row },
                    // Gone between the two reads: the next flush removes it.
                    None => LiveOp::Insert {
                        index,
                        row: json!({ "id": id }),
                    },
                },
            });
        }
        for id in updated {
            if let Some(row) = rows.remove(id) {
                ops.push(LiveOp::Update { row });
            }
        }
        Ok(Some(Changes {
            ids: new,
            total,
            ops: Some(ops),
            reset: None,
        }))
    }

    fn refresh_events(&mut self, sub_id: &str) {
        let Some(sub) = self.events.get(sub_id) else {
            return;
        };
        let params = sub.params.clone();
        let found = match self.expand(&params) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    "-X Live events {sub_id} could not be read again: {}",
                    e.message
                );
                return;
            }
        };
        let Some(sub) = self.events.get_mut(sub_id) else {
            return;
        };
        if sub.occurrences == found.occurrences && sub.truncated == found.truncated {
            return;
        }
        sub.occurrences = found.occurrences.clone();
        sub.truncated = found.truncated;
        let params = EventsChangedParams {
            sub_id: sub_id.to_owned(),
            occurrences: found.occurrences,
            truncated: found.truncated,
        };
        let (conn, outbox) = (sub.conn, sub.outbox.clone());
        if !notify(&self.hub, conn, &outbox, method::EVENTS_CHANGED, params) {
            self.events.remove(sub_id);
        }
    }
}

struct Changes {
    ids: Vec<String>,
    total: i64,
    ops: Option<Vec<LiveOp>>,
    reset: Option<Vec<Value>>,
}

fn notify(hub: &Hub, conn: u64, outbox: &Outbox, name: &str, params: impl Serialize) -> bool {
    let n = Notification::new(name, serde_json::to_value(params).unwrap_or(Value::Null));
    deliver(hub, conn, outbox, airpost_ipc::encode(&n).into())
}

/// The subscription's query: a `query` without paging or a result shape of its own.
fn live_query(params: &Value) -> Result<ModelQuery, ErrorObject> {
    let q: ModelQuery = serde_json::from_value(params.get("query").cloned().unwrap_or_default())
        .map_err(|e| invalid(format!("query: {e}")))?;
    if q.limit.is_some()
        || q.offset.is_some()
        || q.cursor.is_some()
        || q.ids_only == Some(true)
        || q.count == Some(true)
    {
        return Err(invalid(
            "a live query's window picks its rows: leave limit, offset, cursor, idsOnly and \
             count out of the query",
        ));
    }
    Ok(q)
}

fn windowed(query: &ModelQuery, window: QueryWindow) -> ModelQuery {
    let mut q = query.clone();
    q.offset = (window.offset > 0).then_some(window.offset as i64);
    q.limit = window.limit.map(|l| l as i64);
    q
}

fn ids_of(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| r.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Remove(String),
    Insert(String, usize),
    Move(String, usize),
}

/// The steps that turn `old` into `new`, applied in order as [`LiveOp`] says: the removed
/// ids, then each id that isn't in the longest run `old` and `new` keep in the same order,
/// moved or inserted right after the id that precedes it in `new`.
fn diff(old: &[String], new: &[String]) -> Vec<Step> {
    let in_new: HashSet<&str> = new.iter().map(String::as_str).collect();
    let mut steps: Vec<Step> = old
        .iter()
        .filter(|id| !in_new.contains(id.as_str()))
        .map(|id| Step::Remove(id.clone()))
        .collect();
    let mut cur: Vec<&str> = old
        .iter()
        .map(String::as_str)
        .filter(|id| in_new.contains(id))
        .collect();
    let kept: HashSet<&str> = cur.iter().copied().collect();
    let position: HashMap<&str, usize> = new
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let order: Vec<usize> = cur.iter().map(|id| position[id]).collect();
    let stable: HashSet<&str> = longest_increasing(&order)
        .into_iter()
        .map(|i| cur[i])
        .collect();
    for (i, id) in new.iter().enumerate() {
        let id = id.as_str();
        if stable.contains(id) {
            continue;
        }
        let existed = kept.contains(id);
        if existed {
            if let Some(p) = cur.iter().position(|x| *x == id) {
                cur.remove(p);
            }
        }
        let index = match i {
            0 => 0,
            _ => {
                let before = new[i - 1].as_str();
                cur.iter().position(|x| *x == before).map_or(0, |p| p + 1)
            }
        };
        cur.insert(index, id);
        steps.push(if existed {
            Step::Move(id.to_owned(), index)
        } else {
            Step::Insert(id.to_owned(), index)
        });
    }
    steps
}

/// The indexes of one longest strictly increasing subsequence of `seq`.
fn longest_increasing(seq: &[usize]) -> Vec<usize> {
    // tails[k]: the index of the smallest last value of an increasing run of length k + 1.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; seq.len()];
    for (i, &v) in seq.iter().enumerate() {
        let k = tails.partition_point(|&t| seq[t] < v);
        prev[i] = k.checked_sub(1).map(|j| tails[j]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut out = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(i) = at {
        out.push(i);
        at = prev[i];
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(old: &[String], steps: &[Step]) -> Vec<String> {
        let mut rows = old.to_vec();
        for s in steps {
            match s {
                Step::Remove(id) => rows.retain(|r| r != id),
                Step::Insert(id, i) => rows.insert(*i, id.clone()),
                Step::Move(id, i) => {
                    rows.retain(|r| r != id);
                    rows.insert(*i, id.clone());
                }
            }
        }
        rows
    }

    fn ids(s: &str) -> Vec<String> {
        s.chars().map(String::from).collect()
    }

    #[test]
    fn small_changes_take_few_steps() {
        let cases = [
            ("abcd", "abcd", 0),
            ("abcd", "xabcd", 1),
            ("abcd", "abd", 1),
            ("abcd", "bcda", 1),
            ("abcd", "dabc", 1),
            ("abcd", "badc", 2),
            ("cdab", "abcd", 2),
            ("", "ab", 2),
            ("ab", "", 2),
        ];
        for (old, new, n) in cases {
            let (old, new) = (ids(old), ids(new));
            let steps = diff(&old, &new);
            assert_eq!(apply(&old, &steps), new, "{old:?} -> {new:?}: {steps:?}");
            assert_eq!(steps.len(), n, "{old:?} -> {new:?}: {steps:?}");
        }
    }

    #[test]
    fn any_change_applies() {
        // A fixed xorshift, so a failure repeats.
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % n as u64) as usize
        };
        let pool: Vec<String> = (0..12).map(|i| format!("r{i}")).collect();
        for _ in 0..3000 {
            let mut lists = [Vec::new(), Vec::new()];
            for list in &mut lists {
                let len = next(10);
                let mut p = pool.clone();
                while list.len() < len {
                    list.push(p.remove(next(p.len())));
                }
            }
            let [old, new] = lists;
            let steps = diff(&old, &new);
            assert_eq!(apply(&old, &steps), new, "{old:?} -> {new:?}: {steps:?}");
        }
    }

    #[test]
    fn longest_increasing_run() {
        assert_eq!(longest_increasing(&[]), Vec::<usize>::new());
        assert_eq!(longest_increasing(&[2, 0, 1, 3]), vec![1, 2, 3]);
        assert_eq!(longest_increasing(&[3, 2, 1]).len(), 1);
    }

    #[test]
    fn touched_ids_by_class() {
        let mut t = Touched::new();
        let item = DeltaItem::new(
            DELTA_TYPE_PERSIST,
            "Thread",
            vec![json!({"id": "t1"}), json!({"id": "t2"})],
        );
        note_touched(&mut t, &item);
        note_touched(
            &mut t,
            &DeltaItem::new(DELTA_TYPE_PERSIST, "ProcessState", vec![json!({"id": "a"})]),
        );
        note_touched(
            &mut t,
            &DeltaItem::new("metadata-expiration", "Message", vec![json!({"id": "m"})]),
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t["Thread"].len(), 2);

        note_touched(
            &mut t,
            &DeltaItem::new(
                DELTA_TYPE_PERSIST,
                "Message",
                vec![json!({"id": "m1", "threadId": "t3"})],
            ),
        );
        assert!(t[THREADS_OF_MESSAGES].contains("t3"));
        let q = |v: Value| serde_json::from_value::<ModelQuery>(v).unwrap();
        assert_eq!(
            followed_classes(&q(json!({"class": "Thread", "include": ["summary"]}))),
            ["Thread", THREADS_OF_MESSAGES]
        );
        assert_eq!(followed_classes(&q(json!({"class": "Thread"}))), ["Thread"]);
        assert_eq!(
            followed_classes(&q(json!({"class": "Message", "include": ["body"]}))),
            ["Message"]
        );
    }

    #[test]
    fn live_queries_page_by_window_only() {
        assert!(live_query(&json!({"query": {"class": "Thread"}})).is_ok());
        for extra in [
            json!({"limit": 5}),
            json!({"offset": 5}),
            json!({"cursor": ""}),
            json!({"idsOnly": true}),
            json!({"count": true}),
        ] {
            let mut q = json!({"class": "Thread"});
            for (k, v) in extra.as_object().unwrap() {
                q[k] = v.clone();
            }
            let e = live_query(&json!({ "query": q })).unwrap_err();
            assert_eq!(e.code, code::INVALID_PARAMS, "{extra}");
        }
        assert_eq!(
            live_query(&json!({"query": {"class": "Thread", "nope": 1}}))
                .unwrap_err()
                .code,
            code::INVALID_PARAMS
        );
    }
}
