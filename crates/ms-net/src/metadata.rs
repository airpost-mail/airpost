// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Plugin metadata sync with a remote backend. Derived from
//! `MailSync/MetadataWorker.cpp` and the identity-server calls in `MailSync/TaskProcessor.cpp`
//! (Copyright © 2017 Foundry 376, GPL-3.0).
//!
//! Without a Tern server the backend is [`LocalOnly`]: metadata lives in the local database,
//! pushes are dropped and the `metadata` thread does not pull. With one it is
//! [`crate::HttpMetadataBackend`] (docs/backend-plan.md 4.1).
//!
//! Nothing calls the Mailspring identity server (D3). Each C++ call site and its answer:
//!
//! | C++ call | Answer |
//! |---|---|
//! | `MetadataWorker` `/metadata/<aid>`, `/deltas/<aid>/head`, `/deltas/<aid>/streaming` | [`MetadataBackend::pull`]; the thread is not started for [`LocalOnly`]. The HTTP backend streams from cursor `0` instead of head + pages |
//! | `performRemoteSyncbackMetadata` `POST /metadata/<aid>/<id>/<plugin>` | [`MetadataBackend::push`]; a no-op for [`LocalOnly`] |
//! | `performRemoteSendFeatureUsageEvent` `POST /api/feature_usage_event` | [`MetadataBackend::record_feature_usage`]; a no-op |
//! | `DAVWorker` `POST /api/resolve-dav-hosts` (DNS SRV through the server) | ms-dav resolves SRV records locally |
//! | `install-check` `GET <IDENTITY_SERVER>/ping` | the bin probes a TLS host with `HttpClient::probe` |

use ms_core::constants::AI_TRIAGE_PLUGIN;
use ms_core::{Metadata, Result};
use serde_json::Value;

/// Plugin ids that stay on the device: no backend pushes, backfills or applies them. AI
/// triage results are mail content a model read; each device triages
/// its own mail.
pub const LOCAL_ONLY_PLUGINS: &[&str] = &[AI_TRIAGE_PLUGIN];

pub fn is_local_only_plugin(plugin_id: &str) -> bool {
    LOCAL_ONLY_PLUGINS.contains(&plugin_id)
}

/// One `SyncbackMetadataTask` remote phase.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataPush {
    pub account_id: String,
    pub model_id: String,
    /// `modelClassName` as the client sent it (`Thread`, `Message`, `Contact`; any case). The
    /// wire carries it lowercased.
    pub model_class: String,
    pub plugin_id: String,
    pub value: Value,
    /// `modelMetadataNewVersion`.
    pub version: i64,
    /// Lets another device find the model when its id differs there: the message's
    /// Message-ID, or for a thread the Message-ID of the message its id is made from (the
    /// thread anchor, 4.1).
    pub header_message_id: Option<String>,
}

/// What a pull delivers.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataDelta {
    /// One plugin's metadata on one model. The worker applies it if it is newer than what
    /// the model holds, or keeps it in `DetatchedPluginMetadata` until the model arrives.
    Metadata(Metadata),
    /// Thread metadata with its anchor: the Message-ID of the message whose id formed the
    /// thread id on the device that pushed it. Thread ids differ across devices, so the
    /// worker finds the local thread through that message.
    AnchoredThreadMetadata(Metadata, String),
    /// Everything before this point has been delivered: the worker stores the cursor and
    /// resumes from it after a restart (`setDeltaCursor` after each streamed delta).
    Cursor(String),
    /// The server says the identity changed; the client must refresh it
    /// (`ProcessIdentityRefreshNeeded`).
    IdentityRefreshNeeded,
}

/// Where a tracked message's pixel and links point (docs/backend-plan.md 3.6): the server's
/// `publicURL` without a trailing slash, and the user's `trackingNS`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackingTarget {
    pub public_url: String,
    pub ns: String,
}

/// The answer to an expiration claim (docs/backend-plan.md 3.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim {
    /// No server to ask: act.
    NoServer,
    /// This device holds the claim (first caller, or a repeat of its own): act.
    Won,
    /// Another device claimed it first: leave it to that one.
    Lost,
    /// The server couldn't say (network error, refused token, an unexpected answer): act
    /// anyway, since no feature depends on the server (D2), accepting a possible duplicate.
    Unreachable,
}

impl Claim {
    pub fn acts(self) -> bool {
        self != Claim::Lost
    }
}

/// Shared by the foreground (push) and metadata (pull) threads. Implementations never send
/// [`LOCAL_ONLY_PLUGINS`] to a server nor deliver them from one.
pub trait MetadataBackend: Send + Sync {
    fn push(&self, push: &MetadataPush) -> Result<()>;

    /// Streams deltas since `cursor` into `sink` until the backend closes the stream, and
    /// returns the cursor to resume from. Without a cursor (first run, or after a reset)
    /// it delivers the head cursor and then every existing record (`fetchDeltaCursor` and
    /// the `/metadata` pages). The `metadata` thread calls it again as soon as it returns,
    /// so it should block while the backend has nothing new. A retryable `SyncError`
    /// backs off (3 s up to 300 s); any other error stops the account.
    ///
    /// `stop` turns true when the account stops: the pull must then return soon (within
    /// [`ms_core::constants::ACCOUNT_STOP_POLL_MS`] or so), with the cursor of what it
    /// delivered, however long the stream would have stayed open.
    fn pull(
        &self,
        cursor: Option<&str>,
        stop: &dyn Fn() -> bool,
        sink: &mut dyn FnMut(MetadataDelta),
    ) -> Result<String>;

    /// Called by the `metadata` thread before each pull: the `"<serverId>:<userId>"` the
    /// stored cursor must belong to, or `None` for a backend without one. When it differs
    /// from the stored binding the thread discards the cursor and calls [`Self::rebind`].
    fn binding(&self) -> Result<Option<String>> {
        Ok(None)
    }

    /// The binding changed: drop queued pushes, which were meant for another server or
    /// user, and queue `local` (every metadata value this account holds) as backfill.
    fn rebind(&self, _local: &[MetadataPush]) -> Result<()> {
        Ok(())
    }

    /// Sends queued pushes and backfill. Called after the binding check, before `pull`.
    fn drain_outbox(&self) -> Result<()> {
        Ok(())
    }

    /// A client feature was used (`SendFeatureUsageEventTask`). Usage counts fed
    /// Mailspring's plan limits, which Tern does not have.
    fn record_feature_usage(&self, _feature: &str) -> Result<()> {
        Ok(())
    }

    /// Where tracking URLs point, when a server that offers tracking is connected; `None`
    /// sends tracked drafts untracked. It may make a request, so callers ask outside a store
    /// transaction.
    fn tracking(&self) -> Option<TrackingTarget> {
        None
    }

    /// Claims the right to act on `plugin_id`'s expiration at `expiration` (Unix seconds) on
    /// the object a Message-ID names, before the expiration executor acts on it, so that one
    /// device of the user's acts. It makes a request; callers ask outside a store transaction.
    fn claim(&self, _object_id: &str, _plugin_id: &str, _expiration: i64) -> Claim {
        Claim::NoServer
    }

    /// The metadata thread is not started for a local-only backend.
    fn is_local_only(&self) -> bool {
        false
    }
}

/// The default backend: push is a no-op and pull is never started.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalOnly;

impl MetadataBackend for LocalOnly {
    fn push(&self, _push: &MetadataPush) -> Result<()> {
        Ok(())
    }

    fn pull(
        &self,
        cursor: Option<&str>,
        _stop: &dyn Fn() -> bool,
        _sink: &mut dyn FnMut(MetadataDelta),
    ) -> Result<String> {
        Ok(cursor.unwrap_or("0").to_owned())
    }

    fn is_local_only(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_only_answers_every_call_site() {
        let backend: &dyn MetadataBackend = &LocalOnly;
        assert!(backend.is_local_only());
        let push = MetadataPush {
            account_id: "a".into(),
            model_id: "m".into(),
            model_class: "Thread".into(),
            plugin_id: "snooze".into(),
            value: json!({"expiration": 1}),
            version: 2,
            header_message_id: None,
        };
        assert!(backend.push(&push).is_ok());
        assert!(backend.record_feature_usage("snooze").is_ok());

        let mut delivered = 0;
        let cursor = backend
            .pull(Some("42"), &|| false, &mut |_| delivered += 1)
            .unwrap();
        assert_eq!((cursor.as_str(), delivered), ("42", 0));
        assert_eq!(backend.pull(None, &|| false, &mut |_| {}).unwrap(), "0");
    }

    #[test]
    fn triage_results_are_local_only() {
        assert!(is_local_only_plugin("tern-ai-triage"));
        assert!(!is_local_only_plugin("thread-snooze"));
    }
}
