// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! IMAP keywords as labels.

/// Flags other clients set that are not `$`-prefixed and are not labels.
const NON_LABEL_KEYWORDS: [&str; 7] = [
    "junk",
    "nonjunk",
    "notjunk",
    "junkrecorded",
    "redirected",
    "forwarded",
    "old",
];

/// `flag-keyword` (RFC 3501 9, RFC 9051): a non-empty atom of 7-bit ASCII without
/// `( ) { ] % * " \`, space or controls.
pub fn is_keyword_atom(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            (0x21..0x7f).contains(&b)
                && !matches!(b, b'(' | b')' | b'{' | b']' | b'%' | b'*' | b'"' | b'\\')
        })
}

/// A keyword that is a label: `$label1`..`$label5`, `$Important`, or one not starting with
/// `$` and not on the short list of other clients' junk and status flags. Every other
/// `$` keyword is a registered or client flag (`$MDNSent`, `$Junk`, `$hasattachment`, …).
/// Case-insensitive.
pub fn is_label_keyword(keyword: &str) -> bool {
    if !is_keyword_atom(keyword) {
        return false;
    }
    let k = keyword.to_ascii_lowercase();
    match k.strip_prefix('$') {
        Some(rest) => {
            rest == "important"
                || rest
                    .strip_prefix("label")
                    .is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5"))
        }
        None => !NON_LABEL_KEYWORDS.contains(&k.as_str()),
    }
}

/// The label keywords among a copy's keywords: lowercased, sorted, without duplicates, as
/// stored per copy. Servers may return a keyword in any case (Cyrus compares them
/// case-insensitively), and Airpost creates them lowercase.
pub fn label_keywords<'a>(keywords: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    let mut out: Vec<String> = keywords
        .into_iter()
        .filter(|k| is_label_keyword(k))
        .map(|k| k.to_ascii_lowercase())
        .collect();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atoms() {
        for ok in [
            "work",
            "$label1",
            "clients/acme",
            "a=20b",
            "&x-moz-0123",
            "~!#",
        ] {
            assert!(is_keyword_atom(ok), "{ok}");
        }
        for bad in [
            "",
            "two words",
            "a(b",
            "a)b",
            "a{b",
            "a]b",
            "a%",
            "a*",
            "a\"b",
            "a\\b",
            "é",
            "a\tb",
        ] {
            assert!(!is_keyword_atom(bad), "{bad:?}");
        }
    }

    #[test]
    fn which_keywords_are_labels() {
        for label in [
            "work",
            "Clients/Acme",
            "$label1",
            "$Label5",
            "$Important",
            "todo=20list",
        ] {
            assert!(is_label_keyword(label), "{label}");
        }
        for flag in [
            "$label0",
            "$label6",
            "$label10",
            "$Junk",
            "$NotJunk",
            "$MDNSent",
            "$Phishing",
            "$hasattachment",
            "$MailFlagBit0",
            "$muted",
            "Junk",
            "NonJunk",
            "NOTJUNK",
            "JunkRecorded",
            "Redirected",
            "Forwarded",
            "Old",
            "a b",
        ] {
            assert!(!is_label_keyword(flag), "{flag}");
        }
    }

    #[test]
    fn label_keywords_are_lowercased_sorted_and_unique() {
        let kws: Vec<String> = ["Work", "$Junk", "work", "$Label2", "acme", "NonJunk"]
            .map(String::from)
            .into();
        assert_eq!(label_keywords(&kws), vec!["$label2", "acme", "work"]);
    }
}
