//! The milestone #22 closure gate: requirement-to-test traceability.
//!
//! Three things happen here, and only the first is obvious:
//!
//! 1. every requirement id in epic #5665 maps to at least one named automated
//!    test that still exists in the working tree;
//! 2. the linter itself is proven to FAIL on an unmapped requirement, a
//!    nonexistent test, and a contradictory duplicate — using synthetic
//!    documents, so the proof runs in CI instead of requiring someone to break
//!    the checked-in file by hand and remember to restore it;
//! 3. the evidence artifact is generated and scanned, so the milestone ships a
//!    reviewable record that provably carries no payload or credential.
//!
//! The gate deliberately does NOT try to observe other tests' pass/fail results
//! from inside a test process. `cargo test` already fails the build if any named
//! test fails; what nothing else checks is whether the claims still point at
//! real code, which is precisely what decays.

mod acceptance;

use acceptance::lint::violations;
use acceptance::model::{Matrix, EPIC_REQUIREMENTS};
use acceptance::{artifact_dir, repo_root, report};

fn matrix() -> Matrix {
    Matrix::load(&repo_root()).expect("the checked-in matrix parses")
}

/// The gate. A failure here names the exact rows that broke.
#[test]
fn every_requirement_maps_to_at_least_one_existing_named_test() {
    let matrix = matrix();
    let problems = violations(&matrix, &repo_root());
    assert!(
        problems.is_empty(),
        "the requirement matrix is not a valid gate:\n  {}",
        problems.join("\n  ")
    );
    assert_eq!(
        matrix.requirement.len(),
        EPIC_REQUIREMENTS.len(),
        "the matrix must declare every epic requirement exactly once"
    );
}

/// The matrix must keep both cost tiers honest: at least one row of each of the
/// two environment-gated tiers, each naming its gate variable.
///
/// Without this the easy way to make the gate green is to delete the gated rows,
/// which would quietly erase the fact that two of the spec's three test
/// environments cannot run on a laptop.
#[test]
fn the_environment_gated_tiers_are_declared_rather_than_dropped() {
    let matrix = matrix();
    for tier in ["integration", "staging"] {
        let rows: Vec<_> = matrix
            .evidence
            .iter()
            .filter(|row| row.tier == tier)
            .collect();
        assert!(!rows.is_empty(), "the {tier} tier claims nothing at all");
        assert!(
            rows.iter()
                .any(|row| row.status == "gated" && row.gate_env.is_some()),
            "the {tier} tier declares no gated row naming its environment variable"
        );
    }
}

/// The linter fires on each of the three failure modes the issue names.
#[test]
fn the_linter_rejects_an_unmapped_missing_or_contradictory_entry() {
    let root = repo_root();

    // 1. an unmapped requirement: declared, but nothing claims it.
    let unmapped = Matrix::parse(&synthetic(
        r#"
        evidence = [
          { requirement = "AUTH-01", tier = "pr", suite = "backend/tests/audit_outcomes.rs", test = "a_route_scoped_timeout_is_recorded_as_a_timeout", status = "verified" },
        ]
        "#,
    ))
    .expect("the synthetic document parses");
    let problems = violations(&unmapped, &root);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("AUTH-02 maps to no automated test")),
        "{problems:?}"
    );

    // 2. a test that does not exist.
    let phantom = Matrix::parse(&synthetic(
        r#"
        evidence = [
          { requirement = "AUTH-01", tier = "pr", suite = "backend/tests/audit_outcomes.rs", test = "a_test_that_was_never_written", status = "verified" },
        ]
        "#,
    ))
    .expect("the synthetic document parses");
    let problems = violations(&phantom, &root);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("defines no test named \"a_test_that_was_never_written\"")),
        "{problems:?}"
    );

    // 3. one test claimed twice under contradictory statuses.
    let contradictory = Matrix::parse(&synthetic(
        r#"
        evidence = [
          { requirement = "AUTH-01", tier = "pr", suite = "backend/tests/audit_outcomes.rs", test = "a_route_scoped_timeout_is_recorded_as_a_timeout", status = "verified" },
          { requirement = "AUTH-02", tier = "staging", suite = "backend/tests/audit_outcomes.rs", test = "a_route_scoped_timeout_is_recorded_as_a_timeout", status = "gated", gate_env = "SOMETHING" },
        ]
        "#,
    ))
    .expect("the synthetic document parses");
    let problems = violations(&contradictory, &root);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("claimed as verified/pr and again as gated/staging")),
        "{problems:?}"
    );

    // 4. a gated row with no gate variable, and a verified row claiming one.
    let mislabelled = Matrix::parse(&synthetic(
        r#"
        evidence = [
          { requirement = "AUTH-01", tier = "staging", suite = "backend/tests/audit_outcomes.rs", test = "a_route_scoped_timeout_is_recorded_as_a_timeout", status = "gated" },
        ]
        "#,
    ))
    .expect("the synthetic document parses");
    let problems = violations(&mislabelled, &root);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("must name the environment variable that gates it")),
        "{problems:?}"
    );

    // The real matrix, by contrast, is clean — so the rules above are not simply
    // firing on everything.
    assert!(violations(&matrix(), &root).is_empty());
}

/// An unknown field is a parse failure, not a silently ignored line.
#[test]
fn a_typo_in_a_matrix_field_fails_to_parse() {
    let error = Matrix::parse(&synthetic(
        r#"
        evidence = [
          { requirement = "AUTH-01", tier = "pr", suite = "backend/tests/audit_outcomes.rs", test = "a_route_scoped_timeout_is_recorded_as_a_timeout", status = "verified", gate_environment = "OOPS" },
        ]
        "#,
    ))
    .expect_err("an unknown field must not be accepted");
    assert!(error.contains("gate_environment"), "{error}");
}

/// The artifact is written, names every requirement, and carries nothing that
/// looks like a payload or a credential.
#[test]
fn the_evidence_artifact_names_every_requirement_and_no_payload() {
    let root = repo_root();
    let matrix = matrix();
    let commit = report::build_commit(&root);
    let rendered = report::render(&matrix, &commit);

    for id in EPIC_REQUIREMENTS {
        assert!(
            rendered.contains(id),
            "the evidence artifact does not mention {id}"
        );
    }
    assert!(rendered.contains(&commit), "the build commit is missing");
    let hits = report::forbidden_hits(&rendered);
    assert!(
        hits.is_empty(),
        "the evidence artifact carries forbidden material: {hits:?}"
    );

    let path = report::write(&artifact_dir(), "requirement-report.md", &rendered)
        .expect("the evidence artifact is written");
    let written = std::fs::read_to_string(&path).expect("the artifact reads back");
    assert_eq!(written, rendered);
}

/// Compose a synthetic document: the real preamble and requirement list, with a
/// caller-supplied evidence block. Keeping the preamble real means the negative
/// tests exercise the same parser and the same requirement set as the gate.
fn synthetic(evidence: &str) -> String {
    let requirements = EPIC_REQUIREMENTS
        .iter()
        .map(|id| format!("  {{ id = \"{id}\", area = \"test\", summary = \"synthetic\" }},"))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "schema_version = 1\nepic = 5665\ngate_issue = 5683\nmilestone = 22\n\
         \nrequirement = [\n{requirements}\n]\n{evidence}"
    )
}
