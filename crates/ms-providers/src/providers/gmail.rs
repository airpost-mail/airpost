//! Gmail (`SyncWorker.cpp:961-964`, `MailProcessor.cpp:299-304`, `MCIMAPSession.cpp:1074-1103,
//! 4500-4515`, `DAVWorker.cpp:445-460`), Copyright © 2017 Foundry 376, GPL-3.0.

use crate::{ConnectionInfo, MailboxModelKind, OAuthProvider, Provider, ServerProfile};

/// mailcore's `IMAPCapabilityGmail`.
pub fn detect(info: &ConnectionInfo) -> bool {
    info.has_capability("X-GM-EXT-1")
}

pub fn profile() -> ServerProfile {
    ServerProfile {
        provider: Provider::Gmail,
        mailbox_model: MailboxModelKind::GmailLabels,
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
