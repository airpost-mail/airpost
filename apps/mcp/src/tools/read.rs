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
const PAGE: usize = tern_ipc::READ_MAX_LIMIT;

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
            description: "Search for email threads, in Tern's search bar grammar. Plain words must all match somewhere in the thread (sender, recipients, subject, folder names or body), each as a word prefix; \"quoted phrases\" match together. Fields: from:<text>, to:<text>, subject:<text>, in:<folder> (a standard role such as in:inbox, in:sent, in:drafts, in:trash, in:spam, in:archive works across providers; otherwise a folder/label name or path), is:unread|read|starred|unstarred, has:attachment, before:<date> and after:<date> (or since:). Dates: 2026-05-31 or 2026/05/31, \"may 31 2026\", month or weekday names, today, yesterday, \"3 days ago\", 3d/2w/6m/1y, \"last week|month|year\"; after: includes its day, before: excludes it, both in the user's time zone. Terms are combined with AND unless joined by OR; NOT x excludes x (e.g. 'budget NOT in:trash'); parentheses group. Examples: 'in:sent project alpha', 'from:alice is:unread', '(from:bob OR from:carol) has:attachment after:2026/01/01'. Each result's `lastMessageReceivedTimestamp` orders the list, newest first.",
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
        ToolDef {
            name: "list_tracked_messages",
            description: "List sent messages that have open-tracking and/or link-tracking enabled, along with their open/click data. Use sentAfter/sentBefore to bound by send date (e.g. \"everything sent this month\") and openedAfter to filter by when the open/click activity itself happened (e.g. \"opened today\"). Answers questions like \"which of my emails have been opened today\" or \"every email I sent this month with link tracking\".",
            schema: schema(
                json!({
                    "accountId": {"type": "string", "description": "Restrict to a single account"},
                    "trackingType": {"type": "string", "enum": ["opens", "links", "either"], "default": "either", "description": "Only include messages with open-tracking, link-tracking, or either (default)"},
                    "sentAfter": {"type": "string", "description": "ISO 8601 date - only messages sent after this"},
                    "sentBefore": {"type": "string", "description": "ISO 8601 date - only messages sent before this"},
                    "openedAfter": {"type": "string", "description": "ISO 8601 date - only include messages with an open or link-click event at/after this time (filters on activity time, not send date)"},
                    "minOpens": {"type": "number", "description": "Only include messages with at least this many total opens"},
                    "minClicks": {"type": "number", "description": "Only include messages with at least this many total link clicks"},
                    "limit": {"type": "number", "default": 50, "description": "Max results to return (default 50)"},
                    "offset": {"type": "number", "default": 0, "description": "Offset for pagination"}
                }),
                &[],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "get_top_links",
            description: "Rank links from link-tracking-enabled sent messages by total click count, across a time range. Answers questions like \"what are my highest performing links\" or \"which links get clicked the most\".",
            schema: schema(
                json!({
                    "accountId": {"type": "string", "description": "Restrict to a single account"},
                    "sentAfter": {"type": "string", "description": "ISO 8601 date - only consider messages sent after this"},
                    "sentBefore": {"type": "string", "description": "ISO 8601 date - only consider messages sent before this"},
                    "limit": {"type": "number", "default": 10, "description": "Max number of links to return (default 10)"}
                }),
                &[],
            ),
            category: Category::Read,
        },
        ToolDef {
            name: "list_scheduled_sends",
            description: "List drafts with a \"Send Later\" schedule. state is scheduled (sendAt is in the future), due (sendAt passed and the draft hasn't gone out yet: being sent, waiting for the draft's body, or Tern wasn't running) or failed (Tern gave up; error says why and the draft stays unsent until it is rescheduled or sent).",
            schema: schema(
                json!({"accountId": {"type": "string", "description": "Restrict to a single account"}}),
                &[],
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
        let page = ctx.call(account, tern_ipc::method::THREADS_LIST, params)?;
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
    let text = req_str(args, "query")?;
    let account_id = opt_str(args, "accountId")?;
    let limit = opt_count(args, "limit", 150)?;
    let offset = opt_count(args, "offset", 0)?;
    let want = offset + limit;
    let accounts = ctx.accounts_for(account_id.as_deref())?;

    let mut found: Vec<Value> = Vec::new();
    let mut plain: Option<crate::search::Query> = None;
    for account in &accounts {
        let id = account.account_id.as_str();
        if ctx
            .backend
            .has_feature(id, tern_ipc::feature::SEARCH_GRAMMAR)
        {
            found.extend(grammar_search(ctx, id, &text)?);
        } else {
            if plain.is_none() {
                plain = Some(crate::search::parse(&text).map_err(failed)?);
            }
            let query = plain.as_ref().expect("parsed above");
            found.extend(plain_search(ctx, id, query, want)?);
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

/// The engine parses the query (feature `search-grammar`) in the local zone and clock;
/// only the access policy is applied here.
fn grammar_search(ctx: &mut Ctx<'_>, account: &str, text: &str) -> Result<Vec<Value>, ToolError> {
    let params = tern_client::search::grammar_search(text, Some(PAGE));
    let params = serde_json::to_value(params).expect("params");
    let r = ctx
        .call(account, tern_ipc::method::SEARCH, params)
        .map_err(|e| {
            if e.code() == Some(tern_ipc::code::INVALID_PARAMS) {
                failed(format!("The search query can't be used: {e}"))
            } else {
                e.into()
            }
        })?;
    let policy = ctx.policy;
    Ok(array_of(&r, "threads")
        .iter()
        .filter(|t| policy.thread_allowed(t))
        .cloned()
        .collect())
}

/// An engine without the grammar: plain terms through `search`, the filters applied here.
fn plain_search(
    ctx: &mut Ctx<'_>,
    id: &str,
    query: &crate::search::Query,
    want: usize,
) -> Result<Vec<Value>, ToolError> {
    // Every in: must hold; each resolves to the categories it names here.
    let mut groups: Vec<Vec<String>> = Vec::new();
    if !query.folders.is_empty() {
        let cats = ctx.categories(id)?;
        for f in &query.folders {
            let ids = resolve_in(&cats, f);
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            groups.push(ids);
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
        return collect_threads(ctx, id, cat, want, &keep);
    }
    let r = ctx.call(
        id,
        tern_ipc::method::SEARCH,
        json!({"query": query.terms.join(" "), "limit": PAGE}),
    )?;
    Ok(array_of(&r, "threads")
        .iter()
        .filter(|t| keep(t))
        .cloned()
        .collect())
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

pub(super) use tern_client::metadata::tracking_summary;

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

const SEND_LATER_ID: &str = "send-later";
/// Messages a listing tool reads per account before giving up, as [`SCAN_LIMIT`] does for
/// threads.
const METADATA_SCAN_LIMIT: usize = 10_000;

/// Every message of `account` with `plugin_id` metadata and `draft` as given, newest first,
/// through `metadata.list`. An engine without the method gets a message saying so.
fn metadata_messages(
    ctx: &mut Ctx<'_>,
    tool: &str,
    account: &str,
    plugin_id: &str,
    filter: Value,
) -> Result<Vec<Value>, ToolError> {
    let mut out = Vec::new();
    loop {
        let mut params = filter.clone();
        params["accounts"] = json!([account]);
        params["pluginId"] = Value::from(plugin_id);
        params["limit"] = Value::from(PAGE);
        params["offset"] = Value::from(out.len());
        let page = match ctx.call(account, tern_ipc::method::METADATA_LIST, params) {
            Ok(p) => p,
            Err(e) if e.is_unknown_method() => {
                return Err(failed(format!(
                    "{tool} needs a newer Tern engine (one with metadata.list); update Tern"
                )))
            }
            Err(e) => return Err(e.into()),
        };
        let models = array_of(&page, "models");
        out.extend(models.iter().cloned());
        if models.len() < PAGE || out.len() >= METADATA_SCAN_LIMIT {
            return Ok(out);
        }
    }
}

/// An optional ISO 8601 argument as epoch seconds (fractional), the way `new Date()` reads it.
fn opt_date(args: &Value, key: &str) -> Result<Option<f64>, ToolError> {
    let Some(s) = opt_str(args, key)? else {
        return Ok(None);
    };
    super::write::parse_send_at(&s, &chrono::Local)
        .map(|d| Some(d.timestamp_millis() as f64 / 1000.0))
        .ok_or_else(|| failed(format!("Invalid {key} date: '{s}'")))
}

/// `sentAfter`/`sentBefore` as `metadata.list`'s exclusive `after`/`before` on whole seconds.
fn sent_bounds(args: &Value) -> Result<Value, ToolError> {
    let mut filter = json!({"draft": false});
    if let Some(a) = opt_date(args, "sentAfter")? {
        filter["after"] = Value::from(a.floor() as i64);
    }
    if let Some(b) = opt_date(args, "sentBefore")? {
        filter["before"] = Value::from(b.ceil() as i64);
    }
    Ok(filter)
}

/// The sent messages carrying any of `plugins`, once each, newest first, that the policy
/// lets through.
fn tracked_messages(
    ctx: &mut Ctx<'_>,
    tool: &str,
    args: &Value,
    plugins: &[&str],
) -> Result<Vec<Value>, ToolError> {
    let account_id = opt_str(args, "accountId")?;
    let filter = sent_bounds(args)?;
    let mut found: Vec<Value> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for account in ctx.accounts_for(account_id.as_deref())? {
        for plugin in plugins {
            for m in metadata_messages(ctx, tool, &account.account_id, plugin, filter.clone())? {
                if seen.insert(str_of(&m, "id").to_owned()) {
                    found.push(m);
                }
            }
        }
    }
    found.retain(|m| ctx.policy.message_allowed(m));
    found.sort_by(|a, b| {
        i64_of(b, "date")
            .cmp(&i64_of(a, "date"))
            .then_with(|| str_of(b, "id").cmp(str_of(a, "id")))
    });
    Ok(found)
}

fn opt_number(args: &Value, key: &str) -> Result<Option<f64>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(n.as_f64()),
        Some(_) => Err(failed(format!(
            "Invalid arguments: '{key}' must be a number"
        ))),
    }
}

fn timestamps(events: &Value) -> impl Iterator<Item = f64> + '_ {
    array_of(events, "events")
        .iter()
        .filter_map(|e| e.get("timestamp").and_then(Value::as_f64))
}

fn total_clicks(links: &Value) -> i64 {
    array_of(links, "links")
        .iter()
        .map(|l| i64_of(l, "clickCount"))
        .sum()
}

pub(super) fn list_tracked_messages(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let plugins: &[&str] = match opt_str(args, "trackingType")?.as_deref() {
        None | Some("either") => &[OPEN_TRACKING_ID, LINK_TRACKING_ID],
        Some("opens") => &[OPEN_TRACKING_ID],
        Some("links") => &[LINK_TRACKING_ID],
        Some(_) => {
            return Err(failed(
                "Invalid arguments: 'trackingType' must be one of opens, links, either",
            ))
        }
    };
    let opened_after = opt_date(args, "openedAfter")?;
    let min_opens = opt_number(args, "minOpens")?;
    let min_clicks = opt_number(args, "minClicks")?;
    let limit = opt_count(args, "limit", 50)?;
    let offset = opt_count(args, "offset", 0)?;
    let account_id = opt_str(args, "accountId")?;
    let messages = tracked_messages(ctx, "list_tracked_messages", args, plugins)?;

    let candidates: Vec<(Value, Value, Value)> = messages
        .into_iter()
        .map(|m| {
            let (opens, links) = tracking_summary(&m);
            (m, opens, links)
        })
        .filter(|(_, opens, links)| {
            if min_opens.is_some_and(|n| opens.is_null() || (i64_of(opens, "count") as f64) < n) {
                return false;
            }
            if min_clicks.is_some_and(|n| (total_clicks(links) as f64) < n) {
                return false;
            }
            if let Some(t) = opened_after {
                let open = timestamps(opens).any(|ts| ts >= t);
                let click = array_of(links, "links")
                    .iter()
                    .any(|l| timestamps(l).any(|ts| ts >= t));
                if !open && !click {
                    return false;
                }
            }
            true
        })
        .collect();
    let total = candidates.len();
    let page: Vec<Value> = candidates
        .iter()
        .skip(offset)
        .take(limit)
        .map(|(m, opens, links)| {
            json!({
                "id": str_of(m, "id"),
                "threadId": str_of(m, "threadId"),
                "subject": str_of(m, "subject"),
                "to": model::contacts(m, "to"),
                "date": model::iso_date(i64_of(m, "date")),
                "accountId": str_of(m, "aid"),
                "opens": opens,
                "links": links,
            })
        })
        .collect();
    let mut o = Output::text(&json!({"total": total, "messages": page}));
    if let Some(a) = account_id {
        o = o.with_account(&a);
    }
    Ok(o)
}

pub(super) fn get_top_links(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let limit = opt_count(args, "limit", 10)?;
    let account_id = opt_str(args, "accountId")?;
    let messages = tracked_messages(ctx, "get_top_links", args, &[LINK_TRACKING_ID])?;
    let mut rows: Vec<(i64, Value)> = Vec::new();
    for m in &messages {
        let (_, links) = tracking_summary(m);
        for l in array_of(&links, "links") {
            let recipients: std::collections::HashSet<String> = array_of(l, "events")
                .iter()
                .map(|e| e.get("recipient").map(Value::to_string).unwrap_or_default())
                .collect();
            let clicks = i64_of(l, "clickCount");
            rows.push((
                clicks,
                json!({
                    "url": l.get("url"),
                    "clickCount": clicks,
                    "uniqueRecipients": recipients.len(),
                    "messageId": str_of(m, "id"),
                    "threadId": str_of(m, "threadId"),
                    "subject": str_of(m, "subject"),
                    "date": model::iso_date(i64_of(m, "date")),
                    "accountId": str_of(m, "aid"),
                }),
            ));
        }
    }
    rows.sort_by_key(|(clicks, _)| std::cmp::Reverse(*clicks));
    let out: Vec<Value> = rows.into_iter().take(limit).map(|(_, r)| r).collect();
    let mut o = Output::text(&Value::Array(out));
    if let Some(a) = account_id {
        o = o.with_account(&a);
    }
    Ok(o)
}

pub(super) fn list_scheduled_sends(ctx: &mut Ctx<'_>, args: &Value) -> ToolResult {
    let account_id = opt_str(args, "accountId")?;
    let now = chrono::Utc::now().timestamp();
    // Failed sends first (no expiration), then by send time.
    let mut drafts: Vec<(i64, Value)> = Vec::new();
    for account in ctx.accounts_for(account_id.as_deref())? {
        // Without hasExpiration: a send the engine gave up on has none, only an error.
        let filter = json!({"draft": true});
        for d in metadata_messages(
            ctx,
            "list_scheduled_sends",
            &account.account_id,
            SEND_LATER_ID,
            filter,
        )? {
            if !ctx.policy.message_allowed(&d) {
                continue;
            }
            let Some(schedule) = model::scheduled_send(&d, now) else {
                continue;
            };
            let at = plugin_value(&d, SEND_LATER_ID)
                .and_then(|m| m.get("expiration"))
                .and_then(Value::as_i64)
                .unwrap_or(i64::MIN);
            let mut row = json!({
                "headerMessageId": str_of(&d, "hMsgId"),
                "threadId": str_of(&d, "threadId"),
                "subject": str_of(&d, "subject"),
                "to": model::contacts(&d, "to"),
                "accountId": str_of(&d, "aid"),
            });
            if let (Some(row), Value::Object(schedule)) = (row.as_object_mut(), schedule) {
                row.extend(schedule);
            }
            drafts.push((at, row));
        }
    }
    drafts.sort_by_key(|(at, _)| *at);
    let out: Vec<Value> = drafts.into_iter().map(|(_, row)| row).collect();
    let mut o = Output::text(&Value::Array(out));
    if let Some(a) = account_id {
        o = o.with_account(&a);
    }
    Ok(o)
}
