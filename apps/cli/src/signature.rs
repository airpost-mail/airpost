// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern signature list|show|set|delete|default`: the `signatures` and `defaultSignatures`
//! settings, in the shape the desktop's `SignatureStore` writes them: `signatures` maps an id
//! to `{id, title, body, data}` (HTML body; `data` holds a signature template's fields, and no
//! `templateName` for a raw HTML one), `defaultSignatures` maps an address (an account's or an
//! alias's) to a signature id, or null once turned off. The composer, and the drafts the engine
//! writes, start with the default of the From address.

use std::path::PathBuf;

use serde_json::{json, Map, Value};
use tern_ipc::{feature, method, AccountListResult};

use crate::app::Ctx;
use crate::cli::SignatureCommand;
use crate::discovery::{self, Engine};
use crate::error::{CliError, Result};
use crate::output::{self, Table};
use crate::settings;

const SIGNATURES_KEY: &str = "signatures";
const DEFAULTS_KEY: &str = "defaultSignatures";

pub fn run(ctx: &Ctx, engines: &mut [Engine], command: SignatureCommand) -> Result<()> {
    match command {
        SignatureCommand::List => list(ctx, engines),
        SignatureCommand::Show { signature } => show(ctx, engines, &signature),
        SignatureCommand::Set {
            signature,
            title,
            body,
            body_file,
            html,
        } => set(
            ctx,
            engines,
            &signature,
            title.as_deref(),
            body.as_ref(),
            body_file.as_ref(),
            html,
        ),
        SignatureCommand::Delete { signature } => delete(ctx, engines, &signature),
        SignatureCommand::Default {
            signature,
            addresses,
        } => default(ctx, engines, &signature, &addresses),
    }
}

/// The two settings, read from the engine that keeps them.
struct Stored {
    engine: usize,
    signatures: Map<String, Value>,
    defaults: Map<String, Value>,
}

impl Stored {
    fn load(engines: &mut [Engine]) -> Result<Stored> {
        let engine = settings::engine_with(engines, feature::SETTINGS, "keep signatures")?;
        let mut values = settings::read(&mut engines[engine], &[SIGNATURES_KEY, DEFAULTS_KEY])?;
        let take = |v: Option<Value>| match v {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        Ok(Stored {
            engine,
            signatures: take(values.remove(SIGNATURES_KEY)),
            defaults: take(values.remove(DEFAULTS_KEY)),
        })
    }

    /// The id of the signature `want` names: its id, else its title in any case.
    fn find(&self, want: &str) -> Result<String> {
        if self.signatures.contains_key(want) {
            return Ok(want.to_owned());
        }
        let named: Vec<&String> = self
            .signatures
            .iter()
            .filter(|(_, s)| title_of(s).eq_ignore_ascii_case(want.trim()))
            .map(|(id, _)| id)
            .collect();
        match named.as_slice() {
            [one] => Ok((*one).clone()),
            [] => Err(CliError::failed(format!(
                "no signature {want}; `tern signature list` lists them"
            ))),
            many => Err(CliError::failed(format!(
                "several signatures are named {want} ({}); give an id",
                many.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    /// The addresses that start with signature `id`.
    fn used_by(&self, id: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .defaults
            .iter()
            .filter(|(_, s)| s.as_str() == Some(id))
            .map(|(a, _)| a.clone())
            .collect();
        v.sort();
        v
    }

    fn save(&self, engines: &mut [Engine], keys: &[&str]) -> Result<()> {
        let mut values = Map::new();
        for k in keys {
            let v = match *k {
                SIGNATURES_KEY => &self.signatures,
                _ => &self.defaults,
            };
            values.insert((*k).to_owned(), Value::Object(v.clone()));
        }
        settings::write(&mut engines[self.engine], values)
    }

    fn with_uses(&self, id: &str) -> Value {
        let mut s = self.signatures.get(id).cloned().unwrap_or(Value::Null);
        s["defaultFor"] = json!(self.used_by(id));
        s
    }
}

fn title_of(sig: &Value) -> &str {
    sig["title"].as_str().unwrap_or_default()
}

fn list(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let stored = Stored::load(engines)?;
    let mut ids: Vec<&String> = stored.signatures.keys().collect();
    ids.sort_by_key(|id| title_of(&stored.signatures[*id]).to_lowercase());
    if ctx.json {
        let list: Vec<Value> = ids.iter().map(|id| stored.with_uses(id)).collect();
        output::json(&json!({ "signatures": list, "defaults": stored.defaults }));
        return Ok(());
    }
    if ids.is_empty() {
        output::line(
            "No signatures; make one in Tern's Preferences > Signatures or with `tern signature \
             set NAME --body TEXT`",
        );
        return Ok(());
    }
    let mut table = Table::new(vec!["NAME", "DEFAULT FOR", "TEXT", "ID"], 2);
    for id in ids {
        let s = &stored.signatures[id];
        table.row(
            vec![
                output::one_line(title_of(s)),
                stored.used_by(id).join(", "),
                output::one_line(&output::html_to_text(
                    s["body"].as_str().unwrap_or_default(),
                )),
                id.clone(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

fn show(ctx: &Ctx, engines: &mut [Engine], want: &str) -> Result<()> {
    let stored = Stored::load(engines)?;
    let id = stored.find(want)?;
    if ctx.json {
        output::json(&json!({ "signature": stored.with_uses(&id) }));
        return Ok(());
    }
    let s = &stored.signatures[&id];
    let label = |l: &str| ctx.style.bold(&format!("{l:<12}"));
    output::line(&format!("{} {}", label("Name:"), title_of(s)));
    output::line(&format!("{} {id}", label("Id:")));
    let used = stored.used_by(&id);
    if !used.is_empty() {
        output::line(&format!("{} {}", label("Default for:"), used.join(", ")));
    }
    output::line("");
    output::line(&output::body_text(s["body"].as_str().unwrap_or_default()));
    Ok(())
}

fn set(
    ctx: &Ctx,
    engines: &mut [Engine],
    want: &str,
    title: Option<&str>,
    body: Option<&String>,
    body_file: Option<&PathBuf>,
    html: bool,
) -> Result<()> {
    let mut stored = Stored::load(engines)?;
    let text = crate::draft::read_body(body, body_file)?;
    let body = text.map(|t| {
        if html {
            t
        } else {
            crate::draft::text_to_html(&t)
        }
    });
    let existing = stored.find(want).ok();
    let (id, verb) = match existing {
        Some(id) => {
            if title.is_none() && body.is_none() {
                return Err(CliError::usage(
                    "nothing to change: give --body, --body-file or --title",
                ));
            }
            let sig = stored.signatures.get_mut(&id).expect("found");
            if let Some(t) = title {
                sig["title"] = json!(t);
            }
            if let Some(b) = body {
                sig["body"] = json!(b);
                // A signature template would render its fields over the new text.
                if let Some(data) = sig.get_mut("data").and_then(Value::as_object_mut) {
                    data.remove("templateName");
                }
            }
            (id, "Saved")
        }
        None => {
            let body = body.ok_or_else(|| {
                CliError::usage(format!(
                    "no signature {want} yet; give its text with --body or --body-file"
                ))
            })?;
            let id = tern_client::tasks::temp_id();
            let sig = json!({
                "id": id, "title": title.unwrap_or(want.trim()), "body": body, "data": {},
            });
            stored.signatures.insert(id.clone(), sig);
            (id, "Created")
        }
    };
    stored.save(engines, &[SIGNATURES_KEY])?;
    if ctx.json {
        output::json(&json!({ "signature": stored.with_uses(&id) }));
    } else {
        output::line(&format!(
            "{verb} signature {:?} ({id})",
            title_of(&stored.signatures[&id])
        ));
    }
    Ok(())
}

fn delete(ctx: &Ctx, engines: &mut [Engine], want: &str) -> Result<()> {
    let mut stored = Stored::load(engines)?;
    let id = stored.find(want)?;
    let used = stored.used_by(&id);
    let sig = stored.signatures.remove(&id).unwrap_or(Value::Null);
    for a in &used {
        stored.defaults.insert(a.clone(), Value::Null);
    }
    let keys: &[&str] = if used.is_empty() {
        &[SIGNATURES_KEY]
    } else {
        &[SIGNATURES_KEY, DEFAULTS_KEY]
    };
    stored.save(engines, keys)?;
    if ctx.json {
        output::json(&json!({ "id": id, "deleted": true, "defaultFor": used }));
    } else {
        output::line(&format!("Deleted signature {:?} ({id})", title_of(&sig)));
    }
    Ok(())
}

/// The addresses of the running accounts and of their aliases, as they are spelled there.
fn known_addresses(engines: &mut [Engine], ctx: &Ctx) -> Result<Vec<(String, String)>> {
    let targets = discovery::targets(engines, ctx.account())?;
    let mut out: Vec<(String, String)> = targets
        .iter()
        .map(|t| (t.email.clone(), t.email.clone()))
        .collect();
    let mut asked: Vec<usize> = Vec::new();
    for t in &targets {
        if asked.contains(&t.engine) {
            continue;
        }
        asked.push(t.engine);
        let Ok(list) = engines[t.engine]
            .client
            .call_as::<AccountListResult>(method::ACCOUNT_LIST, json!({}))
        else {
            continue;
        };
        for a in list.accounts {
            if !targets.iter().any(|x| x.account_id == a.account_id) {
                continue;
            }
            let aliases = a
                .account
                .as_ref()
                .and_then(|j| j["aliases"].as_array().cloned())
                .unwrap_or_default();
            for alias in aliases.iter().filter_map(Value::as_str) {
                if let Some(address) = alias_address(alias) {
                    out.push((address, a.email.clone()));
                }
            }
        }
    }
    Ok(out)
}

/// The address of an alias, `Name <address>`.
fn alias_address(alias: &str) -> Option<String> {
    let open = alias.rfind('<')?;
    let close = open + alias[open..].find('>')?;
    let address = alias[open + 1..close].trim();
    address.contains('@').then(|| address.to_owned())
}

fn default(ctx: &Ctx, engines: &mut [Engine], want: &str, addresses: &[String]) -> Result<()> {
    let mut stored = Stored::load(engines)?;
    let id = if want.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(stored.find(want)?)
    };
    let picked: Vec<String> = if addresses.is_empty() {
        vec![discovery::one_target(engines, ctx.account())?.email]
    } else {
        let known = known_addresses(engines, ctx)?;
        addresses
            .iter()
            .map(|a| {
                known
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(a.trim()))
                    .map(|(k, _)| k.clone())
                    .ok_or_else(|| {
                        CliError::failed(format!(
                            "{a} is neither an account's address nor an alias's; `tern accounts` \
                             lists the accounts"
                        ))
                    })
            })
            .collect::<Result<_>>()?
    };
    for a in &picked {
        stored.defaults.insert(a.clone(), json!(id));
    }
    stored.save(engines, &[DEFAULTS_KEY])?;
    if ctx.json {
        output::json(
            &json!({ "defaults": picked.iter().map(|a| (a.clone(), json!(id))).collect::<Map<_, _>>() }),
        );
        return Ok(());
    }
    for a in &picked {
        match &id {
            Some(id) => output::line(&format!(
                "{a} starts with signature {:?}",
                title_of(&stored.signatures[id])
            )),
            None => output::line(&format!("{a} starts without a signature")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored() -> Stored {
        let sigs = json!({
            "s1": {"id": "s1", "title": "Work", "body": "<b>W</b>", "data": {"templateName": "T"}},
            "s2": {"id": "s2", "title": "Home", "body": "H", "data": {}},
            "s3": {"id": "s3", "title": "home", "body": "h", "data": {}},
        });
        Stored {
            engine: 0,
            signatures: sigs.as_object().unwrap().clone(),
            defaults: json!({"a@x.test": "s1", "b@x.test": "s1", "c@x.test": null})
                .as_object()
                .unwrap()
                .clone(),
        }
    }

    #[test]
    fn signatures_are_found_by_id_or_title() {
        let s = stored();
        assert_eq!(s.find("s2").unwrap(), "s2");
        assert_eq!(s.find("WORK").unwrap(), "s1");
        let e = s.find("home").unwrap_err().to_string();
        assert!(e.contains("several") && e.contains("s2"), "{e}");
        assert!(s.find("nope").is_err());
        assert_eq!(s.used_by("s1"), ["a@x.test", "b@x.test"]);
        assert_eq!(s.with_uses("s2")["defaultFor"], json!([]));
    }

    #[test]
    fn alias_addresses() {
        assert_eq!(
            alias_address("Ann Example <ann@x.test>").as_deref(),
            Some("ann@x.test")
        );
        assert_eq!(alias_address("Ann"), None);
    }
}
