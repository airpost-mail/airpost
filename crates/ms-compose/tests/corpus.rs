// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! The draft pipeline corpus: inputs and what the desktop made of them
//! (`fixtures/draft_corpus.json`, written by
//! `apps/desktop/tests/integration/draft-corpus-capture.test.ts`), run through this crate.
//!
//! Text, recipients, subjects, signatures, plaintext, mailto drafts and send warnings must match
//! exactly. HTML that went through the sanitizer and the style inliner is compared as a
//! document: tags, attributes and text in order, `<style>` blocks left out, and of a `style`
//! attribute only which properties it sets. What is left of the difference between DOMPurify +
//! juice and ammonia + css-inline is listed below.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use ms_compose::dates::format_quote_date;
use ms_compose::draft::{self, Ctx};
use ms_compose::pixels::remove_tracking_pixels;
use ms_compose::plaintext::{wrap_plaintext, DEFAULT_LINE_WIDTH};
use ms_compose::quote::prepare_body_for_quoting;
use ms_compose::settings::{QuoteTexts, Signature};
use ms_compose::signature::apply_signature;
use ms_compose::validate::{validate, Problem};
use ms_compose::{ComposeSettings, Identities};
use serde_json::{json, Map, Value};
use std::cell::RefCell;

// Where the engine's HTML differs from the desktop's, and the comparison looks past it:
// - juice also keeps the user-agent and pseudo-class rules it could not inline in a `<style>`
//   block; `<style>` blocks are left out.
// - juice writes `valign` next to an inlined `vertical-align`; css-inline doesn't. `valign` is
//   left out.
// - DOMPurify reserialises colours as `rgb()` and lengths as `0px`; css-inline keeps the
//   values as written. Only the property names of a `style` are compared.
// - css-inline applies the user-agent `title { display: none }` to a `<title>` left in the
//   body; juice leaves `<title>` alone. A `<title>`'s `style` is left out.

fn corpus() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/draft_corpus.json");
    serde_json::from_str(&fs::read_to_string(path).expect("corpus")).expect("corpus parses")
}

#[derive(Debug, PartialEq, Eq)]
enum Node {
    Open(String, BTreeMap<String, String>),
    Close(String),
    Text(String),
}

struct Sink {
    out: RefCell<Vec<Node>>,
    in_style: RefCell<bool>,
}

impl TokenSink for Sink {
    type Handle = ();
    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut out = self.out.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.to_string();
                if name == "style" {
                    *self.in_style.borrow_mut() = tag.kind == TagKind::StartTag;
                    return if tag.kind == TagKind::StartTag {
                        TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::Rawtext)
                    } else {
                        TokenSinkResult::Continue
                    };
                }
                if tag.kind == TagKind::EndTag {
                    out.push(Node::Close(name));
                    return TokenSinkResult::Continue;
                }
                let mut attrs = BTreeMap::new();
                for a in tag.attrs {
                    let key = a.name.local.to_string();
                    let value = a.value.to_string();
                    match key.as_str() {
                        "valign" => {}
                        "style" if name == "title" => {}
                        "style" => {
                            let mut props: Vec<String> = value
                                .split(';')
                                .filter_map(|d| d.split_once(':'))
                                .map(|(p, _)| p.trim().to_lowercase())
                                .filter(|p| !p.is_empty())
                                .collect();
                            props.sort();
                            props.dedup();
                            if !props.is_empty() {
                                attrs.insert(key, props.join(","));
                            }
                        }
                        _ => {
                            attrs.insert(key, value);
                        }
                    }
                }
                out.push(Node::Open(name, attrs));
            }
            Token::CharacterTokens(t) if !*self.in_style.borrow() => match out.last_mut() {
                Some(Node::Text(prev)) => prev.push_str(&t),
                _ => out.push(Node::Text(t.to_string())),
            },
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn nodes(html: &str) -> Vec<Node> {
    let sink = Sink {
        out: RefCell::new(Vec::new()),
        in_style: RefCell::new(false),
    };
    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let q = BufferQueue::default();
    q.push_back(StrTendril::from_slice(html));
    let _ = tok.feed(&q);
    tok.end();
    let mut out = tok.sink.out.into_inner();
    for n in &mut out {
        if let Node::Text(t) = n {
            *t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }
    out.retain(|n| !matches!(n, Node::Text(t) if t.is_empty()));
    // `<br/>` and `<br>`, `</br>` aside, end tags of void elements don't exist.
    out.retain(|n| !matches!(n, Node::Close(t) if t == "br" || t == "img" || t == "input"));
    out
}

fn same_document(name: &str, ours: &str, theirs: &str, failures: &mut Vec<String>) {
    let (a, b) = (nodes(ours), nodes(theirs));
    if a != b {
        let at = a
            .iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        failures.push(format!(
            "{name}: documents differ at node {at}:\n  engine:  {:?}\n  desktop: {:?}\n  engine html: {ours}",
            a.get(at),
            b.get(at)
        ));
    }
}

fn texts(v: &Value) -> QuoteTexts {
    serde_json::from_value(v.clone()).expect("quote texts")
}

fn settings(c: &Value, html: bool) -> ComposeSettings {
    let mut s = ComposeSettings {
        html,
        tz: c["tz"].as_str().unwrap().parse().unwrap(),
        quote_texts: texts(&c["quoteTexts"]),
        default_account_id: Some("acc1".into()),
        ..ComposeSettings::default()
    };
    for (id, sig) in c["signatures"].as_object().unwrap() {
        s.signatures.insert(
            id.clone(),
            Signature {
                id: id.clone(),
                body: sig["body"].as_str().unwrap().into(),
            },
        );
    }
    for (email, id) in c["defaultSignatures"].as_object().unwrap() {
        s.default_signatures
            .insert(email.clone(), id.as_str().unwrap().into());
    }
    s
}

fn pick(d: &Value) -> Value {
    let people = |k: &str| {
        Value::Array(
            d[k].as_array()
                .unwrap()
                .iter()
                .map(|c| json!({"name": c["name"].as_str().unwrap_or(""), "email": c["email"]}))
                .collect(),
        )
    };
    let s = |k: &str| json!(d[k].as_str().unwrap_or(""));
    json!({
        "accountId": d["aid"], "from": people("from"), "to": people("to"), "cc": people("cc"),
        "bcc": people("bcc"), "subject": d["subject"], "plaintext": d["plaintext"],
        "threadId": s("threadId"), "rthMsgId": s("rthMsgId"), "fwdMsgId": s("fwdMsgId"),
        "files": d["file_ids"],
    })
}

#[test]
fn dates_match_the_desktop() {
    let c = corpus();
    let tz = c["tz"].as_str().unwrap().parse().unwrap();
    let mut failures = Vec::new();
    for d in c["dates"].as_array().unwrap() {
        let got = format_quote_date(d["at"].as_i64().unwrap(), &texts(&d["texts"]), tz);
        if got != d["desktop"].as_str().unwrap() {
            failures.push(format!(
                "{} {} {}: {got:?} != {:?}",
                d["at"], d["locale"], d["cycle"], d["desktop"]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn quoting_matches_the_desktop() {
    let c = corpus();
    let mut failures = Vec::new();
    for q in c["quoting"].as_array().unwrap() {
        let name = q["name"].as_str().unwrap();
        let m = &q["message"];
        let got = prepare_body_for_quoting(
            m["body"].as_str().unwrap(),
            m["plaintext"].as_bool() == Some(true),
            q["html"].as_bool().unwrap(),
        );
        same_document(name, &got, q["desktop"].as_str().unwrap(), &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn drafts_match_the_desktop() {
    let c = corpus();
    let accounts = Identities::from_accounts_json(c["accounts"].as_array().unwrap());
    let mut failures = Vec::new();
    let mut check = |name: &str, html: bool, got: Value, desktop: &Value| {
        let mut want = desktop.clone();
        let body = want.as_object_mut().unwrap().remove("body").unwrap();
        let body = body.as_str().unwrap();
        let ours = pick(&got);
        if ours != want {
            failures.push(format!(
                "{name}: fields differ\n  engine:  {ours}\n  desktop: {want}"
            ));
        }
        let got_body = got["body"].as_str().unwrap();
        if html {
            same_document(name, got_body, body, &mut failures);
        } else if got_body != body {
            failures.push(format!("{name}: body {got_body:?} != {body:?}"));
        }
    };
    for d in c["drafts"].as_array().unwrap() {
        let name = d["name"].as_str().unwrap();
        let html = d["html"].as_bool().unwrap();
        let s = settings(&c, html);
        let ctx = Ctx {
            settings: &s,
            accounts: &accounts,
            html,
            now: 1_700_000_100,
        };
        let none = Map::new();
        let got = match d["kind"].as_str().unwrap() {
            "reply" => draft::reply_draft(&ctx, &d["message"], false, &none),
            "reply-all" => draft::reply_draft(&ctx, &d["message"], true, &none),
            "forward" => draft::forward_draft(&ctx, &d["message"], &none),
            _ => draft::new_draft(&ctx, d["fields"].as_object().unwrap()),
        }
        .unwrap();
        check(name, html, got, &d["desktop"]);
    }
    for m in c["mailto"].as_array().unwrap() {
        let s = settings(&c, true);
        let ctx = Ctx {
            settings: &s,
            accounts: &accounts,
            html: true,
            now: 0,
        };
        let got = draft::mailto_draft(&ctx, None, m["url"].as_str().unwrap()).unwrap();
        check(m["url"].as_str().unwrap(), true, got, &m["desktop"]);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn signatures_pixels_and_wrapping_match_the_desktop() {
    let c = corpus();
    for s in c["signaturesApplied"].as_array().unwrap() {
        let sig = s["signature"].as_object().map(|o| Signature {
            id: o["id"].as_str().unwrap().into(),
            body: o["body"].as_str().unwrap().into(),
        });
        assert_eq!(
            apply_signature(s["body"].as_str().unwrap(), sig.as_ref()),
            s["desktop"].as_str().unwrap(),
            "{}",
            s["name"]
        );
    }
    for p in c["pixels"].as_array().unwrap() {
        assert_eq!(
            remove_tracking_pixels(p["body"].as_str().unwrap()),
            p["desktop"].as_str().unwrap()
        );
    }
    for w in c["wrap"].as_array().unwrap() {
        assert_eq!(
            wrap_plaintext(w["text"].as_str().unwrap(), DEFAULT_LINE_WIDTH),
            w["desktop"].as_str().unwrap()
        );
    }
}

#[test]
fn send_warnings_match_the_desktop() {
    let c = corpus();
    let accounts = Identities::from_accounts_json(c["accounts"].as_array().unwrap());
    let messages = |l: &[Problem]| l.iter().map(|p| p.message.clone()).collect::<Vec<_>>();
    let mut failures = Vec::new();
    for v in c["validate"].as_array().unwrap() {
        let got = validate(&v["draft"], &accounts, &[]);
        let want = &v["desktop"];
        let lists = [
            ("recipientErrors", messages(&got.recipient_errors)),
            ("recipientWarnings", messages(&got.recipient_warnings)),
            ("miscErrors", messages(&got.misc_errors)),
            ("miscWarnings", messages(&got.misc_warnings)),
        ];
        for (k, ours) in lists {
            if json!(ours) != want[k] {
                failures.push(format!("{} {k}: {ours:?} != {}", v["name"], want[k]));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
