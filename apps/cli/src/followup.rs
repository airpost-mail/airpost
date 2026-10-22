// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern remind`, `tern tracking`, and the `--remind-in`, `--track-opens` and
//! `--track-links` flags of the draft commands. They write and read the metadata of the
//! desktop's send-reminders, open-tracking and link-tracking plugins
//! (`tern_client::metadata`), with the `SyncbackMetadataTask` the desktop queues, so Tern
//! shows them and the engine acts on them. Tracking is done by the engine when it sends,
//! through a connected Tern server; these only turn it on and read what came back.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use serde_json::{json, Value};
use tern_client::metadata::{self as meta, LINK_TRACKING, OPEN_TRACKING};
use tern_client::tasks as builders;
use tern_ipc::{feature, method};

use crate::app::{call, Ctx};
use crate::cli::FollowUpArgs;
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::mail_actions::{by_account, parse_until, require};
use crate::organize::{queue, stamp};
use crate::output::{self, Table};
use crate::read::{find_shown, Shown};

/// `metadata.list` page size.
const PAGE: usize = 200;
/// Messages read per account and plugin before giving up, as tern-mcp's listing tools do.
const SCAN_LIMIT: usize = 10_000;

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn int_of(v: &Value, key: &str) -> i64 {
    v[key]
        .as_i64()
        .or_else(|| v[key].as_f64().map(|f| f as i64))
        .unwrap_or(0)
}

fn is_draft(m: &Value) -> bool {
    m["draft"]
        .as_bool()
        .unwrap_or_else(|| m["draft"].as_i64().unwrap_or(0) != 0)
}

/// A time in the future, for `flag`.
pub fn future(flag: &str, s: &str) -> Result<i64> {
    let when = parse_until(s, Local::now()).ok_or_else(|| {
        CliError::usage(format!(
            "can't read {flag} {s:?}: give 30m, 2h, 3d, 1w, tomorrow, tomorrow 14:30, a date \
             (2026-11-02) or a date and time (2026-11-02T14:30)"
        ))
    })?;
    if when <= Utc::now() {
        return Err(CliError::usage(format!("{flag} {s:?} is in the past")));
    }
    Ok(when.timestamp())
}

/// `--since`: `3h`, `7d`, `2w` ago, or a date or date and time.
fn parse_since<Tz: TimeZone>(s: &str, now: DateTime<Tz>) -> Option<i64> {
    let s = s.trim();
    if let Some(unit) = s.chars().last().filter(char::is_ascii_alphabetic) {
        if let Ok(n) = s[..s.len() - 1].parse::<i64>() {
            let step = match unit.to_ascii_lowercase() {
                'm' => Duration::minutes(n),
                'h' => Duration::hours(n),
                'd' => Duration::days(n),
                'w' => Duration::weeks(n),
                _ => return None,
            };
            return Some((now - step).timestamp());
        }
    }
    parse_until(s, now).map(|t| t.timestamp())
}

fn since(s: Option<&str>) -> Result<Option<i64>> {
    s.map(|s| {
        parse_since(s, Local::now()).ok_or_else(|| {
            CliError::usage(format!(
                "can't read --since {s:?}: give 3h, 7d, 2w, a date (2026-11-02) or a date and \
                 time"
            ))
        })
    })
    .transpose()
}

/// What a draft command's `--remind-in`, `--track-opens` and `--track-links` ask for, read
/// before anything is written.
#[derive(Debug, Default)]
pub struct FollowUp {
    remind_at: Option<i64>,
    opens: bool,
    links: bool,
}

impl FollowUp {
    pub fn parse(args: &FollowUpArgs) -> Result<FollowUp> {
        Ok(FollowUp {
            remind_at: args
                .remind_in
                .as_deref()
                .map(|s| future("--remind-in", s))
                .transpose()?,
            opens: args.track_opens,
            links: args.track_links,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.remind_at.is_none() && !self.tracks()
    }

    pub fn tracks(&self) -> bool {
        self.opens || self.links
    }

    /// Refuses tracking the engine of `t` can't send.
    pub fn check(&self, engines: &[Engine], t: &Target) -> Result<()> {
        let e = &engines[t.engine];
        match meta::tracking_unavailable(
            &e.hello.features,
            e.hello.capabilities.as_ref(),
            self.opens,
            self.links,
        ) {
            Some(why) => Err(CliError::failed(format!("{}: {why}", t.email))),
            None => Ok(()),
        }
    }

    /// Writes the reminder and the tracking toggles onto the stored draft (its JSON, with
    /// `id`, `aid`, `hMsgId` and `plaintext`), one task each, as the composer's buttons
    /// do. The reminder rides on the draft as `thread:send-reminders` and moves to the
    /// thread when it is sent.
    pub fn apply(&self, engines: &mut [Engine], t: &Target, draft: &Value) -> Result<Vec<String>> {
        if self.tracks() && draft["plaintext"].as_bool() == Some(true) {
            return Err(CliError::failed(format!(
                "draft {} is plain text, and tracking needs an HTML draft; it was saved without \
                 tracking",
                str_of(draft, "id")
            )));
        }
        let hmid = str_of(draft, "hMsgId");
        let mut values = Vec::new();
        if let Some(at) = self.remind_at {
            values.push((meta::THREAD_SEND_REMINDERS, meta::draft_reminder(at, hmid)));
        }
        if self.opens {
            values.push((OPEN_TRACKING, meta::open_tracking_on()));
        }
        if self.links {
            values.push((LINK_TRACKING, meta::link_tracking_on()));
        }
        let mut ids = Vec::new();
        for (plugin, value) in values {
            let mut task = builders::syncback_message_metadata(draft, plugin, value, None);
            stamp(&mut task);
            ids.push(str_of(&task, "id").to_owned());
            queue(engines, t, &task)?;
        }
        Ok(ids)
    }

    /// The keys a draft command's `--json` output gains.
    pub fn extend_json(&self, out: &mut Value) {
        if let Some(at) = self.remind_at {
            out["remindAt"] = json!(at);
        }
        if self.tracks() {
            out["tracking"] = json!({"opens": self.opens, "links": self.links});
        }
    }

    pub fn print(&self) {
        if let Some(at) = self.remind_at {
            output::line(&format!(
                "Reminder: {}, if no one has replied",
                output::full_date(at)
            ));
        }
        let tracking: Vec<&str> = [(self.opens, "opens"), (self.links, "link clicks")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, n)| *n)
            .collect();
        if !tracking.is_empty() {
            output::line(&format!("Tracking: {}", tracking.join(", ")));
        }
    }
}

/// `tern remind`.
pub fn remind(
    ctx: &Ctx,
    engines: &mut [Engine],
    id: &str,
    when: Option<&str>,
    clear: bool,
) -> Result<()> {
    let at = match (when, clear) {
        (Some(w), false) => Some(future("--in", w)?),
        _ => None,
    };
    let targets = discovery::targets(engines, ctx.account())?;
    let (shown, t) = find_shown(engines, &targets, id, false)?;
    let (thread, messages, given) = match shown {
        Shown::Message(r) if is_draft(&r["message"]) => {
            let draft = &r["message"];
            let hmid = str_of(draft, "hMsgId");
            let value = match at {
                Some(at) => meta::draft_reminder(at, hmid),
                None => json!({"expiration": null, "sentHeaderMessageId": hmid}),
            };
            let mut task = builders::syncback_message_metadata(
                draft,
                meta::THREAD_SEND_REMINDERS,
                value,
                None,
            );
            stamp(&mut task);
            queue(engines, &t, &task)?;
            return done(
                ctx,
                &t,
                json!({"draftId": str_of(draft, "id"), "headerMessageId": hmid}),
                at,
                &task,
            );
        }
        Shown::Message(r) => {
            let thread_id = str_of(&r["message"], "threadId").to_owned();
            let rt: Value = call(
                engines,
                &t,
                method::THREADS_GET,
                json!({ "accountId": t.account_id, "id": thread_id }),
            )?;
            (
                rt["thread"].clone(),
                rt["messages"].clone(),
                Some(r["message"].clone()),
            )
        }
        Shown::Thread(r) => (r["thread"].clone(), r["messages"].clone(), None),
    };
    let from_me = |m: &Value| {
        !is_draft(m)
            && m["from"]
                .as_array()
                .and_then(|f| f.first())
                .is_some_and(|c| str_of(c, "email").eq_ignore_ascii_case(&t.email))
    };
    // The message the reminder answers: the one given when it is the user's, else the
    // user's newest in the thread.
    let sent = given.filter(|m| from_me(m)).or_else(|| {
        messages
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| from_me(m))
            .max_by_key(|m| int_of(m, "date"))
            .cloned()
    });
    let value = meta::thread_reminder(
        at,
        sent.as_ref().map(|m| str_of(m, "hMsgId")),
        int_of(&thread, "lmrt"),
    );
    let thread_id = str_of(&thread, "id");
    let mut task =
        builders::syncback_thread_metadata(&t.account_id, thread_id, meta::SEND_REMINDERS, value);
    stamp(&mut task);
    queue(engines, &t, &task)?;
    done(ctx, &t, json!({"threadId": thread_id}), at, &task)
}

fn done(ctx: &Ctx, t: &Target, mut out: Value, at: Option<i64>, task: &Value) -> Result<()> {
    if ctx.json {
        out["accountId"] = json!(t.account_id);
        out["remindAt"] = json!(at);
        out["taskId"] = task["id"].clone();
        output::json(&out);
        return Ok(());
    }
    let what = match out["threadId"].as_str() {
        Some(id) => format!("thread {id}"),
        None => format!("draft {}", str_of(&out, "draftId")),
    };
    output::line(&match at {
        Some(at) => format!(
            "Reminder set on {what}: {}, if no one has replied",
            output::full_date(at)
        ),
        None => format!("Reminder cleared on {what}"),
    });
    Ok(())
}

// -- tern tracking -------------------------------------------------------------------------

/// The messages of `t` with `plugin_id` metadata matching `filter`, through `metadata.list`.
fn metadata_messages(
    engines: &mut [Engine],
    t: &Target,
    plugin_id: &str,
    filter: &Value,
) -> Result<Vec<Value>> {
    require(engines, t, feature::METADATA_LIST, "list tracked mail")?;
    let mut out = Vec::new();
    loop {
        let mut params = filter.clone();
        params["accounts"] = json!([t.account_id]);
        params["pluginId"] = json!(plugin_id);
        params["limit"] = json!(PAGE);
        params["offset"] = json!(out.len());
        let page: Value = call(engines, t, method::METADATA_LIST, params)?;
        let models = page["models"].as_array().cloned().unwrap_or_default();
        let n = models.len();
        out.extend(models);
        if n < PAGE || out.len() >= SCAN_LIMIT {
            return Ok(out);
        }
    }
}

/// The sent messages tracking any of `plugins`, once each, newest first.
fn tracked(
    engines: &mut [Engine],
    targets: &[Target],
    plugins: &[&str],
    after: Option<i64>,
) -> Result<Vec<Value>> {
    let mut filter = json!({"draft": false});
    if let Some(a) = after {
        filter["after"] = json!(a);
    }
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for t in targets {
        for p in plugins {
            for m in metadata_messages(engines, t, p, &filter)? {
                if seen.insert(str_of(&m, "id").to_owned()) {
                    found.push(m);
                }
            }
        }
    }
    found.sort_by(|a, b| {
        int_of(b, "date")
            .cmp(&int_of(a, "date"))
            .then_with(|| str_of(b, "id").cmp(str_of(a, "id")))
    });
    Ok(found)
}

fn summary_json(m: &Value) -> Value {
    let (opens, links) = meta::tracking_summary(m);
    json!({
        "id": str_of(m, "id"),
        "threadId": str_of(m, "threadId"),
        "accountId": str_of(m, "aid"),
        "subject": str_of(m, "subject"),
        "to": m["to"],
        "date": int_of(m, "date"),
        "opens": opens,
        "links": links,
    })
}

fn clicks(links: &Value) -> i64 {
    links["links"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| int_of(l, "clickCount"))
        .sum()
}

/// `tern tracking <message-id>`.
pub fn show(ctx: &Ctx, engines: &mut [Engine], id: &str) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let ids = [id.to_owned()];
    let groups = by_account(engines, &targets, &ids, method::MESSAGES_GET, "message")?;
    let t = &groups[0].0;
    let r: Value = call(
        engines,
        t,
        method::MESSAGES_GET,
        json!({ "accountId": t.account_id, "id": id }),
    )?;
    let doc = summary_json(&r["message"]);
    if doc["opens"].is_null() && doc["links"].is_null() {
        return Err(CliError::failed(format!(
            "message {id} wasn't sent with tracking on"
        )));
    }
    if ctx.json {
        output::json(&doc);
        return Ok(());
    }
    output::line(&format!(
        "{} ({})",
        output::one_line(str_of(&doc, "subject")),
        output::full_date(int_of(&doc, "date"))
    ));
    let event = |e: &Value| {
        format!(
            "{} {}",
            output::full_date(int_of(e, "timestamp")),
            output::one_line(
                &e["recipient"]
                    .as_str()
                    .map_or_else(|| e["recipient"].to_string(), str::to_owned)
            )
        )
    };
    let opens = &doc["opens"];
    if !opens.is_null() {
        output::line(&format!("Opens: {}", int_of(opens, "count")));
        for e in opens["events"].as_array().into_iter().flatten() {
            output::line(&format!("  {}", event(e)));
        }
    }
    let links = &doc["links"];
    if !links.is_null() {
        output::line(&format!("Clicks: {}", clicks(links)));
        for l in links["links"].as_array().into_iter().flatten() {
            output::line(&format!(
                "  {} {}",
                int_of(l, "clickCount"),
                output::one_line(str_of(l, "url"))
            ));
            for e in l["events"].as_array().into_iter().flatten() {
                output::line(&format!("    {}", event(e)));
            }
        }
    }
    Ok(())
}

/// `tern tracking list`.
pub fn list(
    ctx: &Ctx,
    engines: &mut [Engine],
    kind: &str,
    since_arg: Option<&str>,
    limit: Option<usize>,
) -> Result<()> {
    let plugins: &[&str] = match kind {
        "opens" => &[OPEN_TRACKING],
        "links" => &[LINK_TRACKING],
        _ => &[OPEN_TRACKING, LINK_TRACKING],
    };
    let after = since(since_arg)?;
    let targets = discovery::targets(engines, ctx.account())?;
    let messages: Vec<Value> = tracked(engines, &targets, plugins, after)?
        .iter()
        .take(limit.unwrap_or(tern_ipc::READ_DEFAULT_LIMIT))
        .map(summary_json)
        .collect();
    if ctx.json {
        output::json(&json!({ "messages": messages }));
        return Ok(());
    }
    if messages.is_empty() {
        output::line("No tracked mail.");
        return Ok(());
    }
    let mut table = Table::new(vec!["DATE", "TO", "SUBJECT", "OPENS", "CLICKS", "ID"], 2);
    for m in &messages {
        let to = m["to"]
            .as_array()
            .map(|a| a.iter().map(output::contact_short).collect::<Vec<_>>())
            .unwrap_or_default()
            .join(", ");
        let opens = &m["opens"];
        table.row(
            vec![
                output::list_date(int_of(m, "date")),
                output::one_line(&to),
                output::one_line(str_of(m, "subject")),
                if opens.is_null() {
                    "-".into()
                } else {
                    int_of(opens, "count").to_string()
                },
                if m["links"].is_null() {
                    "-".into()
                } else {
                    clicks(&m["links"]).to_string()
                },
                str_of(m, "id").to_owned(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

/// `tern tracking links`: every tracked link, most clicked first.
pub fn links(
    ctx: &Ctx,
    engines: &mut [Engine],
    since_arg: Option<&str>,
    limit: Option<usize>,
) -> Result<()> {
    let after = since(since_arg)?;
    let targets = discovery::targets(engines, ctx.account())?;
    let mut rows: Vec<Value> = Vec::new();
    for m in tracked(engines, &targets, &[LINK_TRACKING], after)? {
        let (_, links) = meta::tracking_summary(&m);
        for l in links["links"].as_array().into_iter().flatten() {
            let recipients: HashSet<String> = l["events"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|e| e["recipient"].to_string())
                .collect();
            rows.push(json!({
                "url": l["url"],
                "clickCount": int_of(l, "clickCount"),
                "uniqueRecipients": recipients.len(),
                "messageId": str_of(&m, "id"),
                "threadId": str_of(&m, "threadId"),
                "subject": str_of(&m, "subject"),
                "date": int_of(&m, "date"),
                "accountId": str_of(&m, "aid"),
            }));
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(int_of(r, "clickCount")));
    rows.truncate(limit.unwrap_or(10));
    if ctx.json {
        output::json(&json!({ "links": rows }));
        return Ok(());
    }
    if rows.is_empty() {
        output::line("No tracked links.");
        return Ok(());
    }
    let mut table = Table::new(
        vec!["CLICKS", "RECIPIENTS", "URL", "SUBJECT", "MESSAGE ID"],
        2,
    );
    for r in &rows {
        table.row(
            vec![
                int_of(r, "clickCount").to_string(),
                int_of(r, "uniqueRecipients").to_string(),
                output::one_line(str_of(r, "url")),
                output::one_line(str_of(r, "subject")),
                str_of(r, "messageId").to_owned(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn since_counts_back() {
        let now = DateTime::<FixedOffset>::parse_from_rfc3339("2026-10-18T22:30:00+00:00").unwrap();
        let p = |s: &str| parse_since(s, now);
        assert_eq!(p("3h"), Some(now.timestamp() - 3 * 3600));
        assert_eq!(p("2w"), Some(now.timestamp() - 14 * 86_400));
        assert_eq!(p("2026-10-01T00:00:00Z"), Some(1_790_812_800));
        assert_eq!(p("soon"), None);
    }

    #[test]
    fn follow_up_values_are_the_plugins() {
        let f = FollowUp {
            remind_at: Some(1_900_000_000),
            opens: true,
            links: false,
        };
        let mut out = json!({});
        f.extend_json(&mut out);
        assert_eq!(
            out,
            json!({"remindAt": 1_900_000_000, "tracking": {"opens": true, "links": false}})
        );
        assert!(FollowUp::default().is_empty());
    }
}
