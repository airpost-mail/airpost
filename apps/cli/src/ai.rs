// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern ai summarize|triage|usage` (docs/ai-plan.md section 7): the engine's `ai.summarize`
//! with its `ai.delta` stream, `ai.triage.run` with its `ai.triage.progress`, and `ai.usage`.
//! The CLI sends no provider config: the engine uses the providers and switches it keeps
//! (`core.ai.*`, set in Preferences > AI).

use std::io::IsTerminal;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ms_ipc::{
    encode, feature, method, AiDeltaParams, AiTriageProgressParams, AiTriageRunResult, AiUsageRow,
    ClientError, Id, Message, Request, SettingsGetParams, SettingsValues,
};
use serde_json::{json, Value};
use tern_client::CALL_TIMEOUT;

use crate::app::{call, Ctx};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::mail_actions::{by_account, require};
use crate::output::{self, Table};

/// Longer than the engine's 120 s deadline for one model call, so the engine's error
/// arrives first.
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(150);
const DEFAULT_SINCE_DAYS: u32 = 7;
const AI_ACCOUNTS_KEY: &str = "core.ai.accounts";

pub fn summarize(ctx: &Ctx, engines: &mut [Engine], thread: &str, refresh: bool) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let groups = by_account(
        engines,
        &targets,
        &[thread.to_owned()],
        method::THREADS_GET,
        "thread",
    )?;
    let (t, _) = &groups[0];
    require(engines, t, feature::AI, "summarize")?;
    let stream_id = stream_id();
    let params = json!({
        "accountId": t.account_id,
        "threadId": thread,
        "streamId": stream_id,
        "refresh": refresh,
    });
    let mut streamed = false;
    let result = streaming_call(
        &mut engines[t.engine],
        method::AI_SUMMARIZE,
        params,
        Some(SUMMARY_TIMEOUT),
        |n| {
            if n.method != method::AI_DELTA || ctx.json {
                return;
            }
            if let Ok(d) = serde_json::from_value::<AiDeltaParams>(n.params.clone()) {
                if d.stream_id == stream_id {
                    streamed = true;
                    output::text(&output::plain_text(&d.text));
                }
            }
        },
    )
    .map_err(|e| CliError::call(&t.email, method::AI_SUMMARIZE, e))?;
    if ctx.json {
        output::json(&result);
        return Ok(());
    }
    if streamed {
        output::line("");
    } else {
        output::line(&output::plain_text(
            result["text"].as_str().unwrap_or_default(),
        ));
    }
    let model = output::one_line(result["model"].as_str().unwrap_or("the model"));
    let cached = if result["cached"] == true {
        ", cached"
    } else {
        ""
    };
    output::line(&ctx.style.dim(&format!("({model}{cached})")));
    Ok(())
}

pub fn triage(
    ctx: &Ctx,
    engines: &mut [Engine],
    since: Option<&str>,
    threads: &[String],
) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let jobs: Vec<(Target, Value)> = if threads.is_empty() {
        let days = match since {
            Some(s) => parse_days(s).ok_or_else(|| {
                CliError::usage(format!(
                    "can't read --since {s:?}: give 7d, 2w or a number of days"
                ))
            })?,
            None => DEFAULT_SINCE_DAYS,
        };
        triage_accounts(ctx, engines, targets)?
            .into_iter()
            .map(|t| {
                let params = json!({"accountId": t.account_id, "sinceDays": days});
                (t, params)
            })
            .collect()
    } else {
        by_account(engines, &targets, threads, method::THREADS_GET, "thread")?
            .into_iter()
            .map(|(t, ids)| {
                let params = json!({"accountId": t.account_id, "threadIds": ids});
                (t, params)
            })
            .collect()
    };
    let live = !ctx.json && std::io::stdout().is_terminal();
    let mut out = Vec::new();
    for (t, params) in jobs {
        require(engines, &t, feature::AI_TRIAGE, "triage")?;
        let (job, done, stopped) = run_job(&mut engines[t.engine], params, |done, total| {
            if live {
                output::text(&format!("\r{}: {done}/{total}", t.email));
            }
        })
        .map_err(|e| CliError::call(&t.email, method::AI_TRIAGE_RUN, e))?;
        if live && job.queued > 0 {
            output::text("\r");
        }
        if stopped {
            return Err(CliError::failed(format!(
                "{}: triage stopped after {done} of {} threads: the account stopped, the job \
                 was cancelled or triage was turned off",
                t.email, job.queued
            )));
        }
        if !ctx.json {
            let n = job.queued;
            output::line(&format!(
                "{}: triaged {done} of {n} thread{}",
                t.email,
                if n == 1 { "" } else { "s" }
            ));
        }
        out.push(json!({
            "accountId": t.account_id,
            "jobId": job.job_id,
            "queued": job.queued,
            "done": done,
        }));
    }
    if ctx.json {
        output::json(&json!({ "accounts": out }));
    }
    Ok(())
}

/// The targets to triage the inbox of: all given ones when `--account` picked them, else
/// the ones turned on for AI in the engine's `core.ai.accounts`.
fn triage_accounts(ctx: &Ctx, engines: &mut [Engine], targets: Vec<Target>) -> Result<Vec<Target>> {
    if ctx.account().is_some() {
        return Ok(targets);
    }
    let mut on: Vec<String> = Vec::new();
    for e in engines
        .iter_mut()
        .filter(|e| e.has_feature(feature::AI_TRIAGE))
    {
        let pid = e.entry.pid;
        let params = SettingsGetParams {
            keys: Some(vec![AI_ACCOUNTS_KEY.to_owned()]),
        };
        let v = e
            .client
            .call(method::SETTINGS_GET, json!(params))
            .map_err(|err| {
                CliError::call(&format!("engine pid {pid}"), method::SETTINGS_GET, err)
            })?;
        let values: SettingsValues = serde_json::from_value(v)
            .map_err(|err| CliError::failed(format!("odd settings.get result: {err}")))?;
        if let Some(Value::Array(ids)) = values.values.get(AI_ACCOUNTS_KEY) {
            on.extend(ids.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    let picked: Vec<Target> = targets
        .into_iter()
        .filter(|t| on.contains(&t.account_id))
        .collect();
    if picked.is_empty() {
        return Err(CliError::failed(
            "no account has AI turned on; turn it on in Tern's Preferences > AI",
        ));
    }
    Ok(picked)
}

/// Runs one `ai.triage.run` and waits for its last `ai.triage.progress`: the job, how many
/// threads it got through and whether it stopped early. No read timeout: a rate-limited
/// provider pauses the job for minutes without a word; the engine closing the connection
/// ends the wait with an error.
fn run_job(
    engine: &mut Engine,
    params: Value,
    mut progress: impl FnMut(usize, usize),
) -> std::result::Result<(AiTriageRunResult, usize, bool), ClientError> {
    let r = streaming_call(engine, method::AI_TRIAGE_RUN, params, None, |_| {})?;
    let job: AiTriageRunResult =
        serde_json::from_value(r).map_err(|e| ClientError::Protocol(e.to_string()))?;
    if job.queued == 0 {
        return Ok((job, 0, false));
    }
    let mut done = 0;
    let mut stopped = false;
    with_timeout(engine, None, |engine| loop {
        let n = engine.client.next_notification()?;
        if n.method != method::AI_TRIAGE_PROGRESS {
            continue;
        }
        let Ok(p) = serde_json::from_value::<AiTriageProgressParams>(n.params) else {
            continue;
        };
        if p.job_id == job.job_id {
            done = p.done;
            if p.stopped {
                stopped = true;
                return Ok(());
            }
            progress(p.done, p.total);
            if p.done >= p.total {
                return Ok(());
            }
        }
    })?;
    Ok((job, done, stopped))
}

pub fn usage(ctx: &Ctx, engines: &mut [Engine], days: Option<u32>) -> Result<()> {
    let targets = discovery::targets(engines, ctx.account())?;
    let t = targets
        .iter()
        .find(|t| engines[t.engine].has_feature(feature::AI))
        .ok_or_else(|| CliError::failed("the running engine has no AI; update Tern"))?
        .clone();
    let params = match days {
        Some(d) => json!({ "days": d }),
        None => json!({}),
    };
    let rows: Vec<AiUsageRow> = call(engines, &t, method::AI_USAGE, params)?;
    if ctx.json {
        output::json(&json!(rows));
        return Ok(());
    }
    if rows.is_empty() {
        output::line("No AI calls yet");
        return Ok(());
    }
    let mut table = Table::new(
        vec!["DAY", "PROVIDER", "FEATURE", "REQUESTS", "INPUT", "OUTPUT"],
        1,
    );
    for r in &rows {
        table.row(
            vec![
                r.day.clone(),
                r.provider.clone(),
                r.feature.clone(),
                r.requests.to_string(),
                r.input_tokens.to_string(),
                r.output_tokens.to_string(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

/// Sends `method` and reads until its response, handing each notification on the way to
/// `on_note`. `timeout` replaces the read timeout for the call.
fn streaming_call(
    engine: &mut Engine,
    method: &str,
    params: Value,
    timeout: Option<Duration>,
    mut on_note: impl FnMut(&ms_ipc::Notification),
) -> std::result::Result<Value, ClientError> {
    // String ids never meet the client's own numbered ones.
    let id = Id::Str(stream_id());
    engine
        .client
        .send_raw(&encode(&Request::new(id.clone(), method, params)))?;
    with_timeout(engine, timeout, |engine| loop {
        match engine.client.read_message()? {
            Message::Response(r) if r.id.as_ref() == Some(&id) => {
                return r.into_result().map_err(ClientError::Rpc)
            }
            Message::Response(r) => {
                return Err(ClientError::Protocol(format!(
                    "a response to {:?} while waiting for {id:?}",
                    r.id
                )))
            }
            Message::Notification(n) => {
                on_note(&n);
            }
            Message::Request(_) => {}
        }
    })
}

fn with_timeout<T>(
    engine: &mut Engine,
    timeout: Option<Duration>,
    f: impl FnOnce(&mut Engine) -> std::result::Result<T, ClientError>,
) -> std::result::Result<T, ClientError> {
    engine.client.set_read_timeout(timeout)?;
    let r = f(engine);
    engine.client.set_read_timeout(Some(CALL_TIMEOUT))?;
    r
}

fn stream_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("tern-cli-{}-{nanos}", std::process::id())
}

/// `--since`: `7d`, `2w`, or a number of days.
fn parse_days(s: &str) -> Option<u32> {
    let s = s.trim();
    let (n, unit) = match s.chars().last()? {
        'd' | 'D' => (&s[..s.len() - 1], 1),
        'w' | 'W' => (&s[..s.len() - 1], 7),
        _ => (s, 1),
    };
    let n: u32 = n.parse().ok()?;
    (n > 0).then_some(n.checked_mul(unit)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn since_reads_days_and_weeks() {
        assert_eq!(parse_days("7d"), Some(7));
        assert_eq!(parse_days("2w"), Some(14));
        assert_eq!(parse_days("10"), Some(10));
        assert_eq!(parse_days("0d"), None);
        assert_eq!(parse_days("3h"), None);
        assert_eq!(parse_days("d"), None);
    }
}
