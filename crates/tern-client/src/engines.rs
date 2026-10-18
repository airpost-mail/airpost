// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Connected engines and the router over them. Every live engine under one profile is
//! connected and greeted with `hello`; an account's calls go to the first engine whose
//! `hello` lists it, so a `serve` engine (`engine.json`, scanned first) wins over a
//! per-account entry left behind.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ms_ipc::{
    code, feature, method, Client, ClientError, CountsGetParams, CountsGetResult, FilesGetParams,
    FilesGetResult, FilesPutParams, FilesPutResult, FilesReadParams, FilesReadResult, HelloResult,
    MessagesUnsubscribeParams, MessagesUnsubscribeResult, MetadataListParams, MetadataListResult,
    QueryParams, QueryResult, SettingsGetParams, SettingsValues, TasksListParams, TasksListResult,
    ThreadsSnoozeParams, ThreadsSnoozeResult, ThreadsUnsnoozeParams, PROTOCOL_VERSION,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::discovery::{scan, Entry};

/// How long a connect and `hello` may take before the engine counts as hung.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long any other call may take. `task.queue` answers after the local phase, which can
/// wait out the store's 60 s busy timeout.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// An account some engine holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRef {
    pub account_id: String,
    pub email: String,
}

/// A failed engine call.
#[derive(Debug)]
pub enum EngineError {
    /// No engine is running, or none holds the account.
    NoEngine(String),
    /// The engine answered with an error.
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    /// The connection failed or the engine sent something unreadable.
    Transport(String),
}

impl EngineError {
    pub fn code(&self) -> Option<i64> {
        match self {
            EngineError::Rpc { code, .. } => Some(*code),
            _ => None,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.code() == Some(code::NOT_FOUND)
    }

    /// The engine doesn't know the method: it predates the feature.
    pub fn is_unknown_method(&self) -> bool {
        self.code() == Some(code::METHOD_NOT_FOUND)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::NoEngine(m) | EngineError::Transport(m) => f.write_str(m),
            EngineError::Rpc { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for EngineError {}

impl From<ClientError> for EngineError {
    fn from(e: ClientError) -> Self {
        match e {
            ClientError::Rpc(o) => EngineError::Rpc {
                code: o.code,
                message: o.message,
                data: o.data,
            },
            other => EngineError::Transport(format!("the engine connection failed: {other}")),
        }
    }
}

/// A connected engine.
pub struct Engine {
    /// Its discovery file.
    pub file: PathBuf,
    pub entry: Entry,
    pub hello: HelloResult,
    pub client: Client,
}

impl Engine {
    /// Connects to `entry`'s socket and says `hello` as `client_name` (`tern-cli/0.1.0`).
    /// The error says why, in words for the user; `product` names the client in it.
    pub fn connect(
        file: PathBuf,
        entry: Entry,
        client_name: &str,
        product: &str,
    ) -> Result<Engine, String> {
        let mut client = Client::connect(&entry.socket).map_err(|e| e.to_string())?;
        let hello = client
            .set_read_timeout(Some(HELLO_TIMEOUT))
            .and_then(|()| client.hello(client_name));
        let hello = match hello {
            Ok(h) => h,
            Err(ClientError::Rpc(e)) if e.code == code::UNSUPPORTED_PROTOCOL => {
                return Err(format!(
                    "it speaks protocol {} and {product} speaks {PROTOCOL_VERSION}; update \
                     {product} or Tern so they match",
                    entry.protocol
                ))
            }
            Err(e) if e.is_timeout() => {
                return Err("no answer to hello; the engine may be hung".into())
            }
            Err(e) => return Err(e.to_string()),
        };
        client
            .set_read_timeout(Some(CALL_TIMEOUT))
            .map_err(|e| e.to_string())?;
        Ok(Engine {
            file,
            entry,
            hello,
            client,
        })
    }

    /// Whether `hello.features` lists `name` ([`ms_ipc::feature`]).
    pub fn has_feature(&self, name: &str) -> bool {
        self.hello.features.iter().any(|f| f == name)
    }

    pub fn holds(&self, account_id: &str) -> bool {
        self.hello
            .accounts
            .iter()
            .any(|a| a.account_id == account_id)
    }

    pub fn call_as<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<T, EngineError> {
        Ok(self.client.call_as(method, params)?)
    }

    /// `counts.get` for `accounts` (every account the engine holds when `None`).
    pub fn counts_get(
        &mut self,
        accounts: Option<Vec<String>>,
    ) -> Result<CountsGetResult, EngineError> {
        self.call_as(method::COUNTS_GET, to_params(&CountsGetParams { accounts }))
    }

    pub fn tasks_list(&mut self, params: &TasksListParams) -> Result<TasksListResult, EngineError> {
        self.call_as(method::TASKS_LIST, to_params(params))
    }

    pub fn metadata_list(
        &mut self,
        params: &MetadataListParams,
    ) -> Result<MetadataListResult, EngineError> {
        self.call_as(method::METADATA_LIST, to_params(params))
    }

    /// `settings.get` of `keys` (every stored key when `None`).
    pub fn settings_get(
        &mut self,
        keys: Option<Vec<String>>,
    ) -> Result<SettingsValues, EngineError> {
        self.call_as(method::SETTINGS_GET, to_params(&SettingsGetParams { keys }))
    }

    /// `settings.set`; null clears a key.
    pub fn settings_set(
        &mut self,
        values: serde_json::Map<String, Value>,
    ) -> Result<(), EngineError> {
        self.call_as::<Value>(
            method::SETTINGS_SET,
            to_params(&SettingsValues {
                values,
                ..Default::default()
            }),
        )
        .map(drop)
    }

    /// `query` ([`feature::QUERY`](ms_ipc::feature::QUERY)). `accounts: None` reads every
    /// account in the engine's database, with no check against the accounts it holds.
    pub fn query(&mut self, params: &QueryParams) -> Result<QueryResult, EngineError> {
        self.call_as(method::QUERY, to_params(params))
    }

    /// `files.get` ([`feature::FILES`](ms_ipc::feature::FILES)): the file, downloaded first
    /// when it isn't on disk. Takes as long as the download, up to the engine's wait.
    pub fn files_get(
        &mut self,
        account_id: &str,
        file_id: &str,
    ) -> Result<FilesGetResult, EngineError> {
        self.call_as(
            method::FILES_GET,
            to_params(&FilesGetParams {
                account_id: account_id.to_owned(),
                file_id: file_id.to_owned(),
            }),
        )
    }

    /// `files.read`: one chunk of a downloaded file.
    pub fn files_read(&mut self, params: &FilesReadParams) -> Result<FilesReadResult, EngineError> {
        self.call_as(method::FILES_READ, to_params(params))
    }

    /// `files.put`: a new File for a draft, from `data` (base64) or a `path` the engine
    /// copies.
    pub fn files_put(&mut self, params: &FilesPutParams) -> Result<FilesPutResult, EngineError> {
        self.call_as(method::FILES_PUT, to_params(params))
    }

    /// `threads.snooze` ([`feature::SNOOZE`](ms_ipc::feature::SNOOZE)): the threads move to
    /// Snoozed until `until` (Unix seconds).
    pub fn threads_snooze(
        &mut self,
        account_id: &str,
        thread_ids: &[String],
        until: i64,
    ) -> Result<ThreadsSnoozeResult, EngineError> {
        self.call_as(
            method::THREADS_SNOOZE,
            to_params(&ThreadsSnoozeParams {
                account_id: account_id.to_owned(),
                thread_ids: thread_ids.to_vec(),
                until,
            }),
        )
    }

    /// `threads.unsnooze`: the snooze ends now.
    pub fn threads_unsnooze(
        &mut self,
        account_id: &str,
        thread_ids: &[String],
    ) -> Result<ThreadsSnoozeResult, EngineError> {
        self.call_as(
            method::THREADS_UNSNOOZE,
            to_params(&ThreadsUnsnoozeParams {
                account_id: account_id.to_owned(),
                thread_ids: thread_ids.to_vec(),
            }),
        )
    }

    /// `messages.unsubscribe` ([`feature::UNSUBSCRIBE`](ms_ipc::feature::UNSUBSCRIBE)).
    /// `method` is one of [`ms_ipc::unsubscribe_method`], or the engine's choice. A `web`
    /// result is a link for the caller to open.
    pub fn messages_unsubscribe(
        &mut self,
        account_id: &str,
        message_id: &str,
        method_name: Option<&str>,
    ) -> Result<MessagesUnsubscribeResult, EngineError> {
        self.call_as(
            method::MESSAGES_UNSUBSCRIBE,
            to_params(&MessagesUnsubscribeParams {
                account_id: account_id.to_owned(),
                message_id: message_id.to_owned(),
                method: method_name.map(str::to_owned),
            }),
        )
    }
}

fn to_params(p: &impl serde::Serialize) -> Value {
    serde_json::to_value(p).unwrap_or(Value::Null)
}

/// Every live engine under one profile, connected.
pub struct Engines {
    pub config_dir: PathBuf,
    client_name: String,
    product: String,
    pub engines: Vec<Engine>,
    /// Engines that have a discovery file but couldn't be reached, as messages.
    pub problems: Vec<String>,
}

impl Engines {
    /// Connects to every live engine under `config_dir`. `client_name` goes in `hello`;
    /// `product` names the client in error messages (`tern`, `tern-mcp`).
    pub fn connect(config_dir: &Path, client_name: &str, product: &str) -> Engines {
        let mut e = Engines {
            config_dir: config_dir.to_owned(),
            client_name: client_name.to_owned(),
            product: product.to_owned(),
            engines: Vec::new(),
            problems: Vec::new(),
        };
        e.reconnect();
        e
    }

    /// Rescans the profile and connects again, dropping the old connections: for a client
    /// that outlives an engine restart.
    pub fn reconnect(&mut self) {
        let scan = scan(&self.config_dir.join("run"));
        self.problems = scan
            .unreadable
            .iter()
            .map(|(f, e)| format!("{}: not a discovery file ({e})", f.display()))
            .collect();
        self.engines.clear();
        for (file, entry) in scan.live {
            let (pid, socket) = (entry.pid, entry.socket.clone());
            match Engine::connect(file.clone(), entry, &self.client_name, &self.product) {
                Ok(engine) => self.engines.push(engine),
                Err(e) => self.problems.push(format!(
                    "{}: engine pid {pid} at {}: {e}",
                    file.display(),
                    socket.display()
                )),
            }
        }
    }

    /// Every account, once, in engine order.
    pub fn accounts(&self) -> Vec<AccountRef> {
        let mut out: Vec<AccountRef> = Vec::new();
        for e in &self.engines {
            for a in &e.hello.accounts {
                if !out.iter().any(|o| o.account_id == a.account_id) {
                    out.push(AccountRef {
                        account_id: a.account_id.clone(),
                        email: a.email.clone(),
                    });
                }
            }
        }
        out
    }

    /// The engine that keeps the settings: the first one announcing
    /// [`feature::SETTINGS`], which only a `serve` engine does.
    pub fn settings_engine(&mut self) -> Option<&mut Engine> {
        self.engines
            .iter_mut()
            .find(|e| e.has_feature(feature::SETTINGS))
    }

    /// The engine holding `account_id`.
    pub fn engine_for(&mut self, account_id: &str) -> Option<&mut Engine> {
        self.engines.iter_mut().find(|e| e.holds(account_id))
    }

    /// Calls `method` for `account_id` on the engine holding it, with `accountId` set in
    /// `params`.
    pub fn call(
        &mut self,
        account_id: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Value, EngineError> {
        let Some(engine) = self.engine_for(account_id) else {
            return Err(EngineError::NoEngine(format!(
                "no running engine holds account {account_id}"
            )));
        };
        if let Value::Object(o) = &mut params {
            o.insert("accountId".into(), Value::from(account_id));
        }
        Ok(engine.client.call(method, params)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ms_ipc::AccountInfo;

    fn engine(accounts: &[(&str, &str)], features: &[&str], input: &str) -> Engine {
        Engine {
            file: PathBuf::from("/tmp/run/engine.json"),
            entry: Entry {
                pid: 1,
                socket: PathBuf::from("/tmp/x.sock"),
                protocol: 1,
                account_id: None,
                accounts: None,
            },
            hello: HelloResult {
                protocol: 1,
                engine: "0.1.0".into(),
                pid: 1,
                accounts: accounts
                    .iter()
                    .map(|(id, email)| AccountInfo {
                        account_id: (*id).into(),
                        email: (*email).into(),
                        state: "running".into(),
                        error: None,
                        ..Default::default()
                    })
                    .collect(),
                features: features.iter().map(|f| (*f).to_owned()).collect(),
                capabilities: None,
                ..Default::default()
            },
            client: Client::from_streams(
                Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
                Box::new(std::io::sink()),
            ),
        }
    }

    fn engines(list: Vec<Engine>) -> Engines {
        Engines {
            config_dir: PathBuf::from("/tmp"),
            client_name: "test".into(),
            product: "test".into(),
            engines: list,
            problems: Vec::new(),
        }
    }

    #[test]
    fn routes_by_account_and_dedups() {
        let reply = r#"{"jsonrpc":"2.0","id":1,"result":{"counts":{"f1":{"unread":1,"total":2}}}}"#;
        let mut e = engines(vec![
            engine(&[("a1", "ann@x.test")], &[ms_ipc::feature::COUNTS], "\n"),
            engine(
                &[("a1", "ann@x.test"), ("b1", "bob@x.test")],
                &[],
                &format!("{reply}\n"),
            ),
        ]);
        let ids: Vec<String> = e.accounts().into_iter().map(|a| a.account_id).collect();
        assert_eq!(ids, ["a1", "b1"]);
        assert!(e.engines[0].has_feature("counts") && !e.engines[1].has_feature("counts"));
        let r = e
            .call("b1", method::COUNTS_GET, serde_json::json!({}))
            .unwrap();
        assert_eq!(r["counts"]["f1"]["total"], 2);
        assert!(matches!(
            e.call("zz", method::COUNTS_GET, serde_json::json!({})),
            Err(EngineError::NoEngine(_))
        ));
    }

    #[test]
    fn typed_calls_and_unknown_methods() {
        let unknown = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"unknown method tasks.list"}}"#;
        let mut old = engine(&[("a1", "a@x.test")], &[], &format!("{unknown}\n"));
        let err = old.tasks_list(&TasksListParams::default()).unwrap_err();
        assert!(err.is_unknown_method(), "{err}");
        let ok = r#"{"jsonrpc":"2.0","id":1,"result":{"models":[{"id":"m1"}]}}"#;
        let mut new = engine(&[("a1", "a@x.test")], &[], &format!("{ok}\n"));
        let r = new
            .metadata_list(&MetadataListParams {
                plugin_id: "send-later".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(r.models.len(), 1);

        let page = r#"{"jsonrpc":"2.0","id":1,"result":{"rows":[{"id":"t1"}],"nextCursor":null}}"#;
        let mut e = engine(
            &[("a1", "a@x.test")],
            &[ms_ipc::feature::QUERY],
            &format!("{page}\n"),
        );
        let r = e
            .query(&QueryParams {
                class: "Thread".into(),
                cursor: Some(String::new()),
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!((r.rows.len(), r.next_cursor), (1, Some(None)));
    }

    #[test]
    fn mail_action_calls() {
        let replies = [
            r#"{"jsonrpc":"2.0","id":1,"result":{"taskIds":["t1","t2"]}}"#,
            r#"{"jsonrpc":"2.0","id":2,"result":{"taskIds":["t3"]}}"#,
            r#"{"jsonrpc":"2.0","id":3,"result":{"method":"web","ok":true,"url":"https://x.test/u"}}"#,
        ];
        let mut e = engine(
            &[("a1", "a@x.test")],
            &[ms_ipc::feature::SNOOZE, ms_ipc::feature::UNSUBSCRIBE],
            &format!("{}\n", replies.join("\n")),
        );
        let ids = vec!["th1".to_owned()];
        assert_eq!(
            e.threads_snooze("a1", &ids, 2_000_000_000)
                .unwrap()
                .task_ids,
            ["t1", "t2"]
        );
        assert_eq!(e.threads_unsnooze("a1", &ids).unwrap().task_ids, ["t3"]);
        let r = e.messages_unsubscribe("a1", "m1", Some("web")).unwrap();
        assert_eq!(
            (r.method.as_str(), r.ok, r.url.as_deref()),
            ("web", true, Some("https://x.test/u"))
        );
    }

    #[test]
    fn no_engines_under_an_empty_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let e = Engines::connect(tmp.path(), "test/1", "test");
        assert!(e.engines.is_empty() && e.problems.is_empty());

        // A live pid whose socket isn't there is reported, not connected.
        std::fs::create_dir(tmp.path().join("run")).unwrap();
        let entry = serde_json::json!({"pid": std::process::id(),
                                       "socket": tmp.path().join("gone.sock")});
        std::fs::write(tmp.path().join("run").join("x.json"), entry.to_string()).unwrap();
        let e = Engines::connect(tmp.path(), "test/1", "test");
        assert!(e.engines.is_empty());
        assert!(e.problems[0].contains("gone.sock"), "{:?}", e.problems);
    }
}
