// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The draft pipeline on the socket (docs/desktop-client-plan.md 3.3, stage 13,
//! `feature::DRAFT_PIPELINE`, `serve` only): `draft.create`, `draft.send`, `draft.cancelSend`
//! and `draft.validate`.
//!
//! The connection's half reads what every account shares (the settings, and every account's
//! addresses, which decide who "me" is in a reply) and hands the call to the account's `main`
//! thread, whose half reads and writes the store: drafts are saved with `SyncbackDraftTask`,
//! sends queued as `SendDraftTask` and undo windows written as send-later metadata, through
//! the same local phases a client's tasks take. The builders are `ms_compose`'s.

use ms_compose::draft::{self, Ctx};
use ms_compose::settings::local_tz;
use ms_compose::{ComposeSettings, Identities};
use ms_core::Account;
use ms_ipc::{
    code, draft_kind, method, DraftCancelSendParams, DraftCancelSendResult, DraftCreateParams,
    DraftSendParams, DraftSendResult, DraftValidateParams, ErrorObject, Request as RpcRequest,
};
use ms_store::{Message, Model, Query, Store, Task, Thread};
use ms_sync::expiration_executor::{
    archive_task, finalise_draft, send_later_expiration, send_task_json, stored_body, SEND_ACTIONS,
    SEND_LATER, SNOOZE_BODY_EN, SNOOZE_BODY_KEY,
};
use ms_sync::pending_sends::{recipient_mismatch, PendingBodies, PendingSends};
use serde_json::{json, Map, Value};
use tracing::info;

use crate::dispatch::{Reply, Request};
use crate::socket::Session;

/// What `draft.create` needs from outside the account.
#[derive(Debug)]
pub struct CreateCall {
    pub params: DraftCreateParams,
    pub settings: ComposeSettings,
    pub accounts: Identities,
    /// `resurfacing.snoozeBody`, for `resurface` without a body.
    pub resurface_body: Option<String>,
}

pub fn is_draft_method(name: &str) -> bool {
    matches!(
        name,
        method::DRAFT_CREATE
            | method::DRAFT_SEND
            | method::DRAFT_CANCEL_SEND
            | method::DRAFT_VALIDATE
    )
}

fn invalid(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(
        code::INVALID_PARAMS,
        format!("invalid params: {}", message.into()),
    )
}

fn not_found(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(code::NOT_FOUND, message.into())
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

// -- the connection's half ------------------------------------------------------------------

fn identities(session: &Session) -> Identities {
    let statuses = session.server.accounts.statuses();
    Identities::from_accounts_json(statuses.iter().filter_map(|s| s.account.as_ref()))
}

fn compose_settings(
    session: &Session,
    tz: Option<&str>,
) -> Result<(ComposeSettings, Map<String, Value>), ErrorObject> {
    let tz = match tz {
        Some(name) => name
            .parse::<chrono_tz::Tz>()
            .map_err(|_| invalid(format!("tz {name:?} is not an IANA time zone")))?,
        None => local_tz(),
    };
    let values = match &session.server.settings {
        Some(s) => s.lock().get(None).unwrap_or_default(),
        None => Map::new(),
    };
    Ok((
        ComposeSettings::from_values(|k| values.get(k).cloned(), tz),
        values,
    ))
}

/// `draft.*` from a connection.
pub fn call(session: &Session, req: &RpcRequest) -> Reply {
    match req.method.as_str() {
        method::DRAFT_CREATE => {
            let p = req.params_as::<DraftCreateParams>()?;
            if !draft_kind::ALL.contains(&p.kind.as_str()) {
                return Err(invalid(format!(
                    "kind must be one of {}",
                    draft_kind::ALL.join(", ")
                )));
            }
            let needs_account = !matches!(p.kind.as_str(), draft_kind::NEW | draft_kind::MAILTO);
            let (settings, values) = compose_settings(session, p.tz.as_deref())?;
            let resurface_body = values
                .get(SNOOZE_BODY_KEY)
                .and_then(Value::as_str)
                .map(str::to_owned);
            let accounts = identities(session);
            let account_id = match &p.account_id {
                Some(id) => id.clone(),
                None if needs_account => {
                    return Err(invalid(format!(
                        "{} needs the accountId of the message",
                        p.kind
                    )))
                }
                None => {
                    let ctx = Ctx {
                        settings: &settings,
                        accounts: &accounts,
                        html: true,
                        now: 0,
                    };
                    draft::account_for_new_draft(&ctx)
                        .map(|i| i.account_id.clone())
                        .ok_or_else(|| not_found("there is no account to write from"))?
                }
            };
            session.server.accounts.check(&account_id)?;
            let mut params = p;
            params.account_id = Some(account_id.clone());
            session.dispatch(
                &account_id,
                Request::DraftCreate(Box::new(CreateCall {
                    params,
                    settings,
                    accounts,
                    resurface_body,
                })),
            )
        }
        method::DRAFT_SEND => {
            let p = req.params_as::<DraftSendParams>()?;
            session.server.accounts.check(&p.account_id)?;
            let after = p.after.as_deref().unwrap_or(SEND_ACTIONS[0]);
            if !SEND_ACTIONS.contains(&after) {
                return Err(invalid(format!(
                    "after must be one of {}",
                    SEND_ACTIONS.join(", ")
                )));
            }
            let id = p.account_id.clone();
            session.dispatch(&id, Request::DraftSend(p))
        }
        method::DRAFT_CANCEL_SEND => {
            let p = req.params_as::<DraftCancelSendParams>()?;
            session.server.accounts.check(&p.account_id)?;
            let id = p.account_id.clone();
            session.dispatch(&id, Request::DraftCancelSend(p))
        }
        method::DRAFT_VALIDATE => {
            let p = req.params_as::<DraftValidateParams>()?;
            session.server.accounts.check(&p.account_id)?;
            let id = p.account_id.clone();
            let accounts = identities(session);
            session.dispatch(&id, Request::DraftValidate(p, accounts))
        }
        _ => Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            "not a draft method",
        )),
    }
}

// -- the account's half ---------------------------------------------------------------------

/// The account's `main` thread: its store, account and task queue.
pub struct Here<'a> {
    pub store: &'a Store,
    pub account: &'a Account,
    pub config_dir: &'a std::path::Path,
    pub queue: &'a dyn QueueTask,
}

/// Runs a task's local phase and wakes the foreground worker (`Dispatcher::queue_task`).
pub trait QueueTask {
    fn queue_task(&self, task: Value) -> Reply;
}

impl Here<'_> {
    /// Runs `f` in one write transaction, rolled back when it fails. The expiration executor
    /// sends a due draft in one transaction on its own connection, so a send or a cancel
    /// decided here comes wholly before or after it.
    fn atomically<T>(
        &self,
        name: &str,
        f: impl FnOnce() -> Result<T, ErrorObject>,
    ) -> Result<T, ErrorObject> {
        let mut refused = None;
        let result = self.store.transaction(name, |_| {
            f().map_err(|e| {
                refused = Some(e);
                ms_core::Error::other("rolled back")
            })
        });
        match (result, refused) {
            (_, Some(e)) => Err(e),
            (Ok(v), None) => Ok(v),
            (Err(e), None) => Err(internal(e)),
        }
    }

    fn queue_task(&self, class: &str, data: &Value) -> Result<String, ErrorObject> {
        let task = Task::create(class, self.account.id(), data);
        let id = task.id().to_owned();
        self.queue.queue_task(task.data)?;
        Ok(id)
    }

    fn message(&self, id: &str) -> Result<Message, ErrorObject> {
        let m = self
            .store
            .find::<Message>(&Query::new().equal("id", id))
            .map_err(internal)?
            .filter(|m| m.account_id() == self.account.id())
            .ok_or_else(|| {
                not_found(format!("no message {id} in account {}", self.account.id()))
            })?;
        Ok(m)
    }

    /// The message's JSON with its body, which quoting needs.
    fn message_with_body(&self, id: &str) -> Result<Value, ErrorObject> {
        let m = self.message(id)?;
        let body = stored_body(self.store, id).map_err(internal)?.ok_or_else(|| {
            not_found(format!(
                "the body of message {id} isn't downloaded yet; fetch it (bodies.fetch) and try again"
            ))
        })?;
        let mut json = m.data.clone();
        json["body"] = json!(body);
        Ok(json)
    }

    fn draft(&self, header_message_id: &str) -> Result<Message, ErrorObject> {
        self.store
            .find::<Message>(
                &Query::new()
                    .equal("accountId", self.account.id())
                    .equal("headerMessageId", header_message_id)
                    .equal("draft", true),
            )
            .map_err(internal)?
            .ok_or_else(|| not_found(format!("no draft {header_message_id}")))
    }

    /// A `SendDraftTask` for the draft that hasn't finished.
    fn send_pending(&self, header_message_id: &str) -> Result<bool, ErrorObject> {
        let tasks = self
            .store
            .find_all::<Task>(
                &Query::new()
                    .equal("accountId", self.account.id())
                    .equal_any("status", ["local", "remote"]),
            )
            .map_err(internal)?;
        Ok(tasks.iter().any(|t| {
            t.class() == "SendDraftTask"
                && t.data.get("headerMessageId").and_then(Value::as_str) == Some(header_message_id)
        }))
    }

    /// `SyncbackMetadataTask.forSaving` on a draft.
    fn set_send_later(&self, draft: &Message, value: Value) -> Result<String, ErrorObject> {
        self.queue_task(
            "SyncbackMetadataTask",
            &json!({
                "modelId": draft.id(),
                "modelClassName": "message",
                "modelHeaderMessageId": draft.header_message_id(),
                "pluginId": SEND_LATER,
                "value": value,
            }),
        )
    }

    fn save_draft(&self, draft: &Value) -> Result<(), ErrorObject> {
        let hmid = draft.get("hMsgId").and_then(Value::as_str).unwrap_or("");
        self.queue_task(
            "SyncbackDraftTask",
            &json!({ "headerMessageId": hmid, "draft": draft }),
        )?;
        Ok(())
    }

    // -- draft.create --------------------------------------------------------------------

    pub fn create(&self, call: CreateCall) -> Reply {
        let CreateCall {
            params: p,
            settings,
            accounts,
            resurface_body,
        } = call;
        let ctx = Ctx {
            settings: &settings,
            accounts: &accounts,
            html: p.html.unwrap_or(settings.html),
            now: self.store.now(),
        };
        let fields = p.fields.clone().unwrap_or_default();
        let identity = accounts
            .get(self.account.id())
            .cloned()
            .unwrap_or_else(|| ms_compose::Identity::from_account_json(self.account.as_json()));
        let need = |v: &Option<String>, what: &str| {
            v.clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid(format!("{} needs {what}", p.kind)))
        };
        let mut built = match p.kind.as_str() {
            draft_kind::NEW => {
                let mut fields = fields;
                fields.insert("aid".into(), json!(self.account.id()));
                draft::new_draft(&ctx, &fields).map_err(invalid)?
            }
            draft_kind::MAILTO => {
                let url = need(&p.mailto, "mailto")?;
                let mut d = draft::mailto_draft(&ctx, Some(&identity), &url).map_err(invalid)?;
                for (k, v) in &fields {
                    if draft::FIELD_KEYS.contains(&k.as_str()) && k != "aid" {
                        d[k.as_str()] = v.clone();
                    }
                }
                d
            }
            draft_kind::REPLY | draft_kind::REPLY_ALL => {
                let message_id = need(&p.message_id, "messageId")?;
                let message = self.message_with_body(&message_id)?;
                let reply_all = p.kind == draft_kind::REPLY_ALL;
                if let Some(existing) = self.existing_reply(&message, p.behavior.as_deref())? {
                    let mut d = existing;
                    draft::update_reply_draft(&ctx, &mut d, &message, reply_all);
                    d
                } else {
                    draft::reply_draft(&ctx, &message, reply_all, &fields).map_err(invalid)?
                }
            }
            draft_kind::FORWARD => {
                let message_id = need(&p.message_id, "messageId")?;
                let message = self.message_with_body(&message_id)?;
                draft::forward_draft(&ctx, &message, &fields).map_err(invalid)?
            }
            draft_kind::RESURFACE => {
                let thread_id = need(&p.thread_id, "threadId")?;
                let body = p.body.or(resurface_body);
                self.resurface(&ctx, &identity, &thread_id, p.message_id.as_deref(), body)?
            }
            _ => return Err(invalid("unknown kind")),
        };
        let hmid = built
            .get("hMsgId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        // An existing reply draft keeps its id: one synced from the server or another device
        // has a message id, not `draft_id`, and a new id would save it as a second draft.
        if built
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            built["id"] = json!(ms_core::ids::draft_id(self.account.id(), &hmid));
        }
        if p.save != Some(false) {
            self.save_draft(&built)?;
        }
        Ok(built)
    }

    /// `DraftFactory.candidateDraftForUpdating`: the newest draft replying to the message,
    /// when `behavior` asks for one, with its body.
    fn existing_reply(
        &self,
        message: &Value,
        behavior: Option<&str>,
    ) -> Result<Option<Value>, ErrorObject> {
        let pristine_only = match behavior {
            Some("prefer-existing") => false,
            Some("prefer-existing-if-pristine") => true,
            None => return Ok(None),
            Some(other) => {
                return Err(invalid(format!(
                    "behavior {other:?} must be prefer-existing or prefer-existing-if-pristine"
                )))
            }
        };
        let hmid = message.get("hMsgId").and_then(Value::as_str).unwrap_or("");
        let thread = message
            .get("threadId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let drafts = self
            .store
            .find_all::<Message>(
                &Query::new()
                    .equal("accountId", self.account.id())
                    .equal("threadId", thread)
                    .equal("draft", true)
                    .order_by("date", true),
            )
            .map_err(internal)?;
        // `rthMsgId` lives in the JSON only; the replyToHeaderMessageId column isn't written.
        let chosen = drafts.into_iter().rev().find(|d| {
            d.reply_to_header_message_id() == hmid
                && (!pristine_only || d.data.get("pristine").and_then(Value::as_bool) == Some(true))
        });
        let Some(d) = chosen else { return Ok(None) };
        let mut json = d.data.clone();
        json["body"] = json!(stored_body(self.store, d.id())
            .map_err(internal)?
            .unwrap_or_default());
        Ok(Some(json))
    }

    fn resurface(
        &self,
        ctx: &Ctx,
        identity: &ms_compose::Identity,
        thread_id: &str,
        message_id: Option<&str>,
        body: Option<String>,
    ) -> Result<Value, ErrorObject> {
        let thread = self
            .store
            .find::<Thread>(&Query::new().equal("id", thread_id))
            .map_err(internal)?
            .filter(|t| t.account_id() == self.account.id())
            .ok_or_else(|| not_found(format!("no thread {thread_id}")))?;
        let reply_to = match message_id {
            Some(id) => self.message(id)?.header_message_id().to_owned(),
            None => self
                .store
                .find::<Message>(
                    &Query::new()
                        .equal("threadId", thread_id)
                        .equal("draft", false)
                        .order_by("date", false)
                        .limit(1),
                )
                .map_err(internal)?
                .map(|m| m.header_message_id().to_owned())
                .unwrap_or_default(),
        };
        let body = body
            .filter(|b| !b.trim().is_empty())
            .unwrap_or_else(|| SNOOZE_BODY_EN.to_owned());
        Ok(draft::resurface_draft(
            identity,
            thread.id(),
            thread.subject(),
            &reply_to,
            &body,
            ctx.now,
        ))
    }

    // -- draft.send / draft.cancelSend ---------------------------------------------------

    pub fn send(&self, p: DraftSendParams) -> Reply {
        let pending = PendingSends::new(self.config_dir, self.account.id());
        let (reply, draft_id) = self.atomically("draft.send", || self.send_in_transaction(p))?;
        if let Some(id) = draft_id {
            // Sent now: any bodies kept for an earlier undo window are spent.
            pending.remove(&id);
        }
        Ok(reply)
    }

    /// `draft.send`'s reply, and the draft's id when it was queued to send now.
    fn send_in_transaction(
        &self,
        p: DraftSendParams,
    ) -> Result<(Value, Option<String>), ErrorObject> {
        let draft = self.draft(&p.header_message_id)?;
        if self.send_pending(&p.header_message_id)? {
            return Err(invalid(format!(
                "draft {} is being sent already",
                p.header_message_id
            )));
        }
        let after = p.after.as_deref().unwrap_or(SEND_ACTIONS[0]).to_owned();
        let bodies = p.bodies.unwrap_or_default();
        if let Some(why) = recipient_mismatch(&draft.data, &bodies.per_recipient) {
            return Err(invalid(why));
        }
        let pending = PendingSends::new(self.config_dir, self.account.id());
        let delay = p.delay_ms.unwrap_or(0);
        if delay > 0 {
            // Finalisation runs when it fires; check now that it will go.
            let body = stored_body(self.store, draft.id())
                .map_err(internal)?
                .unwrap_or_default();
            finalise_draft(self.account, &draft, &body).map_err(|why| {
                ErrorObject::new(code::TASK_REJECTED, why.clone())
                    .with_data(json!({ "message": why }))
            })?;
            let expiration = self.store.now() + delay.div_ceil(1000) as i64;
            self.set_send_later(
                &draft,
                json!({ "expiration": expiration, "isUndoSend": true, "actionKey": after }),
            )?;
            // Inside the transaction, so the executor can't fire before the file is there.
            if bodies.is_empty() {
                pending.remove(draft.id());
            } else {
                pending
                    .put(
                        draft.id(),
                        &PendingBodies {
                            expiration,
                            self_body: bodies.self_body.clone(),
                            per_recipient: bodies.per_recipient.clone(),
                        },
                    )
                    .map_err(|e| internal(format!("could not keep the send's bodies: {e}")))?;
            }
            info!(
                "-- Sending draft {} at {expiration} ({after})",
                p.header_message_id
            );
            let reply = to_value(DraftSendResult {
                task_id: None,
                scheduled_at: Some(expiration),
            })?;
            return Ok((reply, None));
        }

        let body = match &bodies.self_body {
            Some(b) => b.clone(),
            None => stored_body(self.store, draft.id()).map_err(internal)?.ok_or_else(|| {
                not_found(format!(
                    "the body of draft {} isn't downloaded yet; fetch it (bodies.fetch) and try again",
                    p.header_message_id
                ))
            })?,
        };
        let finalised = finalise_draft(self.account, &draft, &body).map_err(|why| {
            ErrorObject::new(code::TASK_REJECTED, why.clone()).with_data(json!({ "message": why }))
        })?;
        // A send-later already on the draft goes, so the executor doesn't send it again.
        if send_later_expiration(&draft).is_some() {
            self.set_send_later(&draft, json!({ "expiration": null }))?;
        }
        let draft = self.draft(&p.header_message_id)?;
        let mut data = send_task_json(&draft, &finalised, false);
        if !bodies.per_recipient.is_empty() {
            let per = PendingBodies {
                expiration: 0,
                self_body: None,
                per_recipient: bodies.per_recipient.clone(),
            };
            data["perRecipientBodies"] = per.per_recipient_json(&finalised.body);
        }
        let task_id = self.queue_task("SendDraftTask", &data)?;
        if after == "send-and-archive" {
            if let Some((class, data)) =
                archive_task(self.store, self.account.id(), draft.thread_id()).map_err(internal)?
            {
                self.queue_task(class, &data)?;
            }
        }
        let reply = to_value(DraftSendResult {
            task_id: Some(task_id),
            scheduled_at: None,
        })?;
        Ok((reply, Some(draft.id().to_owned())))
    }

    pub fn cancel_send(&self, p: DraftCancelSendParams) -> Reply {
        // Decided in one transaction: once the executor has queued the SendDraftTask the send
        // can't be called back, and until then it can't queue one.
        let cancelled = self.atomically("draft.cancelSend", || {
            let draft = match self.draft(&p.header_message_id) {
                Ok(d) => d,
                // Sent already: the draft is gone.
                Err(e) if e.code == code::NOT_FOUND => return Ok(None),
                Err(e) => return Err(e),
            };
            if send_later_expiration(&draft).is_none() || self.send_pending(&p.header_message_id)? {
                return Ok(None);
            }
            self.set_send_later(&draft, json!({ "expiration": null, "isUndoSend": true }))?;
            Ok(Some(draft.id().to_owned()))
        })?;
        if let Some(id) = &cancelled {
            PendingSends::new(self.config_dir, self.account.id()).remove(id);
            info!("-- Cancelled the send of draft {}", p.header_message_id);
        }
        to_value(DraftCancelSendResult {
            cancelled: cancelled.is_some(),
        })
    }

    // -- draft.validate ------------------------------------------------------------------

    pub fn validate(&self, p: DraftValidateParams, accounts: Identities) -> Reply {
        let draft = self.draft(&p.header_message_id)?;
        let mut json = draft.data.clone();
        json["body"] = json!(stored_body(self.store, draft.id())
            .map_err(internal)?
            .unwrap_or_default());
        Ok(ms_compose::validate::validate(&json, &accounts, &p.skip_salutation_for).to_json())
    }
}

fn to_value<T: serde::Serialize>(v: T) -> Reply {
    serde_json::to_value(v).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ms_store::{DeltaStream, Folder};
    use ms_sync::expiration_executor::{EngineSettings, Executor};
    use ms_tasks::TaskProcessor;
    use std::path::Path;
    use std::sync::{Arc, Barrier};

    const ACCOUNT: &str = "acct1";

    fn account() -> Arc<Account> {
        Arc::new(
            Account::from_json(json!({
                "id": ACCOUNT, "provider": "imap", "emailAddress": "me@example.com",
                "settings": {},
            }))
            .unwrap(),
        )
    }

    fn open(dir: &Path) -> Store {
        Store::open(dir, DeltaStream::new(Box::new(std::io::sink()))).unwrap()
    }

    fn setup() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let clock = ms_core::ManualClock::new(1_700_000_000);
        ms_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let store = open(dir.path());
        let mut drafts = Folder::new(&ms_core::ids::folder_id(ACCOUNT, "Drafts"), ACCOUNT, 0);
        drafts.set_path("Drafts");
        drafts.set_role("drafts");
        store.save(&mut drafts).unwrap();
        (dir, store)
    }

    /// The local phases, as the account's main thread runs them.
    struct Local<'a>(TaskProcessor<'a>);

    impl QueueTask for Local<'_> {
        fn queue_task(&self, mut json: Value) -> Reply {
            json["v"] = json!(0);
            let mut task = Task::from_json(json).map_err(internal)?;
            self.0.perform_local(&mut task).map_err(internal)?;
            if task.error().is_null() {
                Ok(task.data)
            } else {
                Err(ErrorObject::new(code::TASK_REJECTED, "rejected")
                    .with_data(task.error().clone()))
            }
        }
    }

    fn with_here<T>(dir: &Path, store: &Store, f: impl FnOnce(&Here) -> T) -> T {
        let account = account();
        let local = Local(TaskProcessor::new(account.clone(), store).with_config_dir(dir));
        f(&Here {
            store,
            account: &account,
            config_dir: dir,
            queue: &local,
        })
    }

    fn save_draft(dir: &Path, store: &Store, draft: Value) {
        with_here(dir, store, |h| {
            h.queue_task(
                "SyncbackDraftTask",
                &json!({"headerMessageId": draft["hMsgId"], "draft": draft}),
            )
            .unwrap()
        });
    }

    fn draft(hmid: &str) -> Value {
        json!({
            "__cls": "Message", "aid": ACCOUNT, "v": 0, "hMsgId": hmid, "subject": "Hi",
            "body": "<p>Hi</p>", "plaintext": false,
            "from": [{"name": "Me", "email": "me@example.com"}],
            "to": [{"email": "ann@example.net"}, {"email": "bob@example.net"}],
            "cc": [], "bcc": [], "replyTo": [], "files": [],
        })
    }

    fn drafts_with(store: &Store, hmid: &str) -> Vec<Message> {
        store
            .find_all::<Message>(
                &Query::new()
                    .equal("headerMessageId", hmid)
                    .equal("draft", true),
            )
            .unwrap()
    }

    fn send_tasks(store: &Store, hmid: &str) -> usize {
        store
            .find_all::<Task>(&Query::new().equal("accountId", ACCOUNT))
            .unwrap()
            .iter()
            .filter(|t| t.class() == "SendDraftTask" && t.data["headerMessageId"] == hmid)
            .count()
    }

    fn params<T: serde::de::DeserializeOwned>(v: Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn prefer_existing_keeps_the_id_of_a_reply_draft_synced_from_the_server() {
        let (dir, store) = setup();
        let mut received = Message::new(json!({
            "__cls": "Message", "id": "m1", "aid": ACCOUNT, "v": 0, "hMsgId": "orig@example.net",
            "threadId": "t1", "draft": false, "subject": "Plans", "date": 1_700_000_000,
            "from": [{"name": "Bob", "email": "bob@example.net"}],
            "to": [{"email": "me@example.com"}], "cc": [], "bcc": [], "replyTo": [],
            "files": [], "folders": {}, "labels": [],
        }));
        store.save(&mut received).unwrap();
        store
            .conn()
            .execute(
                "REPLACE INTO MessageBody (id, value) VALUES ('m1', '<p>Lunch?</p>')",
                [],
            )
            .unwrap();
        // A message id, as the Drafts folder's sync gives a draft it found on the server.
        let mut synced = draft("reply-1@example.com");
        synced["id"] = json!("synced-draft-id");
        synced["threadId"] = json!("t1");
        synced["rthMsgId"] = json!("orig@example.net");
        synced["date"] = json!(1_700_000_100);
        save_draft(dir.path(), &store, synced);

        let built = with_here(dir.path(), &store, |h| {
            h.create(CreateCall {
                params: params(json!({
                    "accountId": ACCOUNT, "kind": "reply", "messageId": "m1",
                    "behavior": "prefer-existing",
                })),
                settings: ComposeSettings::default(),
                accounts: Identities::from_accounts_json([account().as_json()]),
                resurface_body: None,
            })
        })
        .unwrap();
        assert_eq!(built["id"], "synced-draft-id");
        let drafts = drafts_with(&store, "reply-1@example.com");
        assert_eq!(drafts.len(), 1, "saved in place, not as a second draft");
        assert_eq!(drafts[0].id(), "synced-draft-id");
    }

    #[test]
    fn send_refuses_per_recipient_bodies_that_leave_out_a_recipient() {
        let (dir, store) = setup();
        save_draft(dir.path(), &store, draft("per-1@example.com"));
        for delay in [None, Some(4000)] {
            let err = with_here(dir.path(), &store, |h| {
                h.send(params(json!({
                    "accountId": ACCOUNT, "headerMessageId": "per-1@example.com",
                    "delayMs": delay,
                    "bodies": {"self": "<p>x</p>", "perRecipient": {"ann@example.net": "<p>a</p>"}},
                })))
            })
            .unwrap_err();
            assert_eq!(err.code, code::INVALID_PARAMS);
            assert!(err.message.contains("bob@example.net"), "{}", err.message);
        }
        assert_eq!(send_tasks(&store, "per-1@example.com"), 0);
        let d = &drafts_with(&store, "per-1@example.com")[0];
        assert_eq!(send_later_expiration(d), None);
        assert!(PendingSends::new(dir.path(), ACCOUNT).ids().is_empty());
    }

    struct NoSettings;

    impl EngineSettings for NoSettings {
        fn get(&self, _: &str) -> Option<Value> {
            None
        }
    }

    /// draft.cancelSend on the main thread and the executor on the metadataExpiration thread,
    /// each on its own connection, racing at the end of the undo window: the cancel wins and
    /// nothing is queued, or the send is queued and the cancel says it was too late.
    #[test]
    fn cancel_send_and_the_executor_never_both_win() {
        let (dir, store) = setup();
        let dir = dir.path();
        for i in 0..45 {
            let hmid = format!("race-{i}@example.com");
            save_draft(dir, &store, draft(&hmid));
            let scheduled = with_here(dir, &store, |h| {
                h.send(params(json!({
                    "accountId": ACCOUNT, "headerMessageId": hmid, "delayMs": 1000,
                    "bodies": {"self": "<p>transformed</p>"},
                })))
            })
            .unwrap();
            let due = scheduled["scheduledAt"].as_i64().unwrap();
            let id = drafts_with(&store, &hmid)[0].id().to_owned();
            // Each order in turn, then the two at once.
            let order = i % 3;
            let (executor_first, cancel_first) = (order == 0, order == 1);
            let barrier = Barrier::new(2);
            let cancelled = std::thread::scope(|s| {
                s.spawn(|| {
                    let store = open(dir);
                    let account = account();
                    let processor = TaskProcessor::new(account.clone(), &store);
                    let executor = Executor::new(
                        account,
                        &store,
                        dir,
                        Arc::new(NoSettings),
                        Box::new(move |task| processor.perform_local(task)),
                        Box::new(|_| {}),
                        Box::new(|| {}),
                    );
                    if !executor_first {
                        barrier.wait();
                    }
                    executor.handle("Message", &id, SEND_LATER, due).unwrap();
                    if executor_first {
                        barrier.wait();
                    }
                });
                let cancel = s.spawn(|| {
                    let store = open(dir);
                    if !cancel_first {
                        barrier.wait();
                    }
                    let cancelled = with_here(dir, &store, |h| {
                        h.cancel_send(params(json!({
                            "accountId": ACCOUNT, "headerMessageId": hmid,
                        })))
                    })
                    .unwrap()["cancelled"]
                        .as_bool()
                        .unwrap();
                    if cancel_first {
                        barrier.wait();
                    }
                    cancelled
                });
                cancel.join().unwrap()
            });
            let sends = send_tasks(&store, &hmid);
            let d = &drafts_with(&store, &hmid)[0];
            assert!(!(executor_first && cancelled), "round {i}: sent first");
            assert!(!cancel_first || cancelled, "round {i}: cancelled first");
            if cancelled {
                assert_eq!(sends, 0, "round {i}: cancelled, yet queued to send");
                assert_eq!(send_later_expiration(d), None, "round {i}");
            } else {
                assert_eq!(sends, 1, "round {i}: not cancelled, yet not sent");
                let task = store
                    .find_all::<Task>(&Query::new().equal("accountId", ACCOUNT))
                    .unwrap()
                    .into_iter()
                    .find(|t| {
                        t.class() == "SendDraftTask" && t.data["headerMessageId"] == hmid.as_str()
                    })
                    .unwrap();
                assert_eq!(
                    task.data["draft"]["body"], "<p>transformed</p>",
                    "round {i}: the kept body goes out"
                );
            }
        }
    }
}
