// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! A model loaded by class name (`MailStore::findGeneric` / `findAllGeneric`), for plugin
//! metadata, which names its object by `objectType`. Derived from Mailspring-Sync
//! `MailSync/MailStore.cpp`, Copyright © 2017 Foundry 376, GPL-3.0. See NOTICE.md.

use airpost_core::{Error, Result};
use serde_json::Value;

use crate::{Contact, Message, Model, Store, Thread};

#[derive(Clone, Debug, PartialEq)]
pub enum GenericModel {
    Message(Message),
    Thread(Thread),
    Contact(Contact),
}

/// The classes metadata can target, matched case-insensitively as in the C++.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenericClass {
    Message,
    Thread,
    Contact,
}

impl GenericClass {
    pub(crate) fn parse(class: &str) -> Result<Self> {
        match class.to_ascii_lowercase().as_str() {
            "message" => Ok(GenericClass::Message),
            "thread" => Ok(GenericClass::Thread),
            "contact" => Ok(GenericClass::Contact),
            _ => Err(Error::other(format!(
                "findGeneric: unsupported model class {class}"
            ))),
        }
    }
}

macro_rules! each {
    ($self:expr, $m:ident => $e:expr) => {
        match $self {
            GenericModel::Message($m) => $e,
            GenericModel::Thread($m) => $e,
            GenericModel::Contact($m) => $e,
        }
    };
}

impl GenericModel {
    pub fn table(&self) -> &'static str {
        match self {
            GenericModel::Message(_) => Message::TABLE,
            GenericModel::Thread(_) => Thread::TABLE,
            GenericModel::Contact(_) => Contact::TABLE,
        }
    }

    pub fn id(&self) -> &str {
        each!(self, m => m.id())
    }

    pub fn account_id(&self) -> &str {
        each!(self, m => m.account_id())
    }

    pub fn json(&self) -> &Value {
        each!(self, m => m.json())
    }

    pub fn delta_json(&self) -> Value {
        each!(self, m => m.delta_json())
    }

    pub fn upsert_metadata(
        &mut self,
        plugin_id: &str,
        value: &Value,
        version: Option<i64>,
    ) -> Option<i64> {
        each!(self, m => m.upsert_metadata(plugin_id, value, version))
    }

    pub fn save(&mut self, store: &Store) -> Result<()> {
        each!(self, m => store.save(m))
    }
}
