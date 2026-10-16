// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `Task`: a client action queued over stdin. Derived from Mailspring-Sync
//! `MailSync/Models/Task.*`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use ms_core::Result;
use rusqlite::Statement;
use serde_json::{json, Value};

use crate::model::{base_json, bind_base, bind_named, json_bool, json_str, object_or_err};
use crate::Model;

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub data: Value,
}

impl Task {
    /// `Task(constructorName, accountId, taskSpecificData)`: a task the engine queues
    /// itself, with a random id and status `local`.
    pub fn create(class: &str, account_id: &str, task_specific_data: &Value) -> Self {
        let mut data = base_json(&ms_core::ids::random_id(), account_id, 0);
        data["__cls"] = json!(class);
        data["status"] = json!("local");
        if let (Some(obj), Some(extra)) = (data.as_object_mut(), task_specific_data.as_object()) {
            for (k, v) in extra {
                obj.insert(k.clone(), v.clone());
            }
        }
        Task { data }
    }

    /// `__cls`, e.g. `SendDraftTask`.
    pub fn class(&self) -> &str {
        json_str(&self.data, "__cls")
    }

    /// `local`, `remote`, `complete` or `cancelled`.
    pub fn status(&self) -> &str {
        json_str(&self.data, "status")
    }

    pub fn set_status(&mut self, s: &str) {
        self.data["status"] = json!(s);
    }

    pub fn should_cancel(&self) -> bool {
        json_bool(&self.data, "should_cancel")
    }

    pub fn set_should_cancel(&mut self) {
        self.data["should_cancel"] = json!(true);
    }

    pub fn error(&self) -> &Value {
        &self.data["error"]
    }

    pub fn set_error(&mut self, e: Value) {
        self.data["error"] = e;
    }
}

impl Model for Task {
    model_boilerplate!("Task", ["id", "data", "accountId", "version", "status"]);

    fn from_json(data: Value) -> Result<Self> {
        object_or_err(&data, "Task")?;
        Ok(Task { data })
    }

    fn bind(&self, stmt: &mut Statement<'_>) -> Result<()> {
        bind_base(self, stmt)?;
        bind_named(stmt, ":status", self.status())?;
        Ok(())
    }
}
