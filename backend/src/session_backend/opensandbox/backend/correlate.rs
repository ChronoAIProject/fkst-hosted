//! Pure correlation between a session and its OpenSandbox sandbox (issue #418).
//!
//! This module is the SINGLE source of truth for how a session's identity + drift
//! state is STAMPED onto a sandbox's `metadata` and RECOVERED back — with no I/O, so
//! the whole mapping is exhaustively unit-testable. Every verb of [`super::OsbBackend`]
//! routes its correlation through here so the stamp and the recover can never drift.
//!
//! ## Grounded correction: metadata-only correlation
//! The upstream Sandbox RESPONSE (GET/list/create) carries `metadata` but NOT
//! `extensions` — `extensions` is a create-REQUEST-only field (confirmed against the
//! `sandbox-lifecycle.yml` spec at tag `server/v0.2.1`). So ALL correlation lives in
//! `metadata`; the create request's `extensions` is always empty.
//!
//! ## The label-value constraint + two unsafe values
//! `metadata` values must be valid Kubernetes label values: `≤63` chars matching
//! `[A-Za-z0-9]([-A-Za-z0-9_.]*[A-Za-z0-9])?` (start + end alphanumeric). Two values
//! the session needs to round-trip are not natively label-safe, so they are encoded:
//!
//! - **`fkst-config-hash`** is a 64-hex SHA-256 digest → exceeds 63. The reconciler
//!   compares the FULL canonical hash for config drift ([`crate::reconcile::desired`]
//!   `config_drifted` → `LivePod.config_hash`), so it must be reconstructed EXACTLY.
//!   It is split into two ≤63-char keys ([`KEY_CONFIG_HASH`] = first 32 hex,
//!   [`KEY_CONFIG_HASH_2`] = last 32 hex) and reassembled by concatenation in
//!   [`to_live_pod`]. Each half is pure hex, so unconditionally label-safe.
//! - **`fkst-work-label`** is an arbitrary-UTF-8 GitHub label (spaces / emoji). The
//!   reconciler DOES consume it on observe (`plan_repo`'s orphan branch reads the
//!   OBSERVED `LivePod.work_label` to emit `RetireWorkIssues`), so it must round-trip.
//!   It is stored as lowercase-hex of its UTF-8 bytes ([`hex_encode`]) — a form that
//!   is unconditionally label-safe (`[0-9a-f]`, always alphanumeric-bounded) — and
//!   decoded back in [`to_live_pod`]. **Length caveat:** hex doubles the byte length,
//!   so a work label longer than 31 UTF-8 bytes exceeds the 63-char value cap and is
//!   rejected LOUDLY by [`is_valid_label_value`] (never a silently-rejected create).
//!   fkst work labels are short DNS-label-ish tokens, so this is not a practical limit.
//!
//! `fkst-owner` / `fkst-repo` are GitHub logins / repo names, normally label-valid and
//! stored RAW (so the `observe_repo` metadata filter matches). A pathological name
//! (too long / bad charset) is caught by [`is_valid_label_value`] at stamp time and
//! fails the create loudly rather than emitting a request the server would reject.

use std::collections::BTreeMap;

use k8s_openapi::chrono::{DateTime, Utc};

use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::reconcile::desired::{LivePod, PodLiveness};
use crate::session_backend::opensandbox::dto::{SandboxState, SandboxView};
use crate::session_backend::{BackendError, SessionHandle};

/// Marks a sandbox as one fkst manages; the value the fleet/observe/resolve filters
/// pin so a foreign sandbox in the same project is never touched.
pub const KEY_MANAGED: &str = "fkst-managed";
/// The deterministic session id (the correlation key everything joins on).
pub const KEY_SESSION_ID: &str = "fkst-session-id";
/// The GitHub App installation id the session's token is minted from.
pub const KEY_INSTALLATION: &str = "fkst-installation-id";
/// The trigger issue number the session was launched for.
pub const KEY_TRIGGER_ISSUE: &str = "fkst-trigger-issue";
/// When the session last reported pending, as decimal epoch SECONDS (label-safe,
/// unlike an RFC3339 string with its `:` separators). `mark_pending` rewrites it.
pub const KEY_LAST_PENDING: &str = "fkst-last-pending-at";
/// The repo owner (raw; the `observe_repo` filter pins it).
pub const KEY_OWNER: &str = "fkst-owner";
/// The repo name (raw; the `observe_repo` filter pins it).
pub const KEY_REPO: &str = "fkst-repo";
/// First 32 hex of the 64-hex config hash (see the module doc for why it is split).
pub const KEY_CONFIG_HASH: &str = "fkst-config-hash";
/// Last 32 hex of the 64-hex config hash; reassembled with [`KEY_CONFIG_HASH`].
pub const KEY_CONFIG_HASH_2: &str = "fkst-config-hash-2";
/// The work label as lowercase-hex of its UTF-8 bytes (see the module doc).
pub const KEY_WORK_LABEL: &str = "fkst-work-label";

/// Where the 64-hex config hash is split into two label-safe halves.
const CONFIG_HASH_SPLIT: usize = 32;

/// Stamp `spec`'s identity + drift state onto a fresh sandbox's `metadata`.
///
/// Every value is validated against the K8s label-value contract; an invalid value is
/// a LOUD error (logged + `Err`) so the caller never issues a create the server would
/// reject. `fkst-last-pending-at` is seeded to now (epoch seconds), mirroring the
/// Kubernetes launcher's `last-pending-at` seeding.
pub fn stamp(spec: &SessionPodSpec) -> Result<BTreeMap<String, String>, BackendError> {
    let now = Utc::now().timestamp();
    let (hash_a, hash_b) = split_config_hash(&spec.config_hash);

    let mut meta = BTreeMap::new();
    put(&mut meta, KEY_MANAGED, "true".to_string())?;
    put(&mut meta, KEY_SESSION_ID, spec.session_id.clone())?;
    put(
        &mut meta,
        KEY_INSTALLATION,
        spec.installation_id.to_string(),
    )?;
    put(
        &mut meta,
        KEY_TRIGGER_ISSUE,
        spec.trigger_issue_number.to_string(),
    )?;
    put(&mut meta, KEY_LAST_PENDING, now.to_string())?;
    put(&mut meta, KEY_OWNER, spec.repo.owner.clone())?;
    put(&mut meta, KEY_REPO, spec.repo.name.clone())?;
    put(&mut meta, KEY_CONFIG_HASH, hash_a)?;
    put(&mut meta, KEY_CONFIG_HASH_2, hash_b)?;
    put(
        &mut meta,
        KEY_WORK_LABEL,
        hex_encode(spec.work_label.as_bytes()),
    )?;
    Ok(meta)
}

/// Recover the kube-free [`SessionHandle`] a live sandbox belongs to from its stamped
/// `metadata`. `None` when the sandbox is not fully one of ours — no session-id, or an
/// owner/repo/installation that does not resolve (the caller WARNs + skips). The
/// trigger issue is optional (a missing/zero/unparseable value still yields a handle).
/// Mirrors the Kubernetes backend's `pod_to_handle`.
pub fn recover(view: &SandboxView) -> Option<SessionHandle> {
    let m = &view.metadata;
    let session_id = m.get(KEY_SESSION_ID)?.clone();
    let installation_id = m.get(KEY_INSTALLATION)?.parse::<i64>().ok()?;
    let owner = m.get(KEY_OWNER)?.clone();
    let name = m.get(KEY_REPO)?.clone();
    let trigger_issue = m
        .get(KEY_TRIGGER_ISSUE)
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n != 0);
    Some(SessionHandle {
        session_id,
        installation_id,
        repo: RepoRef { owner, name },
        trigger_issue,
    })
}

/// Project a sandbox into the planner's [`LivePod`] view. `None` when the sandbox
/// carries no session-id (not one of ours / malformed) — such a sandbox is skipped,
/// never planned on. Field parity with the Kubernetes backend's `pod_to_live`:
/// `config_hash` is the reassembled 64-hex canonical hash (both halves present, else
/// `None` = "no drift decision"); `created_at` falls back to now ONLY on a
/// malformed/absent timestamp (so a real timestamp is never shadowed); `work_label`
/// is the hex-decoded original (absent/undecodable → `None` = no retire-notify).
pub fn to_live_pod(view: &SandboxView) -> Option<LivePod> {
    let m = &view.metadata;
    let session_id = m.get(KEY_SESSION_ID)?.clone();

    let trigger_issue = m
        .get(KEY_TRIGGER_ISSUE)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let liveness = state_to_liveness(&view.state);
    let created_at = view
        .created_at
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    let last_pending_at = m
        .get(KEY_LAST_PENDING)
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|secs| DateTime::from_timestamp(secs, 0));
    let config_hash = match (m.get(KEY_CONFIG_HASH), m.get(KEY_CONFIG_HASH_2)) {
        (Some(a), Some(b)) => Some(format!("{a}{b}")),
        _ => None,
    };
    let work_label = m
        .get(KEY_WORK_LABEL)
        .and_then(|s| hex_decode(s))
        .and_then(|bytes| String::from_utf8(bytes).ok());

    Some(LivePod {
        session_id,
        trigger_issue,
        liveness,
        created_at,
        last_pending_at,
        config_hash,
        work_label,
    })
}

/// Project a sandbox lifecycle state into the planner's coarse [`PodLiveness`],
/// mirroring the Kubernetes backend's `phase_to_liveness`: `Running` is `Live`,
/// terminal states are `Terminal`, and every transitional / pending / unknown state is
/// `Starting` (not yet observed running). There is no `Terminating` here — an
/// OpenSandbox delete 404s instantly (no `deletionTimestamp` window); the respawn
/// shield in [`super::OsbBackend`] synthesises that state instead.
pub fn state_to_liveness(state: &SandboxState) -> PodLiveness {
    match state {
        SandboxState::Running => PodLiveness::Live,
        SandboxState::Pending => PodLiveness::Starting,
        SandboxState::Failed | SandboxState::Terminated => PodLiveness::Terminal,
        SandboxState::Pausing
        | SandboxState::Paused
        | SandboxState::Resuming
        | SandboxState::Stopping => PodLiveness::Starting,
        SandboxState::Unknown(other) => {
            tracing::warn!(
                state = %other,
                "opensandbox correlate: unrecognized sandbox state; treating as Starting"
            );
            PodLiveness::Starting
        }
    }
}

/// Insert `key`=`value` into `meta` after validating `value` against the K8s
/// label-value contract. An invalid value is a loud error — logged + returned — never
/// silently dropped or emitted (which the server would reject on create). The value is
/// non-secret correlation data (owner/repo/label/hash), so logging it aids debugging.
fn put(meta: &mut BTreeMap<String, String>, key: &str, value: String) -> Result<(), BackendError> {
    if !is_valid_label_value(&value) {
        tracing::error!(
            key = %key,
            value = %value,
            "opensandbox correlate: metadata value violates the K8s label-value contract \
             ([A-Za-z0-9]([-A-Za-z0-9_.]*[A-Za-z0-9])?, <=63 chars); refusing to stamp"
        );
        return Err(BackendError::Other(anyhow::anyhow!(
            "opensandbox metadata value for `{key}` is not a valid Kubernetes label value"
        )));
    }
    meta.insert(key.to_string(), value);
    Ok(())
}

/// Whether `value` is a valid Kubernetes label value: non-empty, `≤63` chars, all of
/// `[-A-Za-z0-9_.]`, with the first + last char alphanumeric. All stamped values are
/// ASCII, so byte length equals char count here.
fn is_valid_label_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 63 {
        return false;
    }
    let is_alnum = |b: u8| b.is_ascii_alphanumeric();
    let is_mid = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.');
    is_alnum(bytes[0]) && is_alnum(bytes[bytes.len() - 1]) && bytes.iter().all(|&b| is_mid(b))
}

/// Split a config hash into its two label-safe halves (first [`CONFIG_HASH_SPLIT`]
/// hex, then the rest). Reversible by concatenation. A split point clamped to the
/// string length keeps this total for any input; a canonical 64-hex hash yields two
/// 32-char halves.
fn split_config_hash(hash: &str) -> (String, String) {
    let at = CONFIG_HASH_SPLIT.min(hash.len());
    (hash[..at].to_string(), hash[at..].to_string())
}

/// Lowercase-hex encode raw bytes (`[0..255]` → two chars each). Unconditionally
/// label-safe output.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decode a lowercase/uppercase-hex string back to bytes. `None` on an odd length or a
/// non-hex digit (a malformed value → treated as absent by the caller).
fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
#[path = "correlate_tests.rs"]
mod tests;
