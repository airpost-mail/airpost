// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! `messages.unsubscribe` on the socket: the message's
//! `List-Unsubscribe` header, acted on as the desktop's `list-unsubscribe` plugin does
//! ([`airpost_net::unsubscribe`]). The engine makes the one-click POST itself; a mailto becomes a
//! `SendDraftTask` on the account's `main` thread; a web link goes back to the client to
//! open. Unsubscribe links found only in the body are left to the client.
//!
//! The header is stored when the body is fetched, so for a message whose body isn't there
//! yet the engine asks for it and looks again once it arrives. That wait and the POST run on
//! an `unsubscribe` thread, out of order with the connection's later requests (T21).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use airpost_ipc::{
    code, unsubscribe_method, ErrorObject, MessagesUnsubscribeParams, MessagesUnsubscribeResult,
    Request as RpcRequest,
};
use airpost_net::unsubscribe::{self, Method};
use airpost_store::read::ReadStore;
use serde_json::Value;
use tracing::{info, warn};

use crate::dispatch::{Deferred, Reply, Request};
use crate::socket::{dispatch_on, Server, Session};

const POLL: Duration = Duration::from_millis(100);
/// Calls waiting for a body at once, engine-wide.
const MAX_WAITING: usize = 16;
static WAITING: AtomicUsize = AtomicUsize::new(0);
/// One-click POSTs running at once, engine-wide: each holds a thread for up to the lookup
/// plus the POST's timeout.
const MAX_POSTING: usize = 8;
/// The messages whose one-click POST is running.
static POSTING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// A running one-click POST's place in [`POSTING`], given back on drop.
struct Posting(String);

impl Posting {
    fn start(message_id: &str) -> Result<Posting, ErrorObject> {
        let mut running = POSTING.lock().unwrap_or_else(PoisonError::into_inner);
        if running.iter().any(|m| m == message_id) {
            return Err(internal(format!(
                "a one-click unsubscribe for message {message_id} is already running"
            )));
        }
        if running.len() >= MAX_POSTING {
            return Err(internal(
                "too many one-click unsubscribes are running; try again shortly",
            ));
        }
        running.push(message_id.to_owned());
        Ok(Posting(message_id.to_owned()))
    }
}

impl Drop for Posting {
    fn drop(&mut self) {
        let mut running = POSTING.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = running.iter().position(|m| *m == self.0) {
            running.swap_remove(i);
        }
    }
}

/// Debug builds read `AIRPOST_ENGINE_UNSUBSCRIBE_ALLOW_LOCAL=1` so tests can POST to a local
/// plain-http server.
fn allow_local() -> bool {
    cfg!(debug_assertions)
        && std::env::var("AIRPOST_ENGINE_UNSUBSCRIBE_ALLOW_LOCAL").is_ok_and(|v| v == "1")
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

/// The message's `List-Unsubscribe` and `List-Unsubscribe-Post` values.
#[derive(Clone, Debug, Default)]
struct Headers {
    list: Option<String>,
    post: Option<String>,
}

impl Headers {
    fn of(message: &Value) -> Headers {
        let get = |k: &str| message.get(k).and_then(Value::as_str).map(str::to_owned);
        Headers {
            list: get("hListUnsub"),
            post: get("hListUnsubPost"),
        }
    }
}

/// One call, owned so it can move to the `unsubscribe` thread.
struct Call {
    server: Arc<Server>,
    account_id: String,
    message_id: String,
    wanted: Option<Method>,
}

impl Session {
    pub fn unsubscribe_call(&mut self, req: &RpcRequest) -> Deferred {
        match req
            .params_as::<MessagesUnsubscribeParams>()
            .and_then(|p| self.unsubscribe(p))
        {
            Ok(d) => d,
            Err(e) => Deferred::Now(Err(e)),
        }
    }

    fn unsubscribe(&mut self, p: MessagesUnsubscribeParams) -> Result<Deferred, ErrorObject> {
        self.server.accounts.check(&p.account_id)?;
        let wanted = match p.method.as_deref() {
            None => None,
            Some(name) => Some(Method::from_name(name).ok_or_else(|| {
                ErrorObject::new(
                    code::INVALID_PARAMS,
                    format!(
                        "invalid params: method must be one of {}, {} and {}",
                        unsubscribe_method::ONE_CLICK,
                        unsubscribe_method::MAILTO,
                        unsubscribe_method::WEB
                    ),
                )
            })?),
        };
        let message = self
            .reads
            .message(&p.account_id, &p.message_id)?
            .ok_or_else(|| {
                ErrorObject::new(code::NOT_FOUND, format!("no message {}", p.message_id))
            })?;
        let call = Call {
            server: self.server.clone(),
            account_id: p.account_id,
            message_id: p.message_id,
            wanted,
        };
        let headers = Headers::of(&message);
        if headers.list.is_none() && !self.reads.has_body(&call.message_id)? {
            return call.after_body_fetch();
        }
        let chosen = call.choose(&headers)?;
        Ok(match chosen.0 {
            // The POST may take seconds.
            Method::OneClick => {
                let posting = Posting::start(&call.message_id)?;
                Deferred::Later(Box::new(move || {
                    let _posting = posting;
                    call.act(chosen)
                }))
            }
            _ => Deferred::Now(call.act(chosen)),
        })
    }
}

impl Call {
    fn choose(&self, h: &Headers) -> Result<(Method, String), ErrorObject> {
        unsubscribe::choose(
            h.list.as_deref(),
            h.post.as_deref(),
            self.wanted,
            allow_local(),
        )
        .ok_or_else(|| {
            let what = self.wanted.map_or("unsubscribe link", Method::name);
            ErrorObject::new(
                code::NOT_FOUND,
                format!(
                    "message {} has no {what} in a List-Unsubscribe header",
                    self.message_id
                ),
            )
        })
    }

    /// Asks for the body and answers once it is stored, or after the wait with what is
    /// known then.
    fn after_body_fetch(self) -> Result<Deferred, ErrorObject> {
        if WAITING.fetch_add(1, Ordering::SeqCst) >= MAX_WAITING {
            WAITING.fetch_sub(1, Ordering::SeqCst);
            return Err(internal(
                "too many messages.unsubscribe calls are waiting for bodies; try again shortly",
            ));
        }
        let fetch = Request::NeedBodies(vec![self.message_id.clone()]);
        if let Err(e) = dispatch_on(&self.server, &self.account_id, fetch) {
            WAITING.fetch_sub(1, Ordering::SeqCst);
            return Err(e);
        }
        struct Slot;
        impl Drop for Slot {
            fn drop(&mut self) {
                WAITING.fetch_sub(1, Ordering::SeqCst);
            }
        }
        // Taken here so work dropped without running (a full lane) gives it back.
        let slot = Slot;
        Ok(Deferred::Later(Box::new(move || {
            let headers = self.wait_for_body(self.server.config_dir.clone())?;
            drop(slot);
            let chosen = self.choose(&headers)?;
            let _posting = match chosen.0 {
                Method::OneClick => Some(Posting::start(&self.message_id)?),
                _ => None,
            };
            self.act(chosen)
        })))
    }

    fn wait_for_body(&self, config_dir: PathBuf) -> Result<Headers, ErrorObject> {
        let store = ReadStore::open(&config_dir).map_err(internal)?;
        let deadline = Instant::now() + crate::files::download_wait();
        loop {
            let message = store
                .message(&self.account_id, &self.message_id)
                .map_err(internal)?
                .ok_or_else(|| {
                    ErrorObject::new(code::NOT_FOUND, format!("no message {}", self.message_id))
                })?;
            let fetched = store
                .message_body(&self.message_id)
                .map_err(internal)?
                .is_some();
            if fetched || Instant::now() >= deadline {
                return Ok(Headers::of(&message));
            }
            std::thread::sleep(POLL);
        }
    }

    fn act(&self, (method, url): (Method, String)) -> Reply {
        let id = &self.message_id;
        let done = match method {
            Method::Web => Ok(None),
            Method::Mailto => match unsubscribe::parse_mailto(&url) {
                Err(why) => Err(why.to_string()),
                Ok(mailto) => {
                    info!("Unsubscribing from the list of message {id} by mail");
                    let request = Request::UnsubscribeMailto {
                        message_id: id.clone(),
                        mailto,
                    };
                    let queued = dispatch_on(&self.server, &self.account_id, request)?;
                    Ok(queued["taskId"].as_str().map(str::to_owned))
                }
            },
            Method::OneClick => {
                info!("Unsubscribing from the list of message {id} by one-click POST");
                unsubscribe::one_click(&url, allow_local())
                    .map(|()| None)
                    .map_err(|e| e.to_string())
            }
        };
        if let Err(why) = &done {
            warn!(
                "-X Unsubscribing by {} for message {id} failed: {why}",
                method.name()
            );
        }
        let result = MessagesUnsubscribeResult {
            method: method.name().to_owned(),
            ok: done.is_ok(),
            url: Some(url),
            task_id: done.as_ref().ok().cloned().flatten(),
            error: done.err(),
        };
        serde_json::to_value(result).map_err(internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_click_posts_are_capped_and_one_per_message() {
        let ids: Vec<String> = (0..MAX_POSTING).map(|i| format!("cap-test-{i}")).collect();
        let held: Vec<Posting> = ids.iter().map(|id| Posting::start(id).unwrap()).collect();
        assert!(Posting::start("cap-test-more")
            .err()
            .unwrap()
            .message
            .contains("too many"));
        drop(held);
        let first = Posting::start("cap-test-0").unwrap();
        assert!(Posting::start("cap-test-0")
            .err()
            .unwrap()
            .message
            .contains("already running"));
        drop(first);
        Posting::start("cap-test-0").unwrap();
    }
}
