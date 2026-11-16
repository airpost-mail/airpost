// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Charset decoding and rendering against mailcore2's summary unit-test expectations
//! (`unittest/data/summary`, copied to `tests/corpus/mailcore/summary`). mailcore's
//! expected output is `plainTextRendering()`: a header block, then the flattened HTML
//! rendering with its attachment templates. The port renders attachments differently, so
//! the comparison is the decoded subject and the body's words.

use std::fs;
use std::path::{Path, PathBuf};

use airpost_mime::{flatten_html_with, parse_message, strip_whitespace};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/mailcore/summary")
}

struct Expected {
    subject: String,
    body: String,
    /// `- name, size` lines of mailcore's attachment template.
    attachments: Vec<String>,
}

fn expected(text: &str) -> Expected {
    let (head, body) = text.split_once("\n\n").unwrap_or((text, ""));
    let subject = head
        .lines()
        .find_map(|l| l.strip_prefix("Subject: "))
        .unwrap_or_default()
        .to_owned();
    let mut attachments = Vec::new();
    let mut kept = Vec::new();
    for line in body.lines() {
        let listed = line
            .strip_prefix("- ")
            .and_then(|l| l.rsplit_once(", "))
            .filter(|(_, size)| {
                ["bytes", "KB", "MB", "GB"]
                    .iter()
                    .any(|u| size.ends_with(u))
            });
        match listed {
            Some((name, _)) => attachments.push(name.to_owned()),
            None => kept.push(line),
        }
    }
    Expected {
        subject,
        body: kept.join("\n"),
        attachments,
    }
}

/// mailcore's template shows `extractedSubject`: whitespace collapsed, reply and forward
/// prefixes removed, `No Subject` when empty.
fn summary_subject(subject: &str) -> String {
    let mut s = strip_whitespace(subject);
    loop {
        let lower = s.to_ascii_lowercase();
        let Some(n) = ["re:", "fwd:", "fw:"]
            .iter()
            .find(|p| lower.starts_with(*p))
        else {
            break;
        };
        s = s[n.len()..].trim_start().to_owned();
    }
    if s.is_empty() {
        "No Subject".to_owned()
    } else {
        s
    }
}

/// The body's non-ASCII words: these carry the charset decoding.
fn words(s: &str) -> Vec<String> {
    strip_whitespace(s)
        .split(' ')
        .filter(|w| !w.is_ascii() && !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Inputs where the port's output deliberately differs from mailcore's.
const SUBJECT_DEVIATIONS: &[(&str, &str)] = &[(
    "6144-russian.eml",
    "`=?koi8_r?b?...?=`: mailcore did not know the `koi8_r` spelling and showed Latin-1 \
     mojibake; the port maps it to KOI8-R",
)];

#[test]
fn summaries_match_mailcore() {
    let dir = corpus();
    let mut names: Vec<_> = fs::read_dir(dir.join("input"))
        .expect("summary corpus")
        .map(|e| e.expect("entry").file_name().into_string().expect("name"))
        .collect();
    names.sort();
    assert!(
        names.len() >= 35,
        "summary corpus has {} inputs",
        names.len()
    );

    let mut failures = Vec::new();
    for name in &names {
        let raw = fs::read(dir.join("input").join(name)).expect("input");
        let out = name.trim_end_matches(".eml").to_owned() + ".txt";
        let want = expected(&fs::read_to_string(dir.join("output").join(out)).expect("output"));
        let parsed = parse_message(&raw).expect("parse");

        let subject = summary_subject(parsed.header.subject.as_deref().unwrap_or_default());
        let deviates = SUBJECT_DEVIATIONS.iter().any(|(n, _)| n == name);
        if (subject == want.subject) == deviates {
            failures.push(format!(
                "{name}: subject {subject:?}, mailcore {:?}{}",
                want.subject,
                if deviates {
                    " (listed as a deviation)"
                } else {
                    ""
                }
            ));
        }

        let text = if parsed.plaintext {
            parsed.body.clone()
        } else {
            flatten_html_with(&parsed.body, true, true)
        };
        let ours = strip_whitespace(&text);
        let missing: Vec<_> = words(&want.body)
            .into_iter()
            .filter(|w| !ours.contains(w.as_str()))
            .collect();
        if !missing.is_empty() {
            failures.push(format!("{name}: body lacks {missing:?}"));
        }

        for listed in want.attachments.iter().filter(|n| *n != "Untitled") {
            if !parsed.attachments.iter().any(|f| &f.filename == listed) {
                failures.push(format!("{name}: no file named {listed:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
