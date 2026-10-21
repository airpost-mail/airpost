// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! New and reply drafts as the desktop builds them. An engine with `draft-pipeline` builds
//! them (`draft.create`, unsaved, so the tool can put its text on top before saving); an older
//! one gets the same builders run here (`tern_compose`), with the signatures and quote texts of
//! the settings the engines keep, if any.

use serde_json::{json, Map, Value};
use tern_compose::draft::{self, Ctx as ComposeCtx};
use tern_compose::{ComposeSettings, Identities};
use tern_ipc::{draft_kind, feature, method};

use crate::discovery::AccountRef;
use crate::drafts::Sender;
use crate::tools::{failed, Ctx, ToolError};

/// The settings keys a draft is built with (`tern_compose::settings`).
const COMPOSE_KEYS: [&str; 5] = [
    tern_compose::settings::HTML_KEY,
    tern_compose::settings::SIGNATURES_KEY,
    tern_compose::settings::DEFAULT_SIGNATURES_KEY,
    tern_compose::settings::DEFAULT_ACCOUNT_KEY,
    tern_compose::settings::QUOTE_TEXTS_KEY,
];

fn account_json(s: &Sender) -> Value {
    let mut v = json!({
        "id": s.account_id,
        "name": s.meta.name,
        "emailAddress": s.meta.email,
        "aliases": s.meta.aliases,
    });
    if let Some(a) = &s.meta.default_alias {
        v["defaultAlias"] = json!(a);
    }
    if let Some((kind, value)) = &s.meta.autoaddress {
        v["autoaddress"] = json!({"type": kind, "value": value});
    }
    v
}

/// `kind` is `new`, `reply` or `reply-all`; `message` the replied-to message with its `body`.
/// The draft comes back unsaved.
pub fn build(
    ctx: &mut Ctx<'_>,
    account: &AccountRef,
    kind: &str,
    message: Option<&Value>,
    fields: Map<String, Value>,
) -> Result<Value, ToolError> {
    let aid = account.account_id.as_str();
    if ctx.backend.has_feature(aid, feature::DRAFT_PIPELINE) {
        // The tools' bodies are HTML, whatever the composer's default is.
        let mut params =
            json!({"accountId": aid, "kind": kind, "fields": fields, "save": false, "html": true});
        if let Some(m) = message {
            params["messageId"] = m["id"].clone();
        }
        return ctx
            .call(aid, method::DRAFT_CREATE, params)
            .map_err(|e| failed(format!("The engine could not build the draft: {e}")));
    }
    let accounts: Vec<Value> = ctx
        .backend
        .accounts()
        .iter()
        .map(|a| account_json(&ctx.sender(a)))
        .collect();
    let identities = Identities::from_accounts_json(&accounts);
    let stored = ctx.backend.settings(&COMPOSE_KEYS).unwrap_or_default();
    let settings = ComposeSettings::from_values(
        |k| stored.get(k).cloned(),
        tern_compose::settings::local_tz(),
    );
    let c = ComposeCtx {
        settings: &settings,
        accounts: &identities,
        html: true,
        now: chrono::Utc::now().timestamp(),
    };
    let built = match (kind, message) {
        (draft_kind::NEW, _) => {
            let mut fields = fields;
            fields.insert("aid".into(), json!(aid));
            draft::new_draft(&c, &fields)
        }
        (draft_kind::REPLY, Some(m)) => draft::reply_draft(&c, m, false, &fields),
        (draft_kind::REPLY_ALL, Some(m)) => draft::reply_draft(&c, m, true, &fields),
        _ => Err(format!("can't build a {kind} draft here")),
    };
    built.map_err(|e| failed(format!("Could not build the draft: {e}")))
}

/// The tool's text on top of what the builder wrote (the signature, the quote), as typed at
/// the top of the composer. The draft has been touched.
pub fn with_text(mut draft: Value, text: &str) -> Value {
    let built = draft["body"].as_str().unwrap_or("").to_owned();
    draft["body"] = json!(format!("{text}{built}"));
    draft["pristine"] = json!(false);
    if let Some(o) = draft.as_object_mut() {
        // The engine fills the id in; a SyncbackDraftTask derives it again.
        o.remove("id");
    }
    draft
}
