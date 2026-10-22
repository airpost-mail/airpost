// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Send reminders and tracking: `remindInDays`, `trackOpens` and `trackLinks` on the draft
//! tools, and `set_reminder` on a thread. They write the desktop plugins' metadata
//! (`tern_client::metadata`) with `SyncbackMetadataTask`, as the composer's buttons and the
//! thread's reminder menu do, so Tern shows them and the engine acts on them.

use serde_json::{json, Value};
use tern_client::metadata as meta;

use super::{failed, opt_bool, req_str, schema, Ctx, Output, ToolDef, ToolError, ToolResult};
use crate::discovery::AccountRef;
use crate::drafts;
use crate::model::{self, i64_of, str_of};
use crate::policy::Category;
use crate::tasks;

const DAY: f64 = 86_400.0;

fn remind_schema() -> Value {
    json!({"type": "number", "exclusiveMinimum": 0, "maximum": 365, "description": "Remind the user if no one has replied this many days after the draft is sent (fractions allowed): Tern brings the thread back to the top of the inbox then"})
}

/// The draft tools' follow-up properties.
pub(super) fn draft_properties() -> [(&'static str, Value); 3] {
    [
        ("remindInDays", remind_schema()),
        (
            "trackOpens",
            json!({"type": "boolean", "description": "Track when recipients open the message. Needs a Tern server that offers tracking; fails otherwise"}),
        ),
        (
            "trackLinks",
            json!({"type": "boolean", "description": "Track clicks on the message's links. Needs a Tern server that offers tracking; fails otherwise"}),
        ),
    ]
}

pub(super) fn definitions() -> Vec<ToolDef> {
    vec![ToolDef {
        name: "set_reminder",
        description: "Set or clear a send reminder on a thread, as Tern's reminder menu does: if no one has replied by then, Tern brings the thread back to the top of the inbox. Use it on a thread the user sent the last message in; for a draft, give remindInDays to create_draft, reply_to_thread or update_draft instead.",
        schema: schema(
            json!({
                "threadId": {"type": "string", "description": "The thread"},
                "remindInDays": {"type": "number", "exclusiveMinimum": 0, "maximum": 365, "description": "Days from now (fractions allowed)"},
                "clear": {"type": "boolean", "description": "true to remove the thread's reminder instead"}
            }),
            &["threadId"],
        ),
        category: Category::Write,
    }]
}

fn remind_days(args: &Value) -> Result<Option<f64>, ToolError> {
    match args.get("remindInDays") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(d) if d > 0.0 && d <= 365.0 => Ok(Some(d)),
            _ => Err(failed(
                "Invalid arguments: 'remindInDays' must be more than 0 and at most 365",
            )),
        },
        Some(_) => Err(failed("Invalid arguments: 'remindInDays' must be a number")),
    }
}

fn days_from_now(days: f64) -> i64 {
    chrono::Utc::now().timestamp() + (days * DAY).round() as i64
}

/// What a draft tool's follow-up arguments ask for, read before anything is written.
#[derive(Debug, Default)]
pub(super) struct FollowUp {
    remind_at: Option<i64>,
    opens: bool,
    links: bool,
}

impl FollowUp {
    pub(super) fn parse(args: &Value) -> Result<FollowUp, ToolError> {
        Ok(FollowUp {
            remind_at: remind_days(args)?.map(days_from_now),
            opens: opt_bool(args, "trackOpens")?.unwrap_or(false),
            links: opt_bool(args, "trackLinks")?.unwrap_or(false),
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.remind_at.is_none() && !self.tracks()
    }

    fn tracks(&self) -> bool {
        self.opens || self.links
    }

    /// Refuses tracking the account's engine can't send: it needs the `tracking` feature and
    /// a server offering it, which `engine.status` reports.
    pub(super) fn check(&self, ctx: &mut Ctx<'_>, account: &str) -> Result<(), ToolError> {
        if !self.tracks() {
            return Ok(());
        }
        let has = ctx
            .backend
            .has_feature(account, tern_ipc::feature::TRACKING);
        let features: Vec<String> = if has {
            vec![tern_ipc::feature::TRACKING.to_owned()]
        } else {
            vec![]
        };
        let caps: Option<tern_ipc::Capabilities> = if has {
            let status = ctx.call(account, tern_ipc::method::ENGINE_STATUS, json!({}))?;
            status
                .get("capabilities")
                .and_then(|c| serde_json::from_value(c.clone()).ok())
        } else {
            None
        };
        match meta::tracking_unavailable(&features, caps.as_ref(), self.opens, self.links) {
            Some(why) => Err(failed(format!("Can't track this draft: {why}"))),
            None => Ok(()),
        }
    }

    /// Writes the reminder and the tracking toggles onto the stored draft (its JSON, with
    /// `id`, `aid` and `hMsgId`).
    pub(super) fn apply(
        &self,
        ctx: &mut Ctx<'_>,
        account: &AccountRef,
        draft: &Value,
    ) -> Result<(), ToolError> {
        if self.tracks() && draft.get("plaintext").and_then(Value::as_bool) == Some(true) {
            return Err(failed(
                "This draft is plain text, and tracking needs an HTML draft",
            ));
        }
        let mut values = Vec::new();
        if let Some(at) = self.remind_at {
            values.push((
                meta::THREAD_SEND_REMINDERS,
                meta::draft_reminder(at, str_of(draft, "hMsgId")),
            ));
        }
        if self.opens {
            values.push((meta::OPEN_TRACKING, meta::open_tracking_on()));
        }
        if self.links {
            values.push((meta::LINK_TRACKING, meta::link_tracking_on()));
        }
        for (plugin, value) in values {
            ctx.queue(
                &account.account_id,
                tasks::syncback_message_metadata(draft, plugin, value, None),
            )?;
        }
        Ok(())
    }

    /// The keys a draft tool's result gains.
    pub(super) fn extend(&self, out: &mut Value) {
        if let Some(at) = self.remind_at {
            out["remindAt"] = model::iso_date(at);
        }
        if self.tracks() {
            out["tracking"] = json!({"opens": self.opens, "links": self.links});
        }
    }
}

pub(super) fn set_reminder(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let thread_id = req_str(args, "threadId")?;
    let days = remind_days(args)?;
    let clear = opt_bool(args, "clear")?.unwrap_or(false);
    let at = match (days, clear) {
        (Some(d), false) => Some(days_from_now(d)),
        (None, true) => None,
        _ => {
            return Err(failed(
                "Invalid arguments: give either 'remindInDays' or 'clear': true",
            ))
        }
    };
    let (account, thread, messages) = ctx.find_thread(&thread_id)?;
    let me = drafts::Me::new(&[ctx.sender(&account)]);
    let sent = messages
        .iter()
        .filter(|m| !model::is_draft(m))
        .filter(|m| {
            model::array_of(m, "from")
                .first()
                .is_some_and(|c| me.is_me(str_of(c, "email")))
        })
        .max_by_key(|m| i64_of(m, "date"));
    let value = meta::thread_reminder(
        at,
        sent.map(|m| str_of(m, "hMsgId")),
        i64_of(&thread, "lmrt"),
    );
    ctx.queue(
        &account.account_id,
        tasks::syncback_thread_metadata(
            &account.account_id,
            str_of(&thread, "id"),
            meta::SEND_REMINDERS,
            value,
        ),
    )?;
    Ok(Output::text(&json!({
        "threadId": str_of(&thread, "id"),
        "remindAt": at.map_or(Value::Null, model::iso_date),
        "cleared": at.is_none(),
    }))
    .with_account(&account.account_id))
}
