// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern snooze`, `tern unsnooze` and `tern unsubscribe`:
//! the engine's `threads.snooze`, `threads.unsnooze` and `messages.unsubscribe`. Ids are
//! looked up in every account the command covers, so `--account` is only needed to narrow
//! the search.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde_json::{json, Value};
use tern_ipc::{
    feature, method, unsubscribe_method, MessagesUnsubscribeParams, MessagesUnsubscribeResult,
    ThreadsSnoozeResult,
};

use crate::app::{call, call_raw, is_not_found, Ctx};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output;

/// The hour a date without a time means: the desktop's "morning" (`date-utils.ts`).
const MORNING_HOUR: u32 = 9;

/// `--until`: `30m`, `2h`, `3d`, `1w` from now; `tomorrow` (9:00) or `tomorrow HH:MM`; a date
/// (9:00 that day); or a date and time, local unless it has an offset.
pub fn parse_until<Tz: TimeZone>(s: &str, now: DateTime<Tz>) -> Option<DateTime<Utc>> {
    let s = s.trim();
    let local = now.timezone();
    let at_morning = |d: NaiveDate| {
        local
            .from_local_datetime(&d.and_time(NaiveTime::from_hms_opt(MORNING_HOUR, 0, 0)?))
            .earliest()
            .map(|t| t.with_timezone(&Utc))
    };
    if s.eq_ignore_ascii_case("tomorrow") {
        return at_morning(now.date_naive().succ_opt()?);
    }
    if let Some(time) = s
        .get(..9)
        .filter(|p| p.eq_ignore_ascii_case("tomorrow "))
        .and_then(|_| NaiveTime::parse_from_str(s[9..].trim(), "%H:%M").ok())
    {
        return local
            .from_local_datetime(&now.date_naive().succ_opt()?.and_time(time))
            .earliest()
            .map(|t| t.with_timezone(&Utc));
    }
    let relative = s.strip_prefix('+').unwrap_or(s);
    if let Some(unit) = relative.chars().last().filter(char::is_ascii_alphabetic) {
        if let Ok(n) = relative[..relative.len() - 1].parse::<i64>() {
            let step = match unit.to_ascii_lowercase() {
                'm' => Duration::minutes(n),
                'h' => Duration::hours(n),
                'd' => Duration::days(n),
                'w' => Duration::weeks(n),
                _ => return None,
            };
            return Some((now + step).with_timezone(&Utc));
        }
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    for f in [
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            // A time skipped by a DST change doesn't exist; an ambiguous one takes the first.
            return local
                .from_local_datetime(&n)
                .earliest()
                .map(|t| t.with_timezone(&Utc));
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(at_morning)
}

/// The accounts holding each id, in the order first seen, found with `get` (`threads.get` or
/// `messages.get`).
pub(crate) fn by_account(
    engines: &mut [Engine],
    targets: &[Target],
    ids: &[String],
    get: &str,
    what: &str,
) -> Result<Vec<(Target, Vec<String>)>> {
    let mut groups: Vec<(Target, Vec<String>)> = Vec::new();
    for id in ids {
        let mut found = None;
        for t in targets {
            match call_raw::<Value>(
                engines,
                t,
                get,
                json!({ "accountId": t.account_id, "id": id }),
            ) {
                Ok(_) => {
                    found = Some(t.clone());
                    break;
                }
                Err(e) if is_not_found(&e) => {}
                Err(e) => return Err(CliError::call(&t.email, get, e)),
            }
        }
        let t = found.ok_or_else(|| {
            CliError::failed(format!(
                "no {what} {id} in {}",
                match targets {
                    [one] => one.email.as_str(),
                    _ => "any account",
                }
            ))
        })?;
        match groups
            .iter_mut()
            .find(|(g, _)| g.account_id == t.account_id)
        {
            Some((_, list)) => {
                if !list.contains(id) {
                    list.push(id.clone());
                }
            }
            None => groups.push((t, vec![id.clone()])),
        }
    }
    Ok(groups)
}

pub(crate) fn require(engines: &[Engine], t: &Target, feature: &str, what: &str) -> Result<()> {
    if engines[t.engine].has_feature(feature) {
        Ok(())
    } else {
        Err(CliError::failed(format!(
            "{}: this engine can't {what} (no {feature}); update Tern",
            t.email
        )))
    }
}

pub fn snooze(ctx: &Ctx, engines: &mut [Engine], ids: &[String], until: &str) -> Result<()> {
    let when = parse_until(until, Local::now()).ok_or_else(|| {
        CliError::usage(format!(
            "can't read --until {until:?}: give 30m, 2h, 3d, 1w, tomorrow, a date \
             (2026-11-02) or a date and time (2026-11-02T14:30)"
        ))
    })?;
    if when <= Utc::now() {
        return Err(CliError::usage(format!("--until {until:?} is in the past")));
    }
    run_snooze(ctx, engines, ids, Some(when))
}

pub fn unsnooze(ctx: &Ctx, engines: &mut [Engine], ids: &[String]) -> Result<()> {
    run_snooze(ctx, engines, ids, None)
}

fn run_snooze(
    ctx: &Ctx,
    engines: &mut [Engine],
    ids: &[String],
    until: Option<DateTime<Utc>>,
) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let groups = by_account(engines, &targets, ids, method::THREADS_GET, "thread")?;
    let mut out = Vec::new();
    for (t, thread_ids) in &groups {
        require(engines, t, feature::SNOOZE, "snooze")?;
        let (m, params) = match until {
            Some(u) => (
                method::THREADS_SNOOZE,
                json!({"accountId": t.account_id, "threadIds": thread_ids, "until": u.timestamp()}),
            ),
            None => (
                method::THREADS_UNSNOOZE,
                json!({"accountId": t.account_id, "threadIds": thread_ids}),
            ),
        };
        let r: ThreadsSnoozeResult = call(engines, t, m, params)?;
        out.push(
            json!({"accountId": t.account_id, "threadIds": thread_ids, "taskIds": r.task_ids}),
        );
    }
    if ctx.json {
        let mut doc = json!({ "accounts": out });
        if let Some(u) = until {
            doc["until"] = json!(u.timestamp());
        }
        output::json(&doc);
        return Ok(());
    }
    let n = groups.iter().map(|(_, l)| l.len()).sum::<usize>();
    let threads = format!("{n} thread{}", if n == 1 { "" } else { "s" });
    match until {
        Some(u) => output::line(&format!(
            "Snoozed {threads} until {}",
            output::full_date(u.timestamp())
        )),
        None => output::line(&format!("Unsnoozed {threads}")),
    }
    Ok(())
}

pub fn unsubscribe(
    ctx: &Ctx,
    engines: &mut [Engine],
    message_id: &str,
    method_name: Option<&str>,
) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let ids = [message_id.to_owned()];
    let groups = by_account(engines, &targets, &ids, method::MESSAGES_GET, "message")?;
    let t = &groups[0].0;
    require(engines, t, feature::UNSUBSCRIBE, "unsubscribe")?;
    let params = MessagesUnsubscribeParams {
        account_id: t.account_id.clone(),
        message_id: message_id.to_owned(),
        method: method_name.map(str::to_owned),
    };
    let params = serde_json::to_value(&params).unwrap_or_else(|_| json!({}));
    let r: MessagesUnsubscribeResult = call(engines, t, method::MESSAGES_UNSUBSCRIBE, params)?;
    if ctx.json {
        output::json(&serde_json::to_value(&r).unwrap_or(Value::Null));
    }
    // The URI comes from the list's mail: no control characters reach the terminal.
    let url = output::one_line(r.url.as_deref().unwrap_or_default());
    if !r.ok {
        return Err(CliError::failed(format!(
            "{}: unsubscribing by {} failed: {}",
            t.email,
            output::one_line(&r.method),
            output::one_line(r.error.as_deref().unwrap_or_default())
        )));
    }
    if !ctx.json {
        output::line(&match r.method.as_str() {
            unsubscribe_method::ONE_CLICK => "Unsubscribed (one-click).".to_owned(),
            unsubscribe_method::MAILTO => format!(
                "Sending the unsubscribe message ({}).",
                url.strip_prefix("mailto:").unwrap_or(&url)
            ),
            _ => format!("This list unsubscribes on the web; open {url}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn at(s: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(s).unwrap()
    }

    #[test]
    fn until_forms() {
        let now = at("2026-10-18T22:30:00+02:00");
        let p = |s: &str| parse_until(s, now).map(|t| t.to_rfc3339());
        assert_eq!(p("30m").unwrap(), "2026-10-18T21:00:00+00:00");
        assert_eq!(p("+2h").unwrap(), "2026-10-18T22:30:00+00:00");
        assert_eq!(p("3d").unwrap(), "2026-10-21T20:30:00+00:00");
        assert_eq!(p("1W").unwrap(), "2026-10-25T20:30:00+00:00");
        assert_eq!(p("tomorrow").unwrap(), "2026-10-19T07:00:00+00:00");
        assert_eq!(p("Tomorrow 14:30").unwrap(), "2026-10-19T12:30:00+00:00");
        assert!(p("tomorrow 25:00").is_none());
        assert_eq!(p("2026-11-02").unwrap(), "2026-11-02T07:00:00+00:00");
        assert_eq!(p("2026-11-02T14:30").unwrap(), "2026-11-02T12:30:00+00:00");
        assert_eq!(
            p("2026-11-02 14:30:15").unwrap(),
            "2026-11-02T12:30:15+00:00"
        );
        assert_eq!(
            p("2026-11-02T14:30:00Z").unwrap(),
            "2026-11-02T14:30:00+00:00"
        );
        for bad in ["", "soon", "5y", "h", "2026-13-01", "next week"] {
            assert!(p(bad).is_none(), "{bad}");
        }
    }
}
