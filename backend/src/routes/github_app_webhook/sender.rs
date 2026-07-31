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
//!
//! ## Correlation
//!
//! The same verified seam also publishes the two correlation handles a delivery
//! carries (epic `AUD-05`): GitHub's `X-GitHub-Delivery` id — the value an
//! operator types into the App's *Recent Deliveries* page to find the exact
//! delivery a record describes — and the installation the delivery belongs to.
//! The delivery id comes from a header rather than the signed body, so it is
//! *accepted*, never trusted: only after the HMAC verified, and only when it is
//! short and drawn from the same safe character set as an inbound request id.

use axum::http::{Extensions, HeaderMap};

use serde::Deserialize;

use crate::audit::request::{id::is_acceptable, with_context};
use crate::audit::validate::limits::WEBHOOK_DELIVERY_ID;
use crate::audit::{identity::record_identity, AuditIdentity};

/// Header carrying GitHub's per-delivery UUID.
const DELIVERY_HEADER: &str = "x-github-delivery";

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
/// request's audit identity and correlation.
///
/// MUST be called only after signature verification (see the module docs). A body
/// that does not parse yields an anonymous webhook sender rather than an error:
/// the delivery was authentic, it just did not name anyone this deployment can
/// attribute.
pub(super) fn record_verified_delivery(extensions: &Extensions, headers: &HeaderMap, body: &[u8]) {
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
    let delivery_id = accepted_delivery_id(headers);
    tracing::debug!(
        sender_id,
        installation_id,
        delivery_id = delivery_id.as_deref().unwrap_or(""),
        "github webhook: verified sender identity recorded"
    );
    record_identity(
        extensions,
        AuditIdentity::webhook_sender(sender_id, sender_login, installation_id),
    );
    with_context(extensions, |context| {
        if let Some(delivery_id) = delivery_id {
            context.record_webhook_delivery_id(delivery_id);
        }
        if let Some(installation_id) = installation_id {
            context.record_installation_id(installation_id);
        }
    });
}

/// GitHub's `X-GitHub-Delivery` value, when it is safe to record.
///
/// The header is outside the HMAC, so a value that is over-long, non-ASCII, or
/// carries a separator that could forge a field in a structured log or a
/// downstream query is dropped rather than sanitized: an absent correlation
/// handle is a small loss, a forged one is a corrupt trail. The accepted set
/// mirrors [`crate::audit::request::id`]'s, which comfortably covers the UUID
/// GitHub actually sends.
fn accepted_delivery_id(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(DELIVERY_HEADER)?.to_str().ok()?.trim();
    (is_acceptable(raw) && raw.len() <= WEBHOOK_DELIVERY_ID).then(|| raw.to_string())
}

#[cfg(test)]
#[path = "sender_tests.rs"]
mod tests;
