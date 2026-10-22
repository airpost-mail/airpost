// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Templates ([`feature::TEMPLATES`](crate::feature::TEMPLATES)): the desktop's
//! `composer-templates` files in `<config>/templates`, one `<name>.html` each with its subject
//! in a leading `<meta name="subject">`, read and written in the same format, so a template
//! made in either shows in both. `draft.create {template}` fills a draft from one.

use serde::{Deserialize, Serialize};

/// One template as `templates.list` lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    /// The name without the extension; what `template` params take.
    pub name: String,
    /// The file name, the desktop's template id.
    pub id: String,
    /// Empty when it has none.
    pub subject: String,
    /// The body as one line of text, cut short.
    pub preview: String,
    /// The tagged variables (`<span data-tvar="name">`), in order.
    pub variables: Vec<String>,
    /// HTML; `templates.get` and `templates.save` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplatesListResult {
    /// By name.
    pub templates: Vec<TemplateInfo>,
}

/// `templates.get` and `templates.delete`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateNameParams {
    /// A template's name or its file name.
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateResult {
    pub template: TemplateInfo,
}

/// `templates.save`: from `body` (and `subject`) under `name`, or from a draft as the
/// desktop's "Save as template" makes one: the text above its signature and quote, named
/// after its subject (or `name`), with its subject unless it is a reply or a forward. A name
/// already taken gets ` 1`, ` 2`, … unless `replace`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSaveParams {
    /// Letters, digits, spaces, `-` and `_`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// HTML.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The draft to save, with `messageId` (its id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    /// Write over the template of that name instead of saving under a new one.
    #[serde(default)]
    pub replace: bool,
}

/// One warning about a message (`warnings` on the Message JSON of `messages.get` and
/// `threads.get`, [`feature::MESSAGE_WARNINGS`](crate::feature::MESSAGE_WARNINGS)).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageWarning {
    /// `reply-to-mismatch`: replies go to another domain than the message came from (the
    /// desktop's phishing-detection plugin).
    pub code: String,
    /// The desktop's English text.
    pub message: String,
}
