// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Body rendering: which parts make the body, how they are joined, and which become files.
//!
//! A port of mailcore2's `MCHTMLRenderer.cpp` as modified for Mailspring (Copyright ©
//! 2001-2013 MailCore team, BSD-3-Clause; modifications Copyright © 2017 Foundry 376,
//! GPL-3.0; see NOTICE.md, "mailcore2"), driven with the
//! `CleanHTMLBodyRendererTemplateCallback` of `MailProcessor.cpp`: no templates, no
//! tidying, every filter the identity. Rules:
//! - `multipart/alternative`: HTML, else plain, else the first part; a `text/calendar`
//!   alternative is rendered after it (Exchange invites).
//! - `multipart/mixed` (and signed): parts concatenated; a direct `text/html` child makes
//!   every text part render as HTML.
//! - `multipart/related`: part 0 is rendered, parts 1..n are inline attachments.
//! - `message/rfc822`: its main part.
//!
//! Like the C++ there are two passes over the tree: the first only records whether
//! attachments come before text (mailcore's mixed mode, which the clean callback ignores,
//! and whether a `multipart/mixed` has an HTML child); the second renders and collects.

use crate::charset::decode_text;
use crate::mime::{Part, PartKind};
use crate::text::html_message_content;

/// The rendered body and the parts that become files.
pub(crate) struct Rendering<'a> {
    /// HTML, or plain text when [`Rendering::plaintext`].
    pub body: String,
    /// No HTML part was rendered (mailcore's `PLAINTEXT:` prefix).
    pub plaintext: bool,
    /// Non-text parts reached while rendering, in order (`partAttachments`).
    pub attachments: Vec<&'a Part>,
    /// Parts 1..n of every rendered `multipart/related` (`htmlInlineAttachments`).
    pub related: Vec<&'a Part>,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum RenderState {
    None,
    HadAttachment,
    HadAttachmentThenText,
}

struct Context<'a> {
    first_rendered: bool,
    state: RenderState,
    /// 0 records the state machine only, 1 renders.
    pass: u8,
    has_text_html_part: bool,
    attachments: Option<Vec<&'a Part>>,
    related: Option<Vec<&'a Part>>,
}

/// `htmlForAbstractMessage`.
pub(crate) fn render(main: &Part) -> Rendering<'_> {
    let mut ctx = Context {
        first_rendered: false,
        state: RenderState::None,
        pass: 0,
        has_text_html_part: false,
        attachments: None,
        related: None,
    };
    let _ = render_part(main, &mut ctx);

    ctx.attachments = Some(Vec::new());
    ctx.related = Some(Vec::new());
    ctx.pass = 1;
    let content = render_part(main, &mut ctx).unwrap_or_default();

    Rendering {
        body: content,
        plaintext: !ctx.has_text_html_part,
        attachments: ctx.attachments.unwrap_or_default(),
        related: ctx.related.unwrap_or_default(),
    }
}

fn mime_is(part: &Part, mime: &str) -> bool {
    part.mime_type.as_deref() == Some(mime)
}

/// `isTextPart`. A named part after the first rendered text is an attachment unless it is
/// marked inline.
fn is_text_part(part: &Part, ctx: &Context<'_>) -> bool {
    if !part.inline && (part.attachment || (part.filename.is_some() && ctx.first_rendered)) {
        return false;
    }
    mime_is(part, "text/plain") || mime_is(part, "text/html")
}

/// `partContainsMimeType`.
fn contains_mime_type(part: &Part, mime: &str) -> bool {
    match part.kind {
        PartKind::Single => mime_is(part, mime),
        _ => part.parts.iter().any(|p| contains_mime_type(p, mime)),
    }
}

/// `preferredPartInMultipartAlternative`: the last part containing HTML, else the last
/// containing plain text, else the first.
fn preferred_alternative(part: &Part) -> Option<&Part> {
    let mut html = None;
    let mut text = None;
    for (i, sub) in part.parts.iter().enumerate() {
        if contains_mime_type(sub, "text/html") {
            html = Some(i);
        } else if contains_mime_type(sub, "text/plain") {
            text = Some(i);
        }
    }
    html.or(text)
        .or(if part.parts.is_empty() { None } else { Some(0) })
        .map(|i| &part.parts[i])
}

fn render_part<'a>(part: &'a Part, ctx: &mut Context<'a>) -> Option<String> {
    match part.kind {
        PartKind::Single => render_single(part, ctx),
        PartKind::Message => {
            if ctx.pass == 0 {
                return None;
            }
            part.parts.first().and_then(|main| render_part(main, ctx))
        }
        PartKind::Mixed | PartKind::Signed => render_mixed(part, ctx),
        PartKind::Related => render_related(part, ctx),
        PartKind::Alternative => render_alternative(part, ctx),
    }
}

fn render_single<'a>(part: &'a Part, ctx: &mut Context<'a>) -> Option<String> {
    if !is_text_part(part, ctx) {
        if ctx.pass == 0 {
            if ctx.state == RenderState::None {
                ctx.state = RenderState::HadAttachment;
            }
            return None;
        }
        if let Some(list) = ctx.attachments.as_mut() {
            list.push(part);
        }
        return Some(String::new());
    }

    if ctx.pass == 0 {
        if ctx.state == RenderState::HadAttachment {
            ctx.state = RenderState::HadAttachmentThenText;
        }
        return None;
    }

    if mime_is(part, "text/plain") {
        ctx.first_rendered = true;
        let s = decode_text(&part.data, part.charset.as_deref(), false);
        if ctx.has_text_html_part {
            return Some(html_message_content(&s));
        }
        Some(s)
    } else {
        ctx.has_text_html_part = true;
        let s = decode_text(&part.data, part.charset.as_deref(), true);
        ctx.first_rendered = true;
        Some(s)
    }
}

fn render_alternative<'a>(part: &'a Part, ctx: &mut Context<'a>) -> Option<String> {
    let Some(preferred) = preferred_alternative(part) else {
        return Some(String::new());
    };
    let calendar = part
        .parts
        .iter()
        .rev()
        .find(|p| contains_mime_type(p, "text/calendar"));

    let html = render_part(preferred, ctx)?;
    let mut result = html;
    if let Some(cal) = calendar {
        if let Some(s) = render_part(cal, ctx) {
            result.push_str(&s);
        }
    }
    Some(result)
}

fn render_mixed<'a>(part: &'a Part, ctx: &mut Context<'a>) -> Option<String> {
    if part.parts.iter().any(|p| mime_is(p, "text/html")) {
        ctx.has_text_html_part = true;
    }
    let mut result = String::new();
    for sub in &part.parts {
        let s = render_part(sub, ctx);
        if ctx.pass != 0 {
            result.push_str(&s?);
        }
    }
    Some(result)
}

fn render_related<'a>(part: &'a Part, ctx: &mut Context<'a>) -> Option<String> {
    let Some(root) = part.parts.first() else {
        return if ctx.pass == 0 {
            None
        } else {
            Some(String::new())
        };
    };
    if let Some(list) = ctx.related.as_mut() {
        for other in &part.parts[1..] {
            // The C++ added the subpart itself; a nested multipart contributes its leaves.
            list.extend(other.leaves());
        }
    }
    render_part(root, ctx)
}
