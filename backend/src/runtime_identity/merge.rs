//! The pure backfill decision: fill what is missing, never rewrite what is not.
//!
//! This is the whole immutability guarantee expressed as one total function. It
//! is deliberately backend-free — it sees a metadata map, a key set, and the
//! current registration's identity — so the Kubernetes and OpenSandbox patch
//! paths cannot reach different conclusions about the same runtime.
//!
//! ## Why a conflict blocks the entire patch
//!
//! A runtime whose stamped creator disagrees with the current trigger is telling
//! us one of two things: the trigger was re-assigned after launch, or the
//! metadata was tampered with. Neither is resolved by writing the OTHER keys:
//! that would produce a runtime whose attribution is half from launch and half
//! from now, with nothing recording which half is which. So a conflict yields
//! [`IdentityPlan::Conflict`], writes no ATTRIBUTION value, and is surfaced as
//! [`AttributionSource::Conflict`](super::AttributionSource::Conflict).
//!
//! ## Why a conflict nevertheless writes one key
//!
//! "Writes nothing at all" would make the disagreement live and die inside the
//! process that noticed it. The operations inventory reads runtimes ALONE — it
//! holds no registration to compare against — so it could only ever report the
//! stamp at face value, and a global admin would see `launch_metadata` on
//! precisely the runtime whose attribution is disputed. The conflict therefore
//! writes ONE marker key ([`IdentityKeys::conflict`]), holding the neutral name
//! of the field that disagreed and never either value. It is additive (no
//! attribution value is touched), written once (an existing marker is left
//! exactly as it is, so a repeating sweep cannot re-patch), and STICKY: an
//! observed disagreement is a fact about that runtime incarnation, and an audit
//! trail does not un-record one because the trigger was later edited back.
//!
//! ## Why a missing desired value never conflicts
//!
//! An assignee-derived creator has no id. When the runtime carries an id and the
//! registration does not, the registration is simply making no claim — it is not
//! asserting a different value, so there is nothing to disagree with. The stamp
//! stands and the id is never borrowed from the trigger author.
//!
//! ## Why a backfill also writes a provenance marker
//!
//! A backfill writes byte-for-byte what a launch stamp writes, so without a
//! durable marker the patched runtime would later read back as
//! [`AttributionSource::LaunchMetadata`](super::AttributionSource::LaunchMetadata)
//! — a claim about who launched it that this code cannot make. The marker is
//! therefore added whenever a patch fills anything AND the runtime does not
//! already carry one, and it is deliberately conservative: it says "at least
//! part of this stamp came from the trigger as it read later", never the
//! reverse.

use std::collections::BTreeMap;

use super::keys::{
    IdentityField, IdentityKeys, IDENTITY_SCHEMA_VERSION, SOURCE_BACKFILLED_CURRENT_TRIGGER,
};
use super::{ObservedRuntimeIdentity, RuntimeIdentityMetadata};

/// What a backfill attempt should do to one runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityPlan {
    /// Every key the registration can supply is already present and agrees.
    Complete,
    /// Absent keys that may be added. Never contains a key already present.
    Backfill(Vec<(&'static str, String)>),
    /// A present value disagrees with the registration.
    Conflict {
        /// The offending KEY NAME — a bounded, non-secret string safe for a log
        /// line, and never the two values themselves.
        key: &'static str,
        /// The durable conflict marker to write, or `None` when the runtime
        /// already carries one. Writing it is what lets a later reader holding
        /// only the runtime report the disagreement (see the module docs); it
        /// is the ONLY key a conflict ever writes.
        marker: Option<(&'static str, String)>,
    },
}

/// Decide what, if anything, to write onto a runtime whose metadata is
/// `observed` given the current registration's `desired` identity.
pub fn plan(
    keys: &IdentityKeys,
    observed: &BTreeMap<String, String>,
    desired: &RuntimeIdentityMetadata,
) -> IdentityPlan {
    // Every stampable key with the value the registration would write, in the
    // same order `stamp_pairs` uses, each paired with its backend-neutral field
    // name (the conflict marker's value). `None` means "the registration makes no
    // claim about this key" — which can never conflict and can never backfill.
    let claims: [(&'static str, IdentityField, Option<String>, Comparison); 5] = [
        (
            keys.schema,
            IdentityField::Schema,
            Some(IDENTITY_SCHEMA_VERSION.to_string()),
            // A schema version stamped by a different (future) contract version
            // is not an attribution disagreement: it says the runtime was
            // written by another release, which the reader already tolerates.
            Comparison::Advisory,
        ),
        (
            keys.creator_id,
            IdentityField::CreatorId,
            desired.creator_id.map(|id| id.to_string()),
            Comparison::Exact,
        ),
        (
            keys.creator_login,
            IdentityField::CreatorLogin,
            non_empty(&desired.creator_login),
            Comparison::CaseInsensitive,
        ),
        (
            keys.trigger_author_id,
            IdentityField::TriggerAuthorId,
            Some(desired.trigger_author_id.to_string()),
            Comparison::Exact,
        ),
        (
            keys.trigger_author_login,
            IdentityField::TriggerAuthorLogin,
            non_empty(&desired.trigger_author_login),
            Comparison::CaseInsensitive,
        ),
    ];

    let mut backfill = Vec::new();
    for (key, field, claim, comparison) in claims {
        match (observed.get(key), claim) {
            // Present on both sides: the only place a conflict can arise.
            (Some(existing), Some(claimed)) => {
                if comparison.disagrees(existing, &claimed) {
                    return IdentityPlan::Conflict {
                        key,
                        // Recorded once and never rewritten: the FIRST observed
                        // disagreement is the one worth keeping, and re-patching
                        // every sweep would be an unbounded write loop.
                        marker: (!observed.contains_key(keys.conflict))
                            .then(|| (keys.conflict, field.as_str().to_string())),
                    };
                }
            }
            // Absent on the runtime and claimed by the registration: fill it.
            (None, Some(claimed)) => backfill.push((key, claimed)),
            // The registration claims nothing: leave the runtime exactly as it
            // is, whether or not it carries a value.
            (_, None) => {}
        }
    }

    if backfill.is_empty() {
        return IdentityPlan::Complete;
    }
    // Provenance, recorded exactly once. An existing marker is never rewritten:
    // a runtime stamped at launch that later gains one absent key keeps saying
    // `launch_metadata`, which is where its stamp originated.
    if !observed.contains_key(keys.source) {
        backfill.push((keys.source, SOURCE_BACKFILLED_CURRENT_TRIGGER.to_string()));
    }
    IdentityPlan::Backfill(backfill)
}

/// Whether an ALREADY-READ stamp states everything the registration can, so no
/// backend call is worth making.
///
/// The authoritative decision is still [`plan`] against the runtime's current
/// metadata — this is only the reconcile hot path's fast exit, working from the
/// stamp the pass already read during its own runtime observation. It is
/// deliberately conservative: any disagreement, malformation, or gap answers
/// `false`, which costs one backend call that then decides properly.
pub fn is_settled(observed: &ObservedRuntimeIdentity, desired: &RuntimeIdentityMetadata) -> bool {
    if observed.malformed || observed.schema_version.is_none() {
        return false;
    }
    // `creator_id` is compared as an OPTION on purpose: `None` on both sides is
    // the settled assignee-derived state, while a registration that gained an id
    // the runtime lacks is a genuine backfill opportunity.
    if observed.creator_id != desired.creator_id {
        return false;
    }
    matches(&observed.creator_login, &desired.creator_login)
        && observed.trigger_author_id == Some(desired.trigger_author_id)
        && matches(
            &observed.trigger_author_login,
            &desired.trigger_author_login,
        )
}

/// A stamped login agrees with the registration's, treating "the registration
/// has nothing to say" as agreement (it cannot conflict with what it does not
/// claim).
fn matches(observed: &Option<String>, desired: &str) -> bool {
    match (observed, desired.trim().is_empty()) {
        (_, true) => true,
        (Some(observed), false) => observed.trim().eq_ignore_ascii_case(desired),
        (None, false) => false,
    }
}

/// How a stamped value is compared with the registration's value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Comparison {
    /// Byte equality (numeric ids).
    Exact,
    /// ASCII-case-insensitive (GitHub logins are case-insensitive, and a stamp
    /// written before login normalization existed may differ only in case).
    CaseInsensitive,
    /// A difference is tolerated and never reported (the schema version).
    Advisory,
}

impl Comparison {
    fn disagrees(self, existing: &str, claimed: &str) -> bool {
        match self {
            Comparison::Exact => existing.trim() != claimed,
            Comparison::CaseInsensitive => !existing.trim().eq_ignore_ascii_case(claimed),
            Comparison::Advisory => false,
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
