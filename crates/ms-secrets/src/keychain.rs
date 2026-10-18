// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The macOS Keychain: one generic password item holding the keyset. Items the bundled,
//! signed engine creates stay readable across updates signed with the same Developer ID;
//! unsigned builds are asked about on every rebuild, which is why dev builds use the file
//! store (docs/desktop-client-plan.md section 5).

use security_framework::passwords::{
    delete_generic_password, generic_password, set_generic_password, PasswordOptions,
};

use crate::{Backend, Error, Result, StoreKind};

/// `errSecItemNotFound`.
const ITEM_NOT_FOUND: i32 = -25300;

pub struct KeychainBackend {
    service: String,
    account: String,
}

impl KeychainBackend {
    pub fn new(service: &str, account: &str) -> Self {
        KeychainBackend {
            service: service.to_owned(),
            account: account.to_owned(),
        }
    }

    /// Removes the item (tests).
    pub fn delete(&self) -> Result<()> {
        match delete_generic_password(&self.service, &self.account) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == ITEM_NOT_FOUND => Ok(()),
            Err(e) => Err(Error::Backend(format!("Keychain: {e}"))),
        }
    }
}

impl Backend for KeychainBackend {
    fn kind(&self) -> StoreKind {
        StoreKind::Keychain
    }

    fn load(&self) -> Result<Option<Vec<u8>>> {
        match generic_password(PasswordOptions::new_generic_password(
            &self.service,
            &self.account,
        )) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.code() == ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(Error::Unreadable(format!("Keychain: {e}"))),
        }
    }

    fn save(&self, blob: &[u8]) -> Result<()> {
        set_generic_password(&self.service, &self.account, blob)
            .map_err(|e| Error::Backend(format!("Keychain: {e}")))
    }
}
