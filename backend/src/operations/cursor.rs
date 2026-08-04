//! The keyset cursor: sort keys plus a digest that binds the page to its query.
//!
//! Pagination is keyset-only. `OFFSET` is never used, because an offset page is
//! defined by "how many rows the source decided to skip", which changes the
//! moment a row lands — and, far worse, is a number a caller can move around a
//! result set that authorization filtered. A keyset page is defined by the last
//! row the caller actually received, which is a value they already hold.
//!
//! ## What the digest binds, and why each part is in it
//!
//! ```text
//! effective scope        an admin's `all` page must not resume a `mine` page
//! verified viewer id     a cursor issued to one person is useless to another
//! authorized session id  a lifecycle page is bound to the session that was authorized
//! record kind            the sort key space differs per kind
//! time range             a resumed page must not silently widen its window
//! normalized filters     resuming with different filters is a different query
//! ```
//!
//! The digest is a SHA-256 over that canonical string. It is not a MAC: it is not
//! a secret, and it is not trying to be unforgeable. What it must do is make a
//! cursor from another viewer/scope/session/filter *detectable*, so the answer is
//! a stable `400 invalid_activity_cursor` rather than a silent reset to page one.
//! The actual authorization is unchanged and unconditional — the viewer predicate
//! is re-derived from the verified identity on every request and injected into the
//! source query, so a forged digest buys nothing but a differently-shaped page of
//! the caller's OWN rows.
//!
//! A silent reset is deliberately not an option: it would hand a caller a page
//! they did not ask for and hide the fact that a cursor was tampered with.
//!
//! ## Why the cursor carries its own time range
//!
//! The default window is "the last 24 hours", which is a DIFFERENT window on
//! every request. If a resumed page re-derived it, consecutive pages would tile
//! over shifting boundaries — silently dropping rows at one end and repeating
//! them at the other — and the range component of the digest could never match.
//!
//! So the resolved window travels in the payload and a resumed page uses it.
//! It is inside the digest, so a tampered window fails the same check as any
//! other mutation; and a caller who states an EXPLICIT `from`/`to` that
//! disagrees with the cursor is refused rather than quietly re-windowed.
//!
//! The window is nonetheless the ONE component a caller can choose freely and
//! still produce a matching digest — every other component is re-derived
//! server-side from the current request — so a resumed window is additionally
//! re-checked against the deployment's own bounds before it is adopted. That
//! check lives with the range rules it enforces
//! ([`super::filters::check_range`]), not here, because the digest is not the
//! thing keeping the query bounded.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::AppError;

use super::filters::{ActivityFilters, RecordKind, TimeRange};

/// Cursor payload version. Bumped only with an incompatible payload shape; an
/// unknown version is rejected rather than guessed at.
pub const CURSOR_VERSION: u8 = 1;

/// Longest accepted cursor text. A cursor is server-issued and small; anything
/// larger is either a forgery or a client bug, and decoding it would only spend
/// CPU on an input that cannot validate.
pub const MAX_CURSOR_LEN: usize = 512;

/// Hex characters of the binding digest kept in the payload. 128 bits is far more
/// than enough to make an accidental collision impossible while keeping the
/// encoded cursor short.
const DIGEST_HEX_LEN: usize = 32;

/// Longest accepted event id inside a cursor (a UUID is 36 characters).
const MAX_EVENT_ID_LEN: usize = 64;

/// The sort key of the last row a page returned.
///
/// `(timestamp, event_id)` descending is the total order every source must apply,
/// with the event id breaking ties so rows sharing a millisecond still page
/// deterministically.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorKey {
    pub timestamp: DateTime<Utc>,
    pub event_id: String,
}

impl CursorKey {
    /// RFC3339 UTC with millisecond precision, matching the audit contract's own
    /// timestamp rendering so the comparison the source performs is exact.
    pub fn timestamp_rfc3339(&self) -> String {
        self.timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

/// Everything a cursor is bound to. Built from already-authorized values, never
/// from raw query input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CursorBinding {
    /// `mine` or `all` — the EFFECTIVE scope, not the requested one.
    pub scope: &'static str,
    /// The verified viewer id in personal scope; `None` in global scope, where
    /// the page is not viewer-bound.
    pub viewer_id: Option<i64>,
    /// The exact session id that passed `OperationsVisibility`, when the query
    /// asked for lifecycle rows.
    pub session_id: Option<String>,
    pub record_kind: RecordKind,
    pub range: TimeRange,
    pub filters: ActivityFilters,
}

impl CursorBinding {
    /// The canonical string the digest is taken over.
    ///
    /// Field order is fixed and every component is length-prefixed by its own
    /// `key=` label, so two different bindings cannot serialize to one string by
    /// shifting a separator into a value.
    fn canonical(&self) -> String {
        let mut parts = vec![
            format!("v={CURSOR_VERSION}"),
            format!("scope={}", self.scope),
            format!(
                "viewer={}",
                self.viewer_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string())
            ),
            format!("session={}", self.session_id.as_deref().unwrap_or("-")),
            format!("kind={}", self.record_kind.as_str()),
            format!("from={}", self.range.from_rfc3339()),
            format!("to={}", self.range.to_rfc3339()),
        ];
        parts.extend(self.filters.binding_fields());
        parts.join("\u{1f}")
    }

    /// The truncated hex digest carried in the cursor payload.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.canonical().as_bytes());
        let full = hasher.finalize();
        let mut hex = String::with_capacity(DIGEST_HEX_LEN);
        for byte in full.iter().take(DIGEST_HEX_LEN / 2) {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }
}

/// The wire payload, kept deliberately terse: a cursor is echoed in URLs and
/// browser history, so it carries sort keys and a digest and nothing a reader
/// could mine for identity.
#[derive(Debug, Deserialize, Serialize)]
struct CursorPayload {
    v: u8,
    /// Last returned row's timestamp, RFC3339 UTC.
    ts: String,
    /// Last returned row's event id.
    id: String,
    /// The resolved window's inclusive lower bound, RFC3339 UTC.
    f: String,
    /// The resolved window's exclusive upper bound, RFC3339 UTC.
    t: String,
    /// The binding digest.
    d: String,
}

/// Encode the cursor for the last row of a page.
pub fn encode(key: &CursorKey, binding: &CursorBinding) -> Result<String, AppError> {
    let payload = CursorPayload {
        v: CURSOR_VERSION,
        ts: key.timestamp_rfc3339(),
        id: key.event_id.clone(),
        f: binding.range.from_rfc3339(),
        t: binding.range.to_rfc3339(),
        d: binding.digest(),
    };
    let json = serde_json::to_vec(&payload).map_err(|error| {
        // Structurally unreachable (four owned strings), but a silent `unwrap`
        // on a pagination path is exactly how a panic reaches production.
        AppError::Internal(anyhow::anyhow!(
            "failed to encode an activity cursor: {error}"
        ))
    })?;
    Ok(URL_SAFE_NO_PAD.encode(json))
}

/// Read the window a cursor was issued for, WITHOUT trusting anything else about
/// it.
///
/// This checks the window's SYNTAX and internal ordering only. The caller must
/// still apply the deployment's range bounds to the result — see the module docs
/// and [`super::filters::check_range`] — because the digest binds the window
/// without making it unforgeable.
pub fn peek_range(raw: &str) -> Result<TimeRange, AppError> {
    let payload = parse_payload(raw)?;
    let from = parse_instant(&payload.f)?;
    let to = parse_instant(&payload.t)?;
    if from >= to {
        return Err(reject());
    }
    Ok(TimeRange { from, to })
}

/// Decode and verify a caller-supplied cursor against the current query.
///
/// Every failure — length, base64, JSON, version, timestamp syntax, event-id
/// syntax, digest mismatch — is the same stable `400 invalid_activity_cursor`.
/// The message never restates the cursor or names which check failed, because the
/// interesting case (a cursor minted for another viewer or scope) must not become
/// an oracle for what the server would have accepted.
pub fn decode(raw: &str, binding: &CursorBinding) -> Result<CursorKey, AppError> {
    let payload = parse_payload(raw)?;
    let timestamp = parse_instant(&payload.ts)?;
    // Constant-time comparison is not required (the digest is not a secret) but
    // an exact match is: a cursor whose binding differs by one filter must be
    // refused, never coerced into the current query.
    if payload.d != binding.digest() {
        tracing::info!(
            scope = binding.scope,
            record_kind = binding.record_kind.as_str(),
            "operations: refused an activity cursor bound to a different query"
        );
        return Err(reject());
    }
    Ok(CursorKey {
        timestamp,
        event_id: payload.id,
    })
}

/// The one refusal every cursor failure produces.
///
/// The message never restates the cursor or names which check failed, because
/// the interesting case — a cursor minted for another viewer or scope — must not
/// become an oracle for what the server would have accepted.
fn reject() -> AppError {
    AppError::InvalidActivityCursor(
        "cursor is not valid for this query; start a new page".to_string(),
    )
}

/// Decode the payload and check everything that does not depend on the query:
/// length, encoding, version, and identifier syntax.
fn parse_payload(raw: &str) -> Result<CursorPayload, AppError> {
    if raw.is_empty() || raw.len() > MAX_CURSOR_LEN {
        return Err(reject());
    }
    let bytes = URL_SAFE_NO_PAD.decode(raw).map_err(|_| reject())?;
    let payload: CursorPayload = serde_json::from_slice(&bytes).map_err(|_| reject())?;
    if payload.v != CURSOR_VERSION {
        return Err(reject());
    }
    // The event id becomes a query parameter, so its syntax is bounded exactly
    // like every other identifier on this surface.
    if payload.id.is_empty()
        || payload.id.len() > MAX_EVENT_ID_LEN
        || !payload
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(reject());
    }
    Ok(payload)
}

fn parse_instant(raw: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|_| reject())
}

#[cfg(test)]
#[path = "cursor_tests.rs"]
mod tests;
