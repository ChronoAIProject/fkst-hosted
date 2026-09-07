//! The merge-gate decision.
//!
//! The most important case in this file is the `neutral` one: without it,
//! requiring the check on a branch makes every human pull request on that branch
//! permanently unmergeable.

use super::*;

const INPUT: &str = "sha256:c0ceeffb6f8cb4b312745f79be2ce6a8616f7c09700237fb8a1d9c8b012a5fb8";
const OTHER: &str = "sha256:9999999999999999999999999999999999999999999999999999999999999999";

fn confined() -> Vec<String> {
    vec![
        ".fkst/evolution/docs/queue-work-item.md".to_string(),
        ".fkst/evolution/manifest.json".to_string(),
    ]
}

fn healthy(paths: &[String]) -> GateInputs<'_> {
    GateInputs {
        is_canonical_sync_pr: true,
        authored_by_app: true,
        changed_paths: paths,
        manifest_input_fingerprint: INPUT,
        recomputed_input_fingerprint: INPUT,
        verification_corroborated: true,
        required_assets_present: true,
    }
}

#[test]
fn a_current_confined_verified_sync_pr_passes() {
    let paths = confined();
    let decision = evaluate(&healthy(&paths));
    assert_eq!(decision.conclusion, GateConclusion::Success);
    assert!(decision.summary.contains(INPUT), "{}", decision.summary);
    assert!(decision.conclusion.permits_merge());
}

#[test]
fn a_non_sync_pull_request_is_neutral_and_mergeable() {
    // THE case that keeps a repository usable. A ruleset requires this check on
    // the protected REF — it cannot condition on head branch, author or App — so
    // every human pull request targeting that branch must report it. Publishing
    // nothing leaves them all at "Expected — waiting for status to be reported",
    // unmergeable by anyone, in the repository Evolution exists to document.
    let paths = vec!["backend/src/main.rs".to_string()];
    let decision = evaluate(&GateInputs {
        is_canonical_sync_pr: false,
        authored_by_app: false,
        ..healthy(&paths)
    });
    assert_eq!(decision.conclusion, GateConclusion::Neutral);
    assert!(
        decision.conclusion.permits_merge(),
        "a human pull request must remain mergeable"
    );
}

#[test]
fn a_non_sync_pull_request_is_neutral_even_when_it_would_otherwise_fail() {
    // Neutral is decided FIRST and unconditionally. A human pull request touching
    // product source, against a moved fingerprint, must not be reported as a
    // failure — Evolution asserts nothing about it.
    let paths = vec!["backend/src/main.rs".to_string()];
    let decision = evaluate(&GateInputs {
        is_canonical_sync_pr: false,
        authored_by_app: false,
        recomputed_input_fingerprint: OTHER,
        verification_corroborated: false,
        required_assets_present: false,
        ..healthy(&paths)
    });
    assert_eq!(decision.conclusion, GateConclusion::Neutral);
}

#[test]
fn a_stale_sync_pr_fails_and_names_both_fingerprints() {
    let paths = confined();
    let decision = evaluate(&GateInputs {
        recomputed_input_fingerprint: OTHER,
        ..healthy(&paths)
    });
    assert_eq!(decision.conclusion, GateConclusion::Failure);
    assert!(!decision.conclusion.permits_merge());
    assert!(decision.summary.contains(INPUT), "{}", decision.summary);
    assert!(decision.summary.contains(OTHER), "{}", decision.summary);
}

#[test]
fn an_unconfined_path_fails_and_names_the_offender() {
    let paths = vec![
        ".fkst/evolution/docs/a.md".to_string(),
        "backend/src/main.rs".to_string(),
    ];
    let decision = evaluate(&healthy(&paths));
    assert_eq!(decision.conclusion, GateConclusion::Failure);
    assert!(
        decision.summary.contains("backend/src/main.rs"),
        "{}",
        decision.summary
    );
}

#[test]
fn writing_owner_intent_in_a_sync_pr_fails() {
    // The intent-proposal pull request is a SEPARATE pull request carrying no
    // sync marker. Intent reached through the sync lane is always a violation.
    for path in [
        ".fkst/evolution/config.yaml",
        ".fkst/evolution/intent/product.md",
    ] {
        let paths = vec![path.to_string()];
        let decision = evaluate(&healthy(&paths));
        assert_eq!(decision.conclusion, GateConclusion::Failure, "{path}");
        assert!(decision.summary.contains(path), "{path}");
    }
}

#[test]
fn a_sync_pr_not_authored_by_the_app_fails() {
    let paths = confined();
    let decision = evaluate(&GateInputs {
        authored_by_app: false,
        ..healthy(&paths)
    });
    assert_eq!(decision.conclusion, GateConclusion::Failure);
    assert!(
        decision.title.contains("App-authored"),
        "{}",
        decision.title
    );
}

#[test]
fn uncorroborated_verification_fails() {
    let paths = confined();
    let decision = evaluate(&GateInputs {
        verification_corroborated: false,
        ..healthy(&paths)
    });
    assert_eq!(decision.conclusion, GateConclusion::Failure);
    assert!(
        decision.summary.contains("not evidence"),
        "{}",
        decision.summary
    );
}

#[test]
fn missing_release_assets_fail() {
    let paths = confined();
    let decision = evaluate(&GateInputs {
        required_assets_present: false,
        ..healthy(&paths)
    });
    assert_eq!(decision.conclusion, GateConclusion::Failure);
    assert!(
        decision.title.contains("Release assets"),
        "{}",
        decision.title
    );
}

#[test]
fn confinement_is_reported_before_staleness() {
    // Both are wrong; the path violation is the more actionable report, and a
    // stale-fingerprint message would send the reader after the wrong problem.
    let paths = vec!["backend/src/main.rs".to_string()];
    let decision = evaluate(&GateInputs {
        recomputed_input_fingerprint: OTHER,
        ..healthy(&paths)
    });
    assert!(
        decision.title.contains("write boundary"),
        "{}",
        decision.title
    );
}

#[test]
fn authorship_is_reported_before_everything_else() {
    let paths = vec!["backend/src/main.rs".to_string()];
    let decision = evaluate(&GateInputs {
        authored_by_app: false,
        recomputed_input_fingerprint: OTHER,
        ..healthy(&paths)
    });
    assert!(
        decision.title.contains("App-authored"),
        "{}",
        decision.title
    );
}

#[test]
fn an_empty_change_set_is_confined() {
    // A sync pull request with no changes is degenerate but not a boundary
    // violation; convergence handles it, not the confinement check.
    let paths: Vec<String> = Vec::new();
    assert_eq!(
        evaluate(&healthy(&paths)).conclusion,
        GateConclusion::Success
    );
}

#[test]
fn the_wire_values_are_the_github_conclusions() {
    assert_eq!(GateConclusion::Success.as_str(), "success");
    assert_eq!(GateConclusion::Failure.as_str(), "failure");
    assert_eq!(GateConclusion::Neutral.as_str(), "neutral");
}

#[test]
fn only_failure_blocks_a_merge() {
    assert!(GateConclusion::Success.permits_merge());
    assert!(GateConclusion::Neutral.permits_merge());
    assert!(!GateConclusion::Failure.permits_merge());
}

#[test]
fn the_check_name_is_the_one_the_ruleset_requires() {
    // Changing this string silently disarms the gate: the ruleset would require a
    // check nobody publishes, and every pull request on the branch would hang.
    assert_eq!(GATE_CHECK_NAME, "fkst-evolution/input-current");
}
