// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The read tools.

use serde_json::{json, Value};

use super::{
    failed, opt_bool, opt_count, opt_str, req_str, schema, Ctx, Output, ToolDef, ToolError,
    ToolResult,
};
use crate::model::{self, array_of, i64_of, str_of};
use crate::policy::Category;

/// Threads `search_mail` and `list_threads` look at per account before giving up on
/// filling a page.
const SCAN_LIMIT: usize = 2000;
const PAGE: usize = ms_ipc::READ_MAX_LIMIT;

pub(super) fn definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "list_accounts",
            description: "List all email accounts configured in Tern",
            schema: schema(json!({}), &[]),
            category: Category::Read,
        },
        ToolDef {
            name: "list_folders",
            description: "List all folders/labels for an email account. Each entry includes a `role` — a provider-agnostic identifier (e.g. \"inbox\", \"sent\", \"drafts\", \"trash\", \"spam\", \"archive\", \"all\", \"important\", \"snoozed\") for standard folders, or null for user-created ones. Use `role` rather than `name` to find a standard folder like Sent or Trash, since the display name varies by provider (e.g. \"Sent Mail\" vs \"Sent Items\" vs Gmail's SENT label).",
            schema: schema(
                json!({"accountId": {"type": "string", "description": "The account ID to list folders for"}}),
                &["accountId"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "search_mail",
            description: "Search for email threads. Plain words must all match somewhere in the thread (sender, recipients, subject or body); \"quoted phrases\" match together. Keywords: from:<text>, to:<text> and subject:<text> add their value as a search word; in:<folder> (a folder/label's standard role when it has one, e.g. \"in:sent\", \"in:inbox\", \"in:trash\", \"in:drafts\", \"in:spam\", \"in:archive\" — this works across providers without looking up folder names first; otherwise the folder/label name or path); is:unread|read|starred|unstarred; has:attachment; before:<date> and after:<date>/since:<date> with an exact date like 2026/05/31 or 2026-05-31. Adjacent terms are combined with AND. OR, NOT, parentheses and relative dates (\"yesterday\") are not supported. Examples: 'in:sent project alpha', 'from:alice is:unread', 'to:bob has:attachment after:2026/01/01'. Each result's `lastMessageReceivedTimestamp` orders the list, newest first.",
            schema: schema(
                json!({
                    "query": {"type": "string", "description": "Search query string"},
                    "accountId": {"type": "string", "description": "Optional account ID to scope the search"},
                    "limit": {"type": "number", "default": 150, "description": "Max results to return (default 150)"},
                    "offset": {"type": "number", "default": 0, "description": "Number of results to skip for pagination (default 0)"}
                }),
                &["query"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "list_threads",
            description: "List threads in a specific folder/label. To find a standard folder like Sent or Trash, call list_folders first and pick the entry whose `role` matches (\"inbox\", \"sent\", \"drafts\", \"trash\", \"spam\", \"archive\", \"all\", \"important\", \"snoozed\") — the exact folder name varies by provider, so don't guess it from `name`.",
            schema: schema(
                json!({
                    "folderId": {"type": "string", "description": "The folder/label ID (from list_folders — match by `role` for standard folders)"},
                    "accountId": {"type": "string", "description": "The account ID"},
                    "limit": {"type": "number", "default": 25, "description": "Max results (default 25)"},
                    "offset": {"type": "number", "default": 0, "description": "Offset for pagination"},
                    "unread": {"type": "boolean", "description": "Filter to unread only"}
                }),
                &["folderId", "accountId"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "get_thread",
            description: "Get a thread with all its messages, full body content, and attachment metadata (each message's `files` array; retrieve one with get_attachment). Each message has `draft`: true for the user's own unsent drafts, which are not mail anyone received.",
            schema: schema(
                json!({"threadId": {"type": "string", "description": "The thread ID"}}),
                &["threadId"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "get_message",
            description: "Get a single message with full body content and attachment metadata (`files` array; retrieve one with get_attachment). `draft` is true for the user's own unsent drafts.",
            schema: schema(
                json!({"messageId": {"type": "string", "description": "The message ID"}}),
                &["messageId"],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "get_message_tracking",
            description: "Get open-tracking and link-tracking data for a single sent message: total open count, per-recipient open timestamps, and per-link click counts with per-recipient click timestamps.",
            schema: schema(
                json!({"messageId": {"type": "string", "description": "The message ID"}}),
                &["messageId"],
            ),
            category: Category::Read,
        },
    ]
}

pub(super) fn list_accounts(ctx: &mut Ctx<'_>) -> ToolResult {
    let mut out = Vec::new();
    for a in ctx.allowed_accounts()? {
        let meta = ctx.sender(&a).meta;
        let uses_labels = ctx.uses_labels(&a)?;
        out.push(json!({
            "id": a.account_id,
            "name": meta.name,
            "email": a.email,
            "provider": meta.provider,
            "usesLabels": uses_labels,
        }));
    }
    Ok(Output::text(&Value::Array(out)))
}

pub(super) fn list_folders(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let account = ctx.account(&req_str(args, "accountId")?)?;
    let id = account.account_id.clone();
    let out: Vec<Value> = ctx
        .categories(&id)?
        .iter()
        .filter(|c| ctx.policy.folder_allowed(&id, str_of(c, "id")))
        .map(|c| {
            let role = str_of(c, "role");
            json!({
                "id": str_of(c, "id"),
                "name": model::serialize_category(c)["name"],
                "role": if role.is_empty() { Value::Null } else { Value::from(role) },
                "path": str_of(c, "path"),
            })
        })
        .collect();
    Ok(Output::text(&Value::Array(out)).with_account(&id))
}

/// Category ids that an `in:` value names in this account: a role, else a name or path.
fn resolve_in(categories: &[Value], value: &str) -> Vec<String> {
    let v = value.to_lowercase();
    let by_role: Vec<String> = categories
        .iter()
        .filter(|c| str_of(c, "role").eq_ignore_ascii_case(&v))
        .map(|c| str_of(c, "id").to_owned())
        .collect();
    if !by_role.is_empty() {
        return by_role;
    }
    categories
        .iter()
        .filter(|c| {
            str_of(c, "path").to_lowercase() == v
                || model::category_display_name(c).to_lowercase() == v
        })
        .map(|c| str_of(c, "id").to_owned())
        .collect()
}

fn in_any(thread: &Value, ids: &[String]) -> bool {
    model::thread_category_ids(thread).any(|c| ids.iter().any(|i| i == c))
}

/// Up to `want` threads of `account` in `category` (folder or label id, or every thread)
/// passing `keep`, newest first, looking at no more than [`SCAN_LIMIT`].
fn collect_threads(
    ctx: &mut Ctx<'_>,
    account: &str,
    category: Option<(&str, &str)>,
    want: usize,
    keep: &dyn Fn(&Value) -> bool,
) -> Result<Vec<Value>, ToolError> {
    let mut out = Vec::new();
    let mut before: Option<String> = None;
    let mut scanned = 0;
    loop {
        let mut params = json!({"limit": PAGE});
        if let Some((key, id)) = category {
            params[key] = Value::from(id);
        }
        if let Some(b) = &before {
            params["before"] = Value::from(b.as_str());
        }
        let page = ctx.call(account, ms_ipc::method::THREADS_LIST, params)?;
        let threads = array_of(&page, "threads");
        scanned += threads.len();
        out.extend(threads.iter().filter(|t| keep(t)).cloned());
        before = page.get("next").and_then(Value::as_str).map(str::to_owned);
        if out.len() >= want || before.is_none() || scanned >= SCAN_LIMIT {
            break;
        }
    }
    out.truncate(want);
    Ok(out)
}

fn received_ts(t: &Value) -> i64 {
    i64_of(t, "lmrt")
}

pub(super) fn search_mail(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let query = crate::search::parse(&req_str(args, "query")?).map_err(failed)?;
    let account_id = opt_str(args, "accountId")?;
    let limit = opt_count(args, "limit", 150)?;
    let offset = opt_count(args, "offset", 0)?;
    let want = offset + limit;
    let accounts = ctx.accounts_for(account_id.as_deref())?;

    let mut found: Vec<Value> = Vec::new();
    for account in &accounts {
        let id = account.account_id.as_str();
        // Every in: must hold; each resolves to the categories it names here.
        let mut groups: Vec<Vec<String>> = Vec::new();
        if !query.folders.is_empty() {
            let cats = ctx.categories(id)?;
            for f in &query.folders {
                let ids = resolve_in(&cats, f);
                if ids.is_empty() {
                    groups.clear();
                    break;
                }
                groups.push(ids);
            }
            if groups.is_empty() {
                continue;
            }
        }
        let policy = ctx.policy;
        let keep = |t: &Value| {
            query.matches(t) && groups.iter().all(|g| in_any(t, g)) && policy.thread_allowed(t)
        };
        if query.terms.is_empty() {
            // No words: list the first in: category (or everything) and filter the rest.
            let first = groups.first().and_then(|g| g.first()).cloned();
            let cats = ctx.categories_by_id(id)?;
            let kind = first.as_deref().map(|c| {
                if cats.get(c).is_some_and(|v| str_of(v, "__cls") == "Label") {
                    "labelId"
                } else {
                    "folderId"
                }
            });
            let cat = first.as_deref().zip(kind).map(|(c, k)| (k, c));
            found.extend(collect_threads(ctx, id, cat, want, &keep)?);
        } else {
            let r = ctx.call(
                id,
                ms_ipc::method::SEARCH,
                json!({"query": query.terms.join(" "), "limit": PAGE}),
            )?;
            found.extend(array_of(&r, "threads").iter().filter(|t| keep(t)).cloned());
        }
    }
    found.sort_by_key(|t| std::cmp::Reverse(received_ts(t)));
    let out: Vec<Value> = found
        .iter()
        .skip(offset)
        .take(limit)
        .map(|t| model::thread_summary(t, true))
        .collect();
    let mut o = Output::text(&Value::Array(out));
    if let Some(a) = account_id {
        o = o.with_account(&a);
    }
    Ok(o)
}

pub(super) fn list_threads(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let folder_id = req_str(args, "folderId")?;
    let account = ctx.account(&req_str(args, "accountId")?)?;
    let id = account.account_id.clone();
    if !ctx.policy.folder_allowed(&id, &folder_id) {
        return Err(ToolError::Denied(format!(
            "Access denied: folder '{folder_id}' is excluded from MCP access."
        )));
    }
    let limit = opt_count(args, "limit", 25)?;
    let offset = opt_count(args, "offset", 0)?;
    let unread = opt_bool(args, "unread")?;
    let cats = ctx.categories_by_id(&id)?;
    let Some(cat) = cats.get(&folder_id) else {
        return Err(failed(format!("Folder '{folder_id}' not found")));
    };
    let key = if str_of(cat, "__cls") == "Label" {
        "labelId"
    } else {
        "folderId"
    };
    let policy = ctx.policy;
    let keep = |t: &Value| {
        unread.is_none_or(|u| model::bool_of(t, "unread") == u) && policy.thread_allowed(t)
    };
    let threads = collect_threads(ctx, &id, Some((key, &folder_id)), offset + limit, &keep)?;
    let out: Vec<Value> = threads
        .iter()
        .skip(offset)
        .map(|t| model::thread_summary(t, false))
        .collect();
    Ok(Output::text(&Value::Array(out)).with_account(&id))
}

/// A thread's messages with their bodies, as `get_thread` returns them. Deletion
/// placeholders and messages in excluded folders are left out.
pub(super) fn thread_messages_detail(
    ctx: &mut Ctx<'_>,
    account: &str,
    messages: &[Value],
) -> Result<Vec<Value>, ToolError> {
    let visible: Vec<&Value> = messages
        .iter()
        .filter(|m| !str_of(m, "id").starts_with("deleted-") && ctx.policy.message_allowed(m))
        .collect();
    let ids: Vec<String> = visible.iter().map(|m| str_of(m, "id").to_owned()).collect();
    let bodies = ctx.bodies(account, &ids, super::body_wait())?;
    let folders = ctx.categories_by_id(account)?;
    Ok(visible
        .iter()
        .map(|m| {
            let body = bodies.get(str_of(m, "id")).cloned().flatten();
            model::message_detail(m, body.as_deref(), &folders)
        })
        .collect())
}

pub(super) fn get_thread(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let thread_id = req_str(args, "threadId")?;
    let (account, thread, messages) = ctx.find_thread(&thread_id)?;
    let details = thread_messages_detail(ctx, &account.account_id, &messages)?;
    Ok(Output::text(&model::thread_detail(&thread, details)).with_account(&account.account_id))
}

pub(super) fn get_message(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let message_id = req_str(args, "messageId")?;
    let (account, message) = ctx.find_message(&message_id, None)?;
    let body = ctx.body(&account.account_id, &message_id)?;
    let folders = ctx.categories_by_id(&account.account_id)?;
    Ok(
        Output::text(&model::message_detail(&message, body.as_deref(), &folders))
            .with_account(&account.account_id),
    )
}

const OPEN_TRACKING_ID: &str = "open-tracking";
const LINK_TRACKING_ID: &str = "link-tracking";

fn plugin_value<'a>(message: &'a Value, plugin_id: &str) -> Option<&'a Value> {
    array_of(message, "metadata")
        .iter()
        .find(|m| str_of(m, "pluginId") == plugin_id)
        .map(|m| &m["value"])
        .filter(|v| v.is_object())
}

/// `messageTrackingSummary`: `(opens, links)`, each null without that plugin's metadata.
pub(super) fn tracking_summary(message: &Value) -> (Value, Value) {
    let events = |list: &[Value]| -> Vec<Value> {
        list.iter()
            .map(|e| json!({"recipient": e.get("recipient"), "timestamp": e.get("timestamp")}))
            .collect()
    };
    let opens = plugin_value(message, OPEN_TRACKING_ID).map_or(
        Value::Null,
        |m| json!({"count": i64_of(m, "open_count"), "events": events(array_of(m, "open_data"))}),
    );
    let links = plugin_value(message, LINK_TRACKING_ID).map_or(Value::Null, |m| {
        let links: Vec<Value> = array_of(m, "links")
            .iter()
            .map(|l| {
                json!({"url": l.get("url"), "clickCount": i64_of(l, "click_count"),
                       "events": events(array_of(l, "click_data"))})
            })
            .collect();
        json!({"links": links})
    });
    (opens, links)
}

pub(super) fn get_message_tracking(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let message_id = req_str(args, "messageId")?;
    let (account, message) = ctx.find_message(&message_id, None)?;
    let (opens, links) = tracking_summary(&message);
    if opens.is_null() && links.is_null() {
        return Err(failed(format!(
            "Message '{message_id}' has no open-tracking or link-tracking data"
        )));
    }
    Ok(Output::text(&json!({
        "id": str_of(&message, "id"),
        "threadId": str_of(&message, "threadId"),
        "subject": str_of(&message, "subject"),
        "date": model::iso_date(i64_of(&message, "date")),
        "accountId": str_of(&message, "aid"),
        "draft": model::is_draft(&message),
        "opens": opens,
        "links": links,
    }))
    .with_account(&account.account_id))
}
