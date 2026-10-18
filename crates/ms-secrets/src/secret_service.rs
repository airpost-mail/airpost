// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The Linux Secret Service (GNOME Keyring, KWallet, KeePassXC): one item in the default
//! collection, found by its `service` and `account` attributes, holding the keyset. A locked
//! collection is unlocked, which may show the desktop's prompt; a headless session without a
//! provider gets [`Error::Unavailable`] and should use the file store.

use std::collections::HashMap;

use dbus_secret_service::{EncryptionType, SecretService};

use crate::{Backend, Error, Result, StoreKind};

const CONTENT_TYPE: &str = "application/json";

pub struct SecretServiceBackend {
    service: String,
    account: String,
}

impl SecretServiceBackend {
    pub fn new(service: &str, account: &str) -> Self {
        SecretServiceBackend {
            service: service.to_owned(),
            account: account.to_owned(),
        }
    }

    fn attributes(&self) -> HashMap<&str, &str> {
        HashMap::from([
            ("service", self.service.as_str()),
            ("account", self.account.as_str()),
        ])
    }

    fn connect() -> Result<SecretService> {
        SecretService::connect(EncryptionType::Dh)
            .map_err(|e| Error::Unavailable(format!("Secret Service: {e}")))
    }

    /// Removes the item (tests).
    pub fn delete(&self) -> Result<()> {
        let ss = Self::connect()?;
        let found = ss
            .search_items(self.attributes())
            .map_err(|e| Error::Backend(format!("Secret Service: {e}")))?;
        for item in found.unlocked.iter().chain(found.locked.iter()) {
            item.delete()
                .map_err(|e| Error::Backend(format!("Secret Service: {e}")))?;
        }
        Ok(())
    }
}

impl Backend for SecretServiceBackend {
    fn kind(&self) -> StoreKind {
        StoreKind::SecretService
    }

    fn load(&self) -> Result<Option<Vec<u8>>> {
        let unreadable =
            |e: dbus_secret_service::Error| Error::Unreadable(format!("Secret Service: {e}"));
        let ss = Self::connect()?;
        let found = ss.search_items(self.attributes()).map_err(unreadable)?;
        if let Some(item) = found.unlocked.first() {
            return item.get_secret().map(Some).map_err(unreadable);
        }
        if let Some(item) = found.locked.first() {
            item.unlock().map_err(unreadable)?;
            return item.get_secret().map(Some).map_err(unreadable);
        }
        Ok(None)
    }

    fn save(&self, blob: &[u8]) -> Result<()> {
        let failed = |e: dbus_secret_service::Error| Error::Backend(format!("Secret Service: {e}"));
        let ss = Self::connect()?;
        let collection = ss.get_default_collection().map_err(failed)?;
        if collection.is_locked().map_err(failed)? {
            collection.unlock().map_err(failed)?;
        }
        collection
            .create_item(&self.service, self.attributes(), blob, true, CONTENT_TYPE)
            .map_err(failed)?;
        Ok(())
    }
}
