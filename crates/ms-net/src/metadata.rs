//! Plugin metadata sync with a remote backend (plan 4.3). Derived from
//! `MailSync/MetadataWorker.cpp` and the identity-server calls in `MailSync/TaskProcessor.cpp`
//! (Copyright © 2017 Foundry 376, GPL-3.0).
//!
//! Tern has no metadata server yet, so the default is [`LocalOnly`]: metadata lives in the
//! local database, pushes are dropped and the `metadata` thread does not pull. A future
//! backend implements this trait and nothing else changes.
//!
//! Nothing calls the Mailspring identity server (D3). Each C++ call site and its answer:
//!
//! | C++ call | Answer |
//! |---|---|
//! | `MetadataWorker` `/metadata/<aid>`, `/deltas/<aid>/head`, `/deltas/<aid>/streaming` | [`MetadataBackend::pull`]; the thread is not started for [`LocalOnly`] |
//! | `performRemoteSyncbackMetadata` `POST /metadata/<aid>/<id>/<plugin>` | [`MetadataBackend::push`]; a no-op for [`LocalOnly`] |
//! | `performRemoteSendFeatureUsageEvent` `POST /api/feature_usage_event` | [`MetadataBackend::record_feature_usage`]; a no-op |
//! | `DAVWorker` `POST /api/resolve-dav-hosts` (DNS SRV through the server) | ms-dav resolves SRV records locally |
//! | `install-check` `GET <IDENTITY_SERVER>/ping` | the bin probes a TLS host with `HttpClient::probe` |

use ms_core::{Metadata, Result};
use serde_json::Value;

/// One `SyncbackMetadataTask` remote phase.
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataPush {
    pub account_id: String,
    pub model_id: String,
    /// `modelClassName`: `Thread`, `Message`, `Contact`, …
    pub model_class: String,
    pub plugin_id: String,
    pub value: Value,
    /// `modelMetadataNewVersion`.
    pub version: i64,
    /// Lets the backend attach metadata to a message that has a different id elsewhere.
    pub header_message_id: Option<String>,
}

/// What a pull delivers.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataDelta {
    Metadata(Metadata),
    /// The backend rejected the identity; the client must refresh it
    /// (`ProcessIdentityRefreshNeeded`).
    IdentityRefreshNeeded,
}

/// Shared by the foreground (push) and metadata (pull) threads.
pub trait MetadataBackend: Send + Sync {
    fn push(&self, push: &MetadataPush) -> Result<()>;

    /// Streams deltas since `cursor` into `sink` until the backend closes the stream, and
    /// returns the cursor to resume from.
    fn pull(&self, cursor: Option<&str>, sink: &mut dyn FnMut(MetadataDelta)) -> Result<String>;

    /// A client feature was used (`SendFeatureUsageEventTask`). Usage counts fed
    /// Mailspring's plan limits, which Tern does not have.
    fn record_feature_usage(&self, _feature: &str) -> Result<()> {
        Ok(())
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

    fn pull(&self, cursor: Option<&str>, _sink: &mut dyn FnMut(MetadataDelta)) -> Result<String> {
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
        let cursor = backend.pull(Some("42"), &mut |_| delivered += 1).unwrap();
        assert_eq!((cursor.as_str(), delivered), ("42", 0));
        assert_eq!(backend.pull(None, &mut |_| {}).unwrap(), "0");
    }
}
