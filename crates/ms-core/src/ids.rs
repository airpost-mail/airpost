//! Model ids. Derived from `MailUtils.cpp:524-611` and `Thread.cpp:23`; see
//! docs/engine-port-plan.md section 5 for the message id.
//!
//! Every hashed id is `base58(sha256(src)[0..30])` in the Bitcoin alphabet. The client
//! never parses ids, but they must be deterministic: a message keeps its id across moves,
//! restarts and rebuilds because the hash input comes from the message alone.
//!
//! The message id is frozen by `tests/message_ids.json`. Its inputs are the raw header
//! values as they arrive (the ENVELOPE strings, or the header lines of a full fetch), and
//! the decoding that turns them into the hash input ([`crate::rfc2047`], [`crate::date`],
//! NFC normalisation) lives here rather than in a MIME library, so a change to how bodies
//! or display subjects are decoded can never change an id.

use rand::RngExt;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// The Bitcoin base58 encoding of `bytes`. Leading zero bytes become `'1'`.
pub fn base58(bytes: &[u8]) -> String {
    bs58::encode(bytes)
        .with_alphabet(bs58::Alphabet::BITCOIN)
        .into_string()
}

/// `toBase58(sha256(src), 30)`: the hashed-id format shared by every deterministic id.
pub fn hash_id(src: &str) -> String {
    let digest = Sha256::digest(src.as_bytes());
    base58(&digest[..30])
}

/// The raw inputs of a message id. Strings are passed as they appear on the wire; this
/// module decodes them.
#[derive(Clone, Debug, Default)]
pub struct MessageIdInput<'a> {
    pub account_id: &'a str,
    /// The Date header value, e.g. `"Mon, 7 Feb 1994 21:52:25 -0800"`.
    pub date: Option<&'a str>,
    /// INTERNALDATE as epoch seconds, used when there is no parseable Date header.
    pub internal_date: Option<i64>,
    /// The Subject header value, RFC 2047 encoded words and raw 8-bit bytes allowed. It is
    /// decoded, trimmed and NFC-normalised.
    pub subject: Option<&'a [u8]>,
    /// The addr-specs (`local@domain`) of every To, Cc and Bcc address, in any order.
    pub recipients: &'a [&'a str],
    /// The Message-ID header value, angle brackets optional. `None` when the message has
    /// none: the stored `hMsgId` is then a generated `uuid@hostname`, but the hash uses "".
    pub message_id: Option<&'a str>,
    /// Folder path and UID, which stand in for the date when there is no date at all.
    pub folder_path: &'a str,
    pub uid: u32,
}

/// The string hashed into a message id:
/// `accountId + "-" + date + subject + "-" + participants + "-" + messageId`.
pub fn message_id_source(input: &MessageIdInput<'_>) -> String {
    let date = input
        .date
        .and_then(crate::date::parse_rfc5322)
        .filter(|d| *d > 0)
        .or(input.internal_date.filter(|d| *d > 0));

    let mut src = String::with_capacity(128);
    src.push_str(input.account_id);
    src.push('-');
    match date {
        Some(d) => src.push_str(&d.to_string()),
        None => {
            // Subject and recipients alone collide too often to identify a message, so a
            // message with no date at all is keyed by its location. Its id changes when it
            // moves or UIDVALIDITY changes, which reads as a delete + create.
            tracing::warn!(
                "{}",
                crate::logmsg::message_without_date(input.folder_path, input.uid)
            );
            src.push_str(input.folder_path);
            src.push(':');
            src.push_str(&input.uid.to_string());
        }
    }
    if let Some(subject) = input.subject {
        src.extend(crate::rfc2047::decode(subject).trim().nfc());
    }
    src.push('-');
    let mut participants: Vec<String> = input
        .recipients
        .iter()
        .map(|r| r.trim().to_ascii_lowercase())
        .filter(|r| !r.is_empty())
        .collect();
    participants.sort();
    for p in &participants {
        src.push_str(p);
    }
    src.push('-');
    if let Some(mid) = input.message_id {
        src.push_str(strip_angle_brackets(mid));
    }
    src
}

pub fn message_id(input: &MessageIdInput<'_>) -> String {
    hash_id(&message_id_source(input))
}

/// `<abc@host>` → `abc@host`. Surrounding whitespace and one pair of brackets are removed.
pub fn strip_angle_brackets(value: &str) -> &str {
    let v = value.trim();
    let v = v.strip_prefix('<').unwrap_or(v);
    let v = v.strip_suffix('>').unwrap_or(v);
    v.trim()
}

/// Folder and label ids: `accountId:path`, with the path still in modified UTF-7 as the
/// server listed it (INBOX normalised by the IMAP layer).
pub fn folder_id(account_id: &str, folder_path: &str) -> String {
    hash_id(&format!("{account_id}:{folder_path}"))
}

/// `t:<first message id>`. Not hashed.
pub fn thread_id(first_message_id: &str) -> String {
    format!("t:{first_message_id}")
}

/// `msgId:accountId[:partID][:uniqueID]`, or `msgId:accountId:<description>` for a part
/// with neither (the C++ logged a warning and used mailcore's part description).
pub fn file_id(
    message_id: &str,
    account_id: &str,
    part_id: Option<&str>,
    unique_id: Option<&str>,
    description: &str,
) -> String {
    let mut src = format!("{message_id}:{account_id}");
    if let Some(p) = part_id {
        src.push(':');
        src.push_str(p);
    }
    if let Some(u) = unique_id {
        src.push(':');
        src.push_str(u);
    }
    if part_id.is_none() && unique_id.is_none() {
        src.push(':');
        src.push_str(description);
    }
    hash_id(&src)
}

/// A draft created by the client: `accountId:headerMessageId`.
pub fn draft_id(account_id: &str, header_message_id: &str) -> String {
    hash_id(&format!("{account_id}:{header_message_id}"))
}

/// `accountId-calendarId-icsUID[-recurrenceId]`. The ICS UID is stable across edits; the
/// recurrence id separates an exception from its master.
pub fn event_id(account_id: &str, calendar_id: &str, ics_uid: &str, recurrence_id: &str) -> String {
    let mut src = format!("{account_id}-{calendar_id}-{ics_uid}");
    if !recurrence_id.is_empty() {
        src.push('-');
        src.push_str(recurrence_id);
    }
    hash_id(&src)
}

/// `accountId-url`.
pub fn calendar_id(account_id: &str, url: &str) -> String {
    hash_id(&format!("{account_id}-{url}"))
}

const RANDOM_ID_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";

/// 40 alphanumeric characters from the OS-seeded thread RNG (`idRandomlyGenerated`).
/// Several processes share one database and Contact / ContactGroup use these as primary
/// keys, so a time-seeded generator is not acceptable.
pub fn random_id() -> String {
    let mut rng = rand::rng();
    (0..40)
        .map(|_| RANDOM_ID_CHARSET[rng.random_range(0..RANDOM_ID_CHARSET.len())] as char)
        .collect()
}

/// The id of the invisible message that holds a deleted draft's placement until the
/// deletion reaches the server (`Message::messageWithDeletionPlaceholderFor`).
pub fn deletion_placeholder_id() -> String {
    format!("deleted-{}", random_id())
}

pub fn is_deletion_placeholder_id(id: &str) -> bool {
    id.starts_with("deleted-")
}

/// Prefixes of addresses that never become mail-sourced contacts (`MailUtils.cpp:34-61`).
pub const UNWORTHY_CONTACT_PREFIXES: &[&str] = &[
    "noreply",
    "no-reply",
    "no_reply",
    "auto-confirm",
    "donotreply",
    "do-not-reply",
    "do_not_reply",
    "auto-reply",
    "inmail-hit-reply",
    "updates@",
    "unsubscribe@",
    "mailman-owner",
    "email_notifier",
    "announcement",
    "bounce",
    "notification",
    "notify@",
    "support",
    "alert",
    "news",
    "info",
    "automated",
    "list",
    "distribute",
    "catchall",
    "catch-all",
];

/// The id (and `email` column) of a mail-sourced contact, or `None` for addresses that are
/// likely automated: longer than 40 bytes, an [`UNWORTHY_CONTACT_PREFIXES`] prefix, or
/// containing `@noreply`, `@notifications` or `noreply@` (`MailUtils::contactKeyForEmail`).
pub fn contact_key_for_email(email: &str) -> Option<String> {
    let email = email.to_ascii_lowercase();
    if email.len() > 40 {
        return None;
    }
    if UNWORTHY_CONTACT_PREFIXES
        .iter()
        .any(|p| email.starts_with(p))
    {
        return None;
    }
    if email.contains("@noreply") || email.contains("@notifications") || email.contains("noreply@")
    {
        return None;
    }
    Some(email)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `MailUtils::toBase58`, ported literally, to check the bs58 crate against it.
    fn cpp_base58(input: &[u8]) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let zeroes = input.iter().take_while(|b| **b == 0).count();
        let rest = &input[zeroes..];
        let size = rest.len() * 138 / 100 + 1;
        let mut b58 = vec![0u8; size];
        let mut length = 0usize;
        for &byte in rest {
            let mut carry = byte as u32;
            let mut i = 0usize;
            for slot in b58.iter_mut().rev() {
                if carry == 0 && i >= length {
                    break;
                }
                carry += 256 * (*slot as u32);
                *slot = (carry % 58) as u8;
                carry /= 58;
                i += 1;
            }
            length = i;
        }
        let mut it = size - length;
        while it < size && b58[it] == 0 {
            it += 1;
        }
        let mut out = "1".repeat(zeroes);
        for &d in &b58[it..] {
            out.push(ALPHABET[d as usize] as char);
        }
        out
    }

    #[test]
    fn base58_matches_cpp() {
        let mut cases: Vec<Vec<u8>> = vec![vec![], vec![0], vec![0, 0, 1], vec![255; 30]];
        for i in 0..200u32 {
            cases.push(Sha256::digest(i.to_le_bytes()).to_vec()[..30].to_vec());
        }
        let mut z = vec![0u8, 0, 0];
        z.extend_from_slice(&Sha256::digest(b"zero")[..27]);
        cases.push(z);
        for c in cases {
            assert_eq!(base58(&c), cpp_base58(&c), "{c:?}");
        }
    }

    #[test]
    fn folder_and_simple_ids() {
        assert_eq!(folder_id("acc", "INBOX"), hash_id("acc:INBOX"));
        assert_eq!(thread_id("abc"), "t:abc");
        assert_eq!(event_id("a", "c", "u", ""), hash_id("a-c-u"));
        assert_eq!(event_id("a", "c", "u", "r"), hash_id("a-c-u-r"));
        assert_eq!(calendar_id("a", "/cal/"), hash_id("a-/cal/"));
        assert_eq!(draft_id("a", "x@y"), hash_id("a:x@y"));
        assert_eq!(
            file_id("m", "a", Some("1.2"), Some("3"), "d"),
            hash_id("m:a:1.2:3")
        );
        assert_eq!(file_id("m", "a", None, None, "desc"), hash_id("m:a:desc"));
    }

    #[test]
    fn random_ids() {
        let a = random_id();
        assert_eq!(a.len(), 40);
        assert!(a.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(a, random_id());
        let d = deletion_placeholder_id();
        assert!(is_deletion_placeholder_id(&d));
        assert_eq!(d.len(), 48);
    }

    #[test]
    fn contact_keys() {
        assert_eq!(
            contact_key_for_email("Bob@Example.com").as_deref(),
            Some("bob@example.com")
        );
        assert_eq!(contact_key_for_email("noreply@x.com"), None);
        assert_eq!(contact_key_for_email("x@noreply.github.com"), None);
        assert_eq!(contact_key_for_email("reservations-noreply@x.com"), None);
        assert_eq!(contact_key_for_email("info@x.com"), None);
        assert_eq!(
            contact_key_for_email(&format!("{}@x.com", "a".repeat(40))),
            None
        );
    }
}
