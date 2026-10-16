// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Tern contributors

//! `/tern/v1/login`: the login page the desktop opens in its own window (docs/backend-plan.md
//! 3.4).
//!
//! Success is a 200 page, never a redirect, whose `#tern-login-result` holds
//! base64url(JSON [`LoginResult`]) without padding, so the token never appears in a URL. Each
//! successful login mints a device token named from the `device` field or the User-Agent.
//! Pages are `no-store` and run no scripts.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, REFERRER_POLICY, USER_AGENT, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use maud::{html, Markup, PreEscaped, DOCTYPE};
use serde::Deserialize;

use super::AppState;
use crate::auth;
use crate::config::Signup;
use crate::ids;
use crate::repo::RepoError;
use crate::wire::{LoginResult, LOGIN_RESULT_ELEMENT_ID, LOGIN_RESULT_TYPE, LOGIN_RESULT_VERSION};

const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
     base-uri 'none'; frame-ancestors 'none'";
/// Longest device name taken from the form; longer names are cut.
pub const MAX_DEVICE_CHARS: usize = 64;

const STYLE: &str = r#"
body { font: 15px/1.4 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; margin: 0;
       background: #f5f5f7; color: #222; }
main { max-width: 380px; margin: 48px auto; padding: 0 16px; }
h1 { font-size: 22px; font-weight: 600; margin: 0 0 20px; }
h2 { font-size: 16px; font-weight: 600; margin: 28px 0 12px; }
form { background: #fff; border-radius: 8px; padding: 20px; box-shadow: 0 1px 3px rgba(0,0,0,.1); }
label { display: block; font-size: 13px; color: #555; margin-bottom: 12px; }
input[type=email], input[type=password], input[type=text] { display: block; width: 100%;
       box-sizing: border-box; margin-top: 4px; padding: 8px; font-size: 15px;
       border: 1px solid #ccc; border-radius: 4px; }
button { width: 100%; padding: 9px; font-size: 15px; border: 0; border-radius: 4px;
         background: #2f6fdf; color: #fff; cursor: pointer; }
.error { background: #fdecea; color: #a1281c; padding: 10px 12px; border-radius: 4px; margin-bottom: 16px; }
"#;

#[derive(Debug, Default, Deserialize)]
pub struct LoginQuery {
    #[serde(default)]
    device: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SubmitForm {
    action: String,
    email: String,
    password: String,
    first_name: String,
    last_name: String,
    device: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Login,
    Signup,
}

struct FormPage<'a> {
    device: &'a str,
    signup_open: bool,
    error: Option<(Mode, &'a str)>,
    email: &'a str,
}

fn respond(status: StatusCode, body: Markup) -> Response {
    let mut res = (status, Html(body.into_string())).into_response();
    let h = res.headers_mut();
    h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    res
}

fn layout(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                style { (PreEscaped(STYLE)) }
            }
            body { main { (body) } }
        }
    }
}

fn render_form(page: &FormPage) -> Markup {
    let error_for = |mode: Mode| page.error.filter(|(m, _)| *m == mode).map(|(_, e)| e);
    layout(
        "Sign in to Tern",
        html! {
            h1 { "Sign in to your Tern server" }
            // Relative actions keep working when the server sits under a path prefix.
            form method="post" action="login" {
                @if let Some(e) = error_for(Mode::Login) { div.error role="alert" { (e) } }
                input type="hidden" name="action" value="login";
                input type="hidden" name="device" value=(page.device);
                label { "Email" input type="email" name="email" value=(page.email) required autofocus; }
                label { "Password" input type="password" name="password" required; }
                button type="submit" { "Sign in" }
            }
            @if page.signup_open {
                h2 { "New here? Create an account" }
                form method="post" action="login" {
                    @if let Some(e) = error_for(Mode::Signup) { div.error role="alert" { (e) } }
                    input type="hidden" name="action" value="signup";
                    input type="hidden" name="device" value=(page.device);
                    label { "First name" input type="text" name="first_name" maxlength="100"; }
                    label { "Last name" input type="text" name="last_name" maxlength="100"; }
                    label { "Email" input type="email" name="email" required; }
                    label {
                        "Password (at least " (auth::MIN_PASSWORD_CHARS) " characters)"
                        input type="password" name="password" minlength=(auth::MIN_PASSWORD_CHARS) required;
                    }
                    button type="submit" { "Create account" }
                }
            }
        },
    )
}

/// JSON with every non-ASCII character as a `\u` escape, so a client that decodes the base64
/// with `atob` (one char per byte) gets the same text as one that decodes UTF-8.
fn ascii_json(value: &impl serde::Serialize) -> String {
    let raw = serde_json::to_string(value).expect("login result serializes");
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for unit in c.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

fn render_result(message: &str, result: &LoginResult) -> Markup {
    let encoded = URL_SAFE_NO_PAD.encode(ascii_json(result));
    layout(
        "Tern",
        html! {
            h1 { (message) }
            p { "You can close this window." }
            div id=(LOGIN_RESULT_ELEMENT_ID) hidden { (encoded) }
        },
    )
}

/// The device token's name: the form's `device`, else the User-Agent.
fn token_name(device: &str, headers: &HeaderMap) -> String {
    let device = device.trim();
    if device.is_empty() || device.chars().any(char::is_control) {
        return auth::token_name_from_user_agent(
            headers.get(USER_AGENT).and_then(|v| v.to_str().ok()),
        );
    }
    device.chars().take(MAX_DEVICE_CHARS).collect()
}

fn clip_device(device: &str) -> String {
    device.chars().take(MAX_DEVICE_CHARS).collect()
}

pub async fn form(State(state): State<AppState>, Query(q): Query<LoginQuery>) -> Response {
    let device = clip_device(&q.device);
    let page = FormPage {
        device: &device,
        signup_open: state.0.signup == Signup::Open,
        error: None,
        email: "",
    };
    respond(StatusCode::OK, render_form(&page))
}

pub async fn submit(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(f): Form<SubmitForm>,
) -> Response {
    let mode = if f.action == "signup" {
        Mode::Signup
    } else {
        Mode::Login
    };
    let signup_open = state.0.signup == Signup::Open;
    let device = clip_device(&f.device);
    let fail = |status: StatusCode, message: &str, email: &str| {
        let page = FormPage {
            device: &device,
            signup_open,
            error: Some((mode, message)),
            email,
        };
        respond(status, render_form(&page))
    };

    if mode == Mode::Signup && !signup_open {
        return fail(
            StatusCode::FORBIDDEN,
            "Sign-up is closed on this server.",
            "",
        );
    }
    // Checked before the rate limits, so an oversized email never becomes a limiter key.
    if f.email.trim().len() > auth::MAX_EMAIL_BYTES {
        return fail(StatusCode::BAD_REQUEST, "Enter a valid email address.", "");
    }
    if !state.login_allowed(state.client_ip(peer.ip(), &headers), &f.email) {
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts. Wait a minute and try again.",
            "",
        );
    }

    let token_name = token_name(&f.device, &headers);
    let now = ids::now_ms();
    let email = f.email.clone();
    let password = f.password.clone();
    let _permit = state.password_hash_permit().await;
    let outcome = match mode {
        Mode::Login => {
            state
                .blocking(move |r| {
                    Ok(match auth::login(r, &email, &password, &token_name, now)? {
                        Some((user, token)) => Outcome::SignedIn(user, token),
                        None => Outcome::Rejected(
                            StatusCode::UNAUTHORIZED,
                            "Incorrect email address or password.".into(),
                        ),
                    })
                })
                .await
        }
        Mode::Signup => {
            let (first, last) = (f.first_name.clone(), f.last_name.clone());
            state
                .blocking(move |r| {
                    let new = match auth::new_user(&email, &first, &last, &password) {
                        Ok(n) => n,
                        Err(e) => {
                            return Ok(Outcome::Rejected(
                                StatusCode::BAD_REQUEST,
                                capitalize(&e.to_string()),
                            ))
                        }
                    };
                    let user = match r.create_user(&new, now) {
                        Ok(u) => u,
                        Err(RepoError::EmailTaken) => {
                            return Ok(Outcome::Rejected(
                                StatusCode::CONFLICT,
                                "An account with that email address already exists.".into(),
                            ))
                        }
                        Err(e) => return Err(e),
                    };
                    let token = auth::issue_token(r, &user, &token_name, now)?;
                    Ok(Outcome::SignedIn(user, token))
                })
                .await
        }
    };

    match outcome {
        Ok(Outcome::SignedIn(user, token)) => {
            let result = LoginResult {
                kind: LOGIN_RESULT_TYPE.to_owned(),
                version: LOGIN_RESULT_VERSION,
                token,
                user_id: user.id.clone(),
                server_id: state.0.server_id.clone(),
                email: user.email.clone(),
            };
            let message = format!("Signed in as {}", user.email);
            respond(StatusCode::OK, render_result(&message, &result))
        }
        Ok(Outcome::Rejected(status, message)) => {
            let email = if mode == Mode::Login {
                f.email.as_str()
            } else {
                ""
            };
            fail(status, &message, email)
        }
        Err(e) => {
            // ApiError::Internal logs when turned into a response; log here instead.
            tracing::error!(error = ?e, "login failed");
            fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong. Try again.",
                "",
            )
        }
    }
}

enum Outcome {
    SignedIn(crate::repo::User, String),
    Rejected(StatusCode, String),
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().chain(c).collect::<String>() + ".",
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::ascii_json;

    #[test]
    fn ascii_json_escapes_non_ascii() {
        let s = ascii_json(&serde_json::json!({"firstName": "Zoë 😀", "a": "\"q\""}));
        assert!(s.is_ascii());
        assert!(s.contains("Zo\\u00eb \\ud83d\\ude00"));
        let back: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(back["firstName"], "Zoë 😀");
        assert_eq!(back["a"], "\"q\"");
    }
}
