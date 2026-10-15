//! Plugin metadata (snooze, send-later, reminders, open/link tracking) as it moves between
//! models, the `DetatchedPluginMetadata` table and a [`MetadataBackend`]
//! (`ms_net::metadata`). Derived from `struct Metadata` in `MailSync/MailStore.hpp`.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Metadata {
    pub object_type: String,
    pub object_id: String,
    pub account_id: String,
    pub plugin_id: String,
    pub version: i64,
    pub value: Value,
}

impl Metadata {
    /// The id server's delta shape: `{object_type, object_id, aid, plugin_id, v, value}`
    /// (`MetadataFromJSON`).
    pub fn from_json(v: &Value) -> Option<Metadata> {
        Some(Metadata {
            object_type: v.get("object_type")?.as_str()?.to_owned(),
            object_id: v.get("object_id")?.as_str()?.to_owned(),
            account_id: v.get("aid")?.as_str()?.to_owned(),
            plugin_id: v.get("plugin_id")?.as_str()?.to_owned(),
            version: v.get("v")?.as_i64()?,
            value: v.get("value").cloned().unwrap_or(Value::Null),
        })
    }
}
