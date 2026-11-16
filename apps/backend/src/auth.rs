// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Passwords, device tokens and the `Authorization` header (docs/backend-plan.md 3.4, 5).

use std::sync::OnceLock;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::ids;
use crate::repo::{NewUser, Repo, RepoError, User};

pub const MIN_PASSWORD_CHARS: usize = 8;
pub const MAX_PASSWORD_BYTES: usize = 1024;
/// RFC 5321 caps a forward path at 256 octets including the angle brackets.
pub const MAX_EMAIL_BYTES: usize = 254;
/// Device token names come from the User-Agent; keep them short.
pub const MAX_TOKEN_NAME_CHARS: usize = 200;

/// argon2id with the crate's default (OWASP) parameters, as a PHC string.
pub fn hash_password(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("argon2 with default params and a fresh salt cannot fail")
        .to_string()
}

pub fn verify_password(password: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Spends the same time as a real verification, so a login for an unknown email is not
/// distinguishable by timing.
fn verify_dummy(password: &str) {
    static DUMMY: OnceLock<String> = OnceLock::new();
    let phc = DUMMY.get_or_init(|| hash_password("airpost-dummy-password"));
    let _ = verify_password(password, phc);
}

/// Extracts the token from `Basic base64("<token>:")` (what the desktop and both engines
/// send) or `Bearer <token>`.
pub fn token_from_header(value: &str) -> Option<String> {
    let (scheme, rest) = value.trim().split_once(' ')?;
    let rest = rest.trim();
    let token = if scheme.eq_ignore_ascii_case("basic") {
        let decoded = STANDARD.decode(rest).ok()?;
        let decoded = String::from_utf8(decoded).ok()?;
        match decoded.split_once(':') {
            Some((user, _password)) => user.to_owned(),
            None => decoded,
        }
    } else if scheme.eq_ignore_ascii_case("bearer") {
        rest.to_owned()
    } else {
        return None;
    };
    (!token.is_empty()).then_some(token)
}

pub fn token_name_from_user_agent(ua: Option<&str>) -> String {
    match ua.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ua) => ua.chars().take(MAX_TOKEN_NAME_CHARS).collect(),
        None => "unknown device".to_owned(),
    }
}

/// Mints a device token for `user`. Returns the plaintext token; only its hash is stored.
pub fn issue_token(
    repo: &dyn Repo,
    user: &User,
    name: &str,
    now: i64,
) -> Result<String, RepoError> {
    let token = ids::new_token();
    repo.create_token(&user.id, &ids::hash_token(&token), name, now)?;
    Ok(token)
}

/// Checks email and password and, on success, issues a device token. Disabled users fail like
/// a wrong password.
pub fn login(
    repo: &dyn Repo,
    email: &str,
    password: &str,
    token_name: &str,
    now: i64,
) -> Result<Option<(User, String)>, RepoError> {
    if password.len() > MAX_PASSWORD_BYTES {
        return Ok(None);
    }
    let Some((user, phc)) = repo.login_candidate(email)? else {
        verify_dummy(password);
        return Ok(None);
    };
    if !verify_password(password, &phc) || user.disabled_at.is_some() {
        return Ok(None);
    }
    let token = issue_token(repo, &user, token_name, now)?;
    Ok(Some((user, token)))
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NewUserError {
    #[error("enter a valid email address")]
    Email,
    #[error("the password must have at least {MIN_PASSWORD_CHARS} characters")]
    PasswordTooShort,
    #[error("the password is too long")]
    PasswordTooLong,
    #[error("names are limited to 100 characters")]
    Name,
}

/// Validates sign-up input and hashes the password.
pub fn new_user(
    email: &str,
    first_name: &str,
    last_name: &str,
    password: &str,
) -> Result<NewUser, NewUserError> {
    let email = email.trim();
    let valid_email = email.len() <= MAX_EMAIL_BYTES
        && !email.chars().any(|c| c.is_whitespace() || c.is_control())
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.is_empty() && !domain.contains('@')
        });
    if !valid_email {
        return Err(NewUserError::Email);
    }
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(NewUserError::PasswordTooShort);
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(NewUserError::PasswordTooLong);
    }
    if first_name.chars().count() > 100 || last_name.chars().count() > 100 {
        return Err(NewUserError::Name);
    }
    Ok(NewUser {
        email: email.to_owned(),
        first_name: first_name.to_owned(),
        last_name: last_name.to_owned(),
        password_hash: hash_password(password),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::SqliteRepo;

    #[test]
    fn password_round_trip() {
        let h = hash_password("correct horse");
        assert!(h.starts_with("$argon2id$"));
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong horse", &h));
        assert!(!verify_password("x", "not a phc string"));
    }

    #[test]
    fn header_parsing() {
        let basic = |s: &str| format!("Basic {}", STANDARD.encode(s));
        assert_eq!(
            token_from_header(&basic("airpost_abc:")).as_deref(),
            Some("airpost_abc")
        );
        assert_eq!(
            token_from_header(&basic("airpost_abc")).as_deref(),
            Some("airpost_abc")
        );
        assert_eq!(
            token_from_header(&basic("airpost_abc:ignored")).as_deref(),
            Some("airpost_abc")
        );
        assert_eq!(
            token_from_header("Bearer airpost_abc").as_deref(),
            Some("airpost_abc")
        );
        assert_eq!(
            token_from_header("bearer  airpost_abc ").as_deref(),
            Some("airpost_abc")
        );
        assert_eq!(token_from_header(&basic(":")), None);
        assert_eq!(token_from_header("Basic !!!"), None);
        assert_eq!(token_from_header("Bearer "), None);
        assert_eq!(token_from_header("Digest x"), None);
        assert_eq!(token_from_header("airpost_abc"), None);
    }

    #[test]
    fn token_names() {
        assert_eq!(token_name_from_user_agent(None), "unknown device");
        assert_eq!(token_name_from_user_agent(Some("  ")), "unknown device");
        assert_eq!(
            token_name_from_user_agent(Some("Airpost/1.0")),
            "Airpost/1.0"
        );
        assert_eq!(
            token_name_from_user_agent(Some(&"é".repeat(500)))
                .chars()
                .count(),
            200
        );
    }

    #[test]
    fn new_user_validation() {
        assert!(new_user("a@b", "", "", "12345678").is_ok());
        assert_eq!(
            new_user("ab", "", "", "12345678").unwrap_err(),
            NewUserError::Email
        );
        assert_eq!(
            new_user("a@", "", "", "12345678").unwrap_err(),
            NewUserError::Email
        );
        assert_eq!(
            new_user("a b@c", "", "", "12345678").unwrap_err(),
            NewUserError::Email
        );
        assert_eq!(
            new_user("a@b@c", "", "", "12345678").unwrap_err(),
            NewUserError::Email
        );
        assert_eq!(
            new_user("a@b", "", "", "1234567").unwrap_err(),
            NewUserError::PasswordTooShort
        );
        assert_eq!(
            new_user("a@b", "", "", &"x".repeat(MAX_PASSWORD_BYTES + 1)).unwrap_err(),
            NewUserError::PasswordTooLong
        );
        assert_eq!(
            new_user("a@b", &"n".repeat(101), "", "12345678").unwrap_err(),
            NewUserError::Name
        );
    }

    #[test]
    fn login_issues_a_token_per_call() {
        let dir = tempfile::tempdir().unwrap();
        let repo = SqliteRepo::open(dir.path()).unwrap();
        let user = repo
            .create_user(
                &new_user("a@example.com", "A", "B", "password1").unwrap(),
                1,
            )
            .unwrap();

        assert!(login(&repo, "a@example.com", "wrong-pass", "x", 2)
            .unwrap()
            .is_none());
        assert!(login(&repo, "nobody@example.com", "password1", "x", 2)
            .unwrap()
            .is_none());

        let (u, t1) = login(&repo, "A@Example.com", "password1", "laptop", 3)
            .unwrap()
            .unwrap();
        assert_eq!(u, user);
        let (_, t2) = login(&repo, "a@example.com", "password1", "desktop", 4)
            .unwrap()
            .unwrap();
        assert_ne!(t1, t2);
        assert!(repo.authenticate(&ids::hash_token(&t1)).unwrap().is_some());
        let names: Vec<_> = repo
            .list_tokens(Some(&user.id))
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["laptop", "desktop"]);

        repo.set_user_disabled(&user.id, true, 5).unwrap();
        assert!(login(&repo, "a@example.com", "password1", "x", 6)
            .unwrap()
            .is_none());
    }
}
