// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The account and identity the client hands the engine on launch. Derived from
//! Mailspring-Sync `MailSync/Models/Account.cpp` and `MailSync/Models/Identity.cpp`,
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Neither is stored in the database. Both wrap the client's JSON as given so that
//! `toJSON` round-trips every key, including ones the engine does not read.

use serde_json::{Map, Value};

/// Why account or identity JSON was refused. `Display` is the text the client is shown.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error("type must be object")]
    NotObject,
    #[error("Account is missing required fields:{0}")]
    MissingFields(&'static str),
    #[error("Account has an invalid {key}: {value} (expected \"SSL / TLS\", \"STARTTLS\" or \"none\")")]
    InvalidSecurity { key: String, value: String },
    #[error("Account has {key} \"none\" for {host} without {proto}_allow_cleartext set to that host (confirm sending the password unencrypted)")]
    UnconfirmedCleartext {
        key: String,
        host: String,
        proto: &'static str,
    },
}

/// `settings.imap_security` / `smtp_security`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// `"SSL / TLS"`
    Tls,
    /// `"STARTTLS"`
    StartTls,
    /// `"none"`, the value the client's security menu stores. Only this exact string opts
    /// into cleartext, so a typo or a missing key can never send the password in clear.
    Clear,
}

impl Security {
    /// The setting's value as stored, or why it is refused (unknown, empty, missing or not
    /// a string). `key` names the setting in the message.
    pub fn from_setting(key: &str, value: Option<&Value>) -> Result<Security, AccountError> {
        match value.and_then(Value::as_str) {
            Some("SSL / TLS") => Ok(Security::Tls),
            Some("STARTTLS") => Ok(Security::StartTls),
            Some("none") => Ok(Security::Clear),
            _ => Err(AccountError::InvalidSecurity {
                key: key.to_owned(),
                value: value.map_or_else(|| "missing".to_owned(), Value::to_string),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    data: Value,
}

impl Account {
    /// Wraps account JSON. Fails only when it is not an object; check [`Account::valid`]
    /// before using the accessors.
    pub fn from_json(data: Value) -> Result<Self, AccountError> {
        if !data.is_object() {
            return Err(AccountError::NotObject);
        }
        Ok(Account { data })
    }

    /// `Ok` when usable, otherwise the error the client is shown:
    /// `"Account is missing required fields:<what>"` or an invalid security setting.
    pub fn validate(&self) -> Result<(), AccountError> {
        let missing = self.missing_fields();
        if !missing.is_empty() {
            return Err(AccountError::MissingFields(missing));
        }
        for proto in ["imap", "smtp"] {
            let key = format!("{proto}_security");
            if Security::from_setting(&key, self.settings().get(&key))? == Security::Clear
                && !self.cleartext_confirmed(proto)
            {
                return Err(AccountError::UnconfirmedCleartext {
                    host: Value::from(self.setting_str(&format!("{proto}_host"))).to_string(),
                    key,
                    proto,
                });
            }
        }
        Ok(())
    }

    /// `"none"` sends the password in clear, so a remote host needs the user's confirmation:
    /// `<proto>_allow_cleartext` holds the host it was given for, and stops counting once the
    /// host changes. Loopback hosts (Proton Bridge on 127.0.0.1) never leave the machine.
    /// Accounts onboarded before the confirmation existed store `"none"` without it, often
    /// because discovery picked it, and are refused until the user confirms again.
    fn cleartext_confirmed(&self, proto: &str) -> bool {
        let norm = |h: &str| h.trim().to_ascii_lowercase();
        let host = norm(self.setting_str(&format!("{proto}_host")));
        if ["127.0.0.1", "localhost", "::1", "[::1]"].contains(&host.as_str()) {
            return true;
        }
        let consent = self.setting_str(&format!("{proto}_allow_cleartext"));
        !host.is_empty() && norm(consent) == host
    }

    /// `""` when every required field is there, otherwise the name of what is missing.
    fn missing_fields(&self) -> &'static str {
        let d = &self.data;
        if d.get("id").is_none() || d.get("settings").is_none() {
            return "id or settings";
        }
        if d.get("provider").is_none() {
            return "provider";
        }
        let s = &d["settings"];
        let has = |k: &str| s.get(k).is_some();
        if !(has("refresh_token") || has("imap_password")) {
            return "imap_password or refresh_token";
        }
        if !(has("refresh_token") || has("smtp_password")) {
            return "smtp_password or refresh_token";
        }
        if !(has("imap_port") && has("imap_host") && has("imap_username")) {
            return "imap configuration";
        }
        if !(has("smtp_port") && has("smtp_host")) {
            return "smtp configuration";
        }
        if !s
            .get("imap_allow_insecure_ssl")
            .is_some_and(Value::is_boolean)
        {
            return "imap_allow_insecure_ssl";
        }
        if !s
            .get("smtp_allow_insecure_ssl")
            .is_some_and(Value::is_boolean)
        {
            return "smtp_allow_insecure_ssl";
        }
        ""
    }

    pub fn to_json(&self) -> Value {
        self.data.clone()
    }

    pub fn as_json(&self) -> &Value {
        &self.data
    }

    fn str_at(&self, key: &str) -> &str {
        self.data.get(key).and_then(Value::as_str).unwrap_or("")
    }

    fn setting_str(&self, key: &str) -> &str {
        self.settings()
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    fn setting_port(&self, key: &str) -> u16 {
        match self.settings().get(key) {
            Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
            Some(v) => v.as_u64().unwrap_or(0) as u16,
            None => 0,
        }
    }

    fn setting_bool(&self, key: &str, default: bool) -> bool {
        self.settings()
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }

    pub fn settings(&self) -> &Map<String, Value> {
        static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
        self.data
            .get("settings")
            .and_then(Value::as_object)
            .unwrap_or_else(|| EMPTY.get_or_init(Map::new))
    }

    pub fn id(&self) -> &str {
        self.str_at("id")
    }

    pub fn provider(&self) -> &str {
        self.str_at("provider")
    }

    pub fn email_address(&self) -> &str {
        self.str_at("emailAddress")
    }

    /// The account's address and its aliases' (`Name <email>` or a bare address), trimmed.
    pub fn own_addresses(&self) -> impl Iterator<Item = &str> {
        let aliases = self.data["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|a| match (a.rfind('<'), a.rfind('>')) {
                (Some(l), Some(r)) if l < r => a[l + 1..r].trim(),
                _ => a.trim(),
            });
        std::iter::once(self.email_address().trim()).chain(aliases)
    }

    /// Seconds the background worker waits before its first pass, staggered by the first
    /// character of the id so many accounts do not open the database at once. Ids starting
    /// with c-f (the harness uses `c0ffee-harness`) start immediately.
    pub fn start_delay(&self) -> u64 {
        match self.id().chars().next() {
            Some('0' | '1' | '2') => 1,
            Some('3' | '4' | '5') => 2,
            Some('6' | '7' | '8') => 3,
            Some('9' | 'a' | 'b') => 4,
            _ => 0,
        }
    }

    pub fn refresh_token(&self) -> &str {
        self.setting_str("refresh_token")
    }

    pub fn set_refresh_token(&mut self, token: &str) {
        if let Some(s) = self.data.get_mut("settings").and_then(Value::as_object_mut) {
            s.insert("refresh_token".into(), Value::String(token.into()));
        }
    }

    pub fn refresh_client_id(&self) -> &str {
        self.setting_str("refresh_client_id")
    }

    pub fn imap_host(&self) -> &str {
        self.setting_str("imap_host")
    }

    pub fn imap_port(&self) -> u16 {
        self.setting_port("imap_port")
    }

    pub fn imap_username(&self) -> &str {
        self.setting_str("imap_username")
    }

    pub fn imap_password(&self) -> &str {
        self.setting_str("imap_password")
    }

    /// [`Account::validate`] refuses an invalid value and an unconfirmed `"none"`; an
    /// unvalidated account falls back to TLS, so it fails the handshake instead of sending
    /// in clear.
    pub fn imap_security(&self) -> Security {
        self.security("imap_security")
    }

    pub fn imap_allow_insecure_ssl(&self) -> bool {
        self.setting_bool("imap_allow_insecure_ssl", false)
    }

    pub fn smtp_host(&self) -> &str {
        self.setting_str("smtp_host")
    }

    pub fn smtp_port(&self) -> u16 {
        self.setting_port("smtp_port")
    }

    pub fn smtp_username(&self) -> &str {
        self.setting_str("smtp_username")
    }

    pub fn smtp_password(&self) -> &str {
        self.setting_str("smtp_password")
    }

    /// See [`Account::imap_security`].
    pub fn smtp_security(&self) -> Security {
        self.security("smtp_security")
    }

    fn security(&self, key: &str) -> Security {
        match Security::from_setting(key, self.settings().get(key)) {
            Ok(Security::Clear) if !self.cleartext_confirmed(key.trim_end_matches("_security")) => {
                Security::Tls
            }
            Ok(s) => s,
            Err(_) => Security::Tls,
        }
    }

    pub fn smtp_allow_insecure_ssl(&self) -> bool {
        self.setting_bool("smtp_allow_insecure_ssl", false)
    }

    /// `"login"` verifies SMTP credentials only in `test` mode instead of sending a test
    /// message (accounts without send rights, e.g. O365 shared mailboxes).
    pub fn smtp_verification(&self) -> &str {
        self.setting_str("smtp_verification")
    }

    /// False for accounts where any folder we create is visible to others (O365 shared
    /// mailboxes); snooze is unavailable there.
    pub fn create_helper_folders(&self) -> bool {
        self.setting_bool("create_helper_folders", true)
    }

    pub fn container_folder(&self) -> &str {
        self.setting_str("container_folder")
    }

    /// `save_drafts_to_server` (Preferences "Save drafts to server"), on unless set to
    /// `false`: drafts are APPENDed to the Drafts folder and a server draft is one row across
    /// devices (docs/drafts-sync-design.md). Off, drafts stay local and server drafts are
    /// ordinary messages, as in the C++ engine; the harness runs with it off unless a
    /// scenario turns it on, so A/B runs compare like with like.
    pub fn save_drafts_to_server(&self) -> bool {
        self.setting_bool(SAVE_DRAFTS_TO_SERVER, true)
    }
}

/// The account setting behind [`Account::save_drafts_to_server`].
pub const SAVE_DRAFTS_TO_SERVER: &str = "save_drafts_to_server";

/// The signed-in Mailspring identity. `--identity null` is a normal mode (D2): every
/// feature works without one, and nothing reaches an identity server (D3).
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    data: Value,
}

impl Identity {
    pub fn from_json(data: Value) -> Result<Self, AccountError> {
        if !data.is_object() {
            return Err(AccountError::NotObject);
        }
        Ok(Identity { data })
    }

    pub fn valid(&self) -> bool {
        self.data.get("emailAddress").is_some() && self.data.get("token").is_some()
    }

    pub fn email_address(&self) -> &str {
        self.data
            .get("emailAddress")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn token(&self) -> &str {
        self.data.get("token").and_then(Value::as_str).unwrap_or("")
    }

    /// `createdAt` (`2018-02-15T00:00:00.000Z`) in epoch seconds, if present and readable.
    pub fn created_at(&self) -> Option<i64> {
        let s = self.data.get("createdAt").and_then(Value::as_str)?;
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp())
    }

    pub fn as_json(&self) -> &Value {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn harness_account() -> Value {
        json!({
            "id": "c0ffee-harness", "__cls": "Account", "provider": "imap",
            "emailAddress": "test@example.test",
            "settings": {
                "imap_host": "127.0.0.1", "imap_port": 1143, "imap_username": "test",
                "imap_password": "pass", "imap_security": "none", "imap_allow_insecure_ssl": false,
                "smtp_host": "127.0.0.1", "smtp_port": "10025", "smtp_username": "test",
                "smtp_password": "pass", "smtp_security": "SSL / TLS", "smtp_allow_insecure_ssl": false,
                "container_folder": "", "create_helper_folders": false
            }
        })
    }

    #[test]
    fn accessors() {
        let a = Account::from_json(harness_account()).unwrap();
        assert_eq!(a.validate(), Ok(()));
        assert_eq!(a.start_delay(), 0);
        assert_eq!(a.imap_port(), 1143);
        assert_eq!(a.smtp_port(), 10025);
        assert_eq!(a.imap_security(), Security::Clear);
        assert_eq!(a.smtp_security(), Security::Tls);
        assert!(!a.create_helper_folders());
    }

    #[test]
    fn validity() {
        let mut v = harness_account();
        v["settings"]
            .as_object_mut()
            .unwrap()
            .remove("imap_allow_insecure_ssl");
        assert_eq!(
            Account::from_json(v).unwrap().validate().map_err(|e| e.to_string()),
            Err("Account is missing required fields:imap_allow_insecure_ssl".into())
        );
        assert_eq!(
            Account::from_json(json!({"id": "x"})).unwrap().validate().map_err(|e| e.to_string()),
            Err("Account is missing required fields:id or settings".into())
        );
        assert!(Account::from_json(json!([])).is_err());
    }

    #[test]
    fn security_from_setting() {
        let parse = |v: Value| Security::from_setting("imap_security", Some(&v));
        assert_eq!(parse(json!("SSL / TLS")), Ok(Security::Tls));
        assert_eq!(parse(json!("STARTTLS")), Ok(Security::StartTls));
        assert_eq!(parse(json!("none")), Ok(Security::Clear));
        for bad in [
            json!(""),
            json!("None"),
            json!("NONE"),
            json!("ssl / tls"),
            json!("starttls"),
            json!("TLS"),
            json!(" none"),
            json!(null),
            json!(false),
            json!(0),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad} was accepted");
        }
        assert_eq!(
            Security::from_setting("smtp_security", None).map_err(|e| e.to_string()),
            Err("Account has an invalid smtp_security: missing (expected \"SSL / TLS\", \"STARTTLS\" or \"none\")".into())
        );
        assert_eq!(
            parse(json!("None")).map_err(|e| e.to_string()),
            Err("Account has an invalid imap_security: \"None\" (expected \"SSL / TLS\", \"STARTTLS\" or \"none\")".into())
        );
    }

    #[test]
    fn validate_rejects_invalid_security() {
        for key in ["imap_security", "smtp_security"] {
            for bad in [
                Some(json!("")),
                Some(json!("None")),
                Some(json!("plain")),
                None,
            ] {
                let mut v = harness_account();
                let s = v["settings"].as_object_mut().unwrap();
                match &bad {
                    Some(b) => s.insert(key.into(), b.clone()),
                    None => s.remove(key),
                };
                let a = Account::from_json(v).unwrap();
                let err = a.validate().unwrap_err().to_string();
                assert!(err.contains(key), "{err}");
                // Never Clear: the accessor fails closed.
                let sec = if key == "imap_security" {
                    a.imap_security()
                } else {
                    a.smtp_security()
                };
                assert_eq!(sec, Security::Tls, "{key} {bad:?}");
            }
        }
    }

    #[test]
    fn cleartext_needs_confirmation_for_a_remote_host() {
        let with = |settings: Value| {
            let mut v = harness_account();
            let s = v["settings"].as_object_mut().unwrap();
            for (k, val) in settings.as_object().unwrap() {
                s.insert(k.clone(), val.clone());
            }
            Account::from_json(v).unwrap()
        };
        // Loopback needs nothing.
        for host in ["127.0.0.1", "localhost", "::1", " LocalHost "] {
            let a = with(json!({"imap_host": host}));
            assert_eq!(a.validate(), Ok(()), "{host}");
            assert_eq!(a.imap_security(), Security::Clear);
        }
        // A remote host without the confirmation, with a stale boolean, or with one given
        // for another host: refused, and the accessor fails closed.
        for consent in [
            None,
            Some(json!(true)),
            Some(json!("")),
            Some(json!("imap.other.example")),
        ] {
            let mut s = json!({"imap_host": "imap.example.com"});
            if let Some(c) = &consent {
                s["imap_allow_cleartext"] = c.clone();
            }
            let a = with(s);
            let err = a.validate().unwrap_err().to_string();
            assert!(
                err.starts_with("Account has imap_security \"none\" for \"imap.example.com\""),
                "{consent:?}: {err}"
            );
            assert_eq!(a.imap_security(), Security::Tls, "{consent:?}");
        }
        let a = with(
            json!({"imap_host": "imap.example.com", "imap_allow_cleartext": "IMAP.example.com"}),
        );
        assert_eq!(a.validate(), Ok(()));
        assert_eq!(a.imap_security(), Security::Clear);
        // SMTP the same way.
        let a = with(json!({"smtp_host": "smtp.example.com", "smtp_security": "none"}));
        assert!(a.validate().unwrap_err().to_string().contains("smtp_allow_cleartext"));
        assert_eq!(a.smtp_security(), Security::Tls);
    }

    #[test]
    fn identity() {
        let i = Identity::from_json(
            json!({"emailAddress": "a@b", "token": "t", "createdAt": "2018-02-15T00:00:00.000Z"}),
        )
        .unwrap();
        assert!(i.valid());
        assert_eq!(i.created_at(), Some(1_518_652_800));
    }
}
