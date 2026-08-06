//! Fail-closed configuration validation.
//!
//! These cases mirror `tools/evolution/test/config.test.ts` one for one. Two
//! implementations of one schema is a real divergence risk, so a rule that is
//! tested there and not here is a rule that will drift.

use super::*;

const BASE: &str = r#"
schemaVersion: 1
enabled: true
source:
  branch: "@default"
  productRelevant:
    include: ["backend/src/**", "frontend/src/**"]
    exclude: ["**/*_tests.rs"]
  coverage:
    include: ["**"]
    exclude: [".git/**"]
artifactRepository: "."
intent:
  product: ".fkst/evolution/intent/product.md"
  overrides: ".fkst/evolution/intent/overrides.yaml"
managedOutputs:
  documentation: { enabled: true }
  skills: { enabled: true }
  journeys: { enabled: true }
  screenshots: { enabled: true }
  slides: { enabled: true }
  video: { enabled: true, storage: "github-release" }
absentProducerRoles: []
locales: ["en"]
triggers:
  defaultBranchPush: true
publication:
  mode: "propose"
  requireCurrentSource: true
  requireChecks: true
  allowDirectPush: false
drift:
  policy: "block"
generatorEpoch: 1
retention:
  renderedSnapshots: 10
security:
  runPullRequestCode: false
  allowProductionData: false
  allowProductionCredentials: false
"#;

fn with(from: &str, to: &str) -> String {
    assert!(BASE.contains(from), "fixture must contain {from:?}");
    BASE.replace(from, to)
}

fn expect_err(yaml: &str) -> String {
    parse_config(yaml).expect_err("must fail closed").0
}

#[test]
fn the_baseline_config_parses() {
    let config = parse_config(BASE).expect("valid");
    assert!(config.branch.is_dynamic());
    assert_eq!(config.publication.mode, "propose");
    assert_eq!(config.generator_epoch, 1);
    assert_eq!(config.artifact_repository, ".");
    assert!(config.enabled);
}

#[test]
fn an_unsupported_schema_version_fails_closed() {
    let error = expect_err(&with("schemaVersion: 1", "schemaVersion: 2"));
    assert!(error.contains("unsupported schemaVersion"), "{error}");
}

#[test]
fn an_absent_product_relevant_set_fails_closed() {
    // No default exists. An absent set silently disables ALL cycle admission,
    // and a repository that never regenerates produces no signal.
    let yaml = BASE.replace(
        "  productRelevant:\n    include: [\"backend/src/**\", \"frontend/src/**\"]\n    exclude: [\"**/*_tests.rs\"]\n",
        "",
    );
    let error = expect_err(&yaml);
    assert!(error.contains("productRelevant is required"), "{error}");
}

#[test]
fn an_empty_product_relevant_include_fails_closed() {
    let error = expect_err(&with(
        r#"include: ["backend/src/**", "frontend/src/**"]"#,
        "include: []",
    ));
    assert!(error.contains("must not be empty"), "{error}");
}

#[test]
fn explicitly_naming_a_reserved_prefix_fails_closed() {
    for pattern in [".fkst/evolution/docs/**", "**/.fkst/packages/**"] {
        let error = expect_err(&with(
            r#"include: ["backend/src/**", "frontend/src/**"]"#,
            &format!(r#"include: ["backend/src/**", "{pattern}"]"#),
        ));
        assert!(
            error.contains("may not explicitly name"),
            "{pattern}: {error}"
        );
    }
}

#[test]
fn a_broad_wildcard_is_permitted_and_narrowed_later() {
    // The line is EXPLICIT re-inclusion; `**` is fine and is narrowed by the
    // unconditional removals.
    let config = parse_config(&with(
        r#"include: ["backend/src/**", "frontend/src/**"]"#,
        r#"include: ["**"]"#,
    ))
    .expect("valid");
    assert_eq!(config.product_relevant.include, vec!["**".to_string()]);
}

#[test]
fn a_managed_output_carrying_a_destination_fails_closed() {
    // Destinations are fixed by schema: configuration may enable or disable a
    // class, never relocate it.
    for extra in [
        "path: \"docs/custom\"",
        "directory: \"x\"",
        "destination: \"y\"",
    ] {
        let error = expect_err(&with(
            "documentation: { enabled: true }",
            &format!("documentation: {{ enabled: true, {extra} }}"),
        ));
        assert!(error.contains("unknown field"), "{extra}: {error}");
    }
}

#[test]
fn a_non_github_native_video_storage_fails_closed() {
    let error = expect_err(&with(
        r#"video: { enabled: true, storage: "github-release" }"#,
        r#"video: { enabled: true, storage: "s3" }"#,
    ));
    assert!(error.contains("github-release"), "{error}");
}

#[test]
fn a_disabled_video_class_needs_no_storage() {
    let config = parse_config(&with(
        r#"video: { enabled: true, storage: "github-release" }"#,
        "video: { enabled: false }",
    ))
    .expect("valid");
    assert!(!config.managed_outputs.video.enabled);
}

#[test]
fn requesting_direct_push_fails_closed() {
    let error = expect_err(&with("allowDirectPush: false", "allowDirectPush: true"));
    assert!(error.contains("allowDirectPush"), "{error}");
}

#[test]
fn a_merge_policy_that_cannot_honor_required_checks_fails_closed() {
    let error = expect_err(&with("requireChecks: true", "requireChecks: false"));
    assert!(error.contains("requireChecks"), "{error}");
}

#[test]
fn requesting_privileged_execution_of_an_untrusted_head_fails_closed() {
    let error = expect_err(&with(
        "runPullRequestCode: false",
        "runPullRequestCode: true",
    ));
    assert!(error.contains("runPullRequestCode"), "{error}");
}

#[test]
fn permitting_production_data_or_credentials_fails_closed() {
    for field in ["allowProductionData", "allowProductionCredentials"] {
        let error = expect_err(&with(&format!("{field}: false"), &format!("{field}: true")));
        assert!(error.contains("allowProduction"), "{field}: {error}");
    }
}

#[test]
fn an_unknown_top_level_field_is_rejected_rather_than_ignored() {
    // Silent acceptance would let a misspelled safety policy appear active.
    let error = expect_err(&format!("{BASE}\nallowProdData: true\n"));
    assert!(error.contains("unknown field"), "{error}");
}

#[test]
fn an_unknown_publication_mode_fails_closed() {
    let error = expect_err(&with(r#"mode: "propose""#, r#"mode: "yolo""#));
    assert!(error.contains("publication.mode must be one of"), "{error}");
}

#[test]
fn an_unknown_drift_policy_fails_closed() {
    let error = expect_err(&with(r#"policy: "block""#, r#"policy: "ignore""#));
    assert!(error.contains("drift.policy must be one of"), "{error}");
}

#[test]
fn an_unknown_branch_sentinel_fails_closed() {
    let error = expect_err(&with(r#"branch: "@default""#, r#"branch: "@head""#));
    assert!(error.contains("unknown branch sentinel"), "{error}");
}

#[test]
fn a_literal_branch_name_is_accepted() {
    let config =
        parse_config(&with(r#"branch: "@default""#, r#"branch: "develop""#)).expect("valid");
    assert!(!config.branch.is_dynamic());
    assert_eq!(config.branch.resolve("main"), "develop");
}

#[test]
fn an_empty_artifact_repository_fails_closed() {
    let error = expect_err(&with(
        r#"artifactRepository: ".""#,
        r#"artifactRepository: """#,
    ));
    assert!(error.contains("artifactRepository"), "{error}");
}

#[test]
fn malformed_yaml_is_reported_rather_than_defaulted() {
    let error = expect_err("schemaVersion: [1,\n");
    assert!(error.contains("not valid"), "{error}");
}

// ---- required-class derivation ---------------------------------------------

#[test]
fn required_classes_drops_disabled_classes() {
    let config = parse_config(&with(
        "slides: { enabled: true }",
        "slides: { enabled: false }",
    ))
    .expect("valid");
    let classes = config.required_classes();
    assert!(!classes.contains(&"slides"), "{classes:?}");
    assert!(classes.contains(&"documentation"), "{classes:?}");
}

#[test]
fn required_classes_drops_a_class_whose_producer_role_is_absent() {
    // The verifier must not report an undeployed role's artifacts as missing.
    let config = parse_config(&with(
        "absentProducerRoles: []",
        r#"absentProducerRoles: ["artifact-renderer"]"#,
    ))
    .expect("valid");
    let classes = config.required_classes();
    assert!(!classes.contains(&"video"), "{classes:?}");
    assert!(classes.contains(&"screenshots"), "{classes:?}");
}

#[test]
fn one_absent_role_can_drop_two_classes() {
    // `demo-producer` produces both journeys and screenshots.
    let config = parse_config(&with(
        "absentProducerRoles: []",
        r#"absentProducerRoles: ["demo-producer"]"#,
    ))
    .expect("valid");
    let classes = config.required_classes();
    assert!(!classes.contains(&"journeys"), "{classes:?}");
    assert!(!classes.contains(&"screenshots"), "{classes:?}");
    assert!(classes.contains(&"documentation"), "{classes:?}");
}

#[test]
fn producer_roles_cover_every_class() {
    for class in [
        "documentation",
        "skills",
        "journeys",
        "screenshots",
        "slides",
        "video",
    ] {
        assert_ne!(producer_role(class), "unknown", "{class}");
    }
}

#[test]
fn automerge_managed_is_recognised_and_is_not_the_bootstrap_default() {
    assert!(!parse_config(BASE).expect("valid").is_automerge_managed());
    let config =
        parse_config(&with(r#"mode: "propose""#, r#"mode: "automerge-managed""#)).expect("valid");
    assert!(config.is_automerge_managed());
}

#[test]
fn locales_default_to_english_when_omitted() {
    let config = parse_config(&with(r#"locales: ["en"]"#, "locales: []")).expect("valid");
    assert_eq!(config.locales, vec!["en".to_string()]);
}
