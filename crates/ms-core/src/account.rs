// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The account and identity the client hands the engine on launch. Derived from
//! Mailspring-Sync `MailSync/Models/Account.cpp` and `MailSync/Models/Identity.cpp`,
//! Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! Neither is stored in the database. Both wrap the client's JSON as given so that
//! `toJSON` round-trips every key, including ones the engine does not read.

use serde_json::{Map, Value};

/// `settings.imap_security` / `smtp_security`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    /// `"SSL / TLS"`
    Tls,
    /// `"STARTTLS"`
    StartTls,
    /// anything else
    Clear,
}

impl Security {
    fn from_setting(value: &str) -> Security {
        match value {
            "SSL / TLS" => Security::Tls,
            "STARTTLS" => Security::StartTls,
            _ => Security::Clear,
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
    pub fn from_json(data: Value) -> Result<Self, String> {
        if !data.is_object() {
            return Err("type must be object".into());
        }
        Ok(Account { data })
    }

    /// `""` when usable, otherwise the name of what is missing, appended by the caller to
    /// `"Account is missing required fields:"`.
    pub fn valid(&self) -> &'static str {
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

    pub fn imap_security(&self) -> Security {
        Security::from_setting(self.setting_str("imap_security"))
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

    pub fn smtp_security(&self) -> Security {
        Security::from_setting(self.setting_str("smtp_security"))
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
}

/// The signed-in Mailspring identity. `--identity null` is a normal mode (D2): every
/// feature works without one, and nothing reaches an identity server (D3).
#[derive(Clone, Debug, PartialEq)]
pub struct Identity {
    data: Value,
}

impl Identity {
    pub fn from_json(data: Value) -> Result<Self, String> {
        if !data.is_object() {
            return Err("type must be object".into());
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
        assert_eq!(a.valid(), "");
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
            Account::from_json(v).unwrap().valid(),
            "imap_allow_insecure_ssl"
        );
        assert_eq!(
            Account::from_json(json!({"id": "x"})).unwrap().valid(),
            "id or settings"
        );
        assert!(Account::from_json(json!([])).is_err());
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
