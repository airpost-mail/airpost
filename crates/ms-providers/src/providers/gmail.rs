// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Gmail (`SyncWorker.cpp:961-964`, `MailProcessor.cpp:299-304`, `MCIMAPSession.cpp:1074-1103,
//! 4500-4515`, `DAVWorker.cpp:445-460`), Copyright © 2017 Foundry 376, GPL-3.0.
//! mailcore2 (`MC*.cpp`) is Copyright © 2001-2013 MailCore team, BSD-3-Clause. See NOTICE.md
//! ("mailcore2").

use crate::{
    ConnectionInfo, KeywordLabels, MailboxModelKind, OAuthProvider, Provider, ServerProfile,
};

/// mailcore's `IMAPCapabilityGmail`.
pub fn detect(info: &ConnectionInfo) -> bool {
    info.has_capability("X-GM-EXT-1")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Gmail,
        mailbox_model: MailboxModelKind::GmailLabels,
        // Gmail stores keywords, but its labels are X-GM-LABELS.
        keyword_labels: KeywordLabels::Off,
        oauth: Some(OAuthProvider::Google),
        // Gmail advertises XLIST but its LIST carries the same special-use flags.
        xlist_disabled: true,
        literal_plus_search: true,
        // All Mail is the folder every message lives in.
        skip_all_mail_folder: false,
        people_api_contacts: true,
        caldav_url_template: Some("https://apidata.googleusercontent.com/caldav/v2/{email}"),
        ..ServerProfile::default()
    }
}
