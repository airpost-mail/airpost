// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Where the OAuth token endpoints and the People API live.
//!
//! The defaults are the providers' production URLs (`MakeOAuthRefreshRequest` and
//! `GOOGLE_PEOPLE_ROOT` in Mailspring-Sync, Copyright © 2017 Foundry 376, GPL-3.0; see
//! NOTICE.md). Each can be overridden from the environment so the harness can point the
//! engine at a fake People API and token endpoint.

use tern_providers::OAuthProvider;

/// Overrides Google's token endpoint.
pub const ENV_GOOGLE_TOKEN_URL: &str = "TERN_ENGINE_GOOGLE_TOKEN_URL";
/// Overrides Microsoft's token endpoint.
pub const ENV_MICROSOFT_TOKEN_URL: &str = "TERN_ENGINE_MICROSOFT_TOKEN_URL";
/// Overrides the People API root; must end with `/`.
pub const ENV_GOOGLE_PEOPLE_ROOT: &str = "TERN_ENGINE_GOOGLE_PEOPLE_ROOT";

pub const GOOGLE_PEOPLE_ROOT: &str = "https://people.googleapis.com/v1/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub google_token_url: String,
    pub microsoft_token_url: String,
    pub google_people_root: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            google_token_url: OAuthProvider::Google.token_url().to_owned(),
            microsoft_token_url: OAuthProvider::Microsoft.token_url().to_owned(),
            google_people_root: GOOGLE_PEOPLE_ROOT.to_owned(),
        }
    }
}

impl Endpoints {
    /// The defaults, with any non-empty override from the environment applied.
    pub fn from_env() -> Self {
        let mut e = Endpoints::default();
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        if let Some(v) = var(ENV_GOOGLE_TOKEN_URL) {
            e.google_token_url = v;
        }
        if let Some(v) = var(ENV_MICROSOFT_TOKEN_URL) {
            e.microsoft_token_url = v;
        }
        if let Some(mut v) = var(ENV_GOOGLE_PEOPLE_ROOT) {
            if !v.ends_with('/') {
                v.push('/');
            }
            e.google_people_root = v;
        }
        e
    }

    /// Every endpoint under one root, for tests against a local server.
    pub fn all_at(root: &str) -> Self {
        let root = root.trim_end_matches('/');
        Endpoints {
            google_token_url: format!("{root}/google/token"),
            microsoft_token_url: format!("{root}/microsoft/token"),
            google_people_root: format!("{root}/v1/"),
        }
    }

    pub fn token_url(&self, provider: OAuthProvider) -> &str {
        match provider {
            OAuthProvider::Google => &self.google_token_url,
            OAuthProvider::Microsoft => &self.microsoft_token_url,
        }
    }
}
