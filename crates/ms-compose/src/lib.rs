// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Tern contributors

//! Drafts as the desktop builds them (docs/desktop-client-plan.md 3.3, stage 13), so every
//! client gets the same new, reply, forward and mailto drafts from `draft.create`, and the same
//! send warnings from `draft.validate`.
//!
//! Ports of the desktop's `DraftFactory` (`draft-factory.ts`), the reply participants of
//! `Message` (`message.ts`), the built-in `prepareNewDraft` extensions (the signature,
//! `composer-signature`, and tracking pixels in quotes, `remove-tracking-pixels`), the send
//! checks of `DraftEditingSession` and `composer-templates`, and the plaintext helpers of the
//! composer (`composer-editor/plaintext.ts`). Quoting runs the desktop's two HTML passes with
//! Rust libraries: DOMPurify with the desktop's allowlist becomes ammonia ([`sanitize`]), juice
//! with Chromium's user-agent stylesheet becomes css-inline ([`inline_style`]). The output is
//! the same document, not the same bytes; `tests/corpus.rs` compares it with what the desktop
//! makes of the same inputs (`apps/desktop/app/spec/draft-corpus-capture-spec.ts`) and lists
//! where the two differ.
//!
//! Everything here works on JSON values (the Message and Account JSON the engine stores) and
//! the settings it is given; nothing reads the database or the clock but [`draft`]'s builders,
//! which take `now`.

pub mod contacts;
pub mod data;
pub mod dates;
pub mod draft;
pub mod identity;
pub mod inline_style;
pub mod mailto;
pub mod participants;
pub mod pixels;
pub mod plaintext;
pub mod quote;
pub mod sanitize;
pub mod settings;
pub mod signature;
pub mod tracking;
pub mod validate;

pub use contacts::Contact;
pub use identity::{Identities, Identity};
pub use settings::ComposeSettings;
