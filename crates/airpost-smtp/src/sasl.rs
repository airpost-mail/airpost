// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! The SASL client mechanisms SMTP AUTH uses, replacing cyrus-sasl.
//!
//! Derived from libetpan's `mailesmtp_auth_sasl` (`mailsmtp.c`) and `mailsmtp_oauth2.c`, as
//! driven by mailcore2's `SMTPSession::login` (`MCSMTPSession.cpp`). libetpan is Copyright ©
//! 2001-2005 DINH Viet Hoa and mailcore2 Copyright © 2001-2013 MailCore team, both
//! BSD-3-Clause; Mailspring-Sync's changes are Copyright © 2017 Foundry 376, GPL-3.0. See
//! NOTICE.md ("mailcore2", "libEtPan!").
//!
//! The mechanisms behave like cyrus-sasl's clients: PLAIN sends no authzid when it equals
//! the authcid, LOGIN answers its two challenges in order without reading their text, and
//! DIGEST-MD5 omits the authzid for the same reason and only offers `qop=auth`.

use hmac::{Hmac, KeyInit, Mac};
use md5::{Digest, Md5};

/// The mechanisms we implement, in the order `SMTPSession::login` prefers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mechanism {
    Plain,
    Login,
    DigestMd5,
    CramMd5,
}

impl Mechanism {
    pub fn name(self) -> &'static str {
        match self {
            Mechanism::Plain => "PLAIN",
            Mechanism::Login => "LOGIN",
            Mechanism::DigestMd5 => "DIGEST-MD5",
            Mechanism::CramMd5 => "CRAM-MD5",
        }
    }
}

/// A failed client step: the challenge was malformed or the server's proof did not match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepError(pub String);

/// One authentication exchange in progress.
pub struct SaslClient {
    mechanism: Mechanism,
    user: String,
    password: String,
    /// `smtp/<host>`, for DIGEST-MD5.
    digest_uri: String,
    step: u32,
    /// DIGEST-MD5's expected `rspauth`, once the response has been sent.
    expected_rspauth: Option<String>,
    cnonce: Option<String>,
}

impl SaslClient {
    pub fn new(mechanism: Mechanism, user: &str, password: &str, host: &str) -> Self {
        SaslClient {
            mechanism,
            user: user.to_owned(),
            password: password.to_owned(),
            digest_uri: format!("smtp/{host}"),
            step: 0,
            expected_rspauth: None,
            cnonce: None,
        }
    }

    /// Fixes the DIGEST-MD5 client nonce, for known-answer tests.
    pub fn with_cnonce(mut self, cnonce: &str) -> Self {
        self.cnonce = Some(cnonce.to_owned());
        self
    }

    pub fn mechanism(&self) -> Mechanism {
        self.mechanism
    }

    /// The initial response sent with `AUTH <mech>`, for mechanisms that have one.
    pub fn initial_response(&self) -> Option<Vec<u8>> {
        match self.mechanism {
            Mechanism::Plain => Some(plain_response(&self.user, &self.password)),
            Mechanism::Login | Mechanism::DigestMd5 | Mechanism::CramMd5 => None,
        }
    }

    /// Answers one decoded 334 challenge.
    pub fn step(&mut self, challenge: &[u8]) -> Result<Vec<u8>, StepError> {
        self.step += 1;
        match self.mechanism {
            Mechanism::Plain => {
                // Note: an empty 334 after `AUTH PLAIN <ir>` asks for the response again.
                Ok(plain_response(&self.user, &self.password))
            }
            Mechanism::Login => match self.step {
                1 => Ok(self.user.clone().into_bytes()),
                2 => Ok(self.password.clone().into_bytes()),
                _ => Err(StepError("unexpected LOGIN challenge".into())),
            },
            Mechanism::CramMd5 => match self.step {
                1 => Ok(cram_md5_response(&self.user, &self.password, challenge).into_bytes()),
                _ => Err(StepError("unexpected CRAM-MD5 challenge".into())),
            },
            Mechanism::DigestMd5 => match self.expected_rspauth.take() {
                None => {
                    let cnonce = self.cnonce.clone().unwrap_or_else(random_cnonce);
                    let (response, rspauth) = digest_md5_client_response(
                        &self.user,
                        &self.password,
                        &self.digest_uri,
                        challenge,
                        &cnonce,
                    )?;
                    self.expected_rspauth = Some(rspauth);
                    Ok(response.into_bytes())
                }
                Some(expected) => {
                    let directives = parse_directives(challenge)?;
                    let got = directive(&directives, "rspauth")
                        .ok_or_else(|| StepError("DIGEST-MD5 reply has no rspauth".into()))?;
                    if !got.eq_ignore_ascii_case(&expected) {
                        return Err(StepError("DIGEST-MD5 server proof mismatch".into()));
                    }
                    Ok(Vec::new())
                }
            },
        }
    }
}

fn plain_response(user: &str, password: &str) -> Vec<u8> {
    format!("\0{user}\0{password}").into_bytes()
}

/// RFC 2195: `user SP hex(HMAC-MD5(password, challenge))`.
pub fn cram_md5_response(user: &str, password: &str, challenge: &[u8]) -> String {
    let mut mac = <Hmac<Md5> as KeyInit>::new_from_slice(password.as_bytes())
        .expect("HMAC accepts keys of any length");
    mac.update(challenge);
    format!("{user} {}", hex(&mac.finalize().into_bytes()))
}

/// The XOAUTH2 initial client response (`mailsmtp_oauth2.c`), before base64.
pub fn xoauth2_response(user: &str, access_token: &str) -> Vec<u8> {
    format!("user={user}\x01auth=Bearer {access_token}\x01\x01").into_bytes()
}

fn random_cnonce() -> String {
    let bytes: [u8; 16] = rand::random();
    hex(&bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = Md5::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// The RFC 2831 §2.1.2.1 `response-value` and the `rspauth` the server must answer with,
/// for `qop=auth` without an authzid.
pub fn digest_md5_values(
    user: &str,
    realm: &str,
    password: &str,
    nonce: &str,
    cnonce: &str,
    nc: &str,
    digest_uri: &str,
) -> (String, String) {
    let secret = md5(&[
        user.as_bytes(),
        b":",
        realm.as_bytes(),
        b":",
        password.as_bytes(),
    ]);
    let a1 = md5(&[&secret, b":", nonce.as_bytes(), b":", cnonce.as_bytes()]);
    let ha1 = hex(&a1);
    let kd = |a2: &[u8]| {
        let ha2 = hex(&md5(&[a2]));
        hex(&md5(&[
            ha1.as_bytes(),
            b":",
            nonce.as_bytes(),
            b":",
            nc.as_bytes(),
            b":",
            cnonce.as_bytes(),
            b":auth:",
            ha2.as_bytes(),
        ]))
    };
    let response = kd(format!("AUTHENTICATE:{digest_uri}").as_bytes());
    let rspauth = kd(format!(":{digest_uri}").as_bytes());
    (response, rspauth)
}

fn digest_md5_client_response(
    user: &str,
    password: &str,
    digest_uri: &str,
    challenge: &[u8],
    cnonce: &str,
) -> Result<(String, String), StepError> {
    let directives = parse_directives(challenge)?;
    let nonce = directive(&directives, "nonce")
        .ok_or_else(|| StepError("DIGEST-MD5 challenge has no nonce".into()))?;
    let qop_offered = directive(&directives, "qop").unwrap_or("auth");
    if !qop_offered
        .split(',')
        .any(|q| q.trim().eq_ignore_ascii_case("auth"))
    {
        return Err(StepError(format!(
            "DIGEST-MD5 server requires a security layer (qop={qop_offered})"
        )));
    }
    // RFC 2831 §2.1.2: with no realm offered, A1 uses the empty string and the response
    // omits the directive.
    let realm = directive(&directives, "realm");
    let utf8 = directive(&directives, "charset").is_some_and(|c| c.eq_ignore_ascii_case("utf-8"));
    let nc = "00000001";
    let (response, rspauth) = digest_md5_values(
        user,
        realm.unwrap_or(""),
        password,
        nonce,
        cnonce,
        nc,
        digest_uri,
    );

    let mut out = String::new();
    if utf8 {
        out.push_str("charset=utf-8,");
    }
    out.push_str(&format!("username=\"{}\"", quote(user)));
    if let Some(realm) = realm {
        out.push_str(&format!(",realm=\"{}\"", quote(realm)));
    }
    out.push_str(&format!(
        ",nonce=\"{}\",nc={nc},cnonce=\"{}\",digest-uri=\"{}\",response={response},qop=auth",
        quote(nonce),
        quote(cnonce),
        quote(digest_uri)
    ));
    Ok((out, rspauth))
}

fn quote(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn directive<'a>(directives: &'a [(String, String)], key: &str) -> Option<&'a str> {
    directives
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.as_str())
}

/// Parses an RFC 2831 `digest-challenge`: comma-separated `key=value` pairs whose values are
/// tokens or quoted strings. Empty list elements are allowed.
fn parse_directives(input: &[u8]) -> Result<Vec<(String, String)>, StepError> {
    let text = std::str::from_utf8(input)
        .map_err(|_| StepError("DIGEST-MD5 challenge is not UTF-8".into()))?;
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let skip_ws = |i: &mut usize| {
        while *i < bytes.len() && matches!(bytes[*i], b' ' | b'\t' | b'\r' | b'\n') {
            *i += 1;
        }
    };
    loop {
        skip_ws(&mut i);
        while i < bytes.len() && bytes[i] == b',' {
            i += 1;
            skip_ws(&mut i);
        }
        if i >= bytes.len() {
            return Ok(out);
        }
        let key_start = i;
        while i < bytes.len() && !matches!(bytes[i], b'=' | b',' | b' ' | b'\t') {
            i += 1;
        }
        let key = text[key_start..i].to_owned();
        skip_ws(&mut i);
        if i >= bytes.len() || bytes[i] != b'=' {
            return Err(StepError(format!("malformed DIGEST-MD5 directive {key}")));
        }
        i += 1;
        skip_ws(&mut i);
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let mut v = Vec::new();
            loop {
                match bytes.get(i) {
                    None => return Err(StepError("unterminated DIGEST-MD5 quoted string".into())),
                    Some(b'"') => {
                        i += 1;
                        break;
                    }
                    Some(b'\\') if i + 1 < bytes.len() => {
                        v.push(bytes[i + 1]);
                        i += 2;
                    }
                    Some(&b) => {
                        v.push(b);
                        i += 1;
                    }
                }
            }
            String::from_utf8_lossy(&v).into_owned()
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            text[start..i].trim().to_owned()
        };
        out.push((key, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cram_md5_rfc2195_vector() {
        assert_eq!(
            cram_md5_response(
                "tim",
                "tanstaaftanstaaf",
                b"<1896.697170952@postoffice.reston.mci.net>"
            ),
            "tim b913a602c7eda7a495b4e6e7334d3890"
        );
    }

    #[test]
    fn digest_md5_rfc2831_vector() {
        // RFC 2831 §4, the IMAP example.
        let (response, rspauth) = digest_md5_values(
            "chris",
            "elwood.innosoft.com",
            "secret",
            "OA6MG9tEQGm2hh",
            "OA6MHXh6VqTrRk",
            "00000001",
            "imap/elwood.innosoft.com",
        );
        assert_eq!(response, "d388dad90d4bbd760a152321f2143af7");
        assert_eq!(rspauth, "ea40f60335c427b5527b84dbabcdfffd");
    }

    #[test]
    fn digest_md5_client_exchange() {
        let mut c = SaslClient {
            digest_uri: "imap/elwood.innosoft.com".into(),
            ..SaslClient::new(Mechanism::DigestMd5, "chris", "secret", "unused")
        }
        .with_cnonce("OA6MHXh6VqTrRk");
        let challenge = br#"realm="elwood.innosoft.com",nonce="OA6MG9tEQGm2hh",qop="auth",algorithm=md5-sess,charset=utf-8"#;
        let resp = String::from_utf8(c.step(challenge).unwrap()).unwrap();
        assert_eq!(
            resp,
            "charset=utf-8,username=\"chris\",realm=\"elwood.innosoft.com\",nonce=\"OA6MG9tEQGm2hh\",nc=00000001,cnonce=\"OA6MHXh6VqTrRk\",digest-uri=\"imap/elwood.innosoft.com\",response=d388dad90d4bbd760a152321f2143af7,qop=auth"
        );
        assert_eq!(
            c.step(b"rspauth=ea40f60335c427b5527b84dbabcdfffd").unwrap(),
            b""
        );
    }

    #[test]
    fn digest_md5_rejects_bad_proof() {
        let mut c = SaslClient::new(Mechanism::DigestMd5, "u", "p", "h");
        c.step(br#"nonce="abc""#).unwrap();
        assert!(c.step(b"rspauth=00").is_err());
    }

    #[test]
    fn digest_md5_requires_auth_qop() {
        let mut c = SaslClient::new(Mechanism::DigestMd5, "u", "p", "h");
        assert!(c.step(br#"nonce="abc",qop="auth-conf""#).is_err());
    }

    #[test]
    fn directives_with_escapes_and_empty_elements() {
        let d = parse_directives(br#" ,realm="a\"b", nonce = "n" ,,qop=auth "#).unwrap();
        assert_eq!(
            d,
            vec![
                ("realm".into(), "a\"b".into()),
                ("nonce".into(), "n".into()),
                ("qop".into(), "auth".into()),
            ]
        );
    }

    #[test]
    fn login_answers_in_order() {
        let mut c = SaslClient::new(Mechanism::Login, "user", "pw", "h");
        assert_eq!(c.initial_response(), None);
        assert_eq!(c.step(b"whatever").unwrap(), b"user");
        assert_eq!(c.step(b"").unwrap(), b"pw");
        assert!(c.step(b"").is_err());
    }

    #[test]
    fn plain_and_xoauth2_payloads() {
        let c = SaslClient::new(Mechanism::Plain, "user", "pw", "h");
        assert_eq!(c.initial_response().unwrap(), b"\0user\0pw");
        assert_eq!(
            xoauth2_response("a@b.c", "tok"),
            b"user=a@b.c\x01auth=Bearer tok\x01\x01"
        );
    }
}
