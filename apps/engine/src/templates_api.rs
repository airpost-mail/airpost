// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! `templates.list|get|save|delete` (`feature::TEMPLATES`, `serve` only): the desktop's
//! `composer-templates` files in `<config>/templates`, in its format
//! (`tern_compose::templates`), answered on the connection's thread. The desktop watches the
//! folder, so what is saved here shows there at once.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tern_compose::templates::{self, Template, TEMPLATES_DIR};
use tern_ipc::{
    code, method, ErrorObject, Request as RpcRequest, TemplateInfo, TemplateNameParams,
    TemplateResult, TemplateSaveParams, TemplatesListResult,
};

use crate::dispatch::Reply;
use crate::socket::Session;

/// How long `preview` runs, in characters.
const PREVIEW_CHARS: usize = 120;

pub fn is_template_method(name: &str) -> bool {
    matches!(
        name,
        method::TEMPLATES_LIST
            | method::TEMPLATES_GET
            | method::TEMPLATES_SAVE
            | method::TEMPLATES_DELETE
    )
}

fn invalid(message: impl Into<String>) -> ErrorObject {
    ErrorObject::new(
        code::INVALID_PARAMS,
        format!("invalid params: {}", message.into()),
    )
}

fn internal(e: impl std::fmt::Display) -> ErrorObject {
    ErrorObject::new(code::INTERNAL_ERROR, e.to_string())
}

fn no_template(name: &str) -> ErrorObject {
    ErrorObject::new(
        code::NOT_FOUND,
        format!("no template {name:?}; templates.list lists them"),
    )
}

fn dir(config_dir: &Path) -> PathBuf {
    config_dir.join(TEMPLATES_DIR)
}

/// A template file as it is listed.
struct Entry {
    info: TemplateInfo,
    template: Template,
    path: PathBuf,
}

fn info(id: &str, name: &str, template: &Template) -> TemplateInfo {
    TemplateInfo {
        name: name.to_owned(),
        id: id.to_owned(),
        subject: template.subject.clone(),
        preview: templates::preview(&template.body, PREVIEW_CHARS),
        variables: templates::variables(&template.body),
        body: None,
    }
}

/// Every template, by name; none when the folder doesn't exist. Folders and unreadable files
/// are left out, which the desktop lists but can't use.
fn entries(config_dir: &Path) -> Result<Vec<Entry>, ErrorObject> {
    let read = match std::fs::read_dir(dir(config_dir)) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(internal(format!("can't read the templates folder: {e}"))),
    };
    let mut out: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_owned();
            let name = templates::name_of_file(&id)?.to_owned();
            if !e.file_type().ok()?.is_file() {
                return None;
            }
            let bytes = std::fs::read(e.path()).ok()?;
            let template = templates::parse(&String::from_utf8_lossy(&bytes));
            Some(Entry {
                info: info(&id, &name, &template),
                template,
                path: e.path(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        (a.info.name.to_lowercase(), &a.info.id).cmp(&(b.info.name.to_lowercase(), &b.info.id))
    });
    Ok(out)
}

/// The template `name` names: its file name, else its name.
fn find(config_dir: &Path, name: &str) -> Result<Entry, ErrorObject> {
    let mut all = entries(config_dir)?;
    let at = all
        .iter()
        .position(|e| e.info.id == name)
        .or_else(|| all.iter().position(|e| e.info.name == name))
        .ok_or_else(|| no_template(name))?;
    Ok(all.swap_remove(at))
}

/// The template `draft.create {template}` names.
pub fn load(config_dir: &Path, name: &str) -> Result<Template, ErrorObject> {
    find(config_dir, name).map(|e| e.template)
}

fn with_body(mut info: TemplateInfo, template: &Template) -> TemplateInfo {
    info.body = Some(template.body.clone());
    info
}

fn to_value<T: serde::Serialize>(v: T) -> Reply {
    serde_json::to_value(v).map_err(internal)
}

pub fn call(session: &mut Session, req: &RpcRequest) -> Reply {
    let config_dir = session.server.config_dir.clone();
    match req.method.as_str() {
        method::TEMPLATES_LIST => to_value(TemplatesListResult {
            templates: entries(&config_dir)?.into_iter().map(|e| e.info).collect(),
        }),
        method::TEMPLATES_GET => {
            let p = req.params_as::<TemplateNameParams>()?;
            let e = find(&config_dir, &p.name)?;
            to_value(TemplateResult {
                template: with_body(e.info, &e.template),
            })
        }
        method::TEMPLATES_DELETE => {
            let p = req.params_as::<TemplateNameParams>()?;
            let e = find(&config_dir, &p.name)?;
            std::fs::remove_file(&e.path).map_err(internal)?;
            Ok(serde_json::json!({}))
        }
        method::TEMPLATES_SAVE => {
            let p = req.params_as::<TemplateSaveParams>()?;
            let (name, template) = match (&p.account_id, &p.message_id) {
                (Some(account_id), Some(id)) => from_draft(session, account_id, id, &p)?,
                (None, None) => from_params(&p)?,
                _ => return Err(invalid("a draft takes both accountId and messageId")),
            };
            save(&config_dir, &name, &template, p.replace)
        }
        _ => Err(ErrorObject::new(
            code::METHOD_NOT_FOUND,
            "not a templates method",
        )),
    }
}

/// `_onCreateTemplate` with a name and contents.
fn from_params(p: &TemplateSaveParams) -> Result<(String, Template), ErrorObject> {
    let name = p.name.clone().unwrap_or_default();
    if name.is_empty() {
        return Err(invalid("You must provide a name for your template."));
    }
    let body = p.body.clone().unwrap_or_default();
    if body.is_empty() {
        return Err(invalid("You must provide contents for your template."));
    }
    Ok((
        name,
        Template {
            subject: p.subject.clone().unwrap_or_default().trim().to_owned(),
            body,
        },
    ))
}

/// `_onCreateTemplateFromDraft`.
fn from_draft(
    session: &mut Session,
    account_id: &str,
    id: &str,
    p: &TemplateSaveParams,
) -> Result<(String, Template), ErrorObject> {
    session.server.accounts.check(account_id)?;
    let draft = session
        .reads
        .message(account_id, id)?
        .filter(|m| {
            m.get("draft")
                .is_some_and(|d| d.as_bool() == Some(true) || d.as_i64() == Some(1))
        })
        .ok_or_else(|| ErrorObject::new(code::NOT_FOUND, format!("no draft {id}")))?;
    let body = session.reads.body(id)?.ok_or_else(|| {
        ErrorObject::new(
            code::NOT_FOUND,
            format!("the body of draft {id} isn't downloaded yet; fetch it (bodies.fetch) and try again"),
        )
    })?;
    let subject = draft
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    let name = match &p.name {
        Some(n) => n.clone(),
        None => templates::name_from_subject(&subject),
    };
    if name.is_empty() {
        return Err(invalid("Give your draft a subject to name your template."));
    }
    let contents = templates::body_from_draft(&body);
    if contents.is_empty() {
        return Err(invalid(
            "To create a template you need to fill the body of the current draft.",
        ));
    }
    // A "Re:" or "Fwd:" subject belongs to the thread the draft answers.
    let subject = match &p.subject {
        Some(s) => s.trim().to_owned(),
        None if templates::answers_a_message(&draft) => String::new(),
        None => subject,
    };
    Ok((
        name,
        Template {
            subject,
            body: contents,
        },
    ))
}

/// `saveNewTemplate` (a taken name gets a number), or `saveTemplate` over the one of that
/// name with `replace`.
fn save(config_dir: &Path, name: &str, template: &Template, replace: bool) -> Reply {
    if let Some(e) = templates::name_error(name) {
        return Err(invalid(e));
    }
    let existing = entries(config_dir)?;
    let (name, id) = match existing.iter().find(|e| e.info.name == name) {
        Some(e) if replace => (name.to_owned(), e.info.id.clone()),
        _ => {
            let name = templates::unused_name(name, |n| existing.iter().any(|e| e.info.name == n));
            let id = templates::file_name(&name);
            (name, id)
        }
    };
    let folder = dir(config_dir);
    std::fs::create_dir_all(&folder).map_err(internal)?;
    std::fs::write(folder.join(&id), templates::stringify(template)).map_err(internal)?;
    to_value(TemplateResult {
        template: with_body(info(&id, &name, template), template),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(dir: &Path, name: &str, subject: &str, body: &str, replace: bool) -> Value {
        let t = Template {
            subject: subject.into(),
            body: body.into(),
        };
        save(dir, name, &t, replace).unwrap()["template"].clone()
    }

    #[test]
    fn saves_lists_and_finds_templates_in_the_desktop_format() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert!(entries(dir).unwrap().is_empty());

        let t = saved(
            dir,
            "Follow up",
            "Checking in",
            "<div>Hi <span data-tvar=\"who\">who</span></div>",
            false,
        );
        assert_eq!(t["id"], "Follow up.html");
        assert_eq!(t["variables"], serde_json::json!(["who"]));
        assert_eq!(
            std::fs::read_to_string(dir.join("templates/Follow up.html")).unwrap(),
            "<meta name=\"subject\" content=\"Checking in\"/>\n<div>Hi <span data-tvar=\"who\">who</span></div>"
        );
        // A taken name gets a number, unless replaced.
        assert_eq!(
            saved(dir, "Follow up", "", "<p>2</p>", false)["name"],
            "Follow up 1"
        );
        assert_eq!(
            saved(dir, "Follow up", "", "<p>3</p>", true)["name"],
            "Follow up"
        );
        assert_eq!(load(dir, "Follow up").unwrap().body, "<p>3</p>");
        assert_eq!(load(dir, "Follow up 1.html").unwrap().body, "<p>2</p>");

        // What the desktop leaves there: its own files, dot files, a folder.
        std::fs::write(dir.join("templates/Old.html"), "<p>no subject</p>").unwrap();
        std::fs::write(dir.join("templates/.DS_Store"), "x").unwrap();
        std::fs::create_dir(dir.join("templates/sub")).unwrap();
        let names: Vec<String> = entries(dir)
            .unwrap()
            .into_iter()
            .map(|e| e.info.name)
            .collect();
        assert_eq!(names, ["Follow up", "Follow up 1", "Old"]);
        assert_eq!(load(dir, "Old").unwrap().subject, "");

        assert!(save(dir, "../x", &Template::default(), false).is_err());
        assert_eq!(load(dir, "nope").unwrap_err().code, code::NOT_FOUND);
    }
}
