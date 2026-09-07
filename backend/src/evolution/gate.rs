//! The merge gate: an Evolution-owned check run, not a pre-merge callback.
//!
//! **Why a check run at all.** Autonomous merge must compare the recomputed
//! input fingerprint against the manifest *immediately before merging*, and no
//! native GitHub mechanism can do that. Native auto-merge has no pre-merge
//! callback: once armed, GitHub merges when required checks turn green,
//! regardless of what happened to the base branch in the interim. So the gate is
//! expressed as an artifact GitHub already honours — a required check run the
//! control plane owns and re-evaluates on every reconcile.
//!
//! Base-branch advancement does not change a pull request's head, so nothing in
//! GitHub re-evaluates the gate on its own. The level-triggered reconcile is what
//! flips it, by updating the same check run on the unchanged head.
//!
//! **Why `neutral` on non-sync pull requests is not optional.** A ruleset's
//! required-status-checks rule conditions on the protected REF. It has no
//! head-branch, author or App condition. Once this check is required on a branch,
//! *every* pull request targeting that branch must report it — and with the
//! default `artifactRepository: "."` that branch is the product repository's own
//! default branch, the base of every ordinary human pull request. If the control
//! plane published only on the sync pull request, every human pull request would
//! sit forever at "Expected — waiting for status to be reported" and could not be
//! merged by anyone. Enrolling Evolution would halt development in the repository
//! Evolution exists to document, before it produced a single artifact.
//!
//! `neutral` is the truthful conclusion there: GitHub treats it as not-failing
//! for required-check purposes, and it says exactly what is the case — Evolution
//! asserts nothing about this pull request.

use super::boundary::confinement_violations;

/// The check-run name the artifact repository's ruleset requires.
pub const GATE_CHECK_NAME: &str = "fkst-evolution/input-current";

/// A GitHub check-run conclusion, narrowed to the three this gate publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateConclusion {
    /// The sync pull request is current, confined, verified and complete.
    Success,
    /// The sync pull request must not merge.
    Failure,
    /// Not the canonical sync pull request — Evolution asserts nothing.
    Neutral,
}

impl GateConclusion {
    /// The value sent to GitHub's check-runs API.
    pub fn as_str(self) -> &'static str {
        match self {
            GateConclusion::Success => "success",
            GateConclusion::Failure => "failure",
            GateConclusion::Neutral => "neutral",
        }
    }

    /// Whether this conclusion permits the pull request to merge.
    pub fn permits_merge(self) -> bool {
        // `neutral` permits merging because it is not the sync PR's gate; the
        // gate never blocks a pull request it makes no claim about.
        matches!(self, GateConclusion::Success | GateConclusion::Neutral)
    }
}

/// Everything the gate decision reads. All of it is re-derived per reconcile;
/// none of it is a status field the generator wrote.
#[derive(Debug, Clone)]
pub struct GateInputs<'a> {
    /// Whether this pull request is the lane's canonical sync pull request.
    pub is_canonical_sync_pr: bool,
    /// Whether the pull request is authored by the configured FKST App identity.
    pub authored_by_app: bool,
    /// Every path the pull request changes.
    pub changed_paths: &'a [String],
    /// The input fingerprint recorded in the manifest on the sync branch.
    pub manifest_input_fingerprint: &'a str,
    /// The input fingerprint recomputed from the trusted tree, now.
    pub recomputed_input_fingerprint: &'a str,
    /// Whether every required verification is corroborated by a check run.
    pub verification_corroborated: bool,
    /// Whether every required Release asset exists with a matching hash.
    pub required_assets_present: bool,
}

/// The published gate decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateDecision {
    pub conclusion: GateConclusion,
    /// One-line check-run title.
    pub title: String,
    /// Check-run summary. Records the input fingerprint so a later reader can
    /// tell which product state the gate was asserting about.
    pub summary: String,
}

fn neutral() -> GateDecision {
    GateDecision {
        conclusion: GateConclusion::Neutral,
        title: "Not an Evolution sync pull request".to_string(),
        summary: "FKST Evolution asserts nothing about this pull request. This check is \
                  published so that requiring it on the protected branch does not block \
                  pull requests Evolution does not own."
            .to_string(),
    }
}

fn failure(title: &str, summary: String) -> GateDecision {
    GateDecision {
        conclusion: GateConclusion::Failure,
        title: title.to_string(),
        summary,
    }
}

/// Evaluate the gate for one pull request.
///
/// Checks are ordered so the reported reason is the most actionable one: an
/// unconfined path set is a different problem from a stale fingerprint, and
/// reporting whichever happened to be tested first would send the reader after
/// the wrong thing.
pub fn evaluate(inputs: &GateInputs<'_>) -> GateDecision {
    if !inputs.is_canonical_sync_pr {
        return neutral();
    }

    if !inputs.authored_by_app {
        return failure(
            "Sync pull request is not App-authored",
            "The canonical sync pull request must be authored by the configured FKST App \
             identity. A pull request on the lane's branch authored by anyone else is not \
             Evolution output and must not merge under autonomous policy."
                .to_string(),
        );
    }

    let violations = confinement_violations(inputs.changed_paths.iter().map(String::as_str));
    if !violations.is_empty() {
        return failure(
            "Changes escape the Evolution write boundary",
            format!(
                "Evolution may write only under `.fkst/evolution/`, and never `config.yaml` \
                 or `intent/**`. Offending path(s): {}",
                violations.join(", ")
            ),
        );
    }

    if inputs.recomputed_input_fingerprint != inputs.manifest_input_fingerprint {
        // The reason the gate exists. The base branch moved under an open sync
        // pull request, so its artifacts describe product state that is no longer
        // current — and GitHub re-evaluates nothing on its own, because the PR
        // head did not change.
        return failure(
            "Source has advanced since these artifacts were generated",
            format!(
                "The recomputed input fingerprint no longer matches the manifest.\n\n\
                 manifest:   {}\n\
                 recomputed: {}\n\n\
                 Evolution will regenerate against the current trusted head.",
                inputs.manifest_input_fingerprint, inputs.recomputed_input_fingerprint
            ),
        );
    }

    if !inputs.verification_corroborated {
        return failure(
            "Journey verification is not corroborated",
            "A verification result is corroborated by re-fetching the check run that \
             recorded it and confirming its actor, freshness and conclusion. A status \
             string in the manifest is not evidence."
                .to_string(),
        );
    }

    if !inputs.required_assets_present {
        return failure(
            "Required Release assets are missing or do not match",
            "Every required artifact must exist at its recorded path or as its referenced \
             Release asset, and re-hashing those bytes must reproduce the recorded content \
             hash."
                .to_string(),
        );
    }

    GateDecision {
        conclusion: GateConclusion::Success,
        title: "Artifacts describe the current trusted source".to_string(),
        summary: format!(
            "Input fingerprint {} matches the manifest, every changed path is within the \
             Evolution write boundary, verification is corroborated, and required assets \
             are present.",
            inputs.manifest_input_fingerprint
        ),
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
