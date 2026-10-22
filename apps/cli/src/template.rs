// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `tern template list|show|save|delete`: the engine's `templates.*` (serve only,
//! `templates`), the composer's templates in Preferences > Templates. `tern draft
//! new|reply|reply-all|forward --template` writes a draft from one.

use serde_json::{json, Value};
use tern_ipc::{feature, method, TemplateInfo, TemplateResult, TemplatesListResult};

use crate::app::{call, Ctx};
use crate::cli::{TemplateCommand, TemplateSaveArgs};
use crate::discovery::{self, Engine, Target};
use crate::error::{CliError, Result};
use crate::output::{self, Table};

pub fn run(ctx: &Ctx, engines: &mut [Engine], command: TemplateCommand) -> Result<()> {
    match command {
        TemplateCommand::List => list(ctx, engines),
        TemplateCommand::Show { name } => show(ctx, engines, &name),
        TemplateCommand::Save(args) => save(ctx, engines, &args),
        TemplateCommand::Delete { name } => delete(ctx, engines, &name),
    }
}

/// The engine that keeps the templates: any account of the serve engine.
fn templates_target(engines: &[Engine]) -> Result<Target> {
    discovery::targets(engines, None)?
        .into_iter()
        .find(|t| engines[t.engine].has_feature(feature::TEMPLATES))
        .ok_or_else(|| {
            CliError::failed(
                "the running engine can't read templates (no templates); update Tern \
                 (templates need its serve engine)",
            )
        })
}

fn list(ctx: &Ctx, engines: &mut [Engine]) -> Result<()> {
    let t = templates_target(engines)?;
    let r: TemplatesListResult = call(engines, &t, method::TEMPLATES_LIST, json!({}))?;
    if ctx.json {
        output::json(&json!({ "templates": r.templates }));
        return Ok(());
    }
    if r.templates.is_empty() {
        output::line(
            "No templates; make one in Tern's Preferences > Templates or with `tern template save`",
        );
        return Ok(());
    }
    let mut table = Table::new(vec!["NAME", "SUBJECT", "VARIABLES", "TEXT"], 3);
    for tpl in &r.templates {
        table.row(
            vec![
                tpl.name.clone(),
                tpl.subject.clone(),
                tpl.variables.join(", "),
                tpl.preview.clone(),
            ],
            false,
        );
    }
    table.print(&ctx.style);
    Ok(())
}

fn print_template(ctx: &Ctx, tpl: &TemplateInfo, verb: Option<&str>) {
    if let Some(verb) = verb {
        output::line(&format!("{verb} template {:?}", tpl.name));
        return;
    }
    for (name, value) in [
        ("Name:", tpl.name.as_str()),
        ("Subject:", tpl.subject.as_str()),
        ("Variables:", &tpl.variables.join(", ")),
    ] {
        if !value.is_empty() {
            output::line(&format!(
                "{} {value}",
                ctx.style.bold(&format!("{name:<10}"))
            ));
        }
    }
    output::line("");
    output::line(&output::body_text(tpl.body.as_deref().unwrap_or("")));
}

fn show(ctx: &Ctx, engines: &mut [Engine], name: &str) -> Result<()> {
    let t = templates_target(engines)?;
    let r: TemplateResult = call(engines, &t, method::TEMPLATES_GET, json!({ "name": name }))?;
    if ctx.json {
        output::json(&json!({ "template": r.template }));
    } else {
        print_template(ctx, &r.template, None);
    }
    Ok(())
}

fn save(ctx: &Ctx, engines: &mut [Engine], args: &TemplateSaveArgs) -> Result<()> {
    let mut params = json!({ "replace": args.replace });
    if let Some(n) = &args.name {
        params["name"] = json!(n);
    }
    if let Some(s) = &args.subject {
        params["subject"] = json!(s);
    }
    let t = match &args.from_draft {
        Some(id) => {
            let targets = discovery::targets(engines, ctx.account())?;
            let (t, _, draft_id) = crate::draft::find_draft(engines, &targets, id)?;
            crate::mail_actions::require(engines, &t, feature::TEMPLATES, "save templates")?;
            params["accountId"] = json!(t.account_id);
            params["messageId"] = json!(draft_id);
            t
        }
        None => {
            let text = crate::draft::read_body(args.body.as_ref(), args.body_file.as_ref())?
                .ok_or_else(|| {
                    CliError::usage(
                        "give the template's text with --body or --body-file, or --from-draft",
                    )
                })?;
            let html = if args.html {
                text
            } else {
                crate::draft::text_to_html(&text)
            };
            params["body"] = json!(tag_variables(&html));
            templates_target(engines)?
        }
    };
    let r: TemplateResult = call(engines, &t, method::TEMPLATES_SAVE, params)?;
    if ctx.json {
        output::json(&json!({ "template": r.template }));
    } else {
        print_template(ctx, &r.template, Some("Saved"));
    }
    Ok(())
}

fn delete(ctx: &Ctx, engines: &mut [Engine], name: &str) -> Result<()> {
    let t = templates_target(engines)?;
    call::<Value>(
        engines,
        &t,
        method::TEMPLATES_DELETE,
        json!({ "name": name }),
    )?;
    if ctx.json {
        output::json(&json!({ "name": name, "deleted": true }));
    } else {
        output::line(&format!("Deleted template {name:?}"));
    }
    Ok(())
}

/// `{{name}}` as the composer's tagged variable, which it makes of `{{name}}` typed in.
fn tag_variables(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else { break };
        let name = &after[..end];
        if name.is_empty() || name.contains(['{', '}', '<', '>', '"']) {
            out.push_str(&rest[..start + 2]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(&format!(
            "<span data-tvar=\"{name}\" class=\"template-variable\" title=\"{name}\">{name}</span>"
        ));
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braces_become_tagged_variables() {
        assert_eq!(
            tag_variables("Hi {{first name}}, {{}} {x}"),
            "Hi <span data-tvar=\"first name\" class=\"template-variable\" title=\"first name\">first name</span>, {{}} {x}"
        );
        assert_eq!(
            tag_variables("{{a}}{{b"),
            "<span data-tvar=\"a\" class=\"template-variable\" title=\"a\">a</span>{{b"
        );
    }
}
