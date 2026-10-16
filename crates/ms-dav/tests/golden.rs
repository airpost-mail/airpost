// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Golden tests: every `tests/golden/ics/*.ics` and `tests/golden/vcf/*.vcf` is read and
//! the fields the engine stores are compared with the `.json` beside it. Run with
//! `UPDATE_GOLDEN=1` to rewrite the expected files, then review the diff by hand.
//!
//! ICS floating times are read as UTC so the results do not depend on the machine.

use ms_dav::ical::{parse_events_in, EventFields, FloatingZone};
use ms_dav::{ContactFields, VCard};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn event_json(e: &EventFields) -> Value {
    json!({
        "uid": e.uid,
        "recurrence_id": e.recurrence_id,
        "status": e.status,
        "status_or_default": e.status_or_default(),
        "start": e.start,
        "all_day": e.all_day,
        "rs": e.recurrence_start,
        "re": e.recurrence_end,
        "is_recurring": e.is_recurring,
        "has_dtstamp": e.has_dtstamp,
        "summary": e.summary,
        "description": e.description,
        "location": e.location,
        "organizer": e.organizer,
        "attendees": e.attendees,
        "search_participants": e.search_participants(),
    })
}

fn ics_json(input: &str) -> Value {
    let events = parse_events_in(input, FloatingZone::Zone(chrono_tz::UTC));
    Value::Array(events.iter().map(event_json).collect())
}

fn vcf_json(input: &str) -> Value {
    let f = ContactFields::from_vcard(input);
    let card = VCard::parse(input);
    let mut out = json!({
        "uid": f.uid,
        "name": f.name,
        "email": f.email,
        "emails": f.emails,
        "is_group": f.is_group,
        "member_ids": f.member_ids,
        "version": card.version(),
        "serialized": card.serialize(),
    });
    if card.is_group() {
        let mut edited = card.clone();
        edited.add_members(&["added-1", "m-1", "f-1"]);
        edited.remove_members(&["m-2"]);
        edited.rename("Renamed");
        out["after_edit"] = Value::String(edited.serialize());
    }
    out
}

fn run(dir: &str, ext: &str, f: fn(&str) -> Value) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(dir);
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    inputs.sort();
    assert!(!inputs.is_empty(), "no golden inputs in {}", root.display());
    let mut failures = Vec::new();
    for input in &inputs {
        let text = std::fs::read_to_string(input).unwrap();
        let got = serde_json::to_string_pretty(&f(&text)).unwrap() + "\n";
        let expected_path = input.with_extension("json");
        if update {
            std::fs::write(&expected_path, &got).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (run with UPDATE_GOLDEN=1)",
                expected_path.display()
            )
        });
        if expected.replace("\r\n", "\n") != got {
            failures.push(format!(
                "{}\n--- expected\n{expected}\n--- got\n{got}",
                name(input)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn ics_golden() {
    run("ics", "ics", ics_json);
}

#[test]
fn vcf_golden() {
    run("vcf", "vcf", vcf_json);
}
