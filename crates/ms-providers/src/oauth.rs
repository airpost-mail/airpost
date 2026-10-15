//! OAuth token endpoints (`NetworkRequestUtils.cpp:58-100`).

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

#[cfg(test)]
mod tests {
    use super::*;

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
