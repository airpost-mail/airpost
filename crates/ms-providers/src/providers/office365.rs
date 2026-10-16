// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Office 365 / Outlook.com / Exchange (`SyncWorker.cpp:765-769, 860-864`, `MailUtils.cpp:894-897`,
//! `MCSMTPSession.cpp:91, 638-642`, `NetworkRequestUtils.cpp:58-100`), Copyright © 2017
//! Foundry 376, GPL-3.0.
//! mailcore2 (`MC*.cpp`) is Copyright © 2001-2013 MailCore team, BSD-3-Clause. See NOTICE.md
//! ("mailcore2").

use super::{greeting_has, host_is};
use crate::{ConnectionInfo, OAuthProvider, Provider, ServerProfile};

pub fn detect(info: &ConnectionInfo) -> bool {
    host_is(info, "office365.com")
        || host_is(info, "outlook.com")
        || greeting_has(info, "Microsoft Exchange")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Office365,
        oauth: Some(OAuthProvider::Microsoft),
        list_no_is_connection_error: true,
        late_sent_copies: true,
        ..ServerProfile::default()
    }
}

/// mailcore's Outlook XOAUTH2 variant applies to this SMTP host (`MCSMTPSession.cpp:91`),
/// whatever the IMAP server is.
pub fn smtp_uses_outlook_xoauth2(smtp_host: &str) -> bool {
    smtp_host.eq_ignore_ascii_case("smtp-mail.outlook.com")
}
