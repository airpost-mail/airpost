// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The read commands over the engine's read API. Without `--account`
//! they ask every account and merge: threads and search results by last message time,
//! contacts by how often they were referenced.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use tern_ipc::{
    code, feature, method, ClientError, SearchResult, READ_DEFAULT_LIMIT, READ_MAX_LIMIT,
};

use crate::app::{call, call_raw, is_not_found, Ctx};
use crate::cli::ThreadsArgs;
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output::{self, Table};

/// How long `show --body` waits for a body it asked the engine to download.
const BODY_WAIT: Duration = Duration::from_secs(30);
const BODY_POLL: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Label,
}

impl Kind {
    fn method(self) -> &'static str {
        match self {
            Kind::Folder => method::FOLDERS_LIST,
            Kind::Label => method::LABELS_LIST,
        }
    }

    /// The result's list key, and the noun in messages.
    fn key(self) -> &'static str {
        match self {
            Kind::Folder => "folders",
            Kind::Label => "labels",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Kind::Folder => "folder",
            Kind::Label => "label",
        }
    }
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn int_of(v: &Value, key: &str) -> i64 {
    v[key]
        .as_i64()
        .or_else(|| v[key].as_bool().map(i64::from))
        .unwrap_or(0)
}

fn account_params(t: &Target) -> Value {
    json!({ "accountId": t.account_id })
}

/// The categories of one account: the list and its counts.
fn list_categories(
    engines: &mut [Engine],
    t: &Target,
    kind: Kind,
) -> Result<(Vec<Value>, Map<String, Value>)> {
    let mut r: Value = call(engines, t, kind.method(), account_params(t))?;
    let list = r[kind.key()]
        .as_array_mut()
        .map(std::mem::take)
        .unwrap_or_default();
    let counts = r["counts"]
        .as_object_mut()
        .map(std::mem::take)
        .unwrap_or_default();
    Ok((list, counts))
}

pub fn categories(ctx: &Ctx, engines: &mut [Engine], kind: Kind) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let mut all = Vec::new();
    let mut counts = Map::new();
    let mut rows = Vec::new();
    for t in &targets {
        let (list, c) = list_categories(engines, t, kind)?;
        for v in &list {
            rows.push((t.email.clone(), v.clone(), c.get(str_of(v, "id")).cloned()));
        }
        all.extend(list);
        counts.extend(c);
    }
    if ctx.json {
        let mut doc = Map::new();
        doc.insert(kind.key().into(), Value::Array(all));
        doc.insert("counts".into(), Value::Object(counts));
        output::json(&Value::Object(doc));
        return Ok(());
    }
    let multi = targets.len() > 1;
    let mut headers = vec!["PATH", "ROLE", "UNREAD", "TOTAL", "ID"];
    if multi {
        headers.insert(0, "ACCOUNT");
    }
    let mut table = Table::new(headers, usize::from(multi));
    for (email, v, c) in rows {
        let c = c.unwrap_or(Value::Null);
        let unread = int_of(&c, "unread");
        let mut cells = vec![
            str_of(&v, "path").to_owned(),
            str_of(&v, "role").to_owned(),
            unread.to_string(),
            int_of(&c, "total").to_string(),
            str_of(&v, "id").to_owned(),
        ];
        if multi {
            cells.insert(0, email);
        }
        table.row(cells, unread > 0);
    }
    if table.is_empty() {
        output::line(&format!("No {}s.", kind.noun()));
    } else {
        table.print(&ctx.style);
    }
    Ok(())
}

/// A folder or label of `t` named by id or path (paths compare without case, so `inbox`
/// finds `INBOX`). None when the account has no such category.
fn find_category(
    engines: &mut [Engine],
    t: &Target,
    kind: Kind,
    name: &str,
) -> Result<Option<String>> {
    let (list, _) = list_categories(engines, t, kind)?;
    let by_id = list.iter().find(|v| str_of(v, "id") == name);
    let by_path = || {
        list.iter()
            .find(|v| str_of(v, "path").eq_ignore_ascii_case(name))
    };
    Ok(by_id.or_else(by_path).map(|v| str_of(v, "id").to_owned()))
}

/// Threads newest first: last message time, then id, both descending, as `threads.list`
/// orders them.
pub fn sort_threads(threads: &mut [(Value, usize)]) {
    threads.sort_by_key(|(t, _)| Reverse((int_of(t, "lmt"), str_of(t, "id").to_owned())));
}

pub fn threads(ctx: &Ctx, engines: &mut [Engine], args: &ThreadsArgs) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    if args.before.is_some() && targets.len() > 1 {
        return Err(CliError::usage(
            "--before continues one account's listing; pick the account with --account",
        ));
    }
    let limit = args.limit.unwrap_or(if args.all {
        READ_MAX_LIMIT
    } else {
        READ_DEFAULT_LIMIT
    });
    let mut found: Vec<(Value, usize)> = Vec::new();
    let mut next: Option<String> = None;
    let mut more = false;
    let mut missing: Option<CliError> = None;
    let mut asked = 0;
    for (i, t) in targets.iter().enumerate() {
        let mut params = json!({ "accountId": t.account_id, "limit": limit });
        let category = match (&args.folder, &args.label) {
            (Some(f), _) => Some((Kind::Folder, f)),
            (_, Some(l)) => Some((Kind::Label, l)),
            _ => None,
        };
        if let Some((kind, name)) = category {
            match find_category(engines, t, kind, name)? {
                Some(id) => {
                    params[if kind == Kind::Folder {
                        "folderId"
                    } else {
                        "labelId"
                    }] = json!(id)
                }
                None => {
                    missing = Some(CliError::failed(format!(
                        "{}: no {} {name}; `tern {}` lists them",
                        t.email,
                        kind.noun(),
                        kind.key()
                    )));
                    continue;
                }
            }
        }
        if let Some(role) = &args.role {
            params["role"] = json!(role);
        }
        if let Some(before) = &args.before {
            params["before"] = json!(before);
        }
        asked += 1;
        loop {
            let r: std::result::Result<Value, ClientError> =
                call_raw(engines, t, method::THREADS_LIST, params.clone());
            let mut r = match r {
                Ok(r) => r,
                // A role this account has no folder or label for.
                Err(e) if is_not_found(&e) && targets.len() > 1 => {
                    asked -= 1;
                    missing = Some(CliError::call(&t.email, method::THREADS_LIST, e));
                    break;
                }
                Err(e) => return Err(CliError::call(&t.email, method::THREADS_LIST, e)),
            };
            let page = r["threads"]
                .as_array_mut()
                .map(std::mem::take)
                .unwrap_or_default();
            found.extend(page.into_iter().map(|v| (v, i)));
            let cursor = r["next"].as_str().map(str::to_owned);
            match cursor {
                Some(c) if args.all => params["before"] = json!(c),
                _ => {
                    more |= cursor.is_some();
                    next = cursor;
                    break;
                }
            }
        }
    }
    if asked == 0 {
        return Err(missing.unwrap_or_else(|| CliError::failed("no account to list")));
    }
    sort_threads(&mut found);
    if !args.all && found.len() > limit {
        found.truncate(limit);
        more = true;
    }
    // A cursor pages one account; merged pages have none, `more` still says there are more.
    let next = if targets.len() == 1 { next } else { None };
    if ctx.json {
        let list: Vec<Value> = found.into_iter().map(|(v, _)| v).collect();
        output::json(&json!({ "threads": list, "next": next, "more": more }));
        return Ok(());
    }
    print_threads(ctx, &targets, &found);
    if more && !args.all {
        output::line(
            &ctx.style
                .dim("More threads: --all pages through them, -n shows more."),
        );
    }
    Ok(())
}

/// The first participant's name and how many more there are, leaving out the account's
/// own address unless it is the only one.
pub fn participants(thread: &Value, own_email: &str) -> String {
    let all: Vec<&Value> = thread["participants"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let others: Vec<&Value> = all
        .iter()
        .copied()
        .filter(|p| {
            !p["email"]
                .as_str()
                .unwrap_or("")
                .eq_ignore_ascii_case(own_email)
        })
        .collect();
    let list = if others.is_empty() { all } else { others };
    match list.as_slice() {
        [] => String::new(),
        [one] => output::contact_short(one),
        [first, rest @ ..] => format!("{} +{}", output::contact_short(first), rest.len()),
    }
}

fn print_threads(ctx: &Ctx, targets: &[Target], threads: &[(Value, usize)]) {
    if threads.is_empty() {
        output::line("No threads.");
        return;
    }
    let multi = targets.len() > 1;
    let mut headers = vec!["DATE", "FROM", "SUBJECT", "ID"];
    if multi {
        headers.insert(0, "ACCOUNT");
    }
    let mut table = Table::new(headers, 2 + usize::from(multi));
    for (t, i) in threads {
        let mut subject = String::new();
        if int_of(t, "starred") > 0 {
            subject.push_str("* ");
        }
        subject.push_str(str_of(t, "subject"));
        let mut cells = vec![
            output::list_date(int_of(t, "lmt")),
            output::truncate(&participants(t, &targets[*i].email), 24),
            subject,
            str_of(t, "id").to_owned(),
        ];
        if multi {
            cells.insert(0, targets[*i].email.clone());
        }
        table.row(cells, int_of(t, "unread") > 0);
    }
    table.print(&ctx.style);
}

/// Engines with the search grammar take the query as typed (`from:`, `in:`, `OR`, dates in
/// the local zone) and every account they hold in one call; older ones get it as plain
/// words, account by account.
pub fn search(ctx: &Ctx, engines: &mut [Engine], query: &str, limit: Option<usize>) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let mut found = Vec::new();
    let mut asked = vec![false; targets.len()];
    for (i, t) in targets.iter().enumerate() {
        if asked[i] {
            continue;
        }
        let page: Vec<Value> = if engines[t.engine].has_feature(feature::SEARCH_GRAMMAR) {
            let on_engine: Vec<usize> = (i..targets.len())
                .filter(|&j| targets[j].engine == t.engine)
                .collect();
            let mut params = tern_client::search::grammar_search(query, limit);
            params.accounts = Some(
                on_engine
                    .iter()
                    .map(|&j| targets[j].account_id.clone())
                    .collect(),
            );
            for &j in &on_engine {
                asked[j] = true;
            }
            let params = serde_json::to_value(params).expect("params");
            match call_raw::<SearchResult>(engines, t, method::SEARCH, params) {
                Ok(r) => r.threads,
                // The query doesn't parse, or names a date the engine doesn't read.
                Err(ClientError::Rpc(e)) if e.code == code::INVALID_PARAMS => {
                    let why = e.message.trim_start_matches("invalid params: search: ");
                    return Err(CliError::usage(format!("search: {why}")));
                }
                Err(e) => return Err(CliError::call(&t.email, method::SEARCH, e)),
            }
        } else {
            asked[i] = true;
            let mut params = json!({ "accountId": t.account_id, "query": query });
            if let Some(n) = limit {
                params["limit"] = json!(n);
            }
            let r: SearchResult = call(engines, t, method::SEARCH, params)?;
            r.threads
        };
        for thread in page {
            let owner = targets
                .iter()
                .position(|x| x.engine == t.engine && x.account_id == str_of(&thread, "aid"))
                .unwrap_or(i);
            found.push((thread, owner));
        }
    }
    sort_threads(&mut found);
    found.truncate(limit.unwrap_or(READ_DEFAULT_LIMIT).clamp(1, READ_MAX_LIMIT));
    if ctx.json {
        let list: Vec<Value> = found.into_iter().map(|(v, _)| v).collect();
        output::json(&json!({ "threads": list }));
    } else {
        print_threads(ctx, &targets, &found);
    }
    Ok(())
}

pub fn contacts(
    ctx: &Ctx,
    engines: &mut [Engine],
    query: &str,
    limit: Option<usize>,
) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let mut found: Vec<(Value, usize)> = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        let mut params = json!({ "accountId": t.account_id, "query": query });
        if let Some(n) = limit {
            params["limit"] = json!(n);
        }
        let mut r: Value = call(engines, t, method::CONTACTS_SEARCH, params)?;
        let page = r["contacts"]
            .as_array_mut()
            .map(std::mem::take)
            .unwrap_or_default();
        found.extend(page.into_iter().map(|v| (v, i)));
    }
    found.sort_by_key(|(c, _)| Reverse(int_of(c, "refs")));
    found.truncate(limit.unwrap_or(READ_DEFAULT_LIMIT).clamp(1, READ_MAX_LIMIT));
    if ctx.json {
        let list: Vec<Value> = found.into_iter().map(|(v, _)| v).collect();
        output::json(&json!({ "contacts": list }));
        return Ok(());
    }
    if found.is_empty() {
        output::line("No contacts.");
        return Ok(());
    }
    let multi = targets.len() > 1;
    let mut headers = vec!["NAME", "EMAIL", "REFS"];
    if multi {
        headers.push("ACCOUNT");
    }
    let mut table = Table::new(headers, 0);
    for (c, i) in &found {
        let mut cells = vec![
            str_of(c, "name").to_owned(),
            str_of(c, "email").to_owned(),
            int_of(c, "refs").to_string(),
        ];
        if multi {
            cells.push(targets[*i].email.clone());
        }
        table.row(cells, false);
    }
    table.print(&ctx.style);
    Ok(())
}

enum Shown {
    Message(Value),
    Thread(Value),
}

/// Looks `id` up as a message, then as a thread, in each account until one has it.
fn find_shown(
    engines: &mut [Engine],
    targets: &[Target],
    id: &str,
    body: bool,
) -> Result<(Shown, Target)> {
    for t in targets {
        let params = json!({ "accountId": t.account_id, "id": id, "body": body });
        match call_raw::<Value>(engines, t, method::MESSAGES_GET, params) {
            Ok(r) => return Ok((Shown::Message(r), t.clone())),
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(CliError::call(&t.email, method::MESSAGES_GET, e)),
        }
        let params = json!({ "accountId": t.account_id, "id": id });
        match call_raw::<Value>(engines, t, method::THREADS_GET, params) {
            Ok(r) => return Ok((Shown::Thread(r), t.clone())),
            Err(e) if is_not_found(&e) => {}
            Err(e) => return Err(CliError::call(&t.email, method::THREADS_GET, e)),
        }
    }
    let scope = match targets {
        [one] => one.email.clone(),
        _ => "any account".to_owned(),
    };
    Err(CliError::failed(format!(
        "no message or thread {id} in {scope}; `tern threads` lists thread ids, and \
         `tern show <thread-id>` the message ids"
    )))
}

pub fn show(ctx: &Ctx, engines: &mut [Engine], id: &str, body: bool) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let (shown, t) = find_shown(engines, &targets, id, body)?;
    match shown {
        Shown::Thread(r) => {
            if ctx.json {
                output::json(&with_kind(r, "thread"));
            } else {
                print_thread(ctx, &r);
            }
            Ok(())
        }
        Shown::Message(mut r) => {
            let mut late = None;
            if body && r["body"].is_null() {
                match wait_for_body(engines, &t, id) {
                    Ok(Some(b)) => r["body"] = json!(b),
                    Ok(None) => {
                        late = Some(CliError::failed(format!(
                            "the body of {id} hasn't downloaded after {} s; check the account \
                             is online with `tern status`",
                            BODY_WAIT.as_secs()
                        )))
                    }
                    Err(e) => return Err(e),
                }
            }
            if ctx.json {
                output::json(&with_kind(r, "message"));
            } else {
                let paths = folder_paths(engines, &t);
                print_message(ctx, &r, &paths, body);
            }
            late.map_or(Ok(()), Err)
        }
    }
}

/// `show --json` says which of the two it found.
fn with_kind(mut r: Value, kind: &str) -> Value {
    if let Some(obj) = r.as_object_mut() {
        obj.insert("kind".into(), json!(kind));
    }
    r
}

/// Asks the engine for the body, then polls `messages.get` until it is stored.
fn wait_for_body(engines: &mut [Engine], t: &Target, id: &str) -> Result<Option<String>> {
    call::<Value>(
        engines,
        t,
        method::BODIES_FETCH,
        json!({ "accountId": t.account_id, "ids": [id] }),
    )?;
    let deadline = Instant::now() + BODY_WAIT;
    while Instant::now() < deadline {
        std::thread::sleep(BODY_POLL);
        let r: Value = call(
            engines,
            t,
            method::MESSAGES_GET,
            json!({ "accountId": t.account_id, "id": id, "body": true }),
        )?;
        if let Some(b) = r["body"].as_str() {
            return Ok(Some(b.to_owned()));
        }
    }
    Ok(None)
}

/// Folder id to path, for naming a message's folders. Empty when the list fails.
fn folder_paths(engines: &mut [Engine], t: &Target) -> BTreeMap<String, String> {
    list_categories(engines, t, Kind::Folder)
        .map(|(list, _)| {
            list.iter()
                .map(|f| (str_of(f, "id").to_owned(), str_of(f, "path").to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn header(ctx: &Ctx, name: &str, value: &str) {
    if !value.is_empty() {
        output::line(&format!(
            "{} {}",
            ctx.style.bold(&format!("{name:<8}")),
            value
        ));
    }
}

fn print_message(ctx: &Ctx, r: &Value, paths: &BTreeMap<String, String>, want_body: bool) {
    let m = &r["message"];
    header(ctx, "Subject:", &output::one_line(str_of(m, "subject")));
    header(ctx, "From:", &output::contacts(&m["from"]));
    header(ctx, "To:", &output::contacts(&m["to"]));
    header(ctx, "Cc:", &output::contacts(&m["cc"]));
    header(ctx, "Date:", &output::full_date(int_of(m, "date")));
    let folders: Vec<&str> = m["folders"]
        .as_object()
        .map(|f| {
            f.keys()
                .map(|id| paths.get(id).map_or(id.as_str(), String::as_str))
                .collect()
        })
        .unwrap_or_default();
    header(ctx, "Folders:", &folders.join(", "));
    let labels: Vec<&str> = m["labels"]
        .as_array()
        .map(|l| l.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    header(ctx, "Labels:", &labels.join(", "));
    let mut flags = Vec::new();
    if m["unread"].as_bool() == Some(true) {
        flags.push("unread");
    }
    if m["starred"].as_bool() == Some(true) {
        flags.push("starred");
    }
    if m["draft"].as_bool() == Some(true) {
        flags.push("draft");
    }
    header(ctx, "Flags:", &flags.join(", "));
    let files: Vec<String> = r["files"]
        .as_array()
        .map(|f| {
            f.iter()
                .map(|f| {
                    let name = str_of(f, "filename");
                    let name = if name.is_empty() { "(unnamed)" } else { name };
                    match f["size"].as_u64() {
                        Some(n) => format!("{name} ({})", output::bytes(n)),
                        None => name.to_owned(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    header(ctx, "Files:", &files.join(", "));
    header(ctx, "Thread:", str_of(m, "threadId"));
    header(ctx, "Id:", str_of(m, "id"));
    output::line("");
    match r["body"].as_str() {
        Some(b) => output::line(&output::body_text(b)),
        None => {
            output::line(&output::one_line(str_of(m, "snippet")));
            if !want_body {
                output::line(&ctx.style.dim("(snippet; --body shows the whole message)"));
            }
        }
    }
}

fn print_thread(ctx: &Ctx, r: &Value) {
    let t = &r["thread"];
    let messages = r["messages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    header(ctx, "Subject:", &output::one_line(str_of(t, "subject")));
    header(ctx, "With:", &output::contacts(&t["participants"]));
    let in_categories: Vec<&str> = ["folders", "labels"]
        .iter()
        .flat_map(|k| t[*k].as_array().into_iter().flatten())
        .map(|c| str_of(c, "path"))
        .filter(|p| !p.is_empty())
        .collect();
    header(ctx, "In:", &in_categories.join(", "));
    header(ctx, "Thread:", str_of(t, "id"));
    output::line("");
    let mut table = Table::new(vec!["DATE", "FROM", "SNIPPET", "MESSAGE ID"], 2);
    for m in messages {
        let from = m["from"]
            .as_array()
            .and_then(|a| a.first())
            .map(output::contact_short);
        table.row(
            vec![
                output::list_date(int_of(m, "date")),
                output::truncate(&from.unwrap_or_default(), 24),
                str_of(m, "snippet").to_owned(),
                str_of(m, "id").to_owned(),
            ],
            m["unread"].as_bool() == Some(true),
        );
    }
    table.print(&ctx.style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_merge_newest_first_then_by_id() {
        let mut list = vec![
            (json!({"id": "a", "lmt": 100}), 0),
            (json!({"id": "b", "lmt": 300}), 1),
            (json!({"id": "c", "lmt": 100}), 1),
            (json!({"id": "d"}), 0),
        ];
        sort_threads(&mut list);
        let ids: Vec<&str> = list.iter().map(|(t, _)| str_of(t, "id")).collect();
        assert_eq!(ids, ["b", "c", "a", "d"]);
    }

    #[test]
    fn participants_name_the_first_and_count_the_rest() {
        assert_eq!(participants(&json!({}), "me@x"), "");
        let me_and_ann = json!({"participants": [{"name": "Me", "email": "ME@x"},
                                                 {"name": "Ann", "email": "a@x"}]});
        assert_eq!(participants(&me_and_ann, "me@x"), "Ann");
        assert_eq!(participants(&me_and_ann, "other@x"), "Me +1");
        let only_me = json!({"participants": [{"name": "Me", "email": "me@x"}]});
        assert_eq!(participants(&only_me, "me@x"), "Me");
        let three = json!({"participants": [{"email": "a@x"}, {"email": "b@x"}, {"email": "c@x"}]});
        assert_eq!(participants(&three, "me@x"), "a@x +2");
    }

    #[test]
    fn flags_read_as_numbers_or_booleans() {
        assert_eq!(int_of(&json!({"unread": 2}), "unread"), 2);
        assert_eq!(int_of(&json!({"unread": true}), "unread"), 1);
        assert_eq!(int_of(&json!({}), "unread"), 0);
    }
}
