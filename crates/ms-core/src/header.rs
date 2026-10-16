//! Message headers as both the IMAP layer (ENVELOPE) and the MIME layer (full message)
//! produce them, so ids and models are built the same way from either source.

use serde_json::{json, Value};

/// One address. `email` is the addr-spec (`local@domain`); either part may be missing on
/// malformed mail, in which case it is whatever the server or the parser returned.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(name: Option<&str>, email: &str) -> Self {
        Address {
            name: name.map(str::to_owned),
            email: email.to_owned(),
        }
    }

    /// The contact JSON the client reads: `{"name","email"}`, `name` only when present
    /// (`MailUtils::contactJSONFromAddress`).
    pub fn to_contact_json(&self) -> Value {
        let mut v = json!({ "email": self.email });
        if let Some(n) = &self.name {
            v["name"] = json!(n);
        }
        v
    }
}

/// The headers the engine reads. `*_raw` fields keep the undecoded wire value for
/// [`crate::ids`]; the others are decoded for display and storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageHeader {
    pub date_raw: Option<String>,
    /// The Date header in epoch seconds; `None` when absent or unparseable (mailcore's -1).
    pub date: Option<i64>,
    pub subject_raw: Option<Vec<u8>>,
    pub subject: Option<String>,
    /// Message-ID without angle brackets. `None` when the message has none.
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub from: Vec<Address>,
    pub sender: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    /// Other header fields that were fetched or parsed, in message order, names as sent.
    pub extra: Vec<(String, String)>,
}

impl MessageHeader {
    /// The first value of a header outside the parsed set, by case-insensitive name.
    pub fn extra_value(&self, name: &str) -> Option<&str> {
        self.extra
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The addr-specs of To and Cc, the participants of the message id. Bcc is left out:
    /// the Sent copy we append keeps it and the copies recipients and providers file do
    /// not, and all of them must get the same id (docs/fix-backlog.md F1).
    pub fn recipient_emails(&self) -> Vec<&str> {
        self.to
            .iter()
            .chain(&self.cc)
            .map(|a| a.email.as_str())
            .collect()
    }
}
