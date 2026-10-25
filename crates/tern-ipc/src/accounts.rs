// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Params and results of the account setup methods
//! ([`feature::ACCOUNT_SETUP`](crate::feature::ACCOUNT_SETUP)): discovery, OAuth sign-in, secrets
//! written but never read back, account edits and the Tern server the engine syncs metadata with.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The settings an account id is derived from, in order (the desktop's `idForAccount`).
pub const ACCOUNT_ID_SETTINGS: [&str; 4] =
    ["imap_username", "imap_host", "smtp_username", "smtp_host"];

/// An account's secrets, under the `settings` keys they replace. Write-only: no method
/// returns them.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AccountSecrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smtp_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

impl std::fmt::Debug for AccountSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let shown = |v: &Option<String>| v.as_ref().map(|_| "<redacted>");
        f.debug_struct("AccountSecrets")
            .field("imap_password", &shown(&self.imap_password))
            .field("smtp_password", &shown(&self.smtp_password))
            .field("refresh_token", &shown(&self.refresh_token))
            .finish()
    }
}

impl AccountSecrets {
    /// The non-empty ones, as `(settings key, value)`.
    pub fn entries(&self) -> Vec<(&'static str, &str)> {
        [
            ("imap_password", &self.imap_password),
            ("smtp_password", &self.smtp_password),
            ("refresh_token", &self.refresh_token),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.as_deref().filter(|s| !s.is_empty()).map(|s| (k, s)))
        .collect()
    }

    /// Writes them into `account.settings`.
    pub fn apply_to(&self, account: &mut Value) {
        if let Some(settings) = account.get_mut("settings").and_then(Value::as_object_mut) {
            for (k, v) in self.entries() {
                settings.insert(k.to_owned(), Value::String(v.to_owned()));
            }
        }
    }
}

/// `account.discover` params.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AccountDiscoverParams {
    pub email: String,
    /// The provider preset the user picked: `imap` (default), `gmail`, `office365`,
    /// `outlook`, `yahoo`, `icloud`, `fastmail`, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// `account.discover` result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountDiscoverResult {
    /// `{emailAddress, provider, settings}`: hosts, ports, usernames, security,
    /// `*_allow_insecure_ssl` and `container_folder`. Security is never `none`. An empty
    /// `imap_host` means nothing was found that offers IMAP; the client asks for the servers.
    #[schemars(with = "crate::models::DiscoveredAccountJson")]
    pub account: Value,
    /// `matcher`, `autoconfig`, `provider-table` or `fallback` (the `imap.<domain>` guess).
    pub source: String,
    /// The table entry used, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// `account.oauth.begin` params.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OAuthBeginParams {
    /// `gmail`, `office365` or `outlook`.
    pub provider: String,
    /// Office 365: the shared mailbox to open with the signed-in user's grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_mailbox: Option<String>,
    /// The loopback port the client's redirect receiver listens on (default 12141).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_port: Option<u16>,
    /// Reconnect: the registered account the sign-in renews. `complete` refuses a sign-in
    /// to another mailbox and keeps the account's id and settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// The client's default `container_folder` for a new account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_folder: Option<String>,
}

/// `account.oauth.begin` result. The client opens `authUrl` in a browser and passes the
/// `code` and `state` the provider sends to `redirectUri` to `account.oauth.complete`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OAuthBeginResult {
    pub session_id: String,
    pub auth_url: String,
    pub redirect_uri: String,
    /// The `state` in `authUrl`, so the receiver can drop a redirect meant for another
    /// sign-in before it reaches the engine. The engine checks it again.
    pub state: String,
}

/// `account.oauth.complete` params. A session is used once, and expires 15 minutes after
/// `begin`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCompleteParams {
    pub session_id: String,
    pub code: String,
    pub state: String,
    /// Test the account before storing it (default true), as `account.add`'s `test`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<bool>,
}

/// `account.oauth.complete` result: the account the engine tested, stored and started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCompleteResult {
    /// Its JSON without secrets.
    #[schemars(with = "crate::models::AccountJson")]
    pub account: Value,
    #[serde(flatten)]
    pub added: crate::AccountAddResult,
}

/// `account.setSecrets` params: new credentials for a registered account (reconnect). The
/// account restarts with them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountSetSecretsParams {
    pub account_id: String,
    pub secrets: AccountSecrets,
    /// Test them first, as [`AccountAddParams::test`](crate::AccountAddParams::test).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub test: bool,
}

/// `account.update` params. `patch` holds top-level Account fields to replace and a
/// `settings` object whose keys replace the account's (null removes one). Secrets and `id`
/// can't be patched. Changing where the account connects (host, port, username, security,
/// provider or OAuth client) deletes its stored secrets, so they can't be sent to the new
/// server; the account stays stopped until `account.setSecrets`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdateParams {
    pub account_id: String,
    pub patch: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdateResult {
    /// The account's JSON without secrets.
    #[schemars(with = "crate::models::AccountJson")]
    pub account: Value,
    /// The stored secrets were deleted: send new ones with `account.setSecrets`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub secrets_cleared: bool,
    /// The running account restarted with the new settings.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restarted: bool,
}

/// The Account fields a relayed device without the admin scope may change with
/// `account.update`: how the account is shown, nothing about where it connects.
pub const ACCOUNT_DISPLAY_FIELDS: &[&str] = &["name", "label", "color", "aliases", "defaultAlias"];

/// `account.reorder` params: account ids in the order `account.list` returns them. Ids not
/// named keep their order after the named ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AccountReorderParams {
    pub ids: Vec<String>,
}

/// `secrets.set` params. Names: `integration:<provider>`, `ai:<provider>`,
/// `tern-server-token` and the like; an account's secrets go through `account.*`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretsSetParams {
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for SecretsSetParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsSetParams")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretsDeleteParams {
    pub name: String,
}

/// `secrets.has` params; the result maps each name to whether a value is stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretsHasParams {
    pub names: Vec<String>,
}

pub type SecretsHasResult = BTreeMap<String, bool>;

/// `server.set` params: the Tern server the engine's accounts sync metadata with. Its token
/// is the secret `tern-server-token` (`secrets.set`). Running accounts restart with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServerSetParams {
    pub url: String,
    pub user_id: String,
    pub server_id: String,
}

/// `server.set` and `server.clear` result: the accounts restarted to pick up the server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServerSetResult {
    pub restarted: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secrets_never_print() {
        let s = AccountSecrets {
            imap_password: Some("pw".into()),
            refresh_token: Some(String::new()),
            ..Default::default()
        };
        assert!(!format!("{s:?}").contains("pw"));
        assert_eq!(s.entries(), vec![("imap_password", "pw")]);
        let mut account = json!({"settings": {"imap_password": "old"}});
        s.apply_to(&mut account);
        assert_eq!(account["settings"]["imap_password"], "pw");
        let p = SecretsSetParams {
            name: "integration:deepl".into(),
            value: "k3y".into(),
        };
        assert!(!format!("{p:?}").contains("k3y"));
    }

    #[test]
    fn shapes() {
        let p: OAuthBeginParams =
            serde_json::from_value(json!({"provider": "office365", "sharedMailbox": "s@x.test"}))
                .unwrap();
        assert_eq!(p.shared_mailbox.as_deref(), Some("s@x.test"));
        let r = OAuthCompleteResult {
            account: json!({"id": "a"}),
            added: crate::AccountAddResult {
                account_id: "a".into(),
                state: "running".into(),
                error: None,
                stored: true,
            },
        };
        assert_eq!(
            serde_json::to_value(r).unwrap(),
            json!({"account": {"id": "a"}, "accountId": "a", "state": "running", "stored": true})
        );
        let p: crate::AccountRemoveParams =
            serde_json::from_value(json!({"accountId": "a", "revoke": true, "deleteData": true}))
                .unwrap();
        assert!(p.revoke && p.delete_data);
    }
}
