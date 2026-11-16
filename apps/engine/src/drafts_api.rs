// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The draft pipeline on the socket (`feature::DRAFT_PIPELINE`, `serve` only):
//! `draft.create`, `draft.send`, `draft.cancelSend` and `draft.validate`, and
//! (`feature::DRAFT_EDITING`) `draft.update` and `draft.delete`.
//!
//! The connection's half reads what every account shares (the settings, and every account's
//! addresses, which decide who "me" is in a reply) and hands the call to the account's `main`
//! thread, whose half reads and writes the store: drafts are saved with `SyncbackDraftTask`,
//! sends queued as `SendDraftTask` and undo windows written as send-later metadata, through
//! the same local phases a client's tasks take. The builders are `airpost_compose`'s. Files
//! attached by path are copied on the connection's thread, before the account's half runs,
//! as `files.put` copies them, and deleted again when the call fails.

use airpost_compose::draft::{self, Ctx};
use airpost_compose::settings::local_tz;
use airpost_compose::{ComposeSettings, Identities};
use airpost_core::constants::FILES_DIR;
use airpost_core::Account;
use airpost_ipc::{
    code, draft_kind, method, DraftCancelSendParams, DraftCancelSendResult, DraftCreateParams,
    DraftDeleteParams, DraftDeleteResult, DraftSendParams, DraftSendResult, DraftUpdateParams,
    DraftValidateParams, ErrorObject, Request as RpcRequest, DRAFT_ATTACHMENTS_MAX_BYTES,
};
use airpost_net::{MetadataBackend, TrackingTarget};
use airpost_store::{File, Message, Model, Query, Store, Task, Thread};
use airpost_sync::expiration_executor::{
    apply_tracking, archive_task, finalise_draft, send_later_expiration, send_task_json,
    stored_body, SEND_ACTIONS, SEND_LATER, SNOOZE_BODY_EN, SNOOZE_BODY_KEY,
};
use airpost_sync::pending_sends::{recipient_mismatch, PendingBodies, PendingSends};
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
    /// `attach`, copied in already.
    pub attached: Vec<File>,
    /// `template`, read already.
    pub template: Option<airpost_compose::templates::Template>,
}

/// `draft.update`, with `attach` copied in already.
#[derive(Debug)]
pub struct UpdateCall {
    pub params: DraftUpdateParams,
    pub attached: Vec<File>,
}

pub fn is_draft_method(name: &str) -> bool {
    matches!(
        name,
        method::DRAFT_CREATE
            | method::DRAFT_SEND
            | method::DRAFT_CANCEL_SEND
            | method::DRAFT_VALIDATE
            | method::DRAFT_UPDATE
            | method::DRAFT_DELETE
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
            let template = match &params.template {
                Some(name) => Some(crate::templates_api::load(
                    &session.server.config_dir,
                    name,
                )?),
                None => None,
            };
            let attached = stage(session, &account_id, &params.attach)?;
            let staged = attached.clone();
            let reply = session.dispatch(
                &account_id,
                Request::DraftCreate(Box::new(CreateCall {
                    params,
                    settings,
                    accounts,
                    resurface_body,
                    attached,
                    template,
                })),
            );
            unstage_on_error(session, reply, &staged)
        }
        method::DRAFT_UPDATE => {
            let p = req.params_as::<DraftUpdateParams>()?;
            session.server.accounts.check(&p.account_id)?;
            let id = p.account_id.clone();
            let attached = stage(session, &id, &p.attach)?;
            let staged = attached.clone();
            let reply = session.dispatch(
                &id,
                Request::DraftUpdate(Box::new(UpdateCall {
                    params: p,
                    attached,
                })),
            );
            unstage_on_error(session, reply, &staged)
        }
        method::DRAFT_DELETE => {
            let p = req.params_as::<DraftDeleteParams>()?;
            session.server.accounts.check(&p.account_id)?;
            let id = p.account_id.clone();
            session.dispatch(&id, Request::DraftDelete(p))
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

fn stage(session: &Session, account_id: &str, paths: &[String]) -> Result<Vec<File>, ErrorObject> {
    crate::files::stage_uploads(&session.server.config_dir, account_id, paths)
}

fn unstage_on_error(session: &Session, reply: Reply, staged: &[File]) -> Reply {
    remove_on_error(&session.server.config_dir, reply, staged)
}

fn remove_on_error(config_dir: &std::path::Path, reply: Reply, staged: &[File]) -> Reply {
    if reply.is_err() {
        crate::files::remove_uploads(config_dir, staged);
    }
    reply
}

// -- the account's half ---------------------------------------------------------------------

/// The account's `main` thread: its store, account and task queue.
pub struct Here<'a> {
    pub store: &'a Store,
    pub account: &'a Account,
    pub config_dir: &'a std::path::Path,
    pub queue: &'a dyn QueueTask,
    /// The account's metadata backend: where tracked sends point.
    pub metadata: &'a dyn MetadataBackend,
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
                airpost_core::Error::other("rolled back")
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
            attached,
            template,
        } = call;
        let ctx = Ctx {
            settings: &settings,
            accounts: &accounts,
            html: p.html.unwrap_or(settings.html),
            now: self.store.now(),
        };
        let fields = p.fields.clone().unwrap_or_default();
        // A draft the engine saves with the caller's content in it has been written: a
        // pristine one is never pushed to the server, and the desktop discards it when its
        // composer closes untouched. A caller that saves the draft itself (the desktop's
        // compose-to-contact draft, pristine until edited) decides that on its own.
        let touched = p.save != Some(false)
            && !fields.contains_key("pristine")
            && (draft::sets_content(&fields) || !attached.is_empty() || template.is_some());
        let keep_subject = fields.contains_key("subject");
        let identity = accounts.get(self.account.id()).cloned().unwrap_or_else(|| {
            airpost_compose::Identity::from_account_json(self.account.as_json())
        });
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
                    draft::set_fields(&mut d, &fields);
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
        if let Some(t) = &template {
            airpost_compose::templates::apply(&mut built, t, keep_subject);
        }
        if touched {
            built["pristine"] = json!(false);
        }
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
            built["id"] = json!(airpost_core::ids::draft_id(self.account.id(), &hmid));
        }
        let repeated = self.atomically("draft.create", || {
            let (added, repeated) = self.attach(&mut built, attached)?;
            self.save_files(&added)?;
            if p.save != Some(false) {
                self.save_draft(&built)?;
            }
            Ok(repeated)
        })?;
        crate::files::remove_uploads(self.config_dir, &repeated);
        Ok(built)
    }

    // -- draft.update / draft.delete -----------------------------------------------------

    /// Refuses a change to a draft that is being sent, or will be when a send-later fires.
    fn refuse_while_sending(&self, draft: &Message, change: &str) -> Result<(), ErrorObject> {
        let hmid = draft.header_message_id();
        if self.send_pending(hmid)? {
            return Err(invalid(format!(
                "draft {hmid} is being sent; it can't be {change}"
            )));
        }
        if send_later_expiration(draft).is_some() {
            return Err(invalid(format!(
                "draft {hmid} is scheduled to send; cancel the send (draft.cancelSend) first"
            )));
        }
        Ok(())
    }

    pub fn update(&self, call: UpdateCall) -> Reply {
        let UpdateCall {
            params: p,
            attached,
        } = call;
        let hmid = p.header_message_id.as_str();
        // One transaction, so the expiration executor can't send the draft between the check
        // and the save.
        let (saved, repeated, detached) = self.atomically("draft.update", || {
            let stored = self.draft(hmid)?;
            self.refuse_while_sending(&stored, "edited")?;
            // A draft synced from another device has its `files` and `plaintext` only once its
            // body is here.
            let body = stored_body(self.store, stored.id())
                .map_err(internal)?
                .ok_or_else(|| {
                    not_found(format!(
                        "the body of draft {hmid} isn't downloaded yet; fetch it (bodies.fetch) and try again"
                    ))
                })?;
            let mut json = stored.data.clone();
            json["body"] = json!(body);
            let fields = p.fields.clone().unwrap_or_default();
            draft::set_fields(&mut json, &fields);
            let (added, repeated) = self.attach(&mut json, attached)?;
            let detached = self.detach(&mut json, &p.detach)?;
            let touched =
                draft::sets_content(&fields) || !added.is_empty() || !p.detach.is_empty();
            if touched && !fields.contains_key("pristine") {
                json["pristine"] = json!(false);
            }
            self.save_files(&added)?;
            for f in &detached {
                if let Some(mut row) = self
                    .store
                    .find::<File>(&Query::new().equal("id", f.id()))
                    .map_err(internal)?
                {
                    self.store.remove(&mut row).map_err(internal)?;
                }
            }
            self.save_draft(&json)?;
            let saved = self.draft(hmid)?;
            let mut out = saved.data.clone();
            out["body"] = json!(stored_body(self.store, saved.id())
                .map_err(internal)?
                .unwrap_or_default());
            Ok((out, repeated, detached))
        })?;
        crate::files::remove_uploads(self.config_dir, &repeated);
        crate::files::remove_uploads(self.config_dir, &detached);
        Ok(saved)
    }

    pub fn delete(&self, p: DraftDeleteParams) -> Reply {
        let hmid = p.header_message_id.as_str();
        // As cancel_send: in one transaction with the executor's, the send-later is cancelled
        // and the draft destroyed before it fires, or it has queued the send and the delete
        // is refused.
        let (result, draft_id) = self.atomically("draft.delete", || {
            let draft = self.draft(hmid)?;
            if self.send_pending(hmid)? {
                return Err(invalid(format!(
                    "draft {hmid} is being sent; it can't be deleted"
                )));
            }
            let cancelled_send = send_later_expiration(&draft).is_some();
            if cancelled_send {
                self.set_send_later(&draft, json!({ "expiration": null }))?;
            }
            let task_id =
                self.queue_task("DestroyDraftTask", &json!({ "messageIds": [draft.id()] }))?;
            let result = DraftDeleteResult {
                task_id,
                cancelled_send,
            };
            Ok((result, draft.id().to_owned()))
        })?;
        if result.cancelled_send {
            PendingSends::new(self.config_dir, self.account.id()).remove(&draft_id);
            info!("-- Cancelled the send of draft {hmid}");
        }
        to_value(result)
    }

    // -- attachments ---------------------------------------------------------------------

    /// Adds `attached` to the draft's `files`, but for those it carries already (same name
    /// and bytes), so a command run again doesn't attach a file twice. Answers with the files
    /// added and the ones left out.
    fn attach(
        &self,
        draft: &mut Value,
        attached: Vec<File>,
    ) -> Result<(Vec<File>, Vec<File>), ErrorObject> {
        if attached.is_empty() {
            return Ok(Default::default());
        }
        let root = self.config_dir.join(FILES_DIR);
        let mut files = draft["files"].as_array().cloned().unwrap_or_default();
        let (mut added, mut repeated) = (Vec::new(), Vec::new());
        for f in attached {
            if files.iter().any(|g| same_file(&root, g, &f)) {
                repeated.push(f);
            } else {
                files.push(f.to_json());
                added.push(f);
            }
        }
        let total: u64 = files.iter().filter_map(|f| f["size"].as_u64()).sum();
        if total > DRAFT_ATTACHMENTS_MAX_BYTES {
            return Err(invalid(format!(
                "the draft's attachments would come to {:.1} MB; at most {} MB go with a draft",
                total as f64 / 1e6,
                DRAFT_ATTACHMENTS_MAX_BYTES / 1_000_000
            )));
        }
        set_files(draft, files);
        Ok((added, repeated))
    }

    /// Takes the files `specs` name (by id, else by filename) off the draft. Answers with the
    /// uploads among them, whose copies go with them; a file of a message (a forward's) stays
    /// on disk for that message.
    fn detach(&self, draft: &mut Value, specs: &[String]) -> Result<Vec<File>, ErrorObject> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        let mut files = draft["files"].as_array().cloned().unwrap_or_default();
        let mut uploads = Vec::new();
        for spec in specs {
            let at = match files.iter().position(|f| f["id"] == spec.as_str()) {
                Some(at) => at,
                None => {
                    let named: Vec<usize> = (0..files.len())
                        .filter(|&i| files[i]["filename"] == spec.as_str())
                        .collect();
                    match named[..] {
                        [at] => at,
                        [] => return Err(invalid(format!("the draft has no file {spec:?}"))),
                        _ => {
                            return Err(invalid(format!(
                                "{spec:?} names {} of the draft's files; give the file's id",
                                named.len()
                            )))
                        }
                    }
                }
            };
            let file = File {
                data: files.remove(at),
            };
            if file.message_id().is_empty() {
                uploads.push(file);
            }
        }
        set_files(draft, files);
        Ok(uploads)
    }

    /// The File rows of new uploads, as `files.put` saves them, so `files.get` finds them.
    fn save_files(&self, files: &[File]) -> Result<(), ErrorObject> {
        for f in files {
            self.store.save(&mut f.clone()).map_err(internal)?;
        }
        Ok(())
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
        // As `DraftFactory.replyDraftsInThread`: the newest reply draft answering the message,
        // else the thread's newest reply draft, so a thread keeps one reply draft after a
        // newer message arrives. `rthMsgId` lives in the JSON only; the
        // replyToHeaderMessageId column isn't written.
        let replies: Vec<Message> = drafts
            .into_iter()
            .filter(|d| {
                !d.reply_to_header_message_id().is_empty()
                    && d.forwarded_header_message_id().is_empty()
                    && !airpost_store::models::is_hidden_json(&d.data)
                    && (!pristine_only
                        || d.data.get("pristine").and_then(Value::as_bool) == Some(true))
            })
            .collect();
        let chosen = replies
            .iter()
            .rev()
            .find(|d| d.reply_to_header_message_id() == hmid)
            .or_else(|| replies.last())
            .cloned();
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
        identity: &airpost_compose::Identity,
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
        // A delayed send is tracked when it fires. Asked before the transaction: it may make a
        // request to the server.
        let tracking = match p.delay_ms.unwrap_or(0) {
            0 => self.tracking_target(&p.header_message_id),
            _ => None,
        };
        let (reply, draft_id) = self.atomically("draft.send", || {
            self.send_in_transaction(p, tracking.as_ref())
        })?;
        if let Some(id) = draft_id {
            // Sent now: any bodies kept for an earlier undo window are spent.
            pending.remove(&id);
        }
        Ok(reply)
    }

    /// Where the draft's tracking URLs point, when it tracks anything and the server offers
    /// tracking.
    fn tracking_target(&self, header_message_id: &str) -> Option<TrackingTarget> {
        let draft = self.draft(header_message_id).ok()?;
        airpost_compose::tracking::kinds_of(&draft.data)?;
        self.metadata.tracking()
    }

    /// `draft.send`'s reply, and the draft's id when it was queued to send now.
    fn send_in_transaction(
        &self,
        p: DraftSendParams,
        tracking: Option<&TrackingTarget>,
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
                undo: None,
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
        if let Some(target) = tracking {
            apply_tracking(&mut data, target);
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
            undo: None,
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
        Ok(airpost_compose::validate::validate(&json, &accounts, &p.skip_salutation_for).to_json())
    }
}

fn set_files(draft: &mut Value, files: Vec<Value>) {
    draft["file_ids"] = files.iter().map(|f| f["id"].clone()).collect();
    draft["files"] = Value::Array(files);
}

/// Whether `new` has the name and the bytes of the draft's file `existing`.
fn same_file(root: &std::path::Path, existing: &Value, new: &File) -> bool {
    let existing = File {
        data: existing.clone(),
    };
    if existing.filename() != new.filename() || existing.size() != new.size() {
        return false;
    }
    let read = |f: &File| f.find_in(root).and_then(|p| std::fs::read(p).ok());
    matches!((read(&existing), read(new)), (Some(a), Some(b)) if a == b)
}

fn to_value<T: serde::Serialize>(v: T) -> Reply {
    serde_json::to_value(v).map_err(internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use airpost_store::{DeltaStream, Folder};
    use airpost_sync::expiration_executor::{EngineSettings, Executor};
    use airpost_tasks::TaskProcessor;
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
        let clock = airpost_core::ManualClock::new(1_700_000_000);
        airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let store = open(dir.path());
        let mut drafts = Folder::new(&airpost_core::ids::folder_id(ACCOUNT, "Drafts"), ACCOUNT, 0);
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
        with_backend(dir, store, &airpost_net::LocalOnly, f)
    }

    fn with_backend<T>(
        dir: &Path,
        store: &Store,
        metadata: &dyn MetadataBackend,
        f: impl FnOnce(&Here) -> T,
    ) -> T {
        let account = account();
        let local = Local(TaskProcessor::new(account.clone(), store).with_config_dir(dir));
        f(&Here {
            store,
            account: &account,
            config_dir: dir,
            queue: &local,
            metadata,
        })
    }

    /// A server that offers tracking; counts how often it is asked.
    #[derive(Default)]
    struct Tracking(std::sync::atomic::AtomicUsize);

    impl MetadataBackend for Tracking {
        fn push(&self, _: &airpost_net::MetadataPush) -> airpost_core::Result<()> {
            Ok(())
        }
        fn pull(
            &self,
            cursor: Option<&str>,
            _: &dyn Fn() -> bool,
            _: &mut dyn FnMut(airpost_net::MetadataDelta),
        ) -> airpost_core::Result<String> {
            Ok(cursor.unwrap_or("0").to_owned())
        }
        fn tracking(&self) -> Option<TrackingTarget> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(TrackingTarget {
                public_url: "https://t.example".into(),
                ns: "nsnsnsnsnsnsnsnsnsnsns".into(),
            })
        }
    }

    fn send_task(store: &Store, hmid: &str) -> Task {
        store
            .find_all::<Task>(&Query::new().equal("accountId", ACCOUNT))
            .unwrap()
            .into_iter()
            .find(|t| t.class() == "SendDraftTask" && t.data["headerMessageId"] == hmid)
            .unwrap()
    }

    #[test]
    fn send_now_tracks_a_tracked_draft_and_a_delayed_one_waits_for_the_executor() {
        let (dir, store) = setup();
        let tracked = |hmid: &str| {
            let mut d = draft(hmid);
            d["metadata"] = json!([{"pluginId": "open-tracking", "v": 1,
                                     "value": {"open_count": 0, "open_data": []}}]);
            d
        };
        save_draft(dir.path(), &store, tracked("tr-1@example.com"));
        save_draft(dir.path(), &store, tracked("tr-2@example.com"));
        save_draft(dir.path(), &store, draft("plain-1@example.com"));
        let server = Tracking::default();
        for (hmid, delay) in [
            ("tr-1@example.com", None),
            ("tr-2@example.com", Some(5000)),
            ("plain-1@example.com", None),
        ] {
            with_backend(dir.path(), &store, &server, |h| {
                h.send(params(json!({
                    "accountId": ACCOUNT, "headerMessageId": hmid, "delayMs": delay,
                })))
            })
            .unwrap();
        }
        // Asked for the tracked send now only.
        assert_eq!(server.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        let task = send_task(&store, "tr-1@example.com");
        let bodies = task.data["perRecipientBodies"].as_object().unwrap();
        assert_eq!(bodies["self"], "<p>Hi</p>");
        for who in ["ann@example.net", "bob@example.net"] {
            assert!(bodies[who]
                .as_str()
                .unwrap()
                .contains(r#"<img alt="" src="https://t.example/t/o/nsnsnsnsnsnsnsnsnsnsns/"#));
        }
        let open =
            airpost_store::model::metadata_value(&task.data["draft"], "open-tracking").unwrap();
        assert_eq!(open["recipients"].as_object().unwrap().len(), 2);
        assert!(send_task(&store, "plain-1@example.com").data["perRecipientBodies"].is_null());
        assert_eq!(send_tasks(&store, "tr-2@example.com"), 0);
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
                attached: Vec::new(),
                template: None,
            })
        })
        .unwrap();
        assert_eq!(built["id"], "synced-draft-id");
        let drafts = drafts_with(&store, "reply-1@example.com");
        assert_eq!(drafts.len(), 1, "saved in place, not as a second draft");
        assert_eq!(drafts[0].id(), "synced-draft-id");
    }

    fn create(h: &Here, p: Value) -> Value {
        h.create(CreateCall {
            params: params(p),
            settings: ComposeSettings::default(),
            accounts: Identities::from_accounts_json([account().as_json()]),
            resurface_body: None,
            attached: Vec::new(),
            template: None,
        })
        .unwrap()
    }

    fn stored_pristine(store: &Store, built: &Value) -> Option<bool> {
        let hmid = built["hMsgId"].as_str().unwrap();
        let drafts = drafts_with(store, hmid);
        assert_eq!(drafts.len(), 1, "{hmid}");
        drafts[0].data["pristine"].as_bool()
    }

    #[test]
    fn a_draft_saved_with_the_callers_content_is_not_pristine() {
        let (dir, store) = setup();
        let (message_id, _) = received_reply_target(&store);
        with_here(dir.path(), &store, |h| {
            let body = json!({"body": "<p>Sure</p>"});
            let reply = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": message_id,
                       "behavior": "prefer-existing", "fields": body}),
            );
            assert_eq!(reply["pristine"], false);
            assert_eq!(stored_pristine(&store, &reply), Some(false));

            let to = json!({"to": [{"email": "ann@example.net"}]});
            let new = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "new", "fields": to}),
            );
            assert_eq!(stored_pristine(&store, &new), Some(false));

            // Built from the message alone, or told otherwise: untouched.
            let forward = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "forward", "messageId": message_id}),
            );
            assert_eq!(stored_pristine(&store, &forward), Some(true));
            let told = json!({"body": "<p>Template</p>", "pristine": true});
            let told = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "new", "fields": told}),
            );
            assert_eq!(stored_pristine(&store, &told), Some(true));

            // The desktop saves what it built itself: its compose-to-contact draft stays
            // pristine until the user edits it.
            let unsaved = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "new", "save": false, "fields": to}),
            );
            assert_eq!(unsaved["pristine"], true);
            let desktop_reply = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": message_id,
                       "save": false, "fields": {"threadId": "t1"}}),
            );
            assert_eq!(desktop_reply["pristine"], true);
        });
    }

    #[test]
    fn a_template_fills_the_body_and_the_subject_of_a_new_draft_only() {
        let (dir, store) = setup();
        let (message_id, _) = received_reply_target(&store);
        let template = airpost_compose::templates::Template {
            subject: "Checking in".into(),
            body: "<div>Hi <span data-tvar=\"name\">Name</span></div>".into(),
        };
        let with_template = |h: &Here, p: Value| {
            h.create(CreateCall {
                params: params(p),
                settings: ComposeSettings::default(),
                accounts: Identities::from_accounts_json([account().as_json()]),
                resurface_body: None,
                attached: Vec::new(),
                template: Some(template.clone()),
            })
            .unwrap()
        };
        with_here(dir.path(), &store, |h| {
            let new = with_template(h, json!({"accountId": ACCOUNT, "kind": "new"}));
            assert_eq!(new["subject"], "Checking in");
            assert!(new["body"].as_str().unwrap().starts_with("<div>Hi <span"));
            assert_eq!(stored_pristine(&store, &new), Some(false));

            let mine = with_template(
                h,
                json!({"accountId": ACCOUNT, "kind": "new", "fields": {"subject": "Mine"}}),
            );
            assert_eq!(mine["subject"], "Mine");

            let reply = with_template(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": message_id}),
            );
            assert!(reply["subject"].as_str().unwrap().starts_with("Re: "));
            let body = reply["body"].as_str().unwrap();
            assert!(body.starts_with("<div>Hi <span"), "{body}");
            assert!(body.contains("gmail_quote"), "{body}");
        });
    }

    #[test]
    fn prefer_existing_edits_the_reply_draft_with_the_callers_fields() {
        let (dir, store) = setup();
        let (message_id, _) = received_reply_target(&store);
        with_here(dir.path(), &store, |h| {
            let first = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": message_id,
                       "behavior": "prefer-existing"}),
            );
            assert_eq!(first["pristine"], true);
            let second = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": message_id,
                       "behavior": "prefer-existing",
                       "fields": {"body": "<p>Second</p>", "subject": "Re: Plans, again",
                                  "hMsgId": "other@example.com"}}),
            );
            assert_eq!(second["id"], first["id"]);
            assert_eq!(second["hMsgId"], first["hMsgId"]);
            assert_eq!(second["subject"], "Re: Plans, again");
            assert_eq!(stored_pristine(&store, &second), Some(false));
            let id = second["id"].as_str().unwrap();
            assert_eq!(
                stored_body(&store, id).unwrap().as_deref(),
                Some("<p>Second</p>")
            );
        });
    }

    #[test]
    fn prefer_existing_keeps_the_threads_reply_draft_after_a_newer_message() {
        let (dir, store) = setup();
        let (first_id, thread_id) = received_reply_target(&store);
        let inbox = store
            .find::<Folder>(
                &Query::new()
                    .equal("accountId", ACCOUNT)
                    .equal("role", "inbox"),
            )
            .unwrap()
            .unwrap();
        let later = airpost_imap::ImapMessage {
            uid: 8,
            flags: airpost_core::MessageFlags::SEEN,
            internal_date: Some(1_699_100_000),
            header: Some(airpost_core::MessageHeader {
                date_raw: Some("Sat, 4 Nov 2023 12:13:20 +0000".into()),
                date: Some(1_699_100_000),
                subject_raw: Some(b"Re: Plans".to_vec()),
                subject: Some("Re: Plans".into()),
                message_id: Some("later@example.net".into()),
                in_reply_to: vec!["orig@example.net".into()],
                references: vec!["orig@example.net".into()],
                from: vec![airpost_core::Address::new(Some("Bob"), "bob@example.net")],
                to: vec![airpost_core::Address::new(None, "me@example.com")],
                ..Default::default()
            }),
            ..Default::default()
        };
        let account = account();
        let later = airpost_sync::mail_processor::MailProcessor::new(&account, &store)
            .insert_fallback_to_update_message(&later, &inbox, 1_700_000_100)
            .unwrap();
        assert_eq!(later.thread_id(), thread_id);
        store
            .conn()
            .execute(
                "REPLACE INTO MessageBody (id, value) VALUES (?, '<p>Or dinner?</p>')",
                [later.id()],
            )
            .unwrap();
        with_here(dir.path(), &store, |h| {
            let first = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": first_id,
                       "fields": {"body": "<p>First</p>"}}),
            );
            let second = create(
                h,
                json!({"accountId": ACCOUNT, "kind": "reply", "messageId": later.id(),
                       "behavior": "prefer-existing", "fields": {"body": "<p>Second</p>"}}),
            );
            assert_eq!(second["id"], first["id"]);
        });
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

    /// The delta lines a store wrote, shared with the test.
    #[derive(Clone, Default)]
    struct Lines(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Lines {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A received message in INBOX, ingested as a folder scan does, so it has a thread with
    /// categories and counts. Returns the message's id and thread id.
    fn received_reply_target(store: &Store) -> (String, String) {
        let mut inbox = Folder::new(&airpost_core::ids::folder_id(ACCOUNT, "INBOX"), ACCOUNT, 0);
        inbox.set_path("INBOX");
        inbox.set_role("inbox");
        store.save(&mut inbox).unwrap();
        let remote = airpost_imap::ImapMessage {
            uid: 7,
            flags: airpost_core::MessageFlags::SEEN,
            internal_date: Some(1_699_000_000),
            header: Some(airpost_core::MessageHeader {
                date_raw: Some("Fri, 3 Nov 2023 08:26:40 +0000".into()),
                date: Some(1_699_000_000),
                subject_raw: Some(b"Plans".to_vec()),
                subject: Some("Plans".into()),
                message_id: Some("orig@example.net".into()),
                from: vec![airpost_core::Address::new(Some("Bob"), "bob@example.net")],
                to: vec![airpost_core::Address::new(None, "me@example.com")],
                ..Default::default()
            }),
            ..Default::default()
        };
        let account = account();
        let m = airpost_sync::mail_processor::MailProcessor::new(&account, store)
            .insert_fallback_to_update_message(&remote, &inbox, 1_700_000_000)
            .unwrap();
        store
            .conn()
            .execute(
                "REPLACE INTO MessageBody (id, value) VALUES (?, '<p>Lunch?</p>')",
                [m.id()],
            )
            .unwrap();
        (m.id().to_owned(), m.thread_id().to_owned())
    }

    /// What one way of making a reply draft leaves behind: the draft row, its thread and the
    /// deltas, with what differs by construction (ids, Message-ID, times, body text) blanked.
    #[derive(Debug, PartialEq)]
    struct ReplyState {
        columns: Value,
        json: Value,
        thread: Value,
        categories: Vec<(String, i64, i64)>,
        deltas: Vec<(String, String, Value)>,
    }

    fn reply_state(path: impl FnOnce(&Path, &Store, &str, &str) -> String) -> ReplyState {
        let dir = tempfile::tempdir().unwrap();
        let clock = airpost_core::ManualClock::new(1_700_000_000);
        airpost_store::migrate::migrate(dir.path(), &clock, &mut Vec::new()).unwrap();
        let lines = Lines::default();
        let stream = DeltaStream::new(Box::new(lines.clone()));
        let store = Store::open(dir.path(), stream.clone()).unwrap();
        store.set_clock(Arc::new(clock));
        let mut drafts = Folder::new(&airpost_core::ids::folder_id(ACCOUNT, "Drafts"), ACCOUNT, 0);
        drafts.set_path("Drafts");
        drafts.set_role("drafts");
        store.save(&mut drafts).unwrap();
        let (message_id, thread_id) = received_reply_target(&store);
        stream.flush();
        lines.0.lock().unwrap().clear();

        let hmid = path(dir.path(), &store, &message_id, &thread_id);
        stream.flush();

        let d = &drafts_with(&store, &hmid)[0];
        let columns: Value = store
            .conn()
            .query_row(
                "SELECT threadId, draft, accountId, unread, starred FROM Message WHERE id = ?",
                [d.id()],
                |r| {
                    Ok(json!([
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?
                    ]))
                },
            )
            .unwrap();
        let blank = |mut j: Value| {
            for k in ["id", "hMsgId", "date", "body", "pristine", "v"] {
                if j.get(k).is_some() {
                    j[k] = json!("…");
                }
            }
            j
        };
        let thread = store
            .find::<Thread>(&Query::new().equal("id", &thread_id))
            .unwrap()
            .unwrap();
        let mut thread_json = thread.data.clone();
        for k in ["v", "lmt", "lmst", "lmrt", "fmt"] {
            if thread_json.get(k).is_some() {
                thread_json[k] = json!("…");
            }
        }
        let categories = store
            .conn()
            .prepare(
                "SELECT value, unread, inAllMail FROM ThreadCategory WHERE id = ? ORDER BY value",
            )
            .unwrap()
            .query_map([&thread_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
        let mut deltas = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let item: Value = serde_json::from_str(line).unwrap();
            let class = item["modelClass"].as_str().unwrap_or("").to_owned();
            if class == "Task" {
                continue;
            }
            for m in item["modelJSONs"].as_array().into_iter().flatten() {
                let summary = match class.as_str() {
                    "Message" => json!({
                        "threadId": m["threadId"], "draft": m["draft"],
                        "rthMsgId": m["rthMsgId"], "folders": m["folders"],
                    }),
                    "Thread" => json!({"id": m["id"], "folders": m["folders"]}),
                    _ => json!(null),
                };
                deltas.push((
                    item["type"].as_str().unwrap().to_owned(),
                    class.clone(),
                    summary,
                ));
            }
        }
        ReplyState {
            columns,
            json: blank(d.data.clone()),
            thread: thread_json,
            categories,
            deltas,
        }
    }

    fn create_reply(h: &Here, message_id: &str, save: bool) -> Value {
        h.create(CreateCall {
            params: params(json!({
                "accountId": ACCOUNT, "kind": "reply", "messageId": message_id,
                "save": save, "html": true,
            })),
            settings: ComposeSettings::default(),
            accounts: Identities::from_accounts_json([account().as_json()]),
            resurface_body: None,
            attached: Vec::new(),
            template: None,
        })
        .unwrap()
    }

    /// `draft.create` with `save` (the `airpost` CLI) and with the client saving what it built
    /// (airpost-mcp, and the desktop's DraftFactory on an engine with the draft pipeline) leave
    /// the same row, thread and deltas as a reply the desktop builds itself
    /// (`DraftFactory.createDraftForReply` without the pipeline, `Message.toJSON()` in a
    /// SyncbackDraftTask): the replied-to thread, `rthMsgId`, the Drafts copy on the thread.
    /// The reply to `orig@example.net` as the desktop's DraftFactory serializes it, in
    /// `thread_id` when it names one.
    fn desktop_reply_draft(hmid: &str, thread_id: Option<&str>) -> Value {
        let mut d = draft(hmid);
        d["subject"] = json!("Re: Plans");
        d["to"] = json!([{"name": "Bob", "email": "bob@example.net", "__cls": "Contact"}]);
        d["from"] = json!([{"name": "", "email": "me@example.com", "__cls": "Contact"}]);
        if let Some(thread_id) = thread_id {
            d["threadId"] = json!(thread_id);
        }
        d["rthMsgId"] = json!("orig@example.net");
        d["unread"] = json!(false);
        d["starred"] = json!(false);
        d["draft"] = json!(true);
        d["pristine"] = json!(true);
        d["date"] = json!(1_700_000_000);
        d["metadata"] = json!([]);
        d["events"] = json!([]);
        d
    }

    #[test]
    fn engine_built_reply_drafts_match_the_desktop_reply_draft() {
        let desktop = reply_state(|dir, store, _, thread_id| {
            let hmid = "desktop-reply@example.com";
            save_draft(dir, store, desktop_reply_draft(hmid, Some(thread_id)));
            hmid.to_owned()
        });
        // What the thread list's pencil and the Drafts view read: a draft row in the thread,
        // the thread listed in Drafts, and a Message delta carrying both.
        let drafts_id = airpost_core::ids::folder_id(ACCOUNT, "Drafts");
        assert_eq!(desktop.columns[1], 1, "{desktop:#?}");
        assert_eq!(desktop.json["threadId"], desktop.columns[0], "{desktop:#?}");
        assert!(
            desktop.thread["folders"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["id"] == drafts_id.as_str()),
            "{desktop:#?}"
        );
        assert!(desktop.categories.iter().any(|c| c.0 == drafts_id));
        assert!(desktop
            .deltas
            .iter()
            .any(|(kind, class, m)| kind == "persist"
                && class == "Message"
                && m["draft"] == true
                && m["threadId"] == desktop.columns[0]));

        let saved_by_engine = reply_state(|dir, store, message_id, _| {
            let built = with_here(dir, store, |h| create_reply(h, message_id, true));
            built["hMsgId"].as_str().unwrap().to_owned()
        });
        let saved_by_client = reply_state(|dir, store, message_id, thread_id| {
            let mut built = with_here(dir, store, |h| create_reply(h, message_id, false));
            built.as_object_mut().unwrap().remove("id");
            built["threadId"] = json!(thread_id);
            let hmid = built["hMsgId"].as_str().unwrap().to_owned();
            save_draft(dir, store, built);
            hmid
        });
        for (name, state) in [
            ("draft.create save", &saved_by_engine),
            ("draft.create + SyncbackDraftTask", &saved_by_client),
        ] {
            assert_eq!(state.columns, desktop.columns, "{name}: Message columns");
            assert_eq!(state.thread, desktop.thread, "{name}: thread");
            assert_eq!(
                state.categories, desktop.categories,
                "{name}: ThreadCategory"
            );
            assert_eq!(state.deltas, desktop.deltas, "{name}: deltas");
            for key in ["threadId", "rthMsgId", "draft", "folders", "labels", "aid"] {
                assert_eq!(state.json[key], desktop.json[key], "{name}: {key}");
            }
        }
    }

    /// A reply draft saved in no thread (imported before the message it answers synced)
    /// and put in the thread it answers by the engine (`Store::thread_unthreaded_draft`)
    /// ends as the desktop's reply draft saved in that thread: same row, thread and
    /// categories, and its last deltas are the ones the desktop's save emits.
    #[test]
    fn an_unthreaded_reply_draft_threaded_by_the_engine_matches_the_desktop_reply_draft() {
        let hmid = "desktop-reply@example.com";
        let desktop = reply_state(|dir, store, _, thread_id| {
            save_draft(dir, store, desktop_reply_draft(hmid, Some(thread_id)));
            hmid.to_owned()
        });
        let threaded = reply_state(|dir, store, _, thread_id| {
            save_draft(dir, store, desktop_reply_draft(hmid, None));
            let id = airpost_core::ids::draft_id(ACCOUNT, hmid);
            let joined = store
                .transaction("threadDraft", |s| {
                    s.thread_unthreaded_draft(ACCOUNT, &id, 0, true)
                })
                .unwrap();
            assert_eq!(joined.as_deref(), Some(thread_id));
            hmid.to_owned()
        });
        assert_eq!(threaded.columns, desktop.columns);
        assert_eq!(threaded.thread, desktop.thread);
        assert_eq!(threaded.categories, desktop.categories);
        assert!(
            threaded.deltas.ends_with(&desktop.deltas),
            "{:#?}\n{:#?}",
            threaded.deltas,
            desktop.deltas
        );
        for key in ["threadId", "rthMsgId", "draft", "folders", "labels", "aid"] {
            assert_eq!(threaded.json[key], desktop.json[key], "{key}");
        }
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

    fn update(h: &Here, p: Value) -> Reply {
        let p: DraftUpdateParams = params(p);
        let attached = crate::files::stage_uploads(h.config_dir, ACCOUNT, &p.attach)?;
        let staged = attached.clone();
        let reply = h.update(UpdateCall {
            params: p,
            attached,
        });
        remove_on_error(h.config_dir, reply, &staged)
    }

    fn delete(h: &Here, hmid: &str) -> Reply {
        h.delete(params(
            json!({"accountId": ACCOUNT, "headerMessageId": hmid}),
        ))
    }

    /// A file outside the files root, as a client names one to attach.
    fn source_file(dir: &Path, name: &str, bytes: &[u8]) -> String {
        let src = dir.join("outside");
        std::fs::create_dir_all(&src).unwrap();
        let path = src.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    }

    fn files_root(dir: &Path) -> std::path::PathBuf {
        dir.join(FILES_DIR)
    }

    /// The upload copies under the files root.
    fn copies(dir: &Path) -> usize {
        fn count(p: &Path) -> usize {
            std::fs::read_dir(p)
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|e| {
                            let p = e.path();
                            if p.ends_with(".tmp") {
                                0
                            } else if p.is_dir() {
                                count(&p)
                            } else {
                                1
                            }
                        })
                        .sum()
                })
                .unwrap_or(0)
        }
        count(&files_root(dir))
    }

    #[test]
    fn update_puts_fields_over_the_stored_draft() {
        let (dir, store) = setup();
        let hmid = "upd-1@example.com";
        let mut d = draft(hmid);
        d["pristine"] = json!(true);
        save_draft(dir.path(), &store, d);
        let saved = with_here(dir.path(), &store, |h| {
            update(
                h,
                json!({"accountId": ACCOUNT, "headerMessageId": hmid,
                       "fields": {"subject": "Changed", "to": [{"email": "cy@example.net"}],
                                  "hMsgId": "other@example.com", "aid": "other"}}),
            )
        })
        .unwrap();
        assert_eq!(saved["subject"], "Changed");
        assert_eq!(saved["hMsgId"], hmid);
        assert_eq!(saved["aid"], ACCOUNT);
        assert_eq!(saved["to"][0]["email"], "cy@example.net");
        assert_eq!(saved["body"], "<p>Hi</p>", "the stored body stays");
        assert!(saved["v"].as_i64().unwrap() > 1);
        let stored = &drafts_with(&store, hmid)[0];
        assert_eq!(stored.data["pristine"], false);
        assert_eq!(stored.data["subject"], "Changed");

        let err = with_here(dir.path(), &store, |h| {
            update(
                h,
                json!({"accountId": ACCOUNT, "headerMessageId": "nope@example.com"}),
            )
        })
        .unwrap_err();
        assert_eq!(err.code, code::NOT_FOUND);
    }

    #[test]
    fn update_and_delete_refuse_a_draft_being_sent() {
        let (dir, store) = setup();
        save_draft(dir.path(), &store, draft("sending@example.com"));
        save_draft(dir.path(), &store, draft("undo@example.com"));
        with_here(dir.path(), &store, |h| {
            h.send(params(
                json!({"accountId": ACCOUNT, "headerMessageId": "sending@example.com"}),
            ))
            .unwrap();
            h.send(params(json!({"accountId": ACCOUNT,
                                 "headerMessageId": "undo@example.com", "delayMs": 5000})))
                .unwrap();
            let edit = |hmid: &str| {
                update(
                    h,
                    json!({"accountId": ACCOUNT, "headerMessageId": hmid,
                           "fields": {"subject": "Too late"}}),
                )
                .unwrap_err()
            };
            let e = edit("sending@example.com");
            assert!(e.message.contains("is being sent"), "{}", e.message);
            let e = edit("undo@example.com");
            assert!(e.message.contains("is scheduled to send"), "{}", e.message);
            let e = delete(h, "sending@example.com").unwrap_err();
            assert!(e.message.contains("can't be deleted"), "{}", e.message);
        });
        assert_eq!(send_tasks(&store, "sending@example.com"), 1);
        assert_eq!(
            drafts_with(&store, "undo@example.com")[0].data["subject"],
            "Hi"
        );
    }

    #[test]
    fn attach_and_detach_files_as_the_desktop_does() {
        let (dir, store) = setup();
        let hmid = "att-1@example.com";
        save_draft(dir.path(), &store, draft(hmid));
        let pdf = source_file(dir.path(), "Report.pdf", b"%PDF-1.4 report");
        let notes = source_file(dir.path(), "notes.txt", b"notes");
        let saved = with_here(dir.path(), &store, |h| {
            update(
                h,
                json!({"accountId": ACCOUNT, "headerMessageId": hmid, "attach": [pdf, notes]}),
            )
        })
        .unwrap();
        let files = saved["files"].as_array().unwrap().clone();
        assert_eq!(files.len(), 2);
        let report = File {
            data: files[0].clone(),
        };
        assert_eq!(report.filename(), "Report.pdf");
        assert_eq!(report.content_type(), "application/pdf");
        assert_eq!(report.size(), 15);
        assert_eq!(report.message_id(), "");
        assert_eq!(report.data["aid"], ACCOUNT);
        assert_eq!(report.data["__cls"], "File");
        assert_eq!(
            saved["file_ids"],
            json!([files[0]["id"].clone(), files[1]["id"].clone()])
        );
        // Where the desktop's AttachmentStore and the send read it.
        let copy = report.client_path_in(&files_root(dir.path()));
        assert_eq!(std::fs::read(&copy).unwrap(), b"%PDF-1.4 report");
        assert!(store
            .find::<File>(&Query::new().equal("id", report.id()))
            .unwrap()
            .is_some());
        assert_eq!(drafts_with(&store, hmid)[0].data["pristine"], false);
        assert_eq!(copies(dir.path()), 2);

        // The same file again isn't attached twice; another one by that name is.
        let saved = with_here(dir.path(), &store, |h| {
            update(
                h,
                json!({"accountId": ACCOUNT, "headerMessageId": hmid, "attach": [pdf]}),
            )
        })
        .unwrap();
        assert_eq!(saved["files"].as_array().unwrap().len(), 2);
        assert_eq!(copies(dir.path()), 2, "the repeated copy is deleted");
        std::fs::write(&pdf, b"%PDF-1.4 revised").unwrap();
        let saved = with_here(dir.path(), &store, |h| {
            update(
                h,
                json!({"accountId": ACCOUNT, "headerMessageId": hmid, "attach": [pdf]}),
            )
        })
        .unwrap();
        assert_eq!(saved["files"].as_array().unwrap().len(), 3);

        with_here(dir.path(), &store, |h| {
            let detach = |what: Value| {
                update(
                    h,
                    json!({"accountId": ACCOUNT, "headerMessageId": hmid, "detach": what}),
                )
            };
            let e = detach(json!(["Report.pdf"])).unwrap_err();
            assert!(e.message.contains("names 2"), "{}", e.message);
            let e = detach(json!(["missing.doc"])).unwrap_err();
            assert!(e.message.contains("no file"), "{}", e.message);
            let saved = detach(json!([report.id(), "notes.txt"])).unwrap();
            let left = saved["files"].as_array().unwrap();
            assert_eq!(left.len(), 1);
            assert_eq!(saved["file_ids"], json!([left[0]["id"].clone()]));
        });
        assert!(!copy.exists(), "a detached upload's copy goes");
        assert!(store
            .find::<File>(&Query::new().equal("id", report.id()))
            .unwrap()
            .is_none());
        assert_eq!(copies(dir.path()), 1);
    }

    #[test]
    fn attach_checks_the_path_and_the_size() {
        let (dir, store) = setup();
        let hmid = "att-2@example.com";
        save_draft(dir.path(), &store, draft(hmid));
        let attach = |path: Value| {
            with_here(dir.path(), &store, |h| {
                update(
                    h,
                    json!({"accountId": ACCOUNT, "headerMessageId": hmid, "attach": path}),
                )
            })
            .unwrap_err()
            .message
        };
        assert!(attach(json!(["relative.pdf"])).contains("absolute"));
        let folder = dir.path().join("outside");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(attach(json!([folder.display().to_string()])).contains("directory"));
        let missing = folder.join("missing.pdf").display().to_string();
        assert!(attach(json!([missing])).contains("missing.pdf"));
        // Over the limit together: refused, and no copy is left.
        let big = vec![0u8; (DRAFT_ATTACHMENTS_MAX_BYTES / 2 + 1) as usize];
        let a = source_file(dir.path(), "a.bin", &big);
        let b = source_file(dir.path(), "b.bin", &big);
        let e = with_here(dir.path(), &store, |h| {
            update(
                h,
                json!({"accountId": ACCOUNT, "headerMessageId": hmid, "attach": [a, b]}),
            )
        })
        .unwrap_err();
        assert!(e.message.contains("at most 25 MB"), "{}", e.message);
        assert_eq!(copies(dir.path()), 0, "a failed call leaves no copies");
        assert!(drafts_with(&store, hmid)[0].data["files"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn create_attaches_files_saved_or_not() {
        let (dir, store) = setup();
        let pdf = source_file(dir.path(), "plan.pdf", b"plan");
        with_here(dir.path(), &store, |h| {
            for save in [true, false] {
                let p: DraftCreateParams = params(json!({
                    "accountId": ACCOUNT, "kind": "new", "save": save, "attach": [pdf],
                    "fields": {"to": [{"email": "ann@example.net"}]},
                }));
                let attached = crate::files::stage_uploads(dir.path(), ACCOUNT, &p.attach).unwrap();
                let built = h
                    .create(CreateCall {
                        params: p,
                        settings: ComposeSettings::default(),
                        accounts: Identities::from_accounts_json([account().as_json()]),
                        resurface_body: None,
                        attached,
                        template: None,
                    })
                    .unwrap();
                let file = File {
                    data: built["files"][0].clone(),
                };
                assert_eq!(file.filename(), "plan.pdf");
                assert_eq!(built["file_ids"], json!([file.id()]));
                assert!(file.find_in(&files_root(dir.path())).is_some());
                let hmid = built["hMsgId"].as_str().unwrap();
                let stored = drafts_with(&store, hmid);
                assert_eq!(stored.len(), usize::from(save));
                if save {
                    assert_eq!(stored[0].data["files"][0]["id"], file.id());
                    assert_eq!(stored[0].data["pristine"], false);
                }
            }
        });
    }

    #[test]
    fn delete_destroys_the_draft_and_cancels_its_undo_window() {
        let (dir, store) = setup();
        save_draft(dir.path(), &store, draft("del-1@example.com"));
        save_draft(dir.path(), &store, draft("del-2@example.com"));
        with_here(dir.path(), &store, |h| {
            let r = delete(h, "del-1@example.com").unwrap();
            assert_eq!(r["cancelledSend"], false);
            assert!(!r["taskId"].as_str().unwrap().is_empty());
            h.send(params(json!({
                "accountId": ACCOUNT, "headerMessageId": "del-2@example.com", "delayMs": 5000,
                "bodies": {"self": "<p>kept</p>"},
            })))
            .unwrap();
            let r = delete(h, "del-2@example.com").unwrap();
            assert_eq!(r["cancelledSend"], true);
            assert_eq!(
                delete(h, "del-2@example.com").unwrap_err().code,
                code::NOT_FOUND
            );
        });
        assert!(drafts_with(&store, "del-1@example.com").is_empty());
        assert!(drafts_with(&store, "del-2@example.com").is_empty());
        assert!(PendingSends::new(dir.path(), ACCOUNT).ids().is_empty());
        assert_eq!(send_tasks(&store, "del-2@example.com"), 0);
    }

    /// draft.delete against the executor at the end of the undo window, as
    /// `cancel_send_and_the_executor_never_both_win`: the delete wins and nothing is sent, or
    /// the send is queued and the delete is refused with the draft left to send.
    #[test]
    fn delete_and_the_executor_never_both_win() {
        let (dir, store) = setup();
        let dir = dir.path();
        for i in 0..45 {
            let hmid = format!("del-race-{i}@example.com");
            save_draft(dir, &store, draft(&hmid));
            let scheduled = with_here(dir, &store, |h| {
                h.send(params(json!({
                    "accountId": ACCOUNT, "headerMessageId": hmid, "delayMs": 1000,
                })))
            })
            .unwrap();
            let due = scheduled["scheduledAt"].as_i64().unwrap();
            let id = drafts_with(&store, &hmid)[0].id().to_owned();
            let order = i % 3;
            let (executor_first, delete_first) = (order == 0, order == 1);
            let barrier = Barrier::new(2);
            let deleted = std::thread::scope(|s| {
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
                let delete = s.spawn(|| {
                    let store = open(dir);
                    if !delete_first {
                        barrier.wait();
                    }
                    let r = with_here(dir, &store, |h| delete(h, &hmid));
                    if delete_first {
                        barrier.wait();
                    }
                    match r {
                        Ok(r) => {
                            assert_eq!(r["cancelledSend"], true, "round {i}");
                            true
                        }
                        Err(e) => {
                            assert!(e.message.contains("is being sent"), "round {i}: {e:?}");
                            false
                        }
                    }
                });
                delete.join().unwrap()
            });
            let sends = send_tasks(&store, &hmid);
            assert!(!(executor_first && deleted), "round {i}: sent first");
            assert!(!delete_first || deleted, "round {i}: deleted first");
            if deleted {
                assert_eq!(sends, 0, "round {i}: deleted, yet queued to send");
                assert!(drafts_with(&store, &hmid).is_empty(), "round {i}");
            } else {
                assert_eq!(sends, 1, "round {i}: not deleted, yet not sent");
            }
        }
    }
}
