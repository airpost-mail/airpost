// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The EHLO/HELO argument.
//!
//! Derived from libetpan's `get_hostname_smart_bg`, `hostname_is_valid_fqdn` and
//! `get_address_literal` (`mailsmtp.c`), Mailspring's patches to libetpan. libetpan is
//! Copyright © 2001-2005 DINH Viet Hoa, BSD-3-Clause; the patches are Copyright © 2017
//! Foundry 376, GPL-3.0. See NOTICE.md ("libEtPan!").
//!
//! RFC 5321 §4.1.1.1 wants a fully qualified domain name or an address literal, and Postfix
//! (`reject_non_fqdn_helo_hostname`), Exim and Office 365 enforce it. `gethostname()` often
//! returns neither (`Styx`, `burnination.local.`, `Bens MacBook Pro`, `work_laptop`), so the
//! local name is used only when it really is an FQDN; otherwise the address literal of the
//! connected socket is sent, as Thunderbird does. The harness checks this with
//! `FakeSmtpServer(reject_non_fqdn_helo=True)`.

use std::net::{IpAddr, SocketAddr};

/// Used only when neither the hostname nor the socket address is usable.
const FALLBACK_NAME: &str = "tern.localdomain";

/// The machine's hostname when it is a usable FQDN (`get_local_hostname`).
pub fn local_fqdn() -> Option<String> {
    let name = hostname::get().ok()?.into_string().ok()?;
    // macOS defaults to "<name>.local." and Office 365 rejects the trailing dot with
    // "501 5.5.4 Invalid domain name".
    let name = name.trim_end_matches('.');
    is_valid_fqdn(name).then(|| name.to_owned())
}

/// `get_hostname_smart_bg`: the FQDN unless `use_ip`, then the address literal, then a fixed
/// well-formed name.
pub fn ehlo_name(use_ip: bool, local: Option<SocketAddr>) -> String {
    if !use_ip {
        if let Some(name) = local_fqdn() {
            return name;
        }
    }
    local
        .map(address_literal)
        .unwrap_or_else(|| FALLBACK_NAME.to_owned())
}

/// RFC 5321 §4.1.3 address literal. IPv6 literals carry the `IPv6:` tag (Postfix's
/// `valid_mailhost_addr` rejects `[::1]`); a v4-mapped address is written as IPv4.
pub fn address_literal(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(v4) => format!("[{v4}]"),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => format!("[{v4}]"),
            None => format!("[IPv6:{v6}]"),
        },
    }
}

/// `hostname_is_valid_fqdn`: dotted labels of `[A-Za-z0-9-]`, no label starting or ending
/// with `-`, not all-numeric (that is an address), not `localhost`.
pub fn is_valid_fqdn(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    if name.eq_ignore_ascii_case("localhost") || name.eq_ignore_ascii_case("localhost.localdomain")
    {
        return false;
    }
    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    for label in &labels {
        if label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return false;
        }
    }
    // An all-digit last label means an address ("10.0.0.5"), which must be bracketed.
    !labels
        .last()
        .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fqdn_rules() {
        for ok in ["mail.example.com", "a-b.c", "host1.example.org"] {
            assert!(is_valid_fqdn(ok), "{ok}");
        }
        for bad in [
            "Styx",
            "burnination.local.",
            "Bens MacBook Pro",
            "work_laptop",
            "localhost",
            "localhost.localdomain",
            "10.0.0.5",
            "-a.b",
            "a-.b",
            "a..b",
            "host.123",
            "caf\u{e9}.example",
        ] {
            assert!(!is_valid_fqdn(bad), "{bad}");
        }
    }

    #[test]
    fn literals() {
        assert_eq!(
            address_literal("127.0.0.1:25".parse().unwrap()),
            "[127.0.0.1]"
        );
        assert_eq!(address_literal("[::1]:25".parse().unwrap()), "[IPv6:::1]");
        assert_eq!(
            address_literal("[::ffff:10.1.2.3]:25".parse().unwrap()),
            "[10.1.2.3]"
        );
        assert_eq!(
            ehlo_name(true, Some("127.0.0.1:1".parse().unwrap())),
            "[127.0.0.1]"
        );
        assert_eq!(ehlo_name(true, None), FALLBACK_NAME);
    }
}
