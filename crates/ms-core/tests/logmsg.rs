// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Every `logmsg` function against the regexes the harness and scenarios use
//! (apps/engine/test/harness/mailsync.py, apps/engine/test/scenarios/*.yaml). The patterns
//! are copied verbatim; `re.search` semantics, so they match anywhere in the message.

use ms_core::logmsg::*;
use regex::Regex;

fn assert_matches(pattern: &str, message: &str) {
    let re = Regex::new(pattern).unwrap();
    assert!(
        re.is_match(message),
        "{pattern:?} does not match {message:?}"
    );
}

#[test]
fn harness_core() {
    assert_eq!(sync_loop_complete(), "Sync loop complete.");
    assert_matches(
        r"^Marking all folders as `busy`",
        marking_all_folders_busy(),
    );
    assert!(idling_on_folder("INBOX").starts_with("Idling on folder"));
    assert!(idle_not_supported().starts_with("Connection does not support idling"));
    assert_matches(r"^--sleeping", sleeping());
    assert_matches(r"--sleeping", sleeping());

    // IMAP_LINE in mailsync.py splits the direction off the protocol line.
    let imap_line = Regex::new(r"^(sent|recv) (.*)$").unwrap();
    let line = protocol_line("sent", "12 IDLE");
    let caps = imap_line.captures(&line).unwrap();
    assert_eq!(&caps[1], "sent");
    assert_matches(r"^\d+ IDLE$", &caps[2]);
    assert_matches(
        r"recv \* VANISHED 3",
        &protocol_line("recv", "* VANISHED 3"),
    );
    assert_matches(
        r"recv \* VANISHED",
        &protocol_line("recv", "* VANISHED (EARLIER) 1:5"),
    );
    assert_matches(
        r"sent \d+ APPEND",
        &protocol_line("sent", "7 APPEND \"Sent\" {120}"),
    );
}

#[test]
fn background_sync() {
    assert_matches(
        r"syncFolderUIDRange for INBOX, UIDs: 1 - \*, Heavy: false",
        &sync_folder_uid_range("INBOX", "1 - *", false),
    );
    assert_matches(
        r"Fetching full headers for 1024 \(of \d+ needed\)",
        &fetching_full_headers(1024, 2500),
    );
    assert_matches(
        r"Archive: server applied MESSAGELIMIT, only UIDs \d+ and up were scanned",
        &message_limit_applied("Archive", 451),
    );
    assert_matches(
        r"UIDInvalidity! Resetting placement UIDs in INBOX",
        &uidvalidity_reset("INBOX"),
    );
    assert_matches(
        r"unable to get folder status for Shared",
        &folder_status_failed("Shared", "permission denied"),
    );
    assert_matches(
        r"Orphans older than 8s are swept without waiting for Shared",
        &orphans_swept_without_waiting(8, "Shared", 0.5),
    );
    assert_matches(
        r"swept without waiting for Big",
        &orphans_swept_without_waiting_since_launch(8, "Big"),
    );
    assert_matches(
        r"skipped duplicate",
        &skipped_duplicate_all_folder("All Mail"),
    );
    let limited = condstore_request_limited(1, 4001);
    assert_matches(
        r"request limited to \d+:\d+, remaining changes will be detected via deep scan",
        &limited,
    );
    assert_matches(r"request limited to", &limited);
}

#[test]
fn mail_processing() {
    assert_matches(
        r"Orphan sweep limited to messages orphaned",
        &orphan_sweep_limited(30),
    );
    assert_matches(r"Orphan sweep skipped", orphan_sweep_skipped());
    assert_matches(
        r"has a copy in INBOX \(UID 20\)",
        &message_has_copy_in("abc", "INBOX", 20),
    );
    assert_matches(r"XGMLabels", &xgm_labels_changed(r#"["\\Inbox"]"#, "[]"));
    assert_matches(
        r"lost a placement to another message",
        &lost_placement("abc"),
    );
}

#[test]
fn foreground_and_tasks() {
    assert_matches(r"Fetching body for message ID", &fetching_body("abc"));
    assert_matches(r"Fetching body for message", &fetching_body("abc"));
    assert_matches(
        r"-X Changing copies in INBOX failed",
        &changing_copies_failed("INBOX"),
    );
    assert_matches(r"Expunging all", &expunging_all_deleted("INBOX"));
    assert_matches(
        r"Could not find new UID",
        &could_not_find_new_uid("abc", "Trash"),
    );
    assert_matches(r"COPYUID", &copyuid_mispaired("Trash", 1, 2));
    assert_matches(r"Sleeping 3", &sent_poll_sleeping(0, 1, 3));
    assert!(!Regex::new(r"Sleeping 3")
        .unwrap()
        .is_match(&sent_poll_sleeping(0, 1, 1)));
    assert_matches(
        r"Found a message added to the sent folder by the SMTP gateway",
        &found_sent_copy_from_gateway(5, 1),
    );
    assert_matches(r"Placing a new message", placing_new_sent_message());
    assert_matches(
        r"Found the sent message in \[Gmail\]/All Mail",
        &found_sent_message_in("[Gmail]/All Mail", 9),
    );
    assert_matches(r"Syncing sent message", &syncing_sent_message("Sent", 3));
    assert_matches(
        r"Removing 1 late server copies",
        &removing_late_sent_copies(1, "abc", "Sent", 3),
    );
    assert_matches(
        r"Removing the draft of a send interrupted after SMTP",
        &removing_draft_of_interrupted_send("sent-1@example.test"),
    );
    assert_matches(
        r"Send deferred until the account is online",
        &send_deferred_until_online("sent-1@example.test", "ErrorConnection"),
    );
}

#[test]
fn calendar_contacts_pass() {
    // dav-initial-sync, google-people-initial-sync; anchored as a scenario would anchor it.
    assert_matches(
        r"Calendar sync pass 1, with contacts",
        &calendar_sync_pass(1, true),
    );
    assert_matches(
        r"^Calendar sync pass 1, with contacts$",
        &calendar_sync_pass(1, true),
    );
    assert!(!Regex::new(r"^Calendar sync pass 1, with contacts$")
        .unwrap()
        .is_match(&calendar_sync_pass(1, false)));
    // dav-create-update-delete, google-people-sync-token-expired (anchored).
    assert_matches(
        r"^Calendar sync pass 1 complete\.$",
        &calendar_sync_pass_complete(1),
    );
    assert_matches(
        r"^-X Calendar sync pass 1 failed\.$",
        &calendar_sync_pass_failed(1),
    );
    assert!(!Regex::new(r"^Calendar sync pass 1, with contacts$")
        .unwrap()
        .is_match(&calendar_sync_pass_complete(1)));
}

#[test]
fn nothing_matches_the_absent_only_patterns() {
    let absent = [
        r"UNIQUE constraint",
        r"no such (table|column)",
        r"Unsafe Login",
        r"count is not falling",
        r"\[critical\]",
    ];
    let lines = [
        sync_loop_complete().to_owned(),
        starting_sync("me@example.com"),
        metadata_backend_tern("tern.example.com"),
        metadata_backend_local("no server"),
        waking_all_workers().to_owned(),
        calendar_sync_pass(1, true),
        calendar_sync_pass_complete(1),
        calendar_sync_pass_failed(1),
        message_without_date("INBOX", 3),
        orphan_sweep_skipped().to_owned(),
    ];
    for p in absent {
        let re = Regex::new(p).unwrap();
        for l in &lines {
            assert!(!re.is_match(l), "{p:?} matches {l:?}");
        }
    }
}

#[test]
fn metadata_backend_lines() {
    // backend-metadata-push and backend-identity-invalid; the reason is `invalid <field>` in
    // apps/engine/src/modes.rs.
    assert_matches(
        r"^Metadata backend: tern-server 127\.0\.0\.1",
        &metadata_backend_tern("127.0.0.1"),
    );
    let local = metadata_backend_local("invalid url");
    assert_matches(r"^Metadata backend: local only \(invalid url\)", &local);
    assert!(!Regex::new(r"^Metadata backend: tern-server")
        .unwrap()
        .is_match(&local));
}

#[test]
fn socket_lines() {
    // apps/engine/tests/socket.rs.
    assert_matches(
        r"^Listening on socket /tmp/e\.sock$",
        &socket_listening("/tmp/e.sock"),
    );
    assert_matches(
        r"^-X Dropping socket connection \d+: ",
        &socket_connection_dropped(3, "its queue passed 10 bytes"),
    );
}

#[test]
fn ai_triage_lines() {
    // apps/engine/test/test_ai.py.
    assert_matches(
        r"^- AI triage stored for thread \S+$",
        &ai_triage_stored("t:abc"),
    );
    assert_matches(
        r"^-X AI triage paused for \d+ s: rate-limited$",
        &ai_triage_paused(2, "rate-limited"),
    );
    assert_matches(r"^AI triage resumed$", ai_triage_resumed());
}
