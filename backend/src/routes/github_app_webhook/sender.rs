//! Verified webhook sender identity.
//!
//! Every GitHub App delivery — `installation`, `installation_repositories`,
//! `issues`, `issue_comment`, and every event this deployment does not act on —
//! carries the same two top-level objects: `sender` (the human or App that caused
//! the event) and `installation` (which App installation it belongs to). Parsing
//! them once from the raw body, rather than per event DTO, means an unsupported
//! but validly signed delivery still produces a correctly attributed audit
//! record, and a new event shape gets identity for free.
//!
//! ## Ordering is the security property
//!
//! These fields are attacker-controlled in an UNVERIFIED body: anyone who can
//! POST to the endpoint can claim to be anyone. So this parse runs strictly AFTER
//! `HMAC-SHA256` over the exact raw bytes has verified — never before, and never
//! on a rejected delivery. The signature itself and the raw payload are never
//! retained.
//!
//! `sender.id` is the immutable numeric GitHub id and is authoritative;
//! `sender.login` is a mutable display snapshot. A delivery whose sender carries
//! no id degrades to a login-only actor rather than being dropped: the event
//! still happened, and the audit record says honestly that no immutable identity
//! was available.

use axum::http::Extensions;

use serde::Deserialize;

use crate::audit::{identity::record_identity, AuditIdentity};

/// The shape shared by every App delivery. `serde` ignores everything else.
#[derive(Debug, Deserialize)]
struct SenderEnvelope {
    #[serde(default)]
    sender: Option<SenderIdentity>,
    #[serde(default)]
    installation: Option<InstallationRef>,
}

/// The delivery's sender. `id` is optional purely defensively — GitHub sends it,
/// but an incomplete delivery must degrade, not disappear.
#[derive(Debug, Deserialize)]
struct SenderIdentity {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    login: Option<String>,
}

/// The installation the delivery belongs to; becomes the executing principal id.
#[derive(Debug, Deserialize)]
struct InstallationRef {
    id: i64,
}

/// Parse the verified body's sender/installation and publish them as this
/// request's audit identity.
///
/// MUST be called only after signature verification (see the module docs). A body
/// that does not parse yields an anonymous webhook sender rather than an error:
/// the delivery was authentic, it just did not name anyone this deployment can
/// attribute.
pub(super) fn record_verified_sender(extensions: &Extensions, body: &[u8]) {
    let envelope = serde_json::from_slice::<SenderEnvelope>(body).unwrap_or(SenderEnvelope {
        sender: None,
        installation: None,
    });
    let (sender_id, sender_login) = match envelope.sender {
        Some(sender) => (
            sender.id,
            sender.login.filter(|login| !login.trim().is_empty()),
        ),
        None => (None, None),
    };
    let installation_id = envelope.installation.map(|installation| installation.id);
    tracing::debug!(
        sender_id,
        installation_id,
        "github webhook: verified sender identity recorded"
    );
    record_identity(
        extensions,
        AuditIdentity::webhook_sender(sender_id, sender_login, installation_id),
    );
}

#[cfg(test)]
#[path = "sender_tests.rs"]
mod tests;
