// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The user's addresses: an account's own (`Account.me()`, `meUsingAlias`, `defaultMe`, its
//! auto-cc/bcc) and, across every account, which addresses are "me" (`Contact.isMe`,
//! `AccountStore.accountForEmail`).

use serde_json::Value;

use crate::contacts::{contact_from_string, emails_equivalent, parse_contacts_in_string, Contact};

/// One account as a draft sees it, from its JSON (`account.list`'s `account`).
#[derive(Clone, Debug, Default)]
pub struct Identity {
    pub account_id: String,
    pub name: String,
    pub email: String,
    /// `Name <email>` strings, or a bare address or name.
    pub aliases: Vec<String>,
    pub default_alias: Option<String>,
    /// `autoaddress`: `cc` or `bcc`, and the addresses.
    pub autoaddress: Option<(String, String)>,
}

impl Identity {
    pub fn from_account_json(v: &Value) -> Identity {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let aliases = v
            .get("aliases")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let default_alias = Some(s("defaultAlias")).filter(|a| !a.is_empty());
        let autoaddress = v.get("autoaddress").and_then(|a| {
            let kind = a.get("type").and_then(Value::as_str)?;
            let value = a.get("value").and_then(Value::as_str)?;
            Some((kind.to_owned(), value.to_owned()))
        });
        Identity {
            account_id: s("id"),
            name: s("name"),
            email: s("emailAddress"),
            aliases,
            default_alias,
            autoaddress,
        }
    }

    /// `Account.me()`.
    pub fn me(&self) -> Contact {
        Contact::new(self.name.clone(), self.email.clone())
    }

    /// `Account.meUsingAlias`: an alias without an address is a name over the account's own.
    pub fn me_using_alias(&self, alias: &str) -> Contact {
        if alias.is_empty() {
            return self.me();
        }
        let mut c = contact_from_string(alias);
        if c.email.is_empty() {
            c.email = self.email.clone();
            if c.name.is_empty() {
                c.name = self.name.clone();
            }
        }
        c
    }

    /// `Account.defaultMe()`.
    pub fn default_me(&self) -> Contact {
        match &self.default_alias {
            Some(a) => self.me_using_alias(a),
            None => self.me(),
        }
    }

    /// Every address of this account: its own, then each alias's.
    pub fn addresses(&self) -> Vec<Contact> {
        let mut out = vec![self.me()];
        out.extend(self.aliases.iter().map(|a| self.me_using_alias(a)));
        out
    }

    /// The auto-cc or auto-bcc contacts (`ContactStore.parseContactsInString`), and which.
    pub fn auto_contacts(&self) -> Option<(&str, Vec<Contact>)> {
        let (kind, value) = self.autoaddress.as_ref()?;
        Some((kind.as_str(), parse_contacts_in_string(value)))
    }

    /// `DraftFactory._fromContactForReply`: the alias the message was sent to, when it names
    /// one that isn't the default, else the default From.
    pub fn reply_from(&self, to: &[Contact], cc: &[Contact]) -> Contact {
        let default_me = self.default_me();
        let mut result = default_me.clone();
        for alias_string in &self.aliases {
            let alias = self.me_using_alias(alias_string);
            for recipient in to.iter().chain(cc) {
                let email_not_default = alias.email != default_me.email;
                let emails_match = recipient.email == alias.email;
                let name_not_default = alias.name != default_me.name;
                let names_match = recipient.name == alias.name;
                if emails_match && email_not_default && names_match && name_not_default {
                    return alias;
                }
                if (emails_match && email_not_default) || (names_match && name_not_default) {
                    result = alias.clone();
                }
            }
        }
        result
    }
}

/// Every account the engine holds, in `account.list` order.
#[derive(Clone, Debug, Default)]
pub struct Identities(pub Vec<Identity>);

impl Identities {
    pub fn from_accounts_json<'a>(accounts: impl IntoIterator<Item = &'a Value>) -> Identities {
        Identities(
            accounts
                .into_iter()
                .map(Identity::from_account_json)
                .collect(),
        )
    }

    pub fn get(&self, account_id: &str) -> Option<&Identity> {
        self.0.iter().find(|i| i.account_id == account_id)
    }

    /// `AccountStore.accountForEmail`: the account whose address, or one of whose aliases',
    /// is equivalent to `email`.
    pub fn account_for_email(&self, email: &str) -> Option<&Identity> {
        if let Some(i) = self.0.iter().find(|i| emails_equivalent(email, &i.email)) {
            return Some(i);
        }
        self.0.iter().find(|i| {
            i.addresses()
                .iter()
                .any(|a| emails_equivalent(email, &a.email))
        })
    }

    /// `Contact.isMe()`.
    pub fn is_me(&self, email: &str) -> bool {
        self.account_for_email(email).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn aliases_and_reply_from() {
        let i = Identity::from_account_json(&json!({
            "id": "a1", "name": "Ann", "emailAddress": "ann@example.com",
            "aliases": ["Ann at Work <ann@work.example>", "Desk Name"],
        }));
        assert_eq!(
            i.me_using_alias("Desk Name"),
            Contact::new("Desk Name", "ann@example.com")
        );
        let to = [Contact::new("Ann at Work", "ann@work.example")];
        assert_eq!(i.reply_from(&to, &[]).email, "ann@work.example");
        assert_eq!(
            i.reply_from(&[Contact::new("", "x@y.example")], &[]),
            i.me()
        );
        let all = Identities(vec![i]);
        assert!(all.is_me("ANN+tag@example.com"));
        assert!(all.is_me("ann@work.example"));
        assert!(!all.is_me("bob@example.com"));
    }
}
