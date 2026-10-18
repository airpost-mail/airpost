// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! OAuth endpoints, scopes and the clients the engine signs in with. The token endpoints are
//! derived from Mailspring-Sync `NetworkRequestUtils.cpp:58-100`, the authorization requests,
//! scopes and client ids from Mailspring's `onboarding-constants.ts` and
//! `onboarding-helpers.ts`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.
//!
//! The client ids are Mailspring's registrations until Tern registers its own
//! (docs/desktop-client-plan.md 0.1 item 18): a refresh token stays bound to the client it was
//! issued to, so an account keeps its `refresh_client_id` until it signs in again. Each id comes
//! from the environment at run time, else from the same variable at build time, else the
//! default here.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OAuthProvider {
    Google,
    Microsoft,
}

impl OAuthProvider {
    /// From the account's `provider` setting: `gmail`, `office365` or `outlook`.
    pub fn from_account_provider(provider: &str) -> Option<Self> {
        match provider {
            "gmail" => Some(OAuthProvider::Google),
            "office365" | "outlook" => Some(OAuthProvider::Microsoft),
            _ => None,
        }
    }

    pub fn token_url(self) -> &'static str {
        match self {
            OAuthProvider::Google => "https://www.googleapis.com/oauth2/v4/token",
            OAuthProvider::Microsoft => {
                "https://login.microsoftonline.com/common/oauth2/v2.0/token"
            }
        }
    }

    /// The `scope` sent with a refresh. Microsoft issues a separate access token per
    /// resource, so the refresh names the IMAP and SMTP scopes; Google needs none.
    pub fn refresh_scope(self) -> Option<&'static str> {
        match self {
            OAuthProvider::Google => None,
            OAuthProvider::Microsoft => Some(
                "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send",
            ),
        }
    }
}

/// The Google client id: [`ENV_GMAIL_CLIENT_ID`] at run time, at build time, or Mailspring's.
pub const ENV_GMAIL_CLIENT_ID: &str = "MS_GMAIL_CLIENT_ID";
/// The secret of that client. Google's installed-app clients carry one, which identifies the
/// client rather than proving anything (RFC 8252 §8.5).
pub const ENV_GMAIL_CLIENT_SECRET: &str = "MS_GMAIL_CLIENT_SECRET";
/// The Microsoft (Office 365 and Outlook.com) client id.
pub const ENV_O365_CLIENT_ID: &str = "MS_O365_CLIENT_ID";

const DEFAULT_GMAIL_CLIENT_ID: &str =
    "662287800555-pdiq3r3puob8a44locitndbocua7c30f.apps.googleusercontent.com";
const DEFAULT_O365_CLIENT_ID: &str = "8787a430-6eee-41e1-b914-681d90d35625";

/// The port the desktop's redirect receiver listens on (127.0.0.1), and the default for
/// [`OAuthProvider::redirect_uri`].
pub const DEFAULT_REDIRECT_PORT: u16 = 12141;

pub const GOOGLE_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/auth";
pub const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
/// https://developers.google.com/identity/protocols/oauth2#tokenrevoke
pub const GOOGLE_REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const MICROSOFT_AUTHORIZE_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
pub const MICROSOFT_GRAPH_ME_URL: &str = "https://graph.microsoft.com/v1.0/me";

pub const GMAIL_SCOPES: &[&str] = &[
    "https://mail.google.com/",
    "https://www.googleapis.com/auth/userinfo.email",
    "https://www.googleapis.com/auth/userinfo.profile",
    "https://www.googleapis.com/auth/contacts",
    "https://www.googleapis.com/auth/calendar",
];

/// Graph and Outlook scopes. The code exchange may name only one resource's scopes, so it
/// leaves the Outlook ones out ([`OAuthProvider::exchange_scope`]).
pub const O365_SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "user.read",
    "offline_access",
    "Contacts.ReadWrite",
    "Contacts.ReadWrite.Shared",
    "Calendars.ReadWrite",
    "Calendars.ReadWrite.Shared",
    "https://outlook.office.com/IMAP.AccessAsUser.All",
    "https://outlook.office.com/SMTP.Send",
];

/// A client the engine signs in with.
#[derive(Clone, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    pub secret: Option<String>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("id", &self.id)
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl OAuthProvider {
    /// The client for this provider, `env` read for the run-time overrides.
    pub fn client(self, env: &dyn Fn(&str) -> Option<String>) -> Client {
        let pick = |name: &str, built: Option<&'static str>| {
            env(name)
                .filter(|v| !v.trim().is_empty())
                .or_else(|| built.filter(|v| !v.trim().is_empty()).map(str::to_owned))
        };
        match self {
            OAuthProvider::Google => Client {
                id: pick(ENV_GMAIL_CLIENT_ID, option_env!("MS_GMAIL_CLIENT_ID"))
                    .unwrap_or_else(|| DEFAULT_GMAIL_CLIENT_ID.to_owned()),
                secret: pick(
                    ENV_GMAIL_CLIENT_SECRET,
                    option_env!("MS_GMAIL_CLIENT_SECRET"),
                )
                .or_else(default_gmail_client_secret),
            },
            OAuthProvider::Microsoft => Client {
                id: pick(ENV_O365_CLIENT_ID, option_env!("MS_O365_CLIENT_ID"))
                    .unwrap_or_else(|| DEFAULT_O365_CLIENT_ID.to_owned()),
                secret: None,
            },
        }
    }

    /// [`OAuthProvider::client`] with the process environment.
    pub fn client_from_env(self) -> Client {
        self.client(&|k| std::env::var(k).ok())
    }

    /// Where the provider sends the browser after sign-in: the loopback address the client's
    /// receiver listens on. Microsoft's registration names `localhost` and a path.
    pub fn redirect_uri(self, port: u16) -> String {
        match self {
            OAuthProvider::Google => format!("http://127.0.0.1:{port}"),
            OAuthProvider::Microsoft => format!("http://localhost:{port}/desktop"),
        }
    }

    pub fn authorize_url(self) -> &'static str {
        match self {
            OAuthProvider::Google => GOOGLE_AUTHORIZE_URL,
            OAuthProvider::Microsoft => MICROSOFT_AUTHORIZE_URL,
        }
    }

    /// The sign-in URL with PKCE (`S256`) and `state`.
    pub fn authorization_request(
        self,
        authorize_url: &str,
        client_id: &str,
        redirect_uri: &str,
        code_challenge: &str,
        state: &str,
    ) -> String {
        let mut q: Vec<(&str, String)> = vec![
            ("client_id", client_id.to_owned()),
            ("redirect_uri", redirect_uri.to_owned()),
            ("response_type", "code".to_owned()),
        ];
        match self {
            OAuthProvider::Google => {
                q.push(("scope", GMAIL_SCOPES.join(" ")));
                q.push(("access_type", "offline".to_owned()));
            }
            OAuthProvider::Microsoft => {
                q.push(("scope", O365_SCOPES.join(" ")));
                q.push(("response_mode", "query".to_owned()));
            }
        }
        q.push(("code_challenge", code_challenge.to_owned()));
        q.push(("code_challenge_method", "S256".to_owned()));
        q.push(("state", state.to_owned()));
        q.push((
            "prompt",
            match self {
                OAuthProvider::Google => "select_account consent",
                OAuthProvider::Microsoft => "select_account",
            }
            .to_owned(),
        ));
        let query: Vec<String> = q
            .iter()
            .map(|(k, v)| format!("{k}={}", url_encode(v)))
            .collect();
        format!("{authorize_url}?{}", query.join("&"))
    }

    /// The `scope` of the code exchange: none for Google, the Graph scopes for Microsoft.
    pub fn exchange_scope(self) -> Option<String> {
        match self {
            OAuthProvider::Google => None,
            OAuthProvider::Microsoft => Some(
                O365_SCOPES
                    .iter()
                    .filter(|s| !s.starts_with("https://outlook.office.com"))
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        }
    }
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
pub fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The secret of [`DEFAULT_GMAIL_CLIENT_ID`], kept as `onboarding-constants.ts` kept it:
/// AES-256-CTR over the ciphertext, so the plain text isn't in the source. This is no
/// protection, only what keeps secret scanners from flagging a public client's secret.
fn default_gmail_client_secret() -> Option<String> {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    use base64::Engine as _;
    const KEY: &[u8; 32] = b"don't-be-ev1l-thanks--mailspring";
    const IV: &str = "wgvAx+N05nHqhFxJ9I07jw==";
    const CIPHERTEXT: &str = "1EyEGYVh3NBNIbYEdpdMvOzCH7+vrSciGeYZ1F+W6W+yShk=";
    let b64 = base64::engine::general_purpose::STANDARD;
    let iv: [u8; 16] = b64.decode(IV).ok()?.try_into().ok()?;
    let mut data = b64.decode(CIPHERTEXT).ok()?;
    let cipher = aes::Aes256::new(GenericArray::from_slice(KEY));
    let mut counter = u128::from_be_bytes(iv);
    for chunk in data.chunks_mut(16) {
        let mut block = GenericArray::from(counter.to_be_bytes());
        cipher.encrypt_block(&mut block);
        for (b, k) in chunk.iter_mut().zip(block.iter()) {
            *b ^= k;
        }
        counter = counter.wrapping_add(1);
    }
    String::from_utf8(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clients_come_from_the_environment_first() {
        let none = |_: &str| None;
        let google = OAuthProvider::Google.client(&none);
        if option_env!("MS_GMAIL_CLIENT_ID").is_none() {
            assert_eq!(google.id, DEFAULT_GMAIL_CLIENT_ID);
        }
        let secret = google.secret.expect("a default secret");
        assert!(!secret.is_empty() && secret.is_ascii() && !secret.contains(char::is_whitespace));
        assert!(!format!("{:?}", OAuthProvider::Google.client(&none)).contains(&secret));
        let env = |k: &str| match k {
            ENV_GMAIL_CLIENT_ID => Some("gid".to_owned()),
            ENV_GMAIL_CLIENT_SECRET => Some("gsecret".to_owned()),
            ENV_O365_CLIENT_ID => Some("mid".to_owned()),
            _ => None,
        };
        let google = OAuthProvider::Google.client(&env);
        assert_eq!(
            (google.id.as_str(), google.secret.as_deref()),
            ("gid", Some("gsecret"))
        );
        let ms = OAuthProvider::Microsoft.client(&env);
        assert_eq!((ms.id.as_str(), ms.secret), ("mid", None));
        let blank = |_: &str| Some(" ".to_owned());
        assert_ne!(OAuthProvider::Microsoft.client(&blank).id.trim(), "");
    }

    #[test]
    fn authorization_requests() {
        let url = OAuthProvider::Google.authorization_request(
            GOOGLE_AUTHORIZE_URL,
            "cid",
            &OAuthProvider::Google.redirect_uri(12141),
            "chal",
            "st+/=",
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/auth?client_id=cid&"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A12141&"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("code_challenge=chal&code_challenge_method=S256&state=st%2B%2F%3D"));
        assert!(url.contains("prompt=select_account%20consent"));
        let url = OAuthProvider::Microsoft.authorization_request(
            MICROSOFT_AUTHORIZE_URL,
            "mid",
            &OAuthProvider::Microsoft.redirect_uri(5000),
            "c",
            "s",
        );
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A5000%2Fdesktop"));
        assert!(url.contains("response_mode=query"));
        assert!(url.contains("IMAP.AccessAsUser.All"));
        let scope = OAuthProvider::Microsoft.exchange_scope().unwrap();
        assert!(scope.contains("offline_access") && !scope.contains("outlook.office.com"));
        assert_eq!(OAuthProvider::Google.exchange_scope(), None);
    }

    #[test]
    fn providers() {
        assert_eq!(
            OAuthProvider::from_account_provider("outlook"),
            Some(OAuthProvider::Microsoft)
        );
        assert_eq!(OAuthProvider::from_account_provider("imap"), None);
        assert!(OAuthProvider::Microsoft
            .refresh_scope()
            .unwrap()
            .contains("IMAP.AccessAsUser.All"));
        assert_eq!(OAuthProvider::Google.refresh_scope(), None);
    }
}
