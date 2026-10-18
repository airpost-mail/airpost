// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! An in-memory engine for the tool tests: the read methods over seeded JSON, and a
//! `task.queue` that records tasks and applies a draft save's local phase the way the
//! engine does (id from the account and Message-ID, stored keys kept, body replaced).

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::discovery::{AccountRef, Backend, EngineError};
use crate::model::{array_of, str_of};

#[derive(Default)]
pub struct Fake {
    pub accounts: Vec<AccountRef>,
    pub folders: Vec<Value>,
    pub labels: Vec<Value>,
    pub threads: Vec<Value>,
    pub messages: Vec<Value>,
    pub bodies: HashMap<String, String>,
    /// Bodies `bodies.fetch` downloads: the body, and the keys the engine sets on the
    /// message at the same time (`plaintext`, `files`).
    pub remote_bodies: HashMap<String, (String, Value)>,
    pub queued: Vec<Value>,
    pub tasks: HashMap<String, Value>,
    /// What `hello.features` would list. `tasks.list` and `metadata.list` answer only when
    /// their feature is here, as an older engine doesn't know them.
    pub features: Vec<String>,
    /// The engine's settings store, answered when `features` has `settings`.
    pub settings: serde_json::Map<String, Value>,
    /// The params of each grammar `search` call.
    pub searches: Vec<Value>,
    /// `threads.snooze`, `threads.unsnooze` and `messages.unsubscribe` calls: method, params.
    pub actions: Vec<(String, Value)>,
}

fn not_found(what: &str) -> EngineError {
    EngineError::Rpc {
        code: ms_ipc::code::NOT_FOUND,
        message: format!("no {what}"),
        data: None,
    }
}

impl Fake {
    pub fn account(id: &str, email: &str) -> AccountRef {
        AccountRef {
            account_id: id.into(),
            email: email.into(),
        }
    }

    fn of_account<'a>(list: &'a [Value], account: &str) -> impl Iterator<Item = &'a Value> + 'a {
        let account = account.to_owned();
        list.iter().filter(move |v| str_of(v, "aid") == account)
    }

    fn save_draft(&mut self, account: &str, task: &mut Value) {
        let draft = &mut task["draft"];
        let hmid = str_of(draft, "hMsgId").to_owned();
        let id = match draft.get("id").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => super::write::draft_id(account, &hmid),
        };
        draft["id"] = Value::from(id.as_str());
        if let Some(b) = draft.get("body").and_then(Value::as_str) {
            self.bodies.insert(id.clone(), b.to_owned());
        }
        let mut stored = draft.clone();
        stored.as_object_mut().unwrap().remove("body");
        match self.messages.iter_mut().find(|m| str_of(m, "id") == id) {
            Some(existing) => {
                for (k, v) in stored.as_object().unwrap() {
                    existing[k] = v.clone();
                }
            }
            None => self.messages.push(stored),
        }
    }
}

impl Backend for Fake {
    fn accounts(&self) -> Vec<AccountRef> {
        self.accounts.clone()
    }

    fn has_feature(&self, _account_id: &str, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    fn settings(&mut self, keys: &[&str]) -> Option<serde_json::Map<String, Value>> {
        if !self.features.iter().any(|f| f == ms_ipc::feature::SETTINGS) {
            return None;
        }
        Some(
            self.settings
                .iter()
                .filter(|(k, _)| keys.contains(&k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }

    fn call(&mut self, account: &str, method: &str, params: Value) -> Result<Value, EngineError> {
        let id = str_of(&params, "id").to_owned();
        match method {
            "folders.list" => Ok(
                json!({"folders": Self::of_account(&self.folders, account).collect::<Vec<_>>(), "counts": {}}),
            ),
            "labels.list" => Ok(
                json!({"labels": Self::of_account(&self.labels, account).collect::<Vec<_>>(), "counts": {}}),
            ),
            "threads.list" => {
                let cat = params
                    .get("folderId")
                    .or_else(|| params.get("labelId"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| {
                        let role = params.get("role")?.as_str()?;
                        self.folders
                            .iter()
                            .chain(&self.labels)
                            .find(|c| str_of(c, "aid") == account && str_of(c, "role") == role)
                            .map(|c| str_of(c, "id").to_owned())
                    });
                if params.get("role").is_some() && cat.is_none() {
                    return Err(not_found("role"));
                }
                let mut threads: Vec<Value> = Self::of_account(&self.threads, account)
                    .filter(|t| {
                        cat.as_deref()
                            .is_none_or(|c| crate::model::thread_category_ids(t).any(|id| id == c))
                    })
                    .cloned()
                    .collect();
                threads.sort_by_key(|t| std::cmp::Reverse(crate::model::i64_of(t, "lmt")));
                Ok(json!({"threads": threads, "next": null}))
            }
            "threads.get" => {
                let thread = Self::of_account(&self.threads, account)
                    .find(|t| str_of(t, "id") == id)
                    .cloned()
                    .ok_or_else(|| not_found("thread"))?;
                let mut messages: Vec<Value> = Self::of_account(&self.messages, account)
                    .filter(|m| str_of(m, "threadId") == id)
                    .cloned()
                    .collect();
                messages.sort_by_key(|m| crate::model::i64_of(m, "date"));
                Ok(json!({"thread": thread, "messages": messages}))
            }
            "messages.get" => {
                let message = Self::of_account(&self.messages, account)
                    .find(|m| str_of(m, "id") == id)
                    .cloned()
                    .ok_or_else(|| not_found("message"))?;
                let body = if params["body"] == true {
                    self.bodies.get(&id).cloned()
                } else {
                    None
                };
                let files = array_of(&message, "files").to_vec();
                Ok(json!({"message": message, "body": body, "files": files}))
            }
            "search" if params["grammar"] == json!(true) => {
                // The engine parses the grammar; here every thread of the account matches,
                // and `(x` doesn't parse.
                self.searches.push(params.clone());
                if str_of(&params, "query") == "(x" {
                    return Err(EngineError::Rpc {
                        code: ms_ipc::code::INVALID_PARAMS,
                        message: "invalid params: search: a '(' is never closed".into(),
                        data: None,
                    });
                }
                let threads: Vec<Value> =
                    Self::of_account(&self.threads, account).cloned().collect();
                Ok(json!({"threads": threads}))
            }
            "search" => {
                let q = str_of(&params, "query").to_lowercase();
                let threads: Vec<Value> = Self::of_account(&self.threads, account)
                    .filter(|t| {
                        q.split_whitespace()
                            .all(|w| str_of(t, "subject").to_lowercase().contains(w))
                    })
                    .cloned()
                    .collect();
                Ok(json!({"threads": threads}))
            }
            "bodies.fetch" => {
                for id in array_of(&params, "ids") {
                    let id = id.as_str().unwrap_or_default();
                    let Some((body, fields)) = self.remote_bodies.remove(id) else {
                        continue;
                    };
                    self.bodies.insert(id.to_owned(), body);
                    if let Some(m) = self.messages.iter_mut().find(|m| str_of(m, "id") == id) {
                        for (k, v) in fields.as_object().into_iter().flatten() {
                            m[k] = v.clone();
                        }
                    }
                }
                Ok(json!({}))
            }
            "tasks.get" => self
                .tasks
                .get(&id)
                .map(|t| json!({"task": t}))
                .ok_or_else(|| not_found("task")),
            "task.queue" => {
                let mut task = params["task"].clone();
                if str_of(&task, "__cls") == "SyncbackDraftTask" {
                    self.save_draft(account, &mut task);
                }
                task["status"] = Value::from("remote");
                self.tasks
                    .insert(str_of(&task, "id").to_owned(), task.clone());
                self.queued.push(task.clone());
                Ok(task)
            }
            "tasks.list" if self.has_feature(account, ms_ipc::feature::TASKS_LIST) => {
                let statuses = array_of(&params, "status").to_vec();
                let hmid = params.get("headerMessageId").and_then(Value::as_str);
                let mut tasks: Vec<Value> = self
                    .tasks
                    .values()
                    .filter(|t| str_of(t, "aid") == account)
                    .filter(|t| statuses.is_empty() || statuses.contains(&t["status"]))
                    .filter(|t| {
                        params
                            .get("class")
                            .and_then(Value::as_str)
                            .is_none_or(|c| str_of(t, "__cls") == c)
                    })
                    .filter(|t| {
                        hmid.is_none_or(|h| {
                            str_of(t, "headerMessageId") == h || str_of(&t["draft"], "hMsgId") == h
                        })
                    })
                    .cloned()
                    .collect();
                tasks.sort_by(|a, b| str_of(a, "id").cmp(str_of(b, "id")));
                Ok(json!({"tasks": tasks}))
            }
            "metadata.list" if self.has_feature(account, ms_ipc::feature::METADATA_LIST) => {
                let plugin = str_of(&params, "pluginId");
                let num = |k: &str| params.get(k).and_then(Value::as_i64);
                let mut models: Vec<Value> = Self::of_account(&self.messages, account)
                    .filter(|m| {
                        let Some(meta) = array_of(m, "metadata")
                            .iter()
                            .find(|x| str_of(x, "pluginId") == plugin)
                        else {
                            return false;
                        };
                        let date = crate::model::i64_of(m, "date");
                        let exp = meta["value"].get("expiration").and_then(Value::as_i64);
                        params
                            .get("draft")
                            .and_then(Value::as_bool)
                            .is_none_or(|d| crate::model::is_draft(m) == d)
                            && params
                                .get("hasExpiration")
                                .and_then(Value::as_bool)
                                .is_none_or(|h| exp.is_some() == h)
                            && num("after").is_none_or(|a| date > a)
                            && num("before").is_none_or(|b| date < b)
                    })
                    .cloned()
                    .collect();
                models.sort_by(|a, b| {
                    crate::model::i64_of(b, "date")
                        .cmp(&crate::model::i64_of(a, "date"))
                        .then_with(|| str_of(b, "id").cmp(str_of(a, "id")))
                });
                let offset = num("offset").unwrap_or(0) as usize;
                let limit = num("limit").unwrap_or(50) as usize;
                let models: Vec<Value> = models.into_iter().skip(offset).take(limit).collect();
                Ok(json!({"models": models}))
            }
            "threads.snooze" | "threads.unsnooze" => {
                self.actions.push((method.to_owned(), params));
                Ok(json!({"taskIds": ["task-move", "task-meta"]}))
            }
            "messages.unsubscribe" => {
                self.actions.push((method.to_owned(), params.clone()));
                let message = Self::of_account(&self.messages, account)
                    .find(|m| str_of(m, "id") == str_of(&params, "messageId"))
                    .cloned()
                    .ok_or_else(|| not_found("message"))?;
                let list = str_of(&message, "hListUnsub");
                if list.is_empty() {
                    return Err(not_found("List-Unsubscribe header"));
                }
                let url = list.trim_matches(|c| c == '<' || c == '>');
                Ok(json!({"method": "web", "ok": true, "url": url}))
            }
            other => Err(EngineError::Rpc {
                code: ms_ipc::code::METHOD_NOT_FOUND,
                message: format!("unknown method {other}"),
                data: None,
            }),
        }
    }
}
