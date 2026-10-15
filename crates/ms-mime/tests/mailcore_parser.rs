//! The part tree against mailcore2's own parser unit-test expectations
//! (`unittest/data/parser`, copied to `tests/corpus/mailcore/parser`): part types, unique
//! IDs, MIME types, charsets, filenames, inline flags and Content-IDs.

use std::fs;
use std::path::{Path, PathBuf};

use ms_mime::mime::{parse_tree, Part, PartKind};
use serde_json::{Map, Value};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/mailcore/parser")
}

/// Our tree in the shape of mailcore's `serializable()`, limited to the compared keys.
fn ours(part: &Part) -> Value {
    let mut m = Map::new();
    let kind = match part.kind {
        PartKind::Single => "single",
        PartKind::Message => "message",
        PartKind::Mixed => "multipart/mixed",
        PartKind::Related => "multipart/related",
        PartKind::Alternative => "multipart/alternative",
        PartKind::Signed => "multipart/signed",
    };
    m.insert("partType".into(), kind.into());
    if part.kind == PartKind::Single {
        if let Some(id) = &part.unique_id {
            m.insert("uniqueID".into(), id.clone().into());
        }
        if let Some(t) = &part.mime_type {
            m.insert("mimeType".into(), t.to_ascii_lowercase().into());
        }
        if let Some(c) = &part.charset {
            m.insert("charset".into(), c.to_ascii_lowercase().into());
        }
        if let Some(f) = &part.filename {
            m.insert("filename".into(), f.clone().into());
        }
        if part.inline {
            m.insert("inlineAttachment".into(), "1".into());
        }
        if let Some(c) = &part.content_id {
            m.insert("contentID".into(), c.clone().into());
        }
    }
    match part.kind {
        PartKind::Single => {}
        PartKind::Message => {
            if let Some(main) = part.parts.first() {
                m.insert("mainPart".into(), ours(main));
            }
        }
        _ => {
            m.insert(
                "parts".into(),
                Value::Array(part.parts.iter().map(ours).collect()),
            );
        }
    }
    Value::Object(m)
}

/// mailcore's JSON reduced to the same keys.
fn theirs(v: &Value) -> Value {
    let o = v.as_object().expect("part object");
    let mut m = Map::new();
    let kind = o["partType"].as_str().unwrap_or_default().to_owned();
    m.insert("partType".into(), kind.clone().into());
    if kind == "single" {
        for key in ["uniqueID", "filename", "contentID"] {
            if let Some(s) = o.get(key).and_then(Value::as_str) {
                m.insert(key.into(), s.into());
            }
        }
        for key in ["mimeType", "charset"] {
            if let Some(s) = o.get(key).and_then(Value::as_str) {
                m.insert(key.into(), s.to_ascii_lowercase().into());
            }
        }
        if o.get("inlineAttachment").and_then(Value::as_str) == Some("1") {
            m.insert("inlineAttachment".into(), "1".into());
        }
    }
    if let Some(main) = o.get("mainPart") {
        m.insert("mainPart".into(), theirs(main));
    }
    if kind.starts_with("multipart/") {
        let items = o
            .get("parts")
            .and_then(|p| p.get("items"))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(theirs).collect())
            .unwrap_or_default();
        m.insert("parts".into(), Value::Array(items));
    }
    Value::Object(m)
}

/// Inputs where the port deliberately differs from libetpan.
const DEVIATIONS: &[(&str, &str)] = &[(
    "jwz-22",
    "`multipart/mixed;; boundary=...`: libetpan stops at the empty parameter, loses the \
     boundary and shows the whole body as one opaque part; the port skips the empty \
     parameter and splits the multipart",
)];

#[test]
fn part_trees_match_mailcore() {
    let dir = corpus();
    let mut names: Vec<_> = fs::read_dir(dir.join("input"))
        .expect("parser corpus")
        .map(|e| e.expect("entry").file_name().into_string().expect("name"))
        .collect();
    names.sort();
    assert!(
        names.len() >= 90,
        "parser corpus has {} inputs",
        names.len()
    );

    let mut failures = Vec::new();
    for name in &names {
        let raw = fs::read(dir.join("input").join(name)).expect("input");
        let expected: Value =
            serde_json::from_slice(&fs::read(dir.join("output").join(name)).expect("output"))
                .expect("json");
        let want = theirs(&expected["mainPart"]);
        let got = ours(&parse_tree(&raw).root);
        if let Some((_, why)) = DEVIATIONS.iter().find(|(n, _)| n == name) {
            assert_ne!(
                want, got,
                "{name} now matches mailcore; drop the deviation ({why})"
            );
            continue;
        }
        if want != got {
            failures.push(format!("{name}\n  want: {want}\n  got:  {got}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} trees differ:\n{}",
        failures.len(),
        names.len(),
        failures.join("\n")
    );
}
