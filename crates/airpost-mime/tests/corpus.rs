// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The golden corpus: every message under `tests/corpus/golden` and the copied mailcore2
//! unit-test messages, parsed with [`airpost_mime::parse_message`] and compared with the
//! reviewed expectations in `tests/corpus/expected/<group>/`:
//! - `<name>.json`: plaintext flag, subject, importance, List-Unsubscribe, snippet, the
//!   attachment count and every file (partId, uniqueId, names, type, contentId, inline,
//!   size, SHA-256);
//! - `<name>.body`: the rendered body;
//! - `<name>.text`: the text the snippet and search text are cut from.
//!
//! `AIRPOST_MIME_BLESS=1 cargo test -p airpost-mime --test corpus` rewrites the expectations; review
//! the diff before keeping it.

use std::fs;
use std::path::{Path, PathBuf};

use airpost_mime::{parse_message, safe_filename, ParsedMessage};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const GROUPS: &[(&str, &str)] = &[
    ("golden", "tests/corpus/golden"),
    ("mailcore-summary", "tests/corpus/mailcore/summary/input"),
    ("mailcore-parser", "tests/corpus/mailcore/parser/input"),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn summary(m: &ParsedMessage) -> Value {
    let files: Vec<Value> = m
        .attachments
        .iter()
        .map(|f| {
            json!({
                "partId": f.part_id,
                "uniqueId": f.unique_id,
                "filename": f.filename,
                "safeFilename": safe_filename(&f.filename),
                "contentType": f.mime_type,
                "contentId": f.content_id,
                "inline": f.inline,
                "size": f.size(),
                "sha256": hex(&Sha256::digest(&f.data)),
                "countsAsAttachment": f.counts_as_attachment(),
            })
        })
        .collect();
    json!({
        "plaintext": m.plaintext,
        "subject": m.header.subject,
        "from": m
            .header
            .from
            .iter()
            .map(|a| json!([a.name, a.email]))
            .collect::<Vec<_>>(),
        "messageId": m.header.message_id,
        "importance": m.importance,
        "listUnsubscribe": m.list_unsubscribe,
        "listUnsubscribePost": m.list_unsubscribe_post,
        "snippet": m.snippet(),
        "searchTextLength": m.search_text().encode_utf16().count(),
        "attachmentCount": m.attachments.iter().filter(|f| f.counts_as_attachment()).count(),
        "files": files,
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn inputs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}

fn stem(path: &Path) -> String {
    let name = path.file_name().and_then(|n| n.to_str()).expect("name");
    name.strip_suffix(".eml").unwrap_or(name).to_owned()
}

#[test]
fn golden_corpus() {
    let bless = std::env::var_os("AIRPOST_MIME_BLESS").is_some();
    let mut failures = Vec::new();
    let mut handcrafted = 0;
    let mut total = 0;
    for (group, dir) in GROUPS {
        let expected_dir = root().join("tests/corpus/expected").join(group);
        if bless {
            fs::create_dir_all(&expected_dir).expect("expected dir");
        }
        for input in inputs(&root().join(dir)) {
            total += 1;
            if *group == "golden" {
                handcrafted += 1;
            }
            let name = stem(&input);
            let raw = fs::read(&input).expect("input");
            let parsed = parse_message(&raw).expect("parse_message");
            let got = summary(&parsed);
            let json_path = expected_dir.join(format!("{name}.json"));
            let body_path = expected_dir.join(format!("{name}.body"));
            let text_path = expected_dir.join(format!("{name}.text"));
            if bless {
                let mut pretty = serde_json::to_string_pretty(&got).expect("json");
                pretty.push('\n');
                fs::write(&json_path, pretty).expect("write json");
                fs::write(&body_path, &parsed.body).expect("write body");
                fs::write(&text_path, &parsed.text).expect("write text");
                continue;
            }
            let Ok(want) = fs::read(&json_path) else {
                failures.push(format!("{group}/{name}: no expectation (bless first)"));
                continue;
            };
            let want: Value = serde_json::from_slice(&want).expect("expected json");
            if want != got {
                failures.push(format!(
                    "{group}/{name}: summary differs\n  want: {want}\n  got:  {got}"
                ));
            }
            let want_body = fs::read_to_string(&body_path).unwrap_or_default();
            if want_body != parsed.body {
                failures.push(format!(
                    "{group}/{name}: body differs\n  want: {want_body:?}\n  got:  {:?}",
                    parsed.body
                ));
            }
            let want_text = fs::read_to_string(&text_path).unwrap_or_default();
            if want_text != parsed.text {
                failures.push(format!(
                    "{group}/{name}: text differs\n  want: {want_text:?}\n  got:  {:?}",
                    parsed.text
                ));
            }
        }
    }
    assert!(handcrafted >= 40, "only {handcrafted} handcrafted messages");
    assert!(
        failures.is_empty(),
        "{} of {total} messages differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
