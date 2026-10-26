// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A socket connection's reads ([`is_read_method`](crate::read_api::is_read_method)),
//! answered out of order on the [`workers::READS`] lane (feature `read-pool`,
//! docs/thin-clients-plan.md 4.2), so a slow FTS `query` holds back neither the
//! connection's other reads nor its other requests.
//!
//! Each connection has up to [`POOL_SIZE`] read connections ([`ReadApi`]), opened as its
//! reads need them and kept for its lifetime. A read waits on the connection, in arrival
//! order, while all of them are busy (up to [`MAX_WAITING`]); the lane bounds the threads
//! across connections.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use tern_ipc::{Request as RpcRequest, Response};

use crate::dispatch::Reply;
use crate::read_api::{over_budget, ReadApi};
use crate::socket::{deliver, Hub, Outbox, Server};
use crate::workers;

/// The most reads of one connection that run at once.
pub const POOL_SIZE: usize = 4;

/// The most reads of one connection that wait for a read connection; more are answered
/// `ENGINE_BUSY`.
const MAX_WAITING: usize = 256;

struct Waiting {
    req: RpcRequest,
}

#[derive(Default)]
struct State {
    idle: Vec<ReadApi>,
    /// Read connections opened, idle or running.
    open: usize,
    waiting: VecDeque<Waiting>,
    closed: bool,
}

pub struct ReadPool {
    conn: u64,
    server: Arc<Server>,
    outbox: Arc<Outbox>,
    state: Mutex<State>,
}

impl ReadPool {
    pub fn new(conn: u64, server: Arc<Server>, outbox: Arc<Outbox>) -> Arc<Self> {
        Arc::new(ReadPool {
            conn,
            server,
            outbox,
            state: Mutex::default(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Answers `req` when a read connection is free. A read without an id has no answer and
    /// changes nothing, so it isn't run.
    pub fn submit(self: &Arc<Self>, req: RpcRequest) {
        if req.id.is_none() {
            return;
        }
        let api = {
            let mut st = self.lock();
            if st.closed {
                return;
            }
            match self.take(&mut st) {
                Some(api) => api,
                None if st.waiting.len() < MAX_WAITING => {
                    st.waiting.push_back(Waiting { req });
                    return;
                }
                None => {
                    drop(st);
                    self.answer(req.id, Err(workers::busy(&workers::READS)));
                    return;
                }
            }
        };
        self.run(api, req);
    }

    /// An idle read connection, or a new one while fewer than [`POOL_SIZE`] are open.
    fn take(&self, st: &mut State) -> Option<ReadApi> {
        if let Some(api) = st.idle.pop() {
            return Some(api);
        }
        if st.open >= POOL_SIZE {
            return None;
        }
        st.open += 1;
        Some(ReadApi::new(
            self.server.config_dir.clone(),
            self.server.accounts.clone(),
            self.server.database.clone(),
        ))
    }

    fn run(self: &Arc<Self>, api: ReadApi, req: RpcRequest) {
        // The worker hands the read connection back here; a panic leaves it empty.
        let slot = Arc::new(Mutex::new(Some(api)));
        let work_slot = slot.clone();
        let id = req.id.clone();
        let work = move || {
            let taken = work_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            let Some(mut api) = taken else {
                return Ok(serde_json::Value::Null);
            };
            let reply = api.call(&req);
            *work_slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(api);
            reply
        };
        let pool = self.clone();
        workers::READS.submit(None, work, move |reply| {
            pool.answer(id, reply);
            let api = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            pool.release(api);
        });
    }

    fn answer(&self, id: Option<tern_ipc::Id>, reply: Reply) {
        let Some(id) = id else {
            return;
        };
        let line: Arc<[u8]> = match reply {
            Ok(v) => {
                let line: Arc<[u8]> = tern_ipc::encode(&Response::ok(Some(id.clone()), v)).into();
                match over_budget(line.len()) {
                    Some(e) => tern_ipc::encode(&Response::err(Some(id), e)).into(),
                    None => line,
                }
            }
            Err(e) => tern_ipc::encode(&Response::err(Some(id), e)).into(),
        };
        deliver(&self.hub(), self.conn, &self.outbox, line);
    }

    fn hub(&self) -> Arc<Hub> {
        self.server.hub.clone()
    }

    /// Takes back a read connection (none when its read panicked) and starts the next
    /// waiting read.
    fn release(self: &Arc<Self>, api: Option<ReadApi>) {
        let next = {
            let mut st = self.lock();
            match api {
                Some(api) if !st.closed => st.idle.push(api),
                Some(_) | None => st.open -= 1,
            }
            if st.closed || st.waiting.is_empty() {
                return;
            }
            let Some(api) = self.take(&mut st) else {
                return;
            };
            st.waiting.pop_front().map(|w| (api, w.req))
        };
        if let Some((api, req)) = next {
            self.run(api, req);
        }
    }

    /// The connection closed: waiting reads are dropped and read connections close as their
    /// reads end.
    pub fn close(&self) {
        let mut st = self.lock();
        st.closed = true;
        st.waiting.clear();
        let idle = st.idle.len();
        st.idle.clear();
        st.open -= idle;
    }
}
