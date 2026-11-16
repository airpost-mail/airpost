// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! [`Secret`]: a string that never shows in `Debug` output and can't be serialized.

use std::fmt;

use serde::Deserialize;

/// An API key or account name. Deserializes from a plain JSON string; has no `Serialize`
/// or `Display`, and `Debug` prints `Secret(***)`, so it can't reach a log or a response by
/// accident. Read it with [`Secret::expose`].
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Empty or only whitespace: the user left the field blank.
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Secret::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Secret(value)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// A present, non-blank secret.
pub(crate) fn filled(secret: &Option<Secret>) -> Option<&str> {
    secret
        .as_ref()
        .filter(|s| !s.is_blank())
        .map(|s| s.expose().trim())
}
