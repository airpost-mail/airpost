//! The hint shown to the user when a connection fails in TLS (`MailUtils::tlsFailureAdvice`).
//!
//! Derived from Mailspring-Sync (`MailSync/MailUtils.cpp`), Copyright © 2017 Foundry 376,
//! GPL-3.0. See NOTICE.md.
//!
//! `test` mode returns it as `error_advice` and appends it to the transcript; the
//! foreground worker logs it and puts it in the connect error's debuginfo.
//!
//! The C++ suggested enabling "Allow insecure SSL" after a rejected handshake when the
//! account had it off. That setting lowered OpenSSL's security level to reach TLS below 1.2,
//! which this engine does not have (plan section 2), so the advice never suggests it.

use crate::ErrorCode;

/// The advice for a failed connect, or `None` when TLS is not what failed.
///
/// `tls_error` is what the TLS stack said when it rejected the negotiation
/// (`Session::last_tls_error`, `SmtpClient::last_tls_error`); sessions do not set it for a
/// rejected certificate.
pub fn tls_failure_advice(code: ErrorCode, tls_error: Option<&str>) -> Option<String> {
    let reason = tls_error.map(rejection_reason).unwrap_or_default();

    // A certificate rejection may carry no reason, and must never point at "Allow insecure
    // SSL", which never touched certificate validation. On macOS the platform verifier
    // applies Apple's requirements for TLS server certificates (iOS 13 / macOS 10.15:
    // SHA-1 signatures, RSA keys under 2048 bits, no DNS SAN, no serverAuth EKU, validity
    // over 825 days are rejected however the certificate is trusted), hence the caveat.
    if code == ErrorCode::Certificate {
        let mut advice = String::from("Tern could not verify this server's certificate");
        if reason.is_empty() {
            advice.push_str(". ");
        } else {
            advice.push_str(&format!(" ({reason}). "));
        }
        advice.push_str(
            "If it is self-signed or issued by a private certificate authority, install it in \
             your operating system's trust store (Keychain Access on macOS, the Windows \
             certificate store, or the system CA directory on Linux) and reconnect - on macOS \
             it must also meet Apple's current requirements for TLS server certificates. \
             \"Allow insecure SSL\" does not bypass certificate validation.",
        );
        return Some(advice);
    }

    // Only a failed negotiation with a stated reason gets advice: anything later
    // (authentication above all) has its own cause.
    if !matches!(
        code,
        ErrorCode::Connection | ErrorCode::TLSNotAvailable | ErrorCode::StartTLSNotAvailable
    ) {
        return None;
    }
    if reason.is_empty() || !is_incompatible_server(&reason) {
        return None;
    }
    Some(format!(
        "The server rejected the secure connection ({reason}). This server uses outdated encryption."
    ))
}

/// `_tlsRejectionReason`: the text after the last `::`, which drops OpenSSL-style
/// `error:0A00018A:SSL routines::` prefixes; rustls messages pass through unchanged.
fn rejection_reason(text: &str) -> String {
    match text.rfind("::") {
        Some(sep) if sep + 2 < text.len() => text[sep + 2..].to_owned(),
        _ => text.to_owned(),
    }
}

/// True for the rustls rejections that mean the server offers no protocol version or
/// cipher suite this engine accepts (`rustls::Error::PeerIncompatible`, or the server's
/// handshake_failure / protocol_version / insufficient_security alert). Other failures, a
/// plaintext server answering a ClientHello above all ("received corrupt message"), are not
/// outdated encryption.
fn is_incompatible_server(reason: &str) -> bool {
    reason.starts_with("peer is incompatible")
        || [
            "received fatal alert: HandshakeFailure",
            "received fatal alert: ProtocolVersion",
            "received fatal alert: InsufficientSecurity",
        ]
        .iter()
        .any(|a| reason.starts_with(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_without_reason() {
        let advice = tls_failure_advice(ErrorCode::Certificate, None).unwrap();
        assert!(advice.starts_with("Tern could not verify this server's certificate. If it is"));
        assert!(advice.ends_with("does not bypass certificate validation."));
    }

    #[test]
    fn certificate_with_reason() {
        let advice = tls_failure_advice(
            ErrorCode::Certificate,
            Some("invalid peer certificate: UnknownIssuer"),
        )
        .unwrap();
        assert!(advice.starts_with(
            "Tern could not verify this server's certificate (invalid peer certificate: UnknownIssuer). "
        ));
    }

    #[test]
    fn incompatible_server() {
        let advice = tls_failure_advice(
            ErrorCode::Connection,
            Some("peer is incompatible: NoCipherSuitesInCommon"),
        );
        assert_eq!(
            advice.as_deref(),
            Some(
                "The server rejected the secure connection (peer is incompatible: \
                 NoCipherSuitesInCommon). This server uses outdated encryption."
            )
        );
        assert!(tls_failure_advice(
            ErrorCode::StartTLSNotAvailable,
            Some("received fatal alert: ProtocolVersion")
        )
        .is_some());
    }

    #[test]
    fn openssl_prefix_is_dropped() {
        assert_eq!(
            rejection_reason("error:0A00018A:SSL routines::dh key too small"),
            "dh key too small"
        );
        assert_eq!(rejection_reason("trailing::"), "trailing::");
    }

    #[test]
    fn no_advice() {
        // A plaintext server answering the ClientHello.
        assert_eq!(
            tls_failure_advice(
                ErrorCode::Connection,
                Some("received corrupt message of type InvalidContentType")
            ),
            None
        );
        assert_eq!(tls_failure_advice(ErrorCode::Connection, None), None);
        assert_eq!(
            tls_failure_advice(
                ErrorCode::Authentication,
                Some("peer is incompatible: NoCipherSuitesInCommon")
            ),
            None
        );
    }
}
