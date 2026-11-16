// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Airpost contributors

//! Throwaway driver for the IMAP spike: runs async-imap
//! against the harness servers and prints what it can and cannot see.
//!
//! ```sh
//! cargo run -p airpost-imap --example async_imap_spike            # fake, dovecot, cyrus
//! AIRPOST_IMAP_SERVERS=fake cargo run -p airpost-imap --example async_imap_spike
//! ```

#[path = "../tests/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};

use async_imap::extensions::idle::IdleResponse;
use async_imap::types::UnsolicitedResponse;
use futures::TryStreamExt;
use support::ServerProc;
use tokio::net::TcpStream;

type Session = async_imap::Session<TcpStream>;

async fn login(s: &ServerProc) -> Session {
    let tcp = TcpStream::connect((s.host.as_str(), s.port)).await.unwrap();
    let mut client = async_imap::Client::new(tcp);
    let greeting = client.read_response().await.unwrap().unwrap();
    println!("  greeting: {:?}", greeting.parsed());
    client
        .login(&s.user, &s.password)
        .await
        .map_err(|(e, _)| e)
        .unwrap()
}

fn drain(session: &Session, label: &str) {
    while let Ok(u) = session.unsolicited_responses.try_recv() {
        match u {
            UnsolicitedResponse::Other(r) => {
                println!("  [{label}] unsolicited Other: {:?}", r.parsed())
            }
            other => println!("  [{label}] unsolicited: {other:?}"),
        }
    }
}

async fn unsolicited_and_idle(s: &ServerProc) {
    println!("Q1/Q4 unsolicited responses, IDLE, cross-thread interrupt");
    let mut a = login(s).await;
    let mut b = login(s).await;
    a.run_command_and_check_ok("ENABLE QRESYNC").await.unwrap();
    let mb = a.select("INBOX").await.unwrap();
    println!(
        "  select: exists={} uidnext={:?} hms={:?}",
        mb.exists, mb.uid_next, mb.highest_modseq
    );
    b.select("INBOX").await.unwrap();

    // VANISHED (EARLIER) in reply to UID FETCH ... (CHANGEDSINCE n VANISHED)
    let hms = mb.highest_modseq.unwrap_or(1);
    b.uid_store("5", "+FLAGS.SILENT (\\Deleted)")
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    b.uid_expunge("5")
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let fetched: Vec<_> = a
        .uid_fetch("1:*", format!("(UID FLAGS) (CHANGEDSINCE {hms} VANISHED)"))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    println!(
        "  CHANGEDSINCE fetch returned {} FETCH items",
        fetched.len()
    );
    drain(&a, "after CHANGEDSINCE");

    // IDLE: changes from session B, interrupt from another OS thread.
    let mut idle = a.idle();
    idle.init().await.unwrap();
    let (fut, stop) = idle.wait_with_timeout(Duration::from_secs(60));
    let started = Instant::now();
    let b_task = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let raw = b"From: x@example.test\r\nSubject: during idle\r\nMessage-ID: <spike-1@example.test>\r\n\r\nbody\r\n";
        b.append("INBOX", None, None, raw.as_slice()).await.unwrap();
        b
    };
    let (res, b2) = tokio::join!(fut, b_task);
    b = b2;
    println!(
        "  first IDLE wake after {:?}: {:?}",
        started.elapsed(),
        res.map(|r| match r {
            IdleResponse::NewData(d) => format!("NewData({:?})", d.parsed()),
            other => format!("{other:?}"),
        })
    );
    drop(stop);
    let (fut, stop) = idle.wait_with_timeout(Duration::from_secs(60));
    let started = Instant::now();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        drop(stop);
    });
    let res = fut.await;
    t.join().unwrap();
    println!(
        "  interrupt from another thread after {:?}: {res:?}",
        started.elapsed()
    );
    let mut a = idle.done().await.unwrap();
    drain(&a, "after DONE");

    // A plain (non-QRESYNC) session sees EXPUNGE; check the channel during IDLE.
    b.uid_store("6", "+FLAGS.SILENT (\\Deleted)")
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    b.uid_expunge("6")
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    a.noop().await.unwrap();
    drain(&a, "after NOOP");
    let _ = a.logout().await;
    let _ = b.logout().await;
}

async fn message_limit() {
    println!("Q2 MESSAGELIMIT (fake yahoo-messagelimit, MESSAGELIMIT=50)");
    let mut s = ServerProc::start("fake", "yahoo-messagelimit", false);
    let raws: Vec<String> = (0..60)
        .map(|i| {
            format!(
                "b64:{}",
                b64(format!(
                    "From: a@b.test\r\nSubject: m{i}\r\nMessage-ID: <ml-{i}@b.test>\r\n\r\nx\r\n"
                )
                .as_bytes())
            )
        })
        .collect();
    s.call("populate", serde_json::json!(["INBOX", raws]));
    let mut a = login(&s).await;
    a.select("INBOX").await.unwrap();
    let fetched: Vec<_> = a
        .uid_fetch("1:*", "(UID FLAGS)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    println!("  UID FETCH 1:* returned {} of 60; the tagged OK and its [MESSAGELIMIT] code are consumed inside uid_fetch", fetched.len());
    drain(&a, "after FETCH");
}

fn lenient_envelopes() {
    println!("Q3 lenient ENVELOPE parsing (imap-proto, which async-imap uses)");
    let cases: [(&str, &[u8]); 5] = [
        ("well-formed", b"* 1 FETCH (UID 1 ENVELOPE (\"Mon, 5 Jan 2026 09:00:00 +0000\" \"hi\" ((\"A\" NIL \"a\" \"b.test\")) NIL NIL NIL NIL NIL NIL \"<m@b>\"))\r\n"),
        ("8-bit subject in quoted string", "* 1 FETCH (UID 1 ENVELOPE (NIL \"caf\u{e9}\" NIL NIL NIL NIL NIL NIL NIL NIL))\r\n".as_bytes()),
        ("unescaped quote in display name", b"* 1 FETCH (UID 1 ENVELOPE (NIL \"x\" ((\"Jo \"Q\" Doe\" NIL \"j\" \"d.test\")) NIL NIL NIL NIL NIL NIL NIL))\r\n"),
        ("address group as bare NIL list", b"* 1 FETCH (UID 1 ENVELOPE (NIL \"x\" (NIL) NIL NIL NIL NIL NIL NIL NIL))\r\n"),
        ("NAMESPACE response", b"* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n"),
    ];
    for (label, raw) in cases {
        let r = imap_proto::Response::parse(raw);
        println!(
            "  {label}: {}",
            match r {
                Ok((_, resp)) => format!("parsed: {resp:?}")
                    .chars()
                    .take(120)
                    .collect::<String>(),
                Err(e) => format!("ERROR {e:?}").chars().take(120).collect(),
            }
        );
    }
    for raw in [
        &b"* XLIST (\\HasNoChildren \\Inbox) \"/\" \"Inbox\"\r\n"[..],
        b"a1 OK [MESSAGELIMIT 50 11] FETCH completed\r\n",
        b"* OK [NOMODSEQ] no modseq\r\n",
    ] {
        let r = imap_proto::Response::parse(raw);
        println!(
            "  {:?}: {}",
            String::from_utf8_lossy(raw).trim_end(),
            match r {
                Ok((_, resp)) => format!("parsed: {resp:?}")
                    .chars()
                    .take(140)
                    .collect::<String>(),
                Err(e) => format!("ERROR {e:?}"),
            }
        );
    }
}

async fn compress(s: &ServerProc) {
    println!("Q6 COMPRESS=DEFLATE");
    let mut a = login(s).await;
    let caps = a.capabilities().await.unwrap();
    if !caps.has_str("COMPRESS=DEFLATE") {
        println!("  {} does not advertise COMPRESS=DEFLATE", s.kind);
        return;
    }
    let mut z = a.compress(|s| s).await.unwrap();
    let mb = z.select("INBOX").await.unwrap();
    let fetched: Vec<_> = z
        .uid_fetch("1:*", "(UID FLAGS ENVELOPE)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    println!(
        "  compressed session: exists={} fetched={}",
        mb.exists,
        fetched.len()
    );
}

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    lenient_envelopes();
    println!("Q5 C-prefixed tags: async-imap's IdGenerator always produces \"A0001\"-style tags; there is no hook to change the prefix.");
    for kind in support::kinds() {
        println!("=== {kind}");
        let s = ServerProc::start(&kind, "", true);
        rt.block_on(unsolicited_and_idle(&s));
        rt.block_on(compress(&s));
    }
    if support::kinds().iter().any(|k| k == "fake") {
        rt.block_on(message_limit());
    }
}
