// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Contacts as the desktop's `Contact` model reads and prints them (`contact.ts`), the address
//! parsing of `ContactStore.parseContactsInString` and `Contact.fromString`, and the address
//! equivalence of `utils.ts` (`toEquivalentEmailForm`, `emailIsEquivalent`).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{json, Value};

use crate::data::DESKTOP;

/// `RegExpUtils` `UnicodeEmailChars`.
const UNICODE_EMAIL_CHARS: &str = "\u{00C0}-\u{00FF}\u{0100}-\u{017F}\u{0180}-\u{024F}\u{0250}-\u{02AF}\u{0300}-\u{036F}\u{0370}-\u{03FF}\u{0400}-\u{04FF}\u{0500}-\u{052F}\u{0530}-\u{058F}\u{0590}-\u{05FF}\u{0600}-\u{06FF}\u{0700}-\u{074F}\u{0750}-\u{077F}\u{0780}-\u{07BF}\u{07C0}-\u{07FF}\u{0900}-\u{097F}\u{0980}-\u{09FF}\u{0A00}-\u{0A7F}\u{0A80}-\u{0AFF}\u{0B00}-\u{0B7F}\u{0B80}-\u{0BFF}\u{0C00}-\u{0C7F}\u{0C80}-\u{0CFF}\u{0D00}-\u{0D7F}\u{0D80}-\u{0DFF}\u{0E00}-\u{0E7F}\u{0E80}-\u{0EFF}\u{0F00}-\u{0FFF}\u{1000}-\u{109F}\u{10A0}-\u{10FF}\u{1100}-\u{11FF}\u{1200}-\u{137F}\u{1380}-\u{139F}\u{13A0}-\u{13FF}\u{1400}-\u{167F}\u{1680}-\u{169F}\u{16A0}-\u{16FF}\u{1700}-\u{171F}\u{1720}-\u{173F}\u{1740}-\u{175F}\u{1760}-\u{177F}\u{1780}-\u{17FF}\u{1800}-\u{18AF}\u{1900}-\u{194F}\u{1950}-\u{197F}\u{1980}-\u{19DF}\u{19E0}-\u{19FF}\u{1A00}-\u{1A1F}\u{1B00}-\u{1B7F}\u{1D00}-\u{1D7F}\u{1D80}-\u{1DBF}\u{1DC0}-\u{1DFF}\u{1E00}-\u{1EFF}\u{1F00}-\u{1FFF}\u{20D0}-\u{20FF}\u{2100}-\u{214F}\u{2C00}-\u{2C5F}\u{2C60}-\u{2C7F}\u{2C80}-\u{2CFF}\u{2D00}-\u{2D2F}\u{2D30}-\u{2D7F}\u{2D80}-\u{2DDF}\u{2F00}-\u{2FDF}\u{2FF0}-\u{2FFF}\u{3040}-\u{309F}\u{30A0}-\u{30FF}\u{3100}-\u{312F}\u{3130}-\u{318F}\u{3190}-\u{319F}\u{31C0}-\u{31EF}\u{31F0}-\u{31FF}\u{3200}-\u{32FF}\u{3300}-\u{33FF}\u{3400}-\u{4DBF}\u{4DC0}-\u{4DFF}\u{4E00}-\u{9FFF}\u{A000}-\u{A48F}\u{A490}-\u{A4CF}\u{A700}-\u{A71F}\u{A800}-\u{A82F}\u{A840}-\u{A87F}\u{AC00}-\u{D7AF}\u{F900}-\u{FAFF}";

/// `RegExpUtils.emailRegex()`.
pub static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    let u = UNICODE_EMAIL_CHARS;
    Regex::new(&format!(
        r"[a-z.A-Z{u}0-9!#$%&'*+\-/=?^_`{{|}}~]+@[A-Za-z{u}0-9.\-]+\.[A-Za-z]{{2,63}}"
    ))
    .expect("email regex")
});

static NON_HUMAN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        "(?i)({})",
        DESKTOP.non_human_email_terms.join("|")
    ))
    .expect("not-a-person patterns")
});

/// A name and address, as a draft's `from`, `to`, `cc` and `bcc` hold them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub name: String,
    pub email: String,
}

impl Contact {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Contact {
        Contact {
            name: name.into(),
            email: email.into().trim().to_owned(),
        }
    }

    pub fn from_json(v: &Value) -> Contact {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        Contact::new(s("name"), s("email"))
    }

    pub fn list_from_json(v: &Value) -> Vec<Contact> {
        v.as_array()
            .map(|a| a.iter().map(Contact::from_json).collect())
            .unwrap_or_default()
    }

    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "email": self.email })
    }

    pub fn list_to_json(list: &[Contact]) -> Value {
        Value::Array(list.iter().map(Contact::to_json).collect())
    }

    /// `Contact.toString()`: `Name <email>`, or the address alone when the name is empty or
    /// is the address.
    pub fn display(&self) -> String {
        if !self.name.is_empty() && self.name != self.email {
            format!("{} <{}>", self.name, self.email)
        } else {
            self.email.clone()
        }
    }

    /// `Contact.isValid()`: the whole address matches the email pattern.
    pub fn is_valid(&self) -> bool {
        !self.email.is_empty()
            && EMAIL
                .find(&self.email)
                .is_some_and(|m| m.as_str() == self.email)
    }

    /// `Contact._nameParts()`.
    fn name_parts(&self) -> Vec<String> {
        let mut name = self.name.clone();
        if name.is_empty() {
            name = self.email.clone();
            if let Some(at) = name.find('@').filter(|&at| at > 0) {
                name.truncate(at);
            }
        }
        if name.len() >= 2
            && ((name.starts_with('\'') && name.ends_with('\''))
                || (name.starts_with('"') && name.ends_with('"')))
        {
            name = name[1..name.len() - 1].to_owned();
        }
        static VIA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\svia\s").unwrap());
        if let Some(m) = VIA.find(&name) {
            name.truncate(m.start());
        }
        let name = name.trim().to_owned();

        let mut parts = reverse_name_parts(&name);
        if parts.concat().is_empty() {
            parts = split_whitespace_js(&name);
            let stripped = |p: &str| p.to_lowercase().replacen('.', "", 1);
            if parts
                .first()
                .is_some_and(|p| DESKTOP.name_prefixes.contains(&stripped(p)))
            {
                parts.remove(0);
            }
            if parts
                .last()
                .is_some_and(|p| DESKTOP.name_suffixes.contains(&stripped(p)))
            {
                parts.pop();
            }
        }
        if parts.concat().is_empty() {
            parts = vec![name.clone()];
        }
        if parts.concat().is_empty() {
            parts = vec![self.email.clone()];
        }
        parts
    }

    pub fn full_name(&self) -> String {
        self.name_parts().join(" ")
    }

    pub fn first_name(&self) -> String {
        const EXCLUSIONS: [&str; 8] = ["a", "the", "dr.", "mrs.", "mr.", "mx.", "prof.", "ph.d."];
        self.name_parts()
            .into_iter()
            .find(|p| !EXCLUSIONS.contains(&p.to_lowercase().as_str()))
            .unwrap_or_default()
    }

    pub fn last_name(&self) -> String {
        self.name_parts()
            .get(1..)
            .map(|rest| rest.join(" "))
            .unwrap_or_default()
    }

    /// `Contact.nameAbbreviation()`.
    pub fn name_abbreviation(&self) -> String {
        let first = |s: String| {
            s.chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default()
        };
        let c1 = first(self.first_name());
        let c2 = first(self.last_name());
        if c2 == "(" || c2 == "[" {
            return c1;
        }
        c1 + &c2
    }
}

/// `name.split(/\s+/)`: a leading or trailing run of whitespace gives an empty part, as in
/// JavaScript.
fn split_whitespace_js(s: &str) -> Vec<String> {
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    WS.split(s).map(str::to_owned).collect()
}

/// `Contact._parseReverseNames`: "Last, First (Description)".
fn reverse_name_parts(name: &str) -> Vec<String> {
    let mut split = name.split(", ");
    let last = split.next().unwrap_or("");
    let Some(remainder) = split.next().filter(|r| !r.is_empty()) else {
        return Vec::new();
    };
    let mut rest = remainder.split('(');
    let first = rest.next().unwrap_or("");
    let mut parts = vec![first.trim().to_owned(), last.trim().to_owned()];
    if let Some(description) = rest.next().filter(|d| !d.is_empty()) {
        parts.push(format!("({}", description.trim()));
    }
    parts
}

/// `Utils.toEquivalentEmailForm`: the address without a `+tag`, lowercased. Anything without
/// an `@` becomes `@`, as in the desktop.
pub fn equivalent_email(email: &str) -> String {
    static FORM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^([^+]+).*@(.+)$").unwrap());
    let (user, domain) = FORM
        .captures(email)
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .unwrap_or_default();
    format!("{user}@{domain}").trim().to_lowercase()
}

/// `Utils.emailIsEquivalent`.
pub fn emails_equivalent(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    a == b || equivalent_email(&a) == equivalent_email(&b)
}

/// `Utils.likelyNonHumanEmail`.
pub fn likely_non_human_email(email: &str) -> bool {
    if email.encode_utf16().count() > 48 {
        return true;
    }
    let local = email.split('@').next().unwrap_or("");
    if local.chars().filter(char::is_ascii_digit).count() >= 6 {
        return true;
    }
    NON_HUMAN.is_match(email)
}

fn char_before(s: &str, i: usize) -> Option<char> {
    s.get(..i).and_then(|p| p.chars().next_back())
}

fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..).and_then(|p| p.chars().next())
}

/// `ContactStore.parseContactsInString` without the name lookup: every address in `s`, with the
/// name before it when it is written `Name <email>` or `Name (email)`, else the address as
/// the name.
pub fn parse_contacts_in_string(s: &str) -> Vec<Contact> {
    let mut out = Vec::new();
    let mut last_match_end = 0usize;
    for m in EMAIL.find_iter(s) {
        let start = m.start();
        let mut email = m.as_str().to_owned();
        let quote = |c: Option<char>| matches!(c, Some('\'') | Some('"'));
        if quote(email.chars().next()) && quote(char_at(s, start + email.len())) {
            email = email[1..email.len() - 1].to_owned();
        }
        let leading_paren = matches!(char_before(s, start), Some('(') | Some('<'));
        let trailing_paren = matches!(char_at(s, start + email.len()), Some(')') | Some('>'));
        let mut name = String::new();
        if leading_paren && trailing_paren {
            let mut name_start = last_match_end;
            for c in [',', ';', '\n', '\r'] {
                if let Some(i) = s[..start].rfind(c) {
                    if i + 1 > name_start {
                        name_start = i + 1;
                    }
                }
            }
            let end = start.saturating_sub(1);
            if name_start < end {
                name = s.get(name_start..end).unwrap_or("").trim().to_owned();
            }
        }
        last_match_end = start + email.len() + usize::from(trailing_paren);
        if name.is_empty() {
            name = email.clone();
        }
        let is_quote = |c: Option<char>| matches!(c, Some('"') | Some('\''));
        if is_quote(name.chars().next()) && is_quote(name.chars().next_back()) {
            name = if name.chars().count() >= 2 {
                let mut chars = name.chars();
                chars.next();
                chars.next_back();
                chars.as_str().to_owned()
            } else {
                String::new()
            };
        }
        out.push(Contact::new(name, email));
    }
    out
}

/// `Contact.fromString`: the address in `s` (the last one in angle brackets, else the last
/// one) and the text before it as the name.
pub fn contact_from_string(s: &str) -> Contact {
    let matches: Vec<regex::Match> = EMAIL.find_iter(s).collect();
    let bracketed = matches
        .iter()
        .rfind(|m| char_before(s, m.start()) == Some('<') && char_at(s, m.end()) == Some('>'));
    let chosen = bracketed.or(matches.last());
    let email = chosen.map(|m| m.as_str().to_owned()).unwrap_or_default();
    let mut name = match chosen {
        Some(m) => s[..m.start()].trim().to_owned(),
        None => s.trim().to_owned(),
    };
    if name.ends_with('<') || name.ends_with('(') {
        name.pop();
        name = name.trim().to_owned();
    }
    Contact::new(name, email)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_contact_strings() {
        let got = parse_contacts_in_string("Archive <archive@example.org>, log@example.org");
        assert_eq!(
            got,
            vec![
                Contact::new("Archive", "archive@example.org"),
                Contact::new("log@example.org", "log@example.org"),
            ]
        );
        let got = parse_contacts_in_string("\"Ann B\" <ann@x.example>; (bob@y.example)");
        assert_eq!(got[0], Contact::new("Ann B", "ann@x.example"));
        assert_eq!(got[1].email, "bob@y.example");
        assert_eq!(
            contact_from_string("Ann at Work <ann@work.example>"),
            Contact::new("Ann at Work", "ann@work.example")
        );
        assert_eq!(
            contact_from_string("Just A Name"),
            Contact::new("Just A Name", "")
        );
    }

    #[test]
    fn names_and_validity() {
        assert!(Contact::new("", "a@b.example").is_valid());
        assert!(!Contact::new("", "not-an-address").is_valid());
        let c = Contact::new("Jones, Carol (Acme)", "c@x.example");
        assert_eq!(c.full_name(), "Carol Jones (Acme)");
        assert_eq!(
            Contact::new("Carol Jones", "c@x.example").name_abbreviation(),
            "CJ"
        );
        assert_eq!(Contact::new("", "carol@x.example").full_name(), "carol");
        assert_eq!(Contact::new("Dr. Who", "w@x.example").first_name(), "Who");
        assert!(emails_equivalent("Bob+list@Y.example", "bob@y.example"));
        assert!(likely_non_human_email("noreply@shop.example"));
        assert!(!likely_non_human_email("carol@sender.example"));
    }
}
