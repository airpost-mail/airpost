// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern watch`, and the subscription fan-in `tern task wait` shares: one reader thread per
//! engine, all feeding one channel.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tern_ipc::{
    method, ClientError, DeltaParams, DisconnectParams, MailNewParams, Notification,
    SubscribeParams,
};

use crate::app::Ctx;
use crate::discovery::{self, Engine};
use crate::error::{CliError, Result};
use crate::output;

/// Notifications a reader thread may hold before it stops reading its socket. A consumer
/// that stops reading stdout then stalls the readers, and the engine's per-connection
/// bound drops the connection instead of the CLI buffering without limit.
const QUEUED_NOTIFICATIONS: usize = 64;

enum Event {
    Note(u32, Notification),
    /// The engine's connection ended (closed, or failed with the error).
    Gone {
        pid: u32,
        error: Option<String>,
    },
}

/// Subscribed engines, read on their own threads.
pub struct Subscriptions {
    rx: mpsc::Receiver<Event>,
    open: usize,
    /// account id -> email, for printing.
    pub emails: BTreeMap<String, String>,
}

impl Subscriptions {
    /// Subscribes each engine that holds one of `accounts` (all, when None) to `classes`
    /// (all, when None). An engine already subscribed by the caller can be passed with
    /// `subscribed = true`.
    pub fn start(
        engines: Vec<(Engine, bool)>,
        accounts: Option<Vec<String>>,
        classes: Option<Vec<String>>,
    ) -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel(QUEUED_NOTIFICATIONS);
        let mut emails = BTreeMap::new();
        let mut open = 0;
        for (mut e, subscribed) in engines {
            let held: Vec<String> = e
                .hello
                .accounts
                .iter()
                .map(|a| a.account_id.clone())
                .collect();
            let wanted: Option<Vec<String>> = accounts.as_ref().map(|want| {
                held.iter()
                    .filter(|id| want.contains(id))
                    .cloned()
                    .collect()
            });
            if wanted.as_ref().is_some_and(Vec::is_empty) {
                continue;
            }
            for a in &e.hello.accounts {
                emails.insert(a.account_id.clone(), a.email.clone());
            }
            let pid = e.entry.pid;
            if !subscribed {
                let params = SubscribeParams {
                    accounts: wanted,
                    classes: classes.clone(),
                };
                e.client.subscribe(&params).map_err(|err| {
                    CliError::call(&format!("engine pid {pid}"), method::SUBSCRIBE, err)
                })?;
            }
            e.client
                .set_read_timeout(None)
                .map_err(|err| CliError::failed(format!("engine pid {pid}: {err}")))?;
            let tx = tx.clone();
            std::thread::Builder::new()
                .name(format!("engine-{pid}"))
                .spawn(move || read_loop(e, tx))
                .map_err(|err| CliError::failed(format!("could not start a reader: {err}")))?;
            open += 1;
        }
        Ok(Subscriptions { rx, open, emails })
    }

    /// The next notification, the first engine lost, or None when `deadline` passed.
    pub fn next(&mut self, deadline: Option<Instant>) -> NextEvent {
        if self.open == 0 {
            return NextEvent::AllGone;
        }
        let event = match deadline {
            None => self
                .rx
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            Some(d) => self
                .rx
                .recv_timeout(d.saturating_duration_since(Instant::now())),
        };
        match event {
            Ok(Event::Note(pid, n)) if n.method == method::DISCONNECT => {
                let reason = serde_json::from_value::<DisconnectParams>(n.params)
                    .map(|p| p.reason)
                    .unwrap_or_default();
                self.open -= 1;
                NextEvent::Lost {
                    pid,
                    reason: format!("the engine closed the connection: {reason}"),
                }
            }
            Ok(Event::Note(_, n)) => NextEvent::Note(n),
            Ok(Event::Gone { pid, error }) => {
                self.open -= 1;
                NextEvent::Lost {
                    pid,
                    reason: match error {
                        Some(e) => format!("lost the connection: {e}"),
                        None => "the engine closed the connection".to_owned(),
                    },
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => NextEvent::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => NextEvent::AllGone,
        }
    }
}

pub enum NextEvent {
    Note(Notification),
    /// An engine's connection ended; its deltas stop.
    Lost {
        pid: u32,
        reason: String,
    },
    TimedOut,
    AllGone,
}

/// The error for a lost engine, naming the reason the engine gave.
pub fn lost(pid: u32, reason: &str) -> CliError {
    CliError::failed(format!(
        "engine pid {pid}: {reason}; `tern status` shows whether it is still running"
    ))
}

fn read_loop(mut e: Engine, tx: mpsc::SyncSender<Event>) {
    let pid = e.entry.pid;
    loop {
        match e.client.next_notification() {
            Ok(n) => {
                if tx.send(Event::Note(pid, n)).is_err() {
                    return;
                }
            }
            Err(ClientError::Closed) => {
                let _ = tx.send(Event::Gone { pid, error: None });
                return;
            }
            Err(err) => {
                let _ = tx.send(Event::Gone {
                    pid,
                    error: Some(err.to_string()),
                });
                return;
            }
        }
    }
}

/// What one changed model is called in a delta line.
pub fn model_summary(class: &str, m: &Value) -> String {
    let s = |k: &str| m[k].as_str().unwrap_or("");
    let text = match class {
        "Thread" | "Message" => output::one_line(s("subject")),
        "Folder" | "Label" => s("path").to_owned(),
        "Task" => {
            let status = s("status");
            if m["error"].is_null() {
                format!("{} {status}", s("__cls"))
            } else {
                format!("{} {status} with an error", s("__cls"))
            }
        }
        "Contact" => output::contact(m),
        "ProcessState" => {
            if m["connectionError"].as_bool() == Some(true) {
                "connection error".to_owned()
            } else {
                "connected".to_owned()
            }
        }
        _ => String::new(),
    };
    if text.is_empty() {
        s("id").to_owned()
    } else {
        text
    }
}

/// One line for a delta: time, account, kind, class, count, and the first model.
pub fn delta_line(d: &DeltaParams, account: &str, time: &str) -> String {
    let n = d.model_jsons.len();
    let first = d
        .model_jsons
        .first()
        .map(|m| model_summary(&d.model_class, m))
        .unwrap_or_default();
    let more = if n > 1 {
        format!(" (+{} more)", n - 1)
    } else {
        String::new()
    };
    format!(
        "{time}  {account}  {} {} {n}  {first}{more}",
        d.kind, d.model_class
    )
}

/// One line for a `mail.new`: time, account, and the message and thread ids.
pub fn mail_new_line(p: &MailNewParams, account: &str, time: &str) -> String {
    format!(
        "{time}  {account}  new mail {}  thread {}",
        p.message_id, p.thread_id
    )
}

/// `tern watch`: deltas of `classes` (every class when empty), and `mail.new` either way.
/// `new_mail_only` subscribes to no class, so only `mail.new` arrives.
pub fn watch(
    ctx: &Ctx,
    engines: Vec<Engine>,
    classes: Vec<String>,
    new_mail_only: bool,
) -> Result<()> {
    let accounts = match ctx.account() {
        Some(_) => Some(
            discovery::targets(&engines, ctx.account())?
                .into_iter()
                .map(|t| t.account_id)
                .collect(),
        ),
        None => None,
    };
    let classes = if new_mail_only {
        Some(Vec::new())
    } else {
        (!classes.is_empty()).then_some(classes)
    };
    let engines = engines.into_iter().map(|e| (e, false)).collect();
    let mut subs = Subscriptions::start(engines, accounts, classes)?;
    loop {
        match subs.next(None) {
            NextEvent::Note(n) if n.method == method::DELTA => {
                if ctx.json {
                    output::json(&n.params);
                    continue;
                }
                let Ok(d) = serde_json::from_value::<DeltaParams>(n.params) else {
                    continue;
                };
                let email = subs
                    .emails
                    .get(&d.account_id)
                    .map_or(d.account_id.as_str(), String::as_str);
                let width = ctx.style.width;
                let l = delta_line(&d, email, &output::clock_time());
                output::line(&width.map_or(l.clone(), |w| output::truncate(&l, w)));
            }
            NextEvent::Note(n) if n.method == method::MAIL_NEW => {
                let Ok(p) = serde_json::from_value::<MailNewParams>(n.params.clone()) else {
                    continue;
                };
                if ctx.json {
                    let mut doc = json!({ "event": method::MAIL_NEW });
                    if let (Some(d), Some(params)) = (doc.as_object_mut(), n.params.as_object()) {
                        d.extend(params.clone());
                    }
                    output::json(&doc);
                    continue;
                }
                let email = subs
                    .emails
                    .get(&p.account_id)
                    .map_or(p.account_id.as_str(), String::as_str);
                let l = mail_new_line(&p, email, &output::clock_time());
                output::line(
                    &ctx.style
                        .width
                        .map_or(l.clone(), |w| output::truncate(&l, w)),
                );
            }
            NextEvent::Note(_) | NextEvent::TimedOut => {}
            // A stream missing an account's deltas would read as complete, so losing any
            // engine ends the watch.
            NextEvent::Lost { pid, reason } => {
                if ctx.json {
                    output::json(&json!({ "event": "disconnect", "pid": pid, "reason": reason }));
                }
                return Err(lost(pid, &reason));
            }
            NextEvent::AllGone => {
                return Err(CliError::failed(
                    "no engine holds the accounts to watch; `tern status` lists them",
                ))
            }
        }
    }
}

/// Waits at most `timeout` from now.
pub fn deadline(timeout: Option<u64>) -> Option<Instant> {
    timeout.map(|s| Instant::now() + Duration::from_secs(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tern_client::discovery::Entry;
    use tern_ipc::{AccountInfo, Client, HelloResult};

    /// An engine whose connection yields `input` (NDJSON) and then ends.
    fn engine(pid: u32, account: &str, input: String) -> Engine {
        Engine {
            file: "/tmp/run/x.json".into(),
            entry: Entry {
                pid,
                socket: "/tmp/x.sock".into(),
                protocol: 1,
                account_id: Some(account.into()),
                accounts: None,
            },
            hello: HelloResult {
                protocol: 1,
                engine: "0.1.0".into(),
                pid,
                accounts: vec![AccountInfo {
                    account_id: account.into(),
                    email: format!("{account}@x.test"),
                    state: "running".into(),
                    error: None,
                    ..Default::default()
                }],
                features: Vec::new(),
                capabilities: None,
                ..Default::default()
            },
            client: Client::from_streams(
                Box::new(std::io::Cursor::new(input.into_bytes())),
                Box::new(std::io::sink()),
            ),
        }
    }

    fn note(method: &str, params: Value) -> String {
        format!(
            "{}\n",
            json!({"jsonrpc": "2.0", "method": method, "params": params})
        )
    }

    #[test]
    fn a_lost_engine_is_reported_with_its_reason() {
        let delta = note(
            method::DELTA,
            json!({"accountId": "a", "modelClass": "Task", "type": "persist", "modelJSONs": []}),
        );
        let bye = note(
            method::DISCONNECT,
            json!({"reason": "the connection fell behind"}),
        );
        let mut subs =
            Subscriptions::start(vec![(engine(7, "a", delta + &bye), true)], None, None).unwrap();
        let until = deadline(Some(10));
        assert!(matches!(subs.next(until), NextEvent::Note(n) if n.method == method::DELTA));
        match subs.next(until) {
            NextEvent::Lost { pid, reason } => {
                assert_eq!(pid, 7);
                assert!(reason.contains("fell behind"), "{reason}");
            }
            _ => panic!("expected the engine to be lost"),
        }

        let mut subs =
            Subscriptions::start(vec![(engine(8, "b", String::new()), true)], None, None).unwrap();
        assert!(matches!(subs.next(until), NextEvent::Lost { pid: 8, .. }));
        assert!(matches!(subs.next(until), NextEvent::AllGone));
    }

    #[test]
    fn delta_lines_name_the_first_model() {
        let d = DeltaParams {
            account_id: "a1".into(),
            model_class: "Thread".into(),
            kind: "persist".into(),
            model_jsons: vec![
                json!({"id": "t1", "subject": "Hello\nthere"}),
                json!({"id": "t2"}),
            ],
        };
        assert_eq!(
            delta_line(&d, "me@x.test", "12:00:00"),
            "12:00:00  me@x.test  persist Thread 2  Hello there (+1 more)"
        );
        assert_eq!(
            model_summary(
                "Task",
                &json!({"__cls": "ChangeStarredTask", "status": "complete", "error": null})
            ),
            "ChangeStarredTask complete"
        );
        assert_eq!(
            model_summary(
                "Task",
                &json!({"__cls": "X", "status": "complete", "error": {"key": "k"}})
            ),
            "X complete with an error"
        );
        assert_eq!(
            model_summary("Folder", &json!({"id": "f", "path": "INBOX"})),
            "INBOX"
        );
        assert_eq!(model_summary("File", &json!({"id": "f1"})), "f1");
        let p = MailNewParams {
            account_id: "a1".into(),
            thread_id: "t1".into(),
            message_id: "m1".into(),
        };
        assert_eq!(
            mail_new_line(&p, "me@x.test", "12:00:00"),
            "12:00:00  me@x.test  new mail m1  thread t1"
        );
        assert_eq!(
            model_summary(
                "ProcessState",
                &json!({"id": "a1", "connectionError": true})
            ),
            "connection error"
        );
    }
}
