// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Provider detection, one file per provider. [`resolve`] tries them in [`PROVIDERS`]
//! order and falls back to the generic profile.
//!
//! Detection keys on what the C++ keyed on (host, greeting, capability), plus the greeting
//! where the C++ used a host but the harness's fake personalities run on localhost. Derived
//! from Mailspring-Sync, Copyright © 2017 Foundry 376, GPL-3.0; each provider file names its
//! sources. See NOTICE.md.

use crate::{ConnectionInfo, ServerProfile};

pub mod coremail;
pub mod courier;
pub mod cyrus;
pub mod davmail;
pub mod dovecot;
pub mod generic;
pub mod gmail;
pub mod hermes;
pub mod hmailserver;
pub mod home_pl;
pub mod icloud;
pub mod office365;
pub mod proton_bridge;
pub mod yahoo;
pub mod yandex;
pub mod zoho;

/// A provider's detect function and profile.
pub struct ProviderEntry {
    pub detect: fn(&ConnectionInfo) -> bool,
    pub profile: fn() -> ServerProfile,
}

/// In match order. Hosted services come before server software, because a hosted service
/// may run Dovecot or Cyrus underneath and its own quirks matter more.
pub const PROVIDERS: &[ProviderEntry] = &[
    ProviderEntry {
        detect: gmail::detect,
        profile: gmail::profile,
    },
    ProviderEntry {
        detect: office365::detect,
        profile: office365::profile,
    },
    ProviderEntry {
        detect: icloud::detect,
        profile: icloud::profile,
    },
    ProviderEntry {
        detect: yahoo::detect,
        profile: yahoo::profile,
    },
    ProviderEntry {
        detect: proton_bridge::detect,
        profile: proton_bridge::profile,
    },
    ProviderEntry {
        detect: coremail::detect,
        profile: coremail::profile,
    },
    ProviderEntry {
        detect: yandex::detect,
        profile: yandex::profile,
    },
    ProviderEntry {
        detect: home_pl::detect,
        profile: home_pl::profile,
    },
    ProviderEntry {
        detect: zoho::detect,
        profile: zoho::profile,
    },
    ProviderEntry {
        detect: davmail::detect,
        profile: davmail::profile,
    },
    ProviderEntry {
        detect: hermes::detect,
        profile: hermes::profile,
    },
    ProviderEntry {
        detect: hmailserver::detect,
        profile: hmailserver::profile,
    },
    ProviderEntry {
        detect: courier::detect,
        profile: courier::profile,
    },
    ProviderEntry {
        detect: cyrus::detect,
        profile: cyrus::profile,
    },
    ProviderEntry {
        detect: dovecot::detect,
        profile: dovecot::profile,
    },
];

/// The profile for a connection: the first provider whose `detect` matches `info`, else
/// the generic profile.
pub fn resolve(info: &ConnectionInfo) -> ServerProfile {
    PROVIDERS
        .iter()
        .find(|p| (p.detect)(info))
        .map(|p| (p.profile)())
        .unwrap_or_else(generic::profile)
}

pub(crate) fn greeting_has(info: &ConnectionInfo, needle: &str) -> bool {
    info.greeting.contains(needle)
}

/// `host` is `domain` or a subdomain of it.
pub(crate) fn host_is(info: &ConnectionInfo, domain: &str) -> bool {
    let host = info.host.trim_end_matches('.');
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|rest| rest.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeywordLabels, MailboxModelKind, OAuthProvider, Provider};

    fn info(host: &str, greeting: &str, caps: &str) -> ConnectionInfo {
        ConnectionInfo {
            host: host.into(),
            greeting: greeting.into(),
            id_response: vec![],
            capabilities: caps.split_whitespace().map(str::to_owned).collect(),
        }
    }

    /// Greetings and capabilities as the harness's fake personalities and server images
    /// send them, and as the real services do.
    #[test]
    fn resolution_table() {
        let cases: &[(&str, &str, &str, Provider)] = &[
            (
                "imap.gmail.com",
                "Gimap ready for requests from 127.0.0.1 h1mb2",
                "IMAP4rev1 X-GM-EXT-1 XLIST",
                Provider::Gmail,
            ),
            (
                "127.0.0.1",
                "Gimap ready",
                "IMAP4rev1 X-GM-EXT-1",
                Provider::Gmail,
            ),
            (
                "outlook.office365.com",
                "The Microsoft Exchange IMAP4 service is ready.",
                "IMAP4rev1",
                Provider::Office365,
            ),
            (
                "127.0.0.1",
                "The Microsoft Exchange IMAP4 service is ready.",
                "IMAP4rev1",
                Provider::Office365,
            ),
            (
                "imap-mail.outlook.com",
                "",
                "IMAP4rev1",
                Provider::Office365,
            ),
            (
                "imap.mail.me.com",
                "[ ... ] iCloud IMAP ready",
                "IMAP4rev1 QRESYNC",
                Provider::ICloud,
            ),
            (
                "p02-imap.mail.me.com",
                "iCloud",
                "IMAP4rev1",
                Provider::ICloud,
            ),
            (
                "imap.mail.yahoo.com",
                "IMAP4rev1 Hello",
                "IMAP4rev1",
                Provider::Yahoo,
            ),
            ("imap.aol.com", "", "IMAP4rev1", Provider::Yahoo),
            (
                "127.0.0.1",
                "Welcome! IMAP Server up and ready to accept your request",
                "IMAP4rev1 XYMHIGHESTMODSEQ",
                Provider::Yahoo,
            ),
            (
                "127.0.0.1",
                "Proton Mail Bridge ready.",
                "IMAP4rev1 IDLE",
                Provider::ProtonBridge,
            ),
            (
                "imap.163.com",
                "Coremail System IMap Server Ready(163com[...])",
                "IMAP4rev1 ID",
                Provider::Coremail,
            ),
            (
                "imap.yandex.ru",
                "Yandex IMAP4rev1 at mail.yandex.net",
                "IMAP4rev1 XLIST",
                Provider::Yandex,
            ),
            (
                "imap.home.pl",
                "IdeaImapServer v0.80 ready",
                "IMAP4rev1 XLIST",
                Provider::HomePl,
            ),
            (
                "imap.zoho.com",
                "Zoho Mail IMAP4rev1 Server",
                "IMAP4rev1",
                Provider::Zoho,
            ),
            (
                "localhost",
                "DavMail 6.2.2 IMAP ready",
                "IMAP4rev1",
                Provider::DavMail,
            ),
            (
                "mail.example.test",
                "Hermes IMAP server ready",
                "IMAP4rev1 NAMESPACE",
                Provider::Hermes,
            ),
            (
                "127.0.0.1",
                "hMailServer",
                "IMAP4rev1 IDLE",
                Provider::HMailServer,
            ),
            (
                "127.0.0.1",
                "Courier-IMAP ready. Copyright 1998-2018 Double Precision, Inc.",
                "IMAP4rev1",
                Provider::Courier,
            ),
            (
                "127.0.0.1",
                "localhost Cyrus IMAP 3.6.1 server ready",
                "IMAP4rev1 QRESYNC",
                Provider::Cyrus,
            ),
            (
                "imap.fastmail.com",
                "IMAP4rev1 Server ready",
                "IMAP4rev1",
                Provider::Cyrus,
            ),
            (
                "127.0.0.1",
                "Dovecot ready.",
                "IMAP4rev1 QRESYNC",
                Provider::Dovecot,
            ),
            (
                "127.0.0.1",
                "Dovecot (Ubuntu) ready.",
                "IMAP4rev1",
                Provider::Dovecot,
            ),
            (
                "mail.example.test",
                "IMAP4rev1 server ready",
                "IMAP4rev1",
                Provider::Generic,
            ),
        ];
        for (host, greeting, caps, want) in cases {
            let got = resolve(&info(host, greeting, caps)).provider;
            assert_eq!(got, *want, "{host} / {greeting:?} / {caps}");
        }
    }

    #[test]
    fn profiles() {
        let gmail = resolve(&info("imap.gmail.com", "Gimap ready", "X-GM-EXT-1"));
        assert_eq!(gmail.mailbox_model, MailboxModelKind::GmailLabels);
        assert!(gmail.xlist_disabled && gmail.literal_plus_search && gmail.people_api_contacts);
        assert!(!gmail.skip_all_mail_folder);
        assert_eq!(gmail.oauth, Some(OAuthProvider::Google));
        assert_eq!(gmail.keyword_labels, KeywordLabels::Off);
        assert_eq!(
            gmail.caldav_url("a@gmail.com").as_deref(),
            Some("https://apidata.googleusercontent.com/caldav/v2/a@gmail.com")
        );

        let o365 = resolve(&info("outlook.office365.com", "", ""));
        assert!(o365.list_no_is_connection_error && o365.late_sent_copies);
        assert_eq!(o365.oauth, Some(OAuthProvider::Microsoft));
        assert_eq!(o365.keyword_labels, KeywordLabels::Off);
        let exchange = resolve(&info(
            "mail.example.test",
            "The Microsoft Exchange IMAP4 service is ready.",
            "",
        ));
        assert_eq!(exchange.keyword_labels, KeywordLabels::Off);

        assert!(resolve(&info("imap.mail.me.com", "", "")).disable_qresync);

        let yahoo = resolve(&info("imap.mail.yahoo.com", "", ""));
        assert!(
            !yahoo.trust_copyuid_pairing && yahoo.search_charset_null && yahoo.xymhighestmodseq
        );

        let coremail = resolve(&info(
            "imap.163.com",
            "Coremail System IMap Server Ready",
            "",
        ));
        assert!(
            coremail.id_before_select
                && coremail.c_prefixed_tags
                && coremail.status_uidnext_unreliable
        );

        let courier = resolve(&info("x", "Courier-IMAP ready.", ""));
        assert!(courier.force_idle && courier.force_namespace && courier.reselect_after_append);

        assert!(resolve(&info("x", "Dovecot ready.", "")).bye_on_uidvalidity_change);
        assert!(resolve(&info("x", "Hermes", "")).ignore_namespace_capability);
        assert!(resolve(&info("x", "Yandex IMAP4rev1 ready", "")).tolerate_idle_disconnect);
        assert!(resolve(&info("x", "Yandex IMAP4rev1 ready", "")).xlist_disabled);
        assert!(resolve(&info("x", "IdeaImapServer", "")).xlist_disabled);
        assert!(resolve(&info("x", "hMailServer", "")).status_uidnext_unreliable);

        let generic = resolve(&info("mail.example.test", "ready", ""));
        assert_eq!(generic, ServerProfile::default());
        assert!(generic.skip_all_mail_folder && generic.merge_duplicate_list_entries);
        assert!(!generic.disable_qresync);
        assert_eq!(generic.keyword_labels, KeywordLabels::Auto);
        // Behaviours the C++ applies to every server, whichever provider showed them first.
        assert!(
            !generic.trust_copyuid_pairing
                && generic.reselect_after_append
                && generic.tolerate_idle_disconnect
        );
    }

    #[test]
    fn host_matching() {
        let i = info("imap.mail.yahoo.com.", "", "");
        assert!(host_is(&i, "yahoo.com"));
        assert!(!host_is(&info("notyahoo.com", "", ""), "yahoo.com"));
    }
}
