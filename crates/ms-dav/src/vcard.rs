//! vCards: a lossless line model for editing, and the Contact fields read through calcard.
//! Derived from Mailspring-Sync `MailSync/VCard.*`, `MailSync/DAVUtils.*` and the vCard
//! handling in `MailSync/DAVWorker.cpp` (`ingestAddressDataNode`, `rebuildContactGroup`),
//! Copyright © 2017-2019 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! [`VCard`] splits a card into unfolded property lines and puts it back together without
//! interpreting values, so a card the engine edits keeps every property it doesn't know.
//! [`ContactFields`] decodes values (escapes, `N` components, vCard 2.1 quoted-printable)
//! through calcard, and falls back to the line model for a card calcard rejects.
//!
//! Contact groups come in two forms: vCard 4 `KIND:group` with `MEMBER` (RFC 6350 §6.6.5),
//! and the vCard 3 form iCloud and Fastmail use, `X-ADDRESSBOOKSERVER-KIND:group` with
//! `X-ADDRESSBOOKSERVER-MEMBER`. Members are `urn:uuid:<contact id>`.

use calcard::vcard::{VCard as CalVCard, VCardEntry, VCardProperty as CalProp, VCardValue};

/// The vCard 3 group kind property (iCloud, Fastmail).
pub const X_VCARD3_KIND: &str = "X-ADDRESSBOOKSERVER-KIND";
/// The vCard 3 group member property (iCloud, Fastmail).
pub const X_VCARD3_MEMBER: &str = "X-ADDRESSBOOKSERVER-MEMBER";
/// `Contact.source` for CardDAV contacts.
pub const CARDDAV_SYNC_SOURCE: &str = "carddav";
/// The prefix group members carry.
pub const MEMBER_URN_PREFIX: &str = "urn:uuid:";

/// One property line: `[group.]NAME[;params]:value`. The name is uppercased; the group,
/// parameters and value are kept byte for byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VCardProperty {
    pub group: Option<String>,
    pub name: String,
    pub params: String,
    pub value: String,
}

impl VCardProperty {
    pub fn new(name: &str, value: &str) -> Self {
        VCardProperty {
            group: None,
            name: name.to_ascii_uppercase(),
            params: String::new(),
            value: value.to_string(),
        }
    }

    /// Parses an unfolded line. The value starts at the first `:` outside a quoted
    /// parameter value (RFC 6350 §3.3 lets `"…"` hold colons, e.g. `TYPE="x:y"`). A line
    /// without a colon is a property with no value, which [`VCard::serialize`] drops.
    pub fn parse(line: &str) -> Self {
        let mut quoted = false;
        let mut colon = None;
        for (i, b) in line.bytes().enumerate() {
            match b {
                b'"' => quoted = !quoted,
                b':' if !quoted => {
                    colon = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let (head, value) = match colon {
            Some(i) => (&line[..i], &line[i + 1..]),
            None => (line, ""),
        };
        let (name_part, params) = match head.find(';') {
            Some(i) => (&head[..i], &head[i + 1..]),
            None => (head, ""),
        };
        let (group, name) = match name_part.rfind('.') {
            Some(i) => (Some(name_part[..i].to_string()), &name_part[i + 1..]),
            None => (None, name_part),
        };
        VCardProperty {
            group,
            name: name.to_ascii_uppercase(),
            params: params.to_string(),
            value: value.to_string(),
        }
    }

    pub fn serialize(&self) -> String {
        let mut s = String::new();
        if let Some(g) = &self.group {
            s.push_str(g);
            s.push('.');
        }
        s.push_str(&self.name);
        if !self.params.is_empty() {
            s.push(';');
            s.push_str(&self.params);
        }
        s.push(':');
        s.push_str(&self.value);
        s
    }
}

/// A vCard as its property lines, in order, without BEGIN/END.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VCard {
    pub properties: Vec<VCardProperty>,
}

impl VCard {
    /// Splits on `\n` (dropping a trailing `\r`) and unfolds continuation lines, which
    /// start with a space or tab (RFC 6350 §3.2).
    pub fn parse(vcf: &str) -> Self {
        let mut properties = Vec::new();
        let mut pending = String::new();
        let flush = |pending: &mut String, props: &mut Vec<VCardProperty>| {
            if !pending.is_empty() {
                let prop = VCardProperty::parse(pending);
                if prop.name != "BEGIN" && prop.name != "END" {
                    props.push(prop);
                }
                pending.clear();
            }
        };
        for raw in vcf.split('\n') {
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix([' ', '\t']) {
                pending.push_str(rest);
            } else {
                flush(&mut pending, &mut properties);
                pending.push_str(line);
            }
        }
        flush(&mut pending, &mut properties);
        VCard { properties }
    }

    /// Always false, as in C++ (`VCard::incomplete`, VCard.cpp:129-133): the parser does not
    /// check the schema. Kept so `Contact::mutate_vcf_in_info` callers have the same gate.
    pub fn incomplete(&self) -> bool {
        false
    }

    /// `BEGIN:VCARD`, every property with a non-empty value, `END:VCARD`, CRLF-terminated.
    pub fn serialize(&self) -> String {
        let mut out = String::from("BEGIN:VCARD\r\n");
        for p in self.properties.iter().filter(|p| !p.value.is_empty()) {
            out.push_str(&p.serialize());
            out.push_str("\r\n");
        }
        out.push_str("END:VCARD\r\n");
        out
    }

    pub fn named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a VCardProperty> + 'a {
        self.properties.iter().filter(move |p| p.name == name)
    }

    /// The first `name` property's raw value, `""` when there is none.
    pub fn value(&self, name: &str) -> &str {
        self.properties
            .iter()
            .find(|p| p.name == name)
            .map_or("", |p| p.value.as_str())
    }

    /// Sets the first `name` property's value, appending the property when missing.
    pub fn set_value(&mut self, name: &str, value: &str) {
        let name = name.to_ascii_uppercase();
        match self.properties.iter_mut().find(|p| p.name == name) {
            Some(p) => p.value = value.to_string(),
            None => self.properties.push(VCardProperty::new(&name, value)),
        }
    }

    pub fn add(&mut self, prop: VCardProperty) {
        self.properties.push(prop);
    }

    /// Removes the first property named `name` with `value`; true when one was removed.
    pub fn remove(&mut self, name: &str, value: &str) -> bool {
        match self
            .properties
            .iter()
            .position(|p| p.name == name && p.value == value)
        {
            Some(i) => {
                self.properties.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn version(&self) -> &str {
        self.value("VERSION")
    }

    /// `DAVUtils::isGroupCard`: vCard 4 `KIND:group`, or the vCard 3 extension. The value
    /// is compared ignoring case (RFC 6350 §6.1.4 defines it in ABNF, where literals are
    /// case-insensitive): sabre/dav (Baikal, Nextcloud) serves a stored vCard 4 group to a
    /// vCard 3 client as `X-ADDRESSBOOKSERVER-KIND:GROUP`, which C++ compared exactly and
    /// missed.
    pub fn is_group(&self) -> bool {
        self.value("KIND").eq_ignore_ascii_case("group")
            || self
                .named(X_VCARD3_KIND)
                .any(|p| p.value.eq_ignore_ascii_case("group"))
    }

    /// Groups whose members are written as `MEMBER`: vCard 4.0 cards that state a KIND.
    /// Everything else (vCard 3, iCloud, Fastmail) uses the extension property.
    fn uses_vcard4_members(&self) -> bool {
        self.version() == "4.0" && !self.value("KIND").is_empty()
    }

    /// `DAVUtils::addMembersToGroupCard`, skipping contacts already listed.
    pub fn add_members<S: AsRef<str>>(&mut self, contact_ids: &[S]) {
        let prop = if self.uses_vcard4_members() {
            "MEMBER"
        } else {
            X_VCARD3_MEMBER
        };
        for id in contact_ids {
            let uuid = format!("{MEMBER_URN_PREFIX}{}", id.as_ref());
            if !self.named(prop).any(|p| p.value == uuid) {
                self.add(VCardProperty::new(prop, &uuid));
            }
        }
    }

    /// `DAVUtils::removeMembersFromGroupCard`.
    pub fn remove_members<S: AsRef<str>>(&mut self, contact_ids: &[S]) {
        let prop = if self.uses_vcard4_members() {
            "MEMBER"
        } else {
            X_VCARD3_MEMBER
        };
        for id in contact_ids {
            self.remove(prop, &format!("{MEMBER_URN_PREFIX}{}", id.as_ref()));
        }
    }

    /// The contact ids a group card lists, from both member forms, `urn:uuid:` stripped
    /// (`rebuildContactGroup`).
    pub fn member_ids(&self) -> Vec<String> {
        self.named("MEMBER")
            .chain(self.named(X_VCARD3_MEMBER))
            .map(|p| {
                p.value
                    .strip_prefix(MEMBER_URN_PREFIX)
                    .unwrap_or(&p.value)
                    .to_string()
            })
            .collect()
    }

    /// The card `SyncbackContactGroupTask` creates for a new group: vCard 3 with the
    /// extension kind, which every server accepts.
    pub fn new_group(uid: &str, name: &str) -> VCard {
        let mut card = VCard::parse(&format!(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{uid}\r\nEND:VCARD\r\n"
        ));
        card.set_value("N", name);
        card.add(VCardProperty::new("FN", name));
        card.add(VCardProperty::new(X_VCARD3_KIND, "group"));
        card
    }

    /// Renames a group card: the first `N` and `FN`, added when missing
    /// (`performLocalSyncbackContactGroup`).
    pub fn rename(&mut self, name: &str) {
        self.set_value("N", name);
        self.set_value("FN", name);
    }
}

/// What a Contact row stores from a card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactFields {
    /// `UID`; empty when absent (the worker then derives an id from the href).
    pub uid: String,
    /// `FN`, else the `N` components, else empty.
    pub name: String,
    /// The first `EMAIL`, empty when none.
    pub email: String,
    pub emails: Vec<String>,
    pub is_group: bool,
    /// Group member contact ids, `urn:uuid:` stripped.
    pub member_ids: Vec<String>,
}

impl ContactFields {
    /// Reads a card's fields, decoding values through calcard.
    pub fn from_vcard(vcf: &str) -> ContactFields {
        let lines = VCard::parse(vcf);
        let mut fields = match CalVCard::parse(vcf) {
            Ok(card) => from_calcard(&card),
            Err(_) => from_lines(&lines),
        };
        // Group detection and members read the line model, which the group editing
        // helpers write; the two must agree.
        fields.is_group = lines.is_group();
        fields.member_ids = lines.member_ids();
        fields
    }
}

fn entry_text(e: &VCardEntry) -> String {
    e.values
        .iter()
        .find_map(VCardValue::as_text)
        .unwrap_or("")
        .trim()
        .to_string()
}

/// `N` is family;given;additional;prefix;suffix (RFC 6350 §6.2.2), shown as
/// "prefix given additional family suffix".
fn structured_name(components: &[String]) -> String {
    let at = |i: usize| components.get(i).map(|s| s.trim()).unwrap_or("");
    [at(3), at(1), at(2), at(0), at(4)]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" ")
}

fn n_components(e: &VCardEntry) -> Vec<String> {
    e.values
        .iter()
        .map(|v| match v {
            VCardValue::Component(parts) => parts.join(" "),
            other => other.as_text().unwrap_or("").to_string(),
        })
        .collect()
}

fn from_calcard(card: &CalVCard) -> ContactFields {
    let uid = card
        .property(&CalProp::Uid)
        .map(|e| {
            e.values
                .iter()
                .find_map(|v| match v {
                    VCardValue::Binary(d) => Some(d.to_unwrapped_string()),
                    other => other.as_text().map(String::from),
                })
                .unwrap_or_default()
        })
        .unwrap_or_default()
        .trim()
        .to_string();
    let formatted = card
        .property(&CalProp::Fn)
        .map(entry_text)
        .unwrap_or_default();
    let name = if formatted.is_empty() {
        card.property(&CalProp::N)
            .map(|e| structured_name(&n_components(e)))
            .unwrap_or_default()
    } else {
        formatted
    };
    let emails: Vec<String> = card
        .properties(&CalProp::Email)
        .map(entry_text)
        .filter(|e| !e.is_empty())
        .collect();
    ContactFields {
        uid,
        name,
        email: emails.first().cloned().unwrap_or_default(),
        emails,
        ..Default::default()
    }
}

fn from_lines(card: &VCard) -> ContactFields {
    let formatted = unescape(card.value("FN"));
    let name = if formatted.is_empty() {
        let n = split_unescaped(card.value("N"), ';');
        structured_name(&n)
    } else {
        formatted
    };
    let emails: Vec<String> = card
        .named("EMAIL")
        .map(|p| unescape(&p.value))
        .filter(|e| !e.is_empty())
        .collect();
    ContactFields {
        uid: card.value("UID").trim().to_string(),
        name,
        email: emails.first().cloned().unwrap_or_default(),
        emails,
        ..Default::default()
    }
}

/// RFC 6350 §3.4 text unescaping.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out.trim().to_string()
}

fn split_unescaped(value: &str, sep: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            parts.last_mut().unwrap().push('\\');
            parts.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(c);
        }
    }
    parts.iter().map(|p| unescape(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICLOUD: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Apple Inc.//iOS 17.0//EN\r\nN:Appleseed;Katie;;;\r\nFN:Katie Appleseed\r\nORG:Running Group;\r\nitem1.EMAIL;type=INTERNET;type=pref:katie@example.com\r\nitem1.X-ABLabel:_$!<Other>!$_\r\nitem2.TEL;type=pref:+18472749609\r\nPHOTO;TYPE=PNG;X-ABCROP-RECTANGLE=ABClipRect_1&0&0&440&440&haCIH6MkV8cyYokE\r\n Bet+qw==;VALUE=uri:https://p52-contacts.icloud.com:443/58330283/wbs/013d0be\r\n 221993f5c4b1b4dd7f8081b8ce3fa776c04\r\nREV:2019-10-11T20:03:16Z\r\nUID:6a5c4f29-1a0e-4e43-a4b5-0f1e5ad0f5e1\r\nEND:VCARD\r\n";

    #[test]
    fn line_model_round_trips_groups_and_folds() {
        let card = VCard::parse(ICLOUD);
        let email = card.named("EMAIL").next().unwrap();
        assert_eq!(email.group.as_deref(), Some("item1"));
        assert_eq!(email.value, "katie@example.com");
        let photo = card.named("PHOTO").next().unwrap();
        assert!(photo.params.ends_with("haCIH6MkV8cyYokEBet+qw==;VALUE=uri"));
        assert_eq!(
            photo.value,
            "https://p52-contacts.icloud.com:443/58330283/wbs/013d0be221993f5c4b1b4dd7f8081b8ce3fa776c04"
        );
        let out = card.serialize();
        assert!(out.contains("\r\nitem1.EMAIL;type=INTERNET;type=pref:katie@example.com\r\n"));
        assert_eq!(VCard::parse(&out), card);
    }

    #[test]
    fn quoted_colon_in_params() {
        let p = VCardProperty::parse("item3.URL;TYPE=\"x:y\":https://example.com");
        assert_eq!(p.params, "TYPE=\"x:y\"");
        assert_eq!(p.value, "https://example.com");
    }

    #[test]
    fn fields_through_calcard() {
        let f = ContactFields::from_vcard(ICLOUD);
        assert_eq!(f.uid, "6a5c4f29-1a0e-4e43-a4b5-0f1e5ad0f5e1");
        assert_eq!(f.name, "Katie Appleseed");
        assert_eq!(f.email, "katie@example.com");
        assert!(!f.is_group);
    }

    #[test]
    fn name_from_n_when_fn_missing() {
        let f = ContactFields::from_vcard(
            "BEGIN:VCARD\r\nVERSION:4.0\r\nN:Doe;John;Q.;Dr.;Jr.\r\nEMAIL:j\\,d@x.com\r\nEND:VCARD\r\n",
        );
        assert_eq!(f.name, "Dr. John Q. Doe Jr.");
    }

    #[test]
    fn group_membership_vcard3_and_4() {
        let mut v3 = VCard::new_group("g1", "Team");
        assert_eq!(
            v3.serialize(),
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:g1\r\nN:Team\r\nFN:Team\r\nX-ADDRESSBOOKSERVER-KIND:group\r\nEND:VCARD\r\n"
        );
        v3.add_members(&["a", "b", "a"]);
        assert_eq!(v3.member_ids(), vec!["a", "b"]);
        v3.remove_members(&["a"]);
        assert_eq!(v3.member_ids(), vec!["b"]);
        assert!(v3.is_group());

        let mut v4 =
            VCard::parse("BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:X\r\nEND:VCARD\r\n");
        v4.add_members(&["c"]);
        assert!(v4.serialize().contains("\r\nMEMBER:urn:uuid:c\r\n"));
        v4.rename("Y");
        assert_eq!(v4.value("FN"), "Y");
        let f = ContactFields::from_vcard(&v4.serialize());
        assert!(f.is_group);
        assert_eq!(f.member_ids, vec!["c"]);
    }

    #[test]
    fn group_kind_ignores_case() {
        // A vCard 4 group as sabre/dav serves it to a vCard 3 client.
        let sabre = VCard::parse(
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Family\r\nX-ADDRESSBOOKSERVER-KIND:GROUP\r\nEND:VCARD\r\n",
        );
        assert!(sabre.is_group());
        assert!(
            VCard::parse("BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:Group\r\nEND:VCARD\r\n").is_group()
        );
        assert!(!VCard::parse("BEGIN:VCARD\r\nKIND:individual\r\nEND:VCARD\r\n").is_group());
    }
}
