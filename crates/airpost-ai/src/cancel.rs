// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! [`Sink`] receives streamed text; [`CancelToken`] stops a request from another thread.

use std::fmt;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

/// Where a streamed call ([`crate::Ai::summarize`], [`crate::Ai::draft_reply`]) sends text
/// as it arrives. The engine forwards each piece as an `ai.delta` notification; the call's
/// result still carries the full text.
///
/// Any `FnMut(&str) -> ControlFlow<()>` is a sink without a token.
pub trait Sink {
    /// The next piece of text, in order and never empty. Returning `Break` (the connection
    /// that asked has gone) stops the provider request; the call fails with `cancelled`.
    fn delta(&mut self, text: &str) -> ControlFlow<()>;

    /// A token that stops the request at any point, also while it waits for the first
    /// token (`ai.cancel`, or the connection closing between deltas).
    fn cancel_token(&self) -> Option<&CancelToken> {
        None
    }
}

impl<F: FnMut(&str) -> ControlFlow<()>> Sink for F {
    fn delta(&mut self, text: &str) -> ControlFlow<()> {
        self(text)
    }
}

/// The sink of calls that return only the whole text.
pub(crate) struct Discard;

impl Sink for Discard {
    fn delta(&mut self, _: &str) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }
}

/// The sink of calls that return only the whole text but can be cancelled.
pub(crate) struct Until<'a>(pub Option<&'a CancelToken>);

impl Sink for Until<'_> {
    fn delta(&mut self, _: &str) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    fn cancel_token(&self) -> Option<&CancelToken> {
        self.0
    }
}

/// Cancels a request in flight. Clones share the state; cancelling is permanent.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<Inner>);

#[derive(Default)]
struct Inner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancelToken {
    pub fn new() -> Self {
        CancelToken::default()
    }

    /// Stops the request: the provider connection is dropped and the call returns
    /// `cancelled` promptly.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    /// Resolves once [`CancelToken::cancel`] was called.
    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.0.notify.notified();
            let mut notified = std::pin::pin!(notified);
            // Registered before the flag check, so a cancel between the two still wakes us.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

impl fmt::Debug for CancelToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CancelToken")
            .field(&self.is_cancelled())
            .finish()
    }
}

/// Never resolves without a token.
pub(crate) async fn wait_cancelled(token: Option<&CancelToken>) {
    match token {
        Some(t) => t.cancelled().await,
        None => std::future::pending().await,
    }
}
