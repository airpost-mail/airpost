// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! A scripted transport: routes answer by method, URL and an optional body fragment, and
//! every request is recorded.

#![allow(dead_code)]

use ms_core::{ManualClock, Result};
use ms_dav::{DavAuth, DavClient, HttpRequest, HttpResponse, Transport};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn fixture(path: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

struct Route {
    method: String,
    url: String,
    body_has: Option<String>,
    once: bool,
    used: bool,
    resp: HttpResponse,
}

#[derive(Default)]
struct State {
    routes: Vec<Route>,
    log: Vec<HttpRequest>,
}

#[derive(Clone, Default)]
pub struct Fake {
    state: Arc<Mutex<State>>,
    pub sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl Fake {
    pub fn new() -> Fake {
        Fake::default()
    }

    #[allow(clippy::too_many_arguments)]
    fn add(
        &self,
        method: &str,
        url: &str,
        body_has: Option<&str>,
        once: bool,
        status: u16,
        headers: &[(&str, &str)],
        body: &str,
    ) {
        self.state.lock().unwrap().routes.push(Route {
            method: method.into(),
            url: url.into(),
            body_has: body_has.map(String::from),
            once,
            used: false,
            resp: HttpResponse {
                status,
                url: url.into(),
                headers: headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                body: body.into(),
            },
        });
    }

    /// A route that answers every matching request.
    pub fn on(&self, method: &str, url: &str, status: u16, body: &str) -> &Self {
        self.add(method, url, None, false, status, &[], body);
        self
    }

    /// A route with headers that answers every matching request.
    pub fn on_with(
        &self,
        method: &str,
        url: &str,
        status: u16,
        headers: &[(&str, &str)],
        body: &str,
    ) -> &Self {
        self.add(method, url, None, false, status, headers, body);
        self
    }

    /// A route that answers every request whose body contains `body_has`.
    pub fn on_body(
        &self,
        method: &str,
        url: &str,
        body_has: &str,
        status: u16,
        body: &str,
    ) -> &Self {
        self.add(method, url, Some(body_has), false, status, &[], body);
        self
    }

    /// A route that answers once, matching a body fragment when given.
    pub fn once(
        &self,
        method: &str,
        url: &str,
        body_has: Option<&str>,
        status: u16,
        headers: &[(&str, &str)],
        body: &str,
    ) -> &Self {
        self.add(method, url, body_has, true, status, headers, body);
        self
    }

    pub fn redirect(&self, url: &str, location: &str) -> &Self {
        self.add("GET", url, None, false, 301, &[("Location", location)], "");
        self
    }

    /// Drops every route, keeping the request log: the next server state starts here.
    pub fn reset(&self) -> &Self {
        self.state.lock().unwrap().routes.clear();
        self
    }

    pub fn requests(&self) -> Vec<HttpRequest> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn client(&self) -> DavClient {
        let sleeps = self.sleeps.clone();
        DavClient::new(
            Box::new(self.clone()),
            DavAuth::Basic {
                username: "alice".into(),
                password: "secret".into(),
            },
        )
        .with_clock(Arc::new(ManualClock::new(1_718_000_000)))
        .with_sleep(Box::new(move |d| sleeps.lock().unwrap().push(d)))
    }
}

impl Transport for Fake {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let mut st = self.state.lock().unwrap();
        st.log.push(req.clone());
        let found = st.routes.iter_mut().find(|r| {
            !(r.once && r.used)
                && r.method == req.method
                && r.url == req.url
                && r.body_has
                    .as_ref()
                    .is_none_or(|b| req.body.contains(b.as_str()))
        });
        Ok(match found {
            Some(r) => {
                r.used = true;
                r.resp.clone()
            }
            None => HttpResponse {
                status: 404,
                url: req.url.clone(),
                headers: vec![],
                body: format!("unrouted {} {}", req.method, req.url),
            },
        })
    }
}
