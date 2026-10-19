// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! VCF files (RFC 6350 section 3.1: any number of vCards, one after another), from and to
//! contacts, as the desktop's `VCFImportExport.ts` did it.

use crate::contact::{escape, format_display_name, parse, serialize_birthday, SOURCE_CARDDAV};
use ms_dav::vcard::{unescape, VCard, VCardProperty};
use serde_json::{json, Value};

/// One contact as a vCard (`contactToVCFString`): a CardDAV contact's own card as stored;
/// for a Google or mail contact, a vCard 4.0 built from its details.
pub fn contact_to_vcf(contact: &Value) -> String {
    let source = contact["s"].as_str().unwrap_or_default();
    if source == SOURCE_CARDDAV {
        if let Some(vcf) = contact["info"]["vcf"].as_str() {
            return vcf.to_owned();
        }
    }
    let data = parse(contact).data;
    let mut card = VCard::default();
    let add = |card: &mut VCard, name: &str, value: String, kind: Option<&str>| {
        let mut p = VCardProperty::new(name, &value);
        if let Some(t) = kind.filter(|t| !t.is_empty()) {
            p.params = format!("TYPE={t}");
        }
        card.add(p);
    };
    add(&mut card, "VERSION", "4.0".into(), None);
    let uid = contact["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    add(&mut card, "UID", uid, None);
    let name = &data["name"];
    if name.is_object() {
        let f = |k: &str| escape(name[k].as_str().unwrap_or_default());
        add(
            &mut card,
            "N",
            format!(
                "{};{};;{};{}",
                f("familyName"),
                f("givenName"),
                f("honorificPrefix"),
                f("honorificSuffix")
            ),
            None,
        );
        let display = name["displayName"]
            .as_str()
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format_display_name(name));
        add(&mut card, "FN", escape(&display), None);
    } else if let Some(n) = contact["name"].as_str().filter(|n| !n.is_empty()) {
        add(&mut card, "FN", escape(n), None);
    }
    for (key, prop) in [("company", "ORG"), ("title", "TITLE")] {
        if let Some(v) = data[key].as_str().filter(|v| !v.is_empty()) {
            add(&mut card, prop, escape(v), None);
        }
    }
    let each = |key: &str| data[key].as_array().cloned().unwrap_or_default();
    for (key, prop) in [("emailAddresses", "EMAIL"), ("phoneNumbers", "TEL")] {
        for item in each(key) {
            if let Some(v) = item["value"].as_str().filter(|v| !v.is_empty()) {
                add(&mut card, prop, v.to_owned(), item["type"].as_str());
            }
        }
    }
    for a in each("addresses") {
        let f = |k: &str| escape(a[k].as_str().unwrap_or_default());
        let value = format!(
            ";{};{};{};{};{};{}",
            f("streetAddress"),
            f("extendedAddress"),
            f("city"),
            f("region"),
            f("postalCode"),
            f("country")
        );
        add(&mut card, "ADR", value, a["type"].as_str());
    }
    // Google People birthdays can carry only a free-form `text` and no `date`.
    if let Some(b) = each("birthdays")
        .into_iter()
        .find(|b| b["date"].is_object())
    {
        add(&mut card, "BDAY", serialize_birthday(&b), None);
    }
    for item in each("urls") {
        if let Some(v) = item["value"].as_str().filter(|v| !v.is_empty()) {
            add(&mut card, "URL", v.to_owned(), item["type"].as_str());
        }
    }
    for item in each("relations") {
        if let Some(v) = item["person"].as_str().filter(|v| !v.is_empty()) {
            add(&mut card, "RELATED", escape(v), item["type"].as_str());
        }
    }
    for item in each("nicknames") {
        if let Some(v) = item["value"].as_str().filter(|v| !v.is_empty()) {
            add(&mut card, "NICKNAME", escape(v), None);
        }
    }
    if let Some(n) = data["notes"].as_str().filter(|n| !n.is_empty()) {
        add(&mut card, "NOTE", escape(n), None);
    }
    card.serialize()
}

/// A .vcf file of `contacts` (`contactsToVCFFileContent`), CRLF line ends throughout.
pub fn export(contacts: &[Value]) -> String {
    let mut out = String::new();
    for c in contacts {
        let card = contact_to_vcf(c);
        let normalized = card.replace("\r\n", "\n").replace('\n', "\r\n");
        out.push_str(normalized.trim_end_matches(['\r', '\n']));
        out.push_str("\r\n");
    }
    out
}

/// The vCards in a .vcf file as new CardDAV contacts for `account_id`
/// (`vcfStringToContacts`). Each gets a fresh UID: an imported card is a new item here, and
/// keeping its UID would collide with the original on a server that still has it. A card
/// with neither a name nor an address is skipped.
pub fn import(text: &str, account_id: &str) -> Vec<Value> {
    blocks(text)
        .into_iter()
        .filter_map(|block| {
            let mut card = VCard::parse(&block);
            if !card.properties.iter().any(|p| p.name == "VERSION") {
                card.properties
                    .insert(0, VCardProperty::new("VERSION", "3.0"));
            }
            card.properties.retain(|p| p.name != "UID");
            card.add(VCardProperty::new("UID", &uuid::Uuid::new_v4().to_string()));
            let vcf = card.serialize();
            let parsed = crate::contact::from_vcf(&vcf).data;
            let mut name = parsed["name"]["displayName"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if name.is_empty() {
                name = format_display_name(&parsed["name"]);
            }
            if name.is_empty() {
                name = card
                    .named("FN")
                    .next()
                    .map(|p| unescape(&p.value))
                    .unwrap_or_default();
            }
            let email = parsed["emailAddresses"][0]["value"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if name.is_empty() && email.is_empty() {
                return None;
            }
            Some(json!({
                "__cls": "Contact",
                "aid": account_id,
                "s": SOURCE_CARDDAV,
                "name": name,
                "email": email,
                "info": { "vcf": vcf, "href": "" },
            }))
        })
        .collect()
}

/// Each `BEGIN:VCARD` … `END:VCARD`, case-insensitively.
fn blocks(text: &str) -> Vec<String> {
    let upper = text.to_ascii_uppercase();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(b) = upper[from..].find("BEGIN:VCARD") {
        let start = from + b;
        let Some(e) = upper[start..].find("END:VCARD") else {
            break;
        };
        let end = start + e + "END:VCARD".len();
        out.push(text[start..end].to_owned());
        from = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_with_fresh_uids() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:keep-not\r\nFN:Ann Archer\r\nN:Archer;Ann;;;\r\nEMAIL:ann@example.com\r\nEND:VCARD\r\n\r\nbegin:vcard\nFN:No Version\nEMAIL:nv@example.com\nend:vcard\nBEGIN:VCARD\r\nVERSION:4.0\r\nEND:VCARD\r\n";
        let contacts = import(text, "acc");
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0]["name"], "Ann Archer");
        assert_eq!(contacts[0]["email"], "ann@example.com");
        let vcf = contacts[0]["info"]["vcf"].as_str().unwrap();
        assert!(!vcf.contains("keep-not"));
        assert!(vcf.contains("UID:"));
        assert_eq!(contacts[1]["name"], "No Version");
        assert!(contacts[1]["info"]["vcf"]
            .as_str()
            .unwrap()
            .contains("VERSION:3.0\r\n"));
    }

    #[test]
    fn exports_google_and_mail_contacts() {
        let g = json!({"id": "g1", "s": "gpeople", "name": "Gina", "email": "g@x.com",
            "info": {"names": [{"displayName": "Regina Google", "familyName": "Google", "givenName": "Regina"}],
                     "organizations": [{"name": "Alphabet", "title": "Eng"}],
                     "emailAddresses": [{"value": "regina@example.com"}],
                     "phoneNumbers": [{"type": "mobile", "value": "+351 1"}],
                     "birthdays": [{"date": {"day": 9, "month": 7, "year": 1985}}]}});
        let m = json!({"id": "m1", "s": "mail", "name": "Mail Only Person", "email": "mail@example.com"});
        let out = export(&[g, m]);
        assert!(out.contains("UID:g1\r\nN:Google;Regina;;;\r\nFN:Regina Google\r\nORG:Alphabet\r\nTITLE:Eng\r\nEMAIL:regina@example.com\r\nTEL;TYPE=mobile:+351 1\r\nBDAY:1985-07-09\r\n"));
        assert!(out.contains("N:Only Person;Mail;;;\r\nFN:Mail Only Person\r\nEMAIL:mail@example.com\r\nEND:VCARD\r\n"));
        assert!(!out.contains("\r\r"));
    }
}
