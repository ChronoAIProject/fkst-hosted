//! The ONE projection both adapters funnel their raw facts through.
//!
//! Kubernetes and OpenSandbox disagree about almost everything at the wire level
//! and about nothing at the semantic level: both stamp the same correlation keys,
//! both carry the same [`crate::runtime_identity`] attribution stamp, and both owe
//! the operations view the same normalized answer. So each adapter's job is
//! reduced to *extracting* [`RawRuntimeFacts`] from its own object model, and
//! every decision that could drift between them — metadata state, id parsing,
//! timing, bounding, warning emission — happens exactly once, here.
//!
//! That is what makes the cross-adapter contract tests meaningful: feeding
//! equivalent logical runtimes through both adapters must produce identical
//! normalized rows wherever the backends expose the same facts, and it does
//! because there is only one implementation of "normalized".

use k8s_openapi::chrono::{DateTime, Utc};

use crate::runtime_identity::{ObservedRuntimeIdentity, RuntimeBackendKind};

use super::status::RuntimeInventoryStatus;
use super::text::{
    bounded_operational_text, MAX_RAW_STATUS_BYTES, MAX_STATUS_MESSAGE_BYTES,
    MAX_STATUS_REASON_BYTES,
};
use super::timing;
use super::warning::{InventoryWarningCode, WarningSink};
use super::{RuntimeInventoryItem, RuntimeLifetimePolicy, RuntimeMetadataState};

/// One runtime as its adapter found it: values still in the shape the backend
/// stored them (raw id strings, unparsed timestamps already decoded by the adapter
/// because only it knows the encoding), with no normalization applied yet.
///
/// The `*_raw` id fields stay strings so the "present but unparseable" case — a
/// corrupted installation id — reaches the shared metadata-state decision instead
/// of being flattened to `None` by whichever adapter parsed first.
#[derive(Clone, Debug, Default)]
pub struct RawRuntimeFacts {
    pub runtime_id: String,
    pub runtime_name: Option<String>,
    pub runtime_uid: Option<String>,
    pub backend_location: Option<String>,

    pub session_id: Option<String>,
    pub managed: bool,
    pub identity: ObservedRuntimeIdentity,

    pub owner: Option<String>,
    pub repo: Option<String>,
    pub installation_id_raw: Option<String>,
    pub trigger_issue_raw: Option<String>,

    pub status: RuntimeInventoryStatus,
    /// The backend-native state string, verbatim and unbounded; bounded here.
    pub raw_status: String,
    pub status_reason: Option<String>,
    pub status_message: Option<String>,

    pub created_at: Option<DateTime<Utc>>,
    /// A creation timestamp WAS reported but could not be decoded. Distinct from
    /// `created_at: None` with this flag clear, which means none was reported.
    pub created_at_malformed: bool,
    pub last_pending_at: Option<DateTime<Utc>>,
    pub last_pending_malformed: bool,

    pub restart_count: Option<u32>,
    pub last_transition_at: Option<DateTime<Utc>>,
    pub deletion_timestamp: Option<DateTime<Utc>>,
}

/// Normalize one runtime's raw facts into an inventory item, recording every
/// data-quality problem into `warnings`.
///
/// This function NEVER returns `None` and never skips a runtime: a managed runtime
/// missing every stamp still becomes a row, because an orphan invisible to a
/// global admin is worse than an orphan with empty fields.
pub fn build_item(
    facts: RawRuntimeFacts,
    backend: RuntimeBackendKind,
    observed_at: DateTime<Utc>,
    policy: &RuntimeLifetimePolicy,
    warnings: &mut WarningSink,
) -> RuntimeInventoryItem {
    let runtime_id = facts.runtime_id;
    let session_id = facts.session_id;
    let warn = |warnings: &mut WarningSink, code| {
        warnings.push(code, Some(runtime_id.as_str()), session_id.as_deref());
    };

    if session_id.is_none() {
        warn(warnings, InventoryWarningCode::MissingSessionId);
    }
    if facts.identity.malformed {
        warn(warnings, InventoryWarningCode::MalformedIdentity);
    }
    if facts.created_at_malformed {
        warn(warnings, InventoryWarningCode::MalformedCreatedAt);
    }
    if facts.last_pending_malformed {
        warn(warnings, InventoryWarningCode::MalformedLastPending);
    }
    if facts.status == RuntimeInventoryStatus::Unknown {
        warn(warnings, InventoryWarningCode::UnknownStatus);
    }

    let (installation_id, installation_malformed) = parse_id(facts.installation_id_raw.as_deref());
    let (trigger_issue, trigger_malformed) = parse_id(facts.trigger_issue_raw.as_deref());
    if installation_malformed || trigger_malformed {
        warn(warnings, InventoryWarningCode::MalformedCorrelation);
    }

    let (timing, timing_codes) =
        timing::compute(observed_at, facts.created_at, facts.last_pending_at, policy);
    for code in timing_codes {
        warn(warnings, code);
    }

    // Zero is the reconciler's "unknown trigger" sentinel everywhere else, so it
    // is neither reported as issue #0 nor counted as a known correlation.
    let trigger_issue = trigger_issue.filter(|issue| *issue != 0);

    let malformed = facts.identity.malformed
        || facts.created_at_malformed
        || facts.last_pending_malformed
        || installation_malformed
        || trigger_malformed;
    let metadata_state = metadata_state(
        malformed,
        session_id.is_some(),
        facts.owner.is_some() && facts.repo.is_some(),
        installation_id.is_some(),
        trigger_issue.is_some(),
        facts.created_at.is_some(),
        &facts.identity,
    );

    RuntimeInventoryItem {
        backend,
        runtime_name: facts.runtime_name,
        runtime_uid: facts.runtime_uid,
        backend_location: facts.backend_location,

        managed: facts.managed,
        metadata_state,

        creator_id: facts.identity.creator_id,
        creator_login: facts.identity.creator_login.clone(),
        trigger_author_id: facts.identity.trigger_author_id,
        trigger_author_login: facts.identity.trigger_author_login.clone(),
        attribution_source: facts.identity.attribution_source(),

        repo_full_name: match (facts.owner.as_deref(), facts.repo.as_deref()) {
            (Some(owner), Some(repo)) => Some(format!("{owner}/{repo}")),
            _ => None,
        },
        installation_id,
        trigger_issue,

        status: facts.status,
        raw_status: bounded_operational_text(Some(&facts.raw_status), MAX_RAW_STATUS_BYTES)
            .unwrap_or_default(),
        status_reason: bounded_operational_text(
            facts.status_reason.as_deref(),
            MAX_STATUS_REASON_BYTES,
        ),
        status_message: bounded_operational_text(
            facts.status_message.as_deref(),
            MAX_STATUS_MESSAGE_BYTES,
        ),

        created_at: facts.created_at,
        age_seconds: timing.age_seconds,
        max_lifetime_seconds: timing.max_lifetime_seconds,
        expires_at: timing.expires_at,
        remaining_seconds: timing.remaining_seconds,
        minimum_lifetime_seconds: timing.minimum_lifetime_seconds,
        minimum_lifetime_remaining_seconds: timing.minimum_lifetime_remaining_seconds,
        idle_grace_seconds: timing.idle_grace_seconds,
        last_pending_at: facts.last_pending_at,
        idle_for_seconds: timing.idle_for_seconds,

        restart_count: facts.restart_count,
        last_transition_at: facts.last_transition_at,
        deletion_timestamp: facts.deletion_timestamp,

        runtime_id,
        session_id,
    }
}

/// Parse a stamped decimal id, reporting whether a PRESENT value failed to parse.
///
/// Returns `(None, false)` for an absent value and `(None, true)` for a corrupted
/// one — the distinction the metadata state is built on.
fn parse_id(raw: Option<&str>) -> (Option<i64>, bool) {
    match raw {
        None => (None, false),
        Some(value) => match value.trim().parse::<i64>() {
            Ok(parsed) => (Some(parsed), false),
            Err(_) => (None, true),
        },
    }
}

/// Decide the runtime's metadata state.
///
/// `Malformed` outranks everything: a corrupted value is a stronger signal than a
/// missing one and must not be masked by other fields being present. `Complete`
/// requires the full correlation set AND an attribution stamp written by a control
/// plane that knew the contract — a runtime whose attribution is
/// `partial_metadata` or `unknown_legacy` is, by definition, not complete.
fn metadata_state(
    malformed: bool,
    has_session_id: bool,
    has_repo: bool,
    has_installation: bool,
    has_trigger_issue: bool,
    has_created_at: bool,
    identity: &ObservedRuntimeIdentity,
) -> RuntimeMetadataState {
    if malformed {
        return RuntimeMetadataState::Malformed;
    }
    let attribution_complete = matches!(
        identity.attribution_source(),
        crate::runtime_identity::AttributionSource::LaunchMetadata
            | crate::runtime_identity::AttributionSource::BackfilledCurrentTrigger
    );
    let complete = has_session_id
        && has_repo
        && has_installation
        && has_trigger_issue
        && has_created_at
        && attribution_complete;
    if complete {
        RuntimeMetadataState::Complete
    } else {
        RuntimeMetadataState::Partial
    }
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
