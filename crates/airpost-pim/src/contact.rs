// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The shared contact shape the desktop's contact editor shows (`ContactBase` in
//! `ContactInfoMapping.ts`), read from and written back to a Contact's `info`: a CardDAV
//! vCard (`{vcf, href}`) or a Google People person. A contact with no `info` was found in
//! mail and is read-only.
//!
//! Writing changes only what changed against the parsed form, and in a vCard only the
//! properties those fields map to, so properties the editor never shows survive.
//!
//! The shape (JSON, camelCase as the desktop has it): `name {displayName, familyName,
//! givenName, honorificPrefix, honorificSuffix}`, `photoURL?`, `nicknames [{value, type?}]`,
//! `company`, `title`, `phoneNumbers`, `emailAddresses`, `urls` (`[{value, type?}]`),
//! `relations [{person, type?}]`, `addresses [{formattedValue, type?, city, country,
//! postalCode, region, streetAddress, extendedAddress}]`, `birthdays [{date {year, month,
//! day}}]`, `notes?`.

use crate::{Error, Result};
use serde_json::{json, Map, Value};
use tern_dav::vcard::{unescape, VCard, VCardProperty};

/// Where a contact's details come from, and whether they can be edited.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub origin: &'static str,
    pub readonly: bool,
    pub data: Value,
}

impl Parsed {
    pub fn to_json(&self) -> Value {
        json!({
            "metadata": { "origin": self.origin, "readonly": self.readonly },
            "data": self.data,
        })
    }
}

pub const SOURCE_CARDDAV: &str = "carddav";
pub const SOURCE_GOOGLE: &str = "gpeople";

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The contact's details (`parse`): from its vCard, its Google person, or, with no `info`,
/// from its name and address as mail found them.
pub fn parse(contact: &Value) -> Parsed {
    let info = &contact["info"];
    let empty = !info.is_object() || info.as_object().is_some_and(Map::is_empty);
    if empty {
        return from_mail(str_of(contact, "name"), str_of(contact, "email"));
    }
    if let Some(vcf) = info.get("vcf").and_then(Value::as_str) {
        return from_vcf(vcf);
    }
    from_google(info)
}

/// A contact mail found (`fromContact`): read-only.
pub fn from_mail(name: &str, email: &str) -> Parsed {
    let mut parts = name.split(' ');
    let given = parts.next().unwrap_or("").to_owned();
    let family = parts.collect::<Vec<_>>().join(" ");
    Parsed {
        origin: "Found in Sent Mail",
        readonly: true,
        data: json!({
            "emailAddresses": [{ "value": email }],
            "title": "",
            "company": "",
            "name": {
                "displayName": name,
                "givenName": given,
                "familyName": family,
                "honorificPrefix": "",
                "honorificSuffix": "",
            },
        }),
    }
}

/// `TYPE` values of a property's parameters, lowercased; vCard 2.1's bare `WORK;INTERNET`
/// counts too.
fn types_of(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in params.split(';').filter(|p| !p.is_empty()) {
        match part.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("TYPE") => {
                out.extend(
                    v.split(',')
                        .map(|t| t.trim().trim_matches('"').to_lowercase())
                        .filter(|t| !t.is_empty()),
                );
            }
            Some(_) => {}
            None => out.push(part.trim().to_lowercase()),
        }
    }
    out
}

fn param_of(params: &str, name: &str) -> Option<String> {
    params.split(';').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(name)
            .then(|| v.trim().trim_matches('"').to_owned())
    })
}

/// The one type the editor shows: the first that is not `internet` or `pref`.
fn type_of(params: &str) -> Option<String> {
    types_of(params)
        .into_iter()
        .find(|t| t != "internet" && t != "pref")
}

/// Splits a structured value on unescaped `;`, unescaping each component.
fn components(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            let last = parts.last_mut().unwrap_or_else(|| unreachable!());
            match c {
                'n' | 'N' => last.push('\n'),
                other => last.push(other),
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == ';' {
            parts.push(String::new());
        } else if let Some(last) = parts.last_mut() {
            last.push(c);
        }
    }
    parts
}

/// vCard TEXT escaping (RFC 6350 section 3.4).
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.replace("\r\n", "\n").chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ',' => out.push_str("\\,"),
            ';' => out.push_str("\\;"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

fn value_and_type(p: &VCardProperty) -> Value {
    let mut m = Map::new();
    m.insert("value".into(), json!(unescape(&p.value)));
    if let Some(t) = type_of(&p.params) {
        m.insert("type".into(), json!(t));
    }
    Value::Object(m)
}

/// A postal address on separate lines (`formatAddress`).
pub fn format_address(a: &Value) -> String {
    let f = |k: &str| str_of(a, k).to_owned();
    let locality = [f("city"), f("region"), f("postalCode")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    [
        f("streetAddress"),
        f("extendedAddress"),
        locality,
        f("country"),
    ]
    .into_iter()
    .filter(|l| !l.is_empty())
    .collect::<Vec<_>>()
    .join("\n")
    .replace("\\n", "\n")
    .replace("\\,", ",")
}

fn parse_address(p: &VCardProperty) -> Value {
    let mut c = components(&p.value);
    c.resize(7, String::new());
    let mut a = json!({
        "city": c[3],
        "country": c[6],
        "postalCode": c[5],
        "region": c[4],
        "streetAddress": c[1],
        "extendedAddress": c[2],
        "formattedValue": "",
    });
    if let Some(t) = type_of(&p.params) {
        a["type"] = json!(t);
    }
    let label = param_of(&p.params, "LABEL").map(|l| unescape(&l));
    a["formattedValue"] = json!(label
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| format_address(&a)));
    a
}

/// `YYYY-MM-DD`, `YYYYMMDD`, `--MM-DD` or `--MMDD`, a time part ignored; unknown parts are
/// -1, as the editor reads them.
pub fn parse_birthday(value: &str) -> Value {
    let v = value.trim();
    let v = v.split(['T', 't']).next().unwrap_or(v);
    let num = |s: &str| s.parse::<i64>().unwrap_or(-1);
    let (y, m, d) = if let Some(rest) = v.strip_prefix("--") {
        let digits: String = rest.chars().filter(char::is_ascii_digit).collect();
        if digits.len() == 4 {
            (-1, num(&digits[..2]), num(&digits[2..]))
        } else {
            (-1, -1, -1)
        }
    } else if v.len() == 8 && v.chars().all(|c| c.is_ascii_digit()) {
        (num(&v[..4]), num(&v[4..6]), num(&v[6..]))
    } else {
        let mut parts = v.split('-');
        (
            parts.next().map(num).unwrap_or(-1),
            parts.next().map(num).unwrap_or(-1),
            parts.next().map(num).unwrap_or(-1),
        )
    };
    json!({ "year": y, "month": m, "day": d })
}

/// `YYYY-MM-DD`, each part zero-padded.
pub fn serialize_birthday(b: &Value) -> String {
    let date = &b["date"];
    let part = |k: &str, width: usize| {
        let n = date[k].as_i64().unwrap_or(0).max(0);
        let s = n.to_string();
        let s = if s.len() > width {
            s[..width].to_owned()
        } else {
            s
        };
        format!("{s:0>width$}")
    };
    format!(
        "{}-{}-{}",
        part("year", 4),
        part("month", 2),
        part("day", 2)
    )
}

fn display_name(given: &str, family: &str) -> String {
    format!("{given} {family}").trim().to_owned()
}

/// `formatDisplayName`: given and family name.
pub fn format_display_name(name: &Value) -> String {
    display_name(str_of(name, "givenName"), str_of(name, "familyName"))
}

/// A CardDAV contact's details (`fromVCF`).
pub fn from_vcf(vcf: &str) -> Parsed {
    let card = VCard::parse(vcf);
    let first = |n: &'static str| card.properties.iter().find(move |p| p.name == n);
    let fn_value = first("FN").map(|p| unescape(&p.value)).unwrap_or_default();
    let name = match first("N") {
        Some(n) => {
            let mut c = components(&n.value);
            c.resize(5, String::new());
            let display = [c[3].as_str(), c[1].as_str(), c[0].as_str(), c[4].as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            json!({
                "givenName": c[1],
                "familyName": c[0],
                "honorificPrefix": c[3],
                "honorificSuffix": c[4],
                "displayName": if display.is_empty() { fn_value.clone() } else { display },
            })
        }
        None => json!({
            "givenName": "",
            "familyName": "",
            "honorificPrefix": "",
            "honorificSuffix": "",
            "displayName": fn_value,
        }),
    };
    let mut data = Map::new();
    data.insert("name".into(), name);
    data.insert(
        "company".into(),
        json!(first("ORG")
            .map(|o| components(&o.value).remove(0))
            .unwrap_or_default()),
    );
    data.insert(
        "nicknames".into(),
        Value::Array(card.named("NICKNAME").map(value_and_type).collect()),
    );
    data.insert(
        "title".into(),
        json!(first("TITLE")
            .map(|t| unescape(&t.value))
            .unwrap_or_default()),
    );
    data.insert(
        "phoneNumbers".into(),
        Value::Array(card.named("TEL").map(value_and_type).collect()),
    );
    data.insert(
        "emailAddresses".into(),
        Value::Array(card.named("EMAIL").map(value_and_type).collect()),
    );
    data.insert(
        "urls".into(),
        Value::Array(card.named("URL").map(value_and_type).collect()),
    );
    data.insert(
        "addresses".into(),
        Value::Array(card.named("ADR").map(parse_address).collect()),
    );
    if let Some(b) = first("BDAY") {
        let value_type = param_of(&b.params, "VALUE");
        if value_type.is_none_or(|v| v.eq_ignore_ascii_case("date")) {
            data.insert(
                "birthdays".into(),
                json!([{ "date": parse_birthday(&b.value) }]),
            );
        }
    }
    if let Some(n) = first("NOTE") {
        data.insert("notes".into(), json!(unescape(&n.value)));
    }
    if let Some(photo) = first("PHOTO").map(|p| p.value.trim().to_owned()) {
        // iCloud photos need the account's credentials to fetch, which the client lacks.
        if !photo.is_empty() && !is_icloud_photo(&photo) {
            data.insert("photoURL".into(), json!(photo));
        }
    }
    Parsed {
        origin: "CardDAV",
        readonly: false,
        data: Value::Object(data),
    }
}

fn is_icloud_photo(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.and_then(|r| r.split(['/', '?', '#']).next())
        .map(|host| {
            host.split(':')
                .next()
                .unwrap_or(host)
                .ends_with("contacts.icloud.com")
        })
        .unwrap_or(false)
}

/// A Google People person's details (`fromGoogle`): one name and one organization, the rest
/// passed through.
pub fn from_google(info: &Value) -> Parsed {
    let org = info["organizations"].get(0).cloned().unwrap_or(json!({}));
    let mut data = Map::new();
    data.insert(
        "name".into(),
        info["names"].get(0).cloned().unwrap_or_else(|| {
            json!({
                "givenName": "",
                "familyName": "",
                "honorificPrefix": "",
                "honorificSuffix": "",
                "displayName": "",
            })
        }),
    );
    data.insert("title".into(), json!(str_of(&org, "title")));
    data.insert("company".into(), json!(str_of(&org, "name")));
    if let Some(bio) = info["biographies"][0]["value"]
        .as_str()
        .filter(|b| !b.is_empty())
    {
        data.insert("notes".into(), json!(bio));
    }
    if let Some(obj) = info.as_object() {
        for (k, v) in obj {
            data.insert(k.clone(), v.clone());
        }
    }
    Parsed {
        origin: "Google Contacts",
        readonly: false,
        data: Value::Object(data),
    }
}

/// The fields of `next` that differ from the contact's parsed details.
pub fn changes(contact: &Value, next: &Value) -> Map<String, Value> {
    let original = parse(contact).data;
    let mut out = Map::new();
    if let Some(obj) = next.as_object() {
        for (k, v) in obj {
            if original.get(k) != Some(v) && !(v.is_null() && original.get(k).is_none()) {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    out
}

/// The contact with `next`'s changed fields written into its `info`, `name` and `email`
/// (`apply`). A contact mail found has no details to write and is refused.
pub fn apply(contact: &Value, next: &Value) -> Result<Value> {
    let changed = changes(contact, next);
    let mut out = contact.clone();
    let info = &contact["info"];
    let empty = !info.is_object() || info.as_object().is_some_and(Map::is_empty);
    if empty {
        if changed.is_empty() {
            return Ok(out);
        }
        return Err(Error::new(
            "read-only",
            "this contact was found in mail and has no details to edit",
        ));
    }
    if info.get("vcf").is_some() {
        apply_to_vcf(&mut out, &changed);
    } else {
        apply_to_google(&mut out, &changed);
    }
    Ok(out)
}

fn typed_property(name: &str, value: &str, kind: Option<&str>, v3: bool) -> VCardProperty {
    let mut p = VCardProperty::new(name, value);
    if let Some(t) = kind.filter(|t| !t.is_empty()) {
        let t = if v3 {
            t.to_ascii_uppercase()
        } else {
            t.to_owned()
        };
        p.params = format!("TYPE={t}");
    }
    p
}

/// Replaces every `name` property with `props`, at the first one's position (the end when
/// there was none).
fn set_all(card: &mut VCard, name: &str, props: Vec<VCardProperty>) {
    let at = card
        .properties
        .iter()
        .position(|p| p.name == name)
        .unwrap_or(card.properties.len());
    card.properties.retain(|p| p.name != name);
    let at = at.min(card.properties.len());
    for (i, p) in props.into_iter().enumerate() {
        card.properties.insert(at + i, p);
    }
}

fn set_one(card: &mut VCard, name: &str, value: String) {
    match card.properties.iter_mut().find(|p| p.name == name) {
        Some(p) => p.value = value,
        None => card.add(VCardProperty::new(name, &value)),
    }
    let mut seen = false;
    card.properties.retain(|p| {
        if p.name != name {
            return true;
        }
        let keep = !seen;
        seen = true;
        keep
    });
}

fn list(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

/// Writes changed fields into a vCard (`applyToVCF`). Empty lists remove the property.
fn apply_to_vcf(contact: &mut Value, changed: &Map<String, Value>) {
    let vcf = contact["info"]["vcf"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut card = VCard::parse(&vcf);
    let v3 = card.version().starts_with('3') || card.version().starts_with('2');
    for (key, val) in changed {
        match key.as_str() {
            "name" => {
                let mut parts = card
                    .named("N")
                    .next()
                    .map(|n| n.value.split(';').map(str::to_owned).collect::<Vec<_>>())
                    .unwrap_or_default();
                parts.resize(5.max(parts.len()), String::new());
                parts[0] = escape(str_of(val, "familyName"));
                parts[1] = escape(str_of(val, "givenName"));
                if val.get("honorificPrefix").is_some() {
                    parts[3] = escape(str_of(val, "honorificPrefix"));
                }
                if val.get("honorificSuffix").is_some() {
                    parts[4] = escape(str_of(val, "honorificSuffix"));
                }
                set_one(&mut card, "N", parts.join(";"));
                let display = format_display_name(val);
                set_one(&mut card, "FN", escape(&display));
                contact["name"] = json!(display);
            }
            "title" => {
                let t = val.as_str().unwrap_or_default();
                if t.is_empty() {
                    card.properties.retain(|p| p.name != "TITLE");
                } else {
                    set_one(&mut card, "TITLE", escape(t));
                }
            }
            "company" => {
                let c = escape(val.as_str().unwrap_or_default());
                match card.properties.iter_mut().find(|p| p.name == "ORG") {
                    Some(org) => {
                        let mut parts: Vec<String> =
                            org.value.split(';').map(str::to_owned).collect();
                        parts[0] = c;
                        org.value = parts.join(";");
                    }
                    None if !c.is_empty() => card.add(VCardProperty::new("ORG", &c)),
                    None => {}
                }
            }
            "nicknames" | "phoneNumbers" | "emailAddresses" | "urls" => {
                let name = match key.as_str() {
                    "nicknames" => "NICKNAME",
                    "phoneNumbers" => "TEL",
                    "emailAddresses" => "EMAIL",
                    _ => "URL",
                };
                let props = list(val)
                    .iter()
                    .filter(|i| !str_of(i, "value").is_empty())
                    .map(|i| {
                        let value = if name == "NICKNAME" {
                            escape(str_of(i, "value"))
                        } else {
                            str_of(i, "value").to_owned()
                        };
                        typed_property(name, &value, i["type"].as_str(), v3)
                    })
                    .collect();
                set_all(&mut card, name, props);
            }
            "birthdays" => {
                let props = list(val)
                    .iter()
                    .filter(|b| b["date"].is_object())
                    .map(|b| VCardProperty::new("BDAY", &serialize_birthday(b)))
                    .collect();
                set_all(&mut card, "BDAY", props);
            }
            "addresses" => {
                let props = list(val)
                    .iter()
                    .map(|a| {
                        let value = [
                            String::new(),
                            escape(str_of(a, "streetAddress")),
                            escape(str_of(a, "extendedAddress")),
                            escape(str_of(a, "city")),
                            escape(str_of(a, "region")),
                            escape(str_of(a, "postalCode")),
                            escape(str_of(a, "country")),
                        ]
                        .join(";");
                        typed_property("ADR", &value, a["type"].as_str(), v3)
                    })
                    .collect();
                set_all(&mut card, "ADR", props);
            }
            "notes" => match val.as_str().filter(|n| !n.is_empty()) {
                Some(n) => set_one(&mut card, "NOTE", escape(n)),
                None => card.properties.retain(|p| p.name != "NOTE"),
            },
            "photoURL" => {}
            other => tracing::debug!(field = other, "no vCard property for this contact field"),
        }
    }
    if let Some(email) = card
        .named("EMAIL")
        .next()
        .map(|p| p.value.trim().to_owned())
    {
        contact["email"] = json!(email);
    }
    contact["info"]["vcf"] = json!(card.serialize());
}

/// Writes changed fields into a Google People person (`applyToGoogle`).
fn apply_to_google(contact: &mut Value, changed: &Map<String, Value>) {
    let metadata = json!({ "primary": true });
    let mut name_after = None;
    let mut email_after = None;
    {
        let info = &mut contact["info"];
        for (key, val) in changed {
            let mut val = val.clone();
            match key.as_str() {
                "name" => {
                    if val.get("displayName").is_some() {
                        val["displayName"] = json!(format_display_name(&val));
                    }
                    let names = info["names"].as_array_mut();
                    match names.filter(|n| !n.is_empty()) {
                        Some(names) => {
                            if let (Some(first), Some(v)) =
                                (names[0].as_object_mut(), val.as_object())
                            {
                                for (k, x) in v {
                                    first.insert(k.clone(), x.clone());
                                }
                            }
                        }
                        None => {
                            let mut n = json!({ "metadata": metadata });
                            if let (Some(o), Some(v)) = (n.as_object_mut(), val.as_object()) {
                                for (k, x) in v {
                                    o.insert(k.clone(), x.clone());
                                }
                            }
                            info["names"] = json!([n]);
                        }
                    }
                    name_after = Some(format_display_name(&val));
                }
                "emailAddresses" => {
                    email_after = Some(str_of(&val[0], "value").to_owned());
                    info[key.as_str()] = val;
                }
                "title" | "company" => {
                    if info["organizations"].as_array().is_none_or(Vec::is_empty) {
                        info["organizations"] =
                            json!([{ "title": "", "name": "", "metadata": metadata }]);
                    }
                    let field = if key == "title" { "title" } else { "name" };
                    info["organizations"][0][field] = val;
                }
                "addresses" => {
                    let addresses: Vec<Value> = list(&val)
                        .into_iter()
                        .map(|mut a| {
                            a["formattedValue"] = json!(format_address(&a));
                            a
                        })
                        .collect();
                    info["addresses"] = Value::Array(addresses);
                }
                "notes" => {
                    if info["biographies"].as_array().is_none_or(Vec::is_empty) {
                        info["biographies"] = json!([{
                            "value": val,
                            "contentType": "TEXT_PLAIN",
                            "metadata": metadata,
                        }]);
                    } else {
                        info["biographies"][0]["value"] = val;
                    }
                }
                _ => info[key.as_str()] = val,
            }
        }
    }
    if let Some(n) = name_after {
        contact["name"] = json!(n);
    }
    if let Some(e) = email_after {
        contact["email"] = json!(e);
    }
}

/// A new contact for an account: an empty vCard 3.0 with a fresh UID, or an empty Google
/// person.
pub fn empty_contact(account_id: &str, google: bool) -> Value {
    let info = if google {
        json!({ "names": [], "resourceName": "", "etag": "", "addresses": [] })
    } else {
        json!({
            "vcf": format!(
                "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{}\r\nEND:VCARD\r\n",
                uuid::Uuid::new_v4()
            ),
            "href": "",
        })
    };
    json!({
        "__cls": "Contact",
        "aid": account_id,
        "s": if google { SOURCE_GOOGLE } else { SOURCE_CARDDAV },
        "name": "",
        "email": "",
        "info": info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANN: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:c1\r\nFN:Ann Archer\r\nN:Archer;Ann;;Dr.;PhD\r\nEMAIL;TYPE=INTERNET,WORK:ann@example.com\r\nEMAIL;TYPE=HOME:ann@home.example\r\nTEL;TYPE=CELL:+1 555 0100\r\nORG:Acme;R&D\r\nTITLE:Chief\\; Engineer\r\nADR;TYPE=WORK:;Suite 4;1 Main St;Springfield;IL;62701;USA\r\nBDAY:1980-04-05\r\nNOTE:Line one\\nLine two\r\nURL:https://example.com\r\nNICKNAME:Annie\r\nX-CUSTOM:keep me\r\nEND:VCARD\r\n";

    fn ann() -> Value {
        json!({"id": "c1", "aid": "a", "s": "carddav", "name": "X", "email": "x@example.com",
               "info": {"vcf": ANN, "href": "/c1.vcf"}})
    }

    #[test]
    fn reads_a_vcard() {
        let p = parse(&ann());
        assert!(!p.readonly);
        let d = &p.data;
        assert_eq!(d["name"]["displayName"], "Dr. Ann Archer PhD");
        assert_eq!(d["title"], "Chief; Engineer");
        assert_eq!(d["company"], "Acme");
        assert_eq!(
            d["emailAddresses"][0],
            json!({"value": "ann@example.com", "type": "work"})
        );
        assert_eq!(
            d["addresses"][0]["formattedValue"],
            "Suite 4\n1 Main St\nSpringfield IL 62701\nUSA"
        );
        assert_eq!(
            d["birthdays"][0]["date"],
            json!({"year": 1980, "month": 4, "day": 5})
        );
        assert_eq!(d["notes"], "Line one\nLine two");
        assert_eq!(d["urls"][0], json!({"value": "https://example.com"}));
    }

    #[test]
    fn writes_only_what_changed() {
        let c = ann();
        let mut next = parse(&c).data;
        next["title"] = json!("Boss, Big");
        next["phoneNumbers"] = json!([]);
        next["birthdays"] = json!([{"date": {"year": 1990, "month": 2, "day": 3}}]);
        next["name"]["givenName"] = json!("Anne");
        let out = apply(&c, &next).unwrap();
        let vcf = out["info"]["vcf"].as_str().unwrap();
        assert!(vcf.contains("TITLE:Boss\\, Big\r\n"));
        assert!(!vcf.contains("TEL"));
        assert!(vcf.contains("BDAY:1990-02-03\r\n"));
        assert!(vcf.contains("N:Archer;Anne;;Dr.;PhD\r\n"));
        assert!(vcf.contains("FN:Anne Archer\r\n"));
        assert!(vcf.contains("X-CUSTOM:keep me\r\n"));
        assert!(vcf.contains("EMAIL;TYPE=INTERNET,WORK:ann@example.com\r\n"));
        assert_eq!(out["name"], "Anne Archer");
        assert_eq!(out["info"]["href"], "/c1.vcf");
    }

    #[test]
    fn mail_contacts_are_read_only() {
        let c = json!({"id": "m", "name": "Mail Only Person", "email": "m@x.com", "s": "mail"});
        let p = parse(&c);
        assert!(p.readonly);
        assert_eq!(p.data["name"]["familyName"], "Only Person");
        let mut next = p.data.clone();
        next["title"] = json!("x");
        assert_eq!(apply(&c, &next).unwrap_err().code, "read-only");
    }

    #[test]
    fn google_people() {
        let c = json!({"id": "g", "s": "gpeople", "name": "Gina", "email": "g@x.com",
            "info": {"names": [{"displayName": "Gina Google", "familyName": "Google", "givenName": "Gina"}],
                     "organizations": [{"name": "Goog", "title": "Eng"}],
                     "biographies": [{"value": "Bio"}],
                     "emailAddresses": [{"value": "g@x.com"}]}});
        let p = parse(&c);
        assert_eq!(p.data["company"], "Goog");
        assert_eq!(p.data["notes"], "Bio");
        let mut next = p.data.clone();
        next["company"] = json!("Alphabet");
        next["emailAddresses"] = json!([{"value": "r@x.com"}]);
        next["name"]["givenName"] = json!("Regina");
        let out = apply(&c, &next).unwrap();
        assert_eq!(out["info"]["organizations"][0]["name"], "Alphabet");
        assert_eq!(out["email"], "r@x.com");
        assert_eq!(out["name"], "Regina Google");
        assert_eq!(out["info"]["names"][0]["displayName"], "Regina Google");
    }

    #[test]
    fn birthdays_pad() {
        assert_eq!(
            serialize_birthday(&json!({"date": {"year": 985, "month": 7, "day": 9}})),
            "0985-07-09"
        );
        assert_eq!(
            parse_birthday("19850709"),
            json!({"year": 1985, "month": 7, "day": 9})
        );
        assert_eq!(
            parse_birthday("--0709"),
            json!({"year": -1, "month": 7, "day": 9})
        );
    }
}
