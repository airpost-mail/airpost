// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! What `query` can match and sort on: per class, the queryable attributes by the desktop's
//! model key (`lastMessageReceivedTimestamp`), each with the JSON key it has in the class's
//! view struct ([`crate::models`]) and where the SQL finds it. tern-store compiles only what
//! this table names; `cargo run -p tern-ipc --bin tern-ipc-schema` writes it to
//! `crates/tern-store/src/model_query_allowlist.json`, which tern-store embeds.
//!
//! The desktop's models declare the same attributes `queryable`;
//! `apps/desktop/tests/integration/model-query-capture.test.ts` fails when they and the
//! checked-in file differ.

use serde_json::{json, Map, Value};

/// Where an attribute lives in the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrKind {
    String {
        column: &'static str,
    },
    Number {
        column: &'static str,
    },
    Boolean {
        column: &'static str,
    },
    Datetime {
        column: &'static str,
    },
    /// Matched through a join table, `containsAny`/`contains`; the join table also holds the
    /// listed attributes so a thread list filters and sorts there.
    Collection {
        join_table: &'static str,
        join_queryable_by: &'static [&'static str],
    },
    /// Stored in another table, loaded with `include`.
    JoinedData {
        table: &'static str,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryAttr {
    /// The desktop's model key, which `query` names.
    pub name: &'static str,
    /// The key in the model JSON (the view struct's field), when the attribute is one.
    pub json: Option<&'static str>,
    pub kind: AttrKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueryClass {
    pub class: &'static str,
    pub attributes: &'static [QueryAttr],
    /// `(attribute, "asc" | "desc")` when the class sorts by default.
    pub natural_order: Option<(&'static str, &'static str)>,
}

const fn attr(name: &'static str, json: &'static str, kind: AttrKind) -> QueryAttr {
    QueryAttr {
        name,
        json: Some(json),
        kind,
    }
}

const fn string(name: &'static str, json: &'static str) -> QueryAttr {
    attr(name, json, AttrKind::String { column: name })
}

const fn number(name: &'static str, json: &'static str) -> QueryAttr {
    attr(name, json, AttrKind::Number { column: name })
}

const fn boolean(name: &'static str, json: &'static str) -> QueryAttr {
    attr(name, json, AttrKind::Boolean { column: name })
}

const fn datetime(name: &'static str, json: &'static str) -> QueryAttr {
    attr(name, json, AttrKind::Datetime { column: name })
}

const ACCOUNT_ID: QueryAttr = string("accountId", "aid");
const ID: QueryAttr = string("id", "id");
const VERSION: QueryAttr = number("version", "v");
const PLUGIN_METADATA: QueryAttr = attr(
    "pluginMetadata",
    "metadata",
    AttrKind::Collection {
        join_table: "ModelPluginMetadata",
        join_queryable_by: &[],
    },
);

const CATEGORY: &[QueryAttr] = &[
    ACCOUNT_ID,
    ID,
    string("path", "path"),
    string("role", "role"),
];

/// The classes `query` serves.
pub const QUERY_CLASSES: &[QueryClass] = &[
    QueryClass {
        class: "Thread",
        attributes: &[
            ACCOUNT_ID,
            // A Thread's folders and labels together, through ThreadCategory.
            QueryAttr {
                name: "categories",
                json: None,
                kind: AttrKind::Collection {
                    join_table: "ThreadCategory",
                    join_queryable_by: &[
                        "inAllMail",
                        "lastMessageReceivedTimestamp",
                        "lastMessageSentTimestamp",
                        "unread",
                    ],
                },
            },
            datetime("firstMessageTimestamp", "fmt"),
            ID,
            boolean("inAllMail", "inAllMail"),
            datetime("lastMessageReceivedTimestamp", "lmrt"),
            datetime("lastMessageSentTimestamp", "lmst"),
            PLUGIN_METADATA,
            boolean("starred", "starred"),
            string("subject", "subject"),
            boolean("unread", "unread"),
            VERSION,
        ],
        natural_order: Some(("lastMessageReceivedTimestamp", "desc")),
    },
    QueryClass {
        class: "Message",
        attributes: &[
            ACCOUNT_ID,
            attr(
                "body",
                "body",
                AttrKind::JoinedData {
                    table: "MessageBody",
                },
            ),
            datetime("date", "date"),
            boolean("draft", "draft"),
            string("headerMessageId", "hMsgId"),
            ID,
            PLUGIN_METADATA,
            boolean("starred", "starred"),
            string("threadId", "threadId"),
            boolean("unread", "unread"),
            VERSION,
        ],
        natural_order: Some(("date", "asc")),
    },
    QueryClass {
        class: "Folder",
        attributes: CATEGORY,
        natural_order: None,
    },
    QueryClass {
        class: "Label",
        attributes: CATEGORY,
        natural_order: None,
    },
    QueryClass {
        class: "Contact",
        attributes: &[
            ACCOUNT_ID,
            attr(
                "contactGroups",
                "gis",
                AttrKind::Collection {
                    join_table: "ContactContactGroup",
                    join_queryable_by: &[],
                },
            ),
            string("email", "email"),
            boolean("hidden", "h"),
            ID,
            number("refs", "refs"),
            string("source", "s"),
        ],
        natural_order: Some(("id", "desc")),
    },
    QueryClass {
        class: "ContactGroup",
        attributes: &[ACCOUNT_ID, ID],
        natural_order: Some(("name", "asc")),
    },
    QueryClass {
        class: "ContactBook",
        attributes: &[ACCOUNT_ID, ID],
        natural_order: Some(("id", "asc")),
    },
    QueryClass {
        class: "Event",
        attributes: &[
            ACCOUNT_ID,
            string("calendarId", "cid"),
            string("icsuid", "icsuid"),
            ID,
            number("recurrenceEnd", "re"),
            string("recurrenceId", "rid"),
            number("recurrenceStart", "rs"),
            // The desktop declares it; the engine's Event table has no column for it.
            string("status", "status"),
        ],
        natural_order: Some(("id", "desc")),
    },
    QueryClass {
        class: "Calendar",
        attributes: &[ACCOUNT_ID, ID],
        natural_order: None,
    },
    QueryClass {
        class: "Task",
        attributes: &[ACCOUNT_ID, ID, string("status", "status")],
        natural_order: None,
    },
    QueryClass {
        class: "File",
        attributes: &[ACCOUNT_ID, string("filename", "filename"), ID],
        natural_order: None,
    },
];

/// The table as `model_query_allowlist.json` holds it.
pub fn allowlist() -> Value {
    let mut classes = Map::new();
    for c in QUERY_CLASSES {
        let mut attributes = Map::new();
        let mut sorted: Vec<&QueryAttr> = c.attributes.iter().collect();
        sorted.sort_by_key(|a| a.name);
        for a in sorted {
            let entry = match a.kind {
                AttrKind::String { column } => json!({"kind": "string", "column": column}),
                AttrKind::Number { column } => json!({"kind": "number", "column": column}),
                AttrKind::Boolean { column } => json!({"kind": "boolean", "column": column}),
                AttrKind::Datetime { column } => json!({"kind": "datetime", "column": column}),
                AttrKind::Collection {
                    join_table,
                    join_queryable_by,
                } => json!({
                    "kind": "collection",
                    "joinTable": join_table,
                    "joinQueryableBy": join_queryable_by,
                }),
                AttrKind::JoinedData { table } => json!({"kind": "joinedData", "table": table}),
            };
            attributes.insert(a.name.into(), entry);
        }
        let natural = c
            .natural_order
            .map(|(attr, dir)| json!({"attr": attr, "dir": dir}))
            .unwrap_or(Value::Null);
        classes.insert(
            c.class.into(),
            json!({"attributes": attributes, "naturalOrder": natural}),
        );
    }
    json!({
        "generatedBy": "crates/tern-ipc/src/query_attrs.rs (cargo run -p tern-ipc --bin tern-ipc-schema)",
        "classes": classes,
    })
}

/// [`allowlist`] as the checked-in file holds it.
pub fn allowlist_text() -> String {
    let mut text = serde_json::to_string_pretty(&allowlist()).expect("the allowlist serialises");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::protocol;

    /// The view struct whose JSON keys a class's attributes name.
    fn view_def(class: &str) -> &'static str {
        match class {
            "Thread" => "ThreadJson",
            "Message" => "MessageJson",
            "Folder" => "FolderJson",
            "Label" => "LabelJson",
            "Contact" => "ContactJson",
            "ContactGroup" => "ContactGroupJson",
            "ContactBook" => "ContactBookJson",
            "Event" => "EventJson",
            "Calendar" => "CalendarJson",
            // Every task has the base keys.
            "Task" => "ChangeUnreadTaskJson",
            "File" => "FileJson",
            other => panic!("no view struct for {other}"),
        }
    }

    #[test]
    fn json_keys_are_view_struct_fields() {
        let schema = protocol();
        for c in QUERY_CLASSES {
            let props = &schema["$defs"][view_def(c.class)]["properties"];
            assert!(props.is_object(), "{} has no properties", c.class);
            for a in c.attributes {
                if let Some(key) = a.json {
                    assert!(
                        props.get(key).is_some(),
                        "{}.{} names {key}, which {} doesn't have",
                        c.class,
                        a.name,
                        view_def(c.class)
                    );
                }
            }
            if let Some((attr, dir)) = c.natural_order {
                assert!(dir == "asc" || dir == "desc");
                assert!(
                    c.attributes.iter().any(|a| a.name == attr) || attr == "name",
                    "{}'s natural order {attr} isn't an attribute",
                    c.class
                );
            }
        }
    }

    #[test]
    fn query_class_names_are_the_classes() {
        let schema = protocol();
        let names: Vec<&str> = QUERY_CLASSES.iter().map(|c| c.class).collect();
        assert_eq!(
            schema["$defs"]["QueryClassName"]["enum"],
            serde_json::json!(names)
        );
    }

    #[test]
    fn the_checked_in_allowlist_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tern-store/src/model_query_allowlist.json"
        );
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            on_disk == allowlist_text(),
            "crates/tern-store/src/model_query_allowlist.json is out of date: run \
             `cargo run -p tern-ipc --bin tern-ipc-schema`"
        );
    }
}
