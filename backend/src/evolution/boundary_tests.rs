//! The write boundary. Every case here is a containment case.

use super::*;

#[test]
fn managed_output_paths_are_writable() {
    for path in [
        ".fkst/evolution/docs/page.md",
        ".fkst/evolution/skills/x/SKILL.md",
        ".fkst/evolution/journeys/j.spec.ts",
        ".fkst/evolution/screenshots/a.png",
        ".fkst/evolution/slides/deck.md",
        ".fkst/evolution/observed/capabilities.yaml",
        ".fkst/evolution/changes/2026/ab/abc.yaml",
        ".fkst/evolution/manifest.json",
    ] {
        assert!(is_writable_by_evolution(path), "{path}");
    }
}

#[test]
fn owner_owned_paths_are_never_writable() {
    // Human intent is not an autonomous managed output. Evolution may PROPOSE a
    // change to these, but only in a separate pull request that carries no sync
    // marker and that autonomous policy never merges.
    assert!(!is_writable_by_evolution(".fkst/evolution/config.yaml"));
    assert!(!is_writable_by_evolution(
        ".fkst/evolution/intent/product.md"
    ));
    assert!(!is_writable_by_evolution(
        ".fkst/evolution/intent/overrides.yaml"
    ));
}

#[test]
fn everything_outside_the_root_is_refused() {
    for path in [
        "backend/src/main.rs",
        "README.md",
        ".github/workflows/rust-ci.yml",
        ".fkst/packages/catalog.toml",
        ".fkst/evolutionary/x.md",
        "fkst/evolution/x.md",
        ".fkst/evolution",
    ] {
        assert!(!is_writable_by_evolution(path), "{path}");
    }
}

#[test]
fn a_traversal_segment_is_refused_even_under_the_root() {
    // Passes a naive prefix test while naming a file outside the root.
    assert!(!is_writable_by_evolution(
        ".fkst/evolution/../../backend/src/main.rs"
    ));
    assert!(!is_writable_by_evolution(".fkst/evolution/docs/../../../x"));
}

#[test]
fn a_path_merely_containing_the_root_is_refused() {
    // The comparison is a PREFIX, not a substring: an attacker-chosen path that
    // mentions the root elsewhere must not pass.
    assert!(!is_writable_by_evolution("src/.fkst/evolution/docs/a.md"));
}

#[test]
fn confinement_violations_name_every_offending_path_in_order() {
    let changed = vec![
        ".fkst/evolution/docs/ok.md",
        "backend/src/main.rs",
        ".fkst/evolution/config.yaml",
        ".fkst/evolution/screenshots/ok.png",
        ".fkst/evolution/intent/product.md",
    ];
    assert_eq!(
        confinement_violations(changed),
        vec![
            "backend/src/main.rs",
            ".fkst/evolution/config.yaml",
            ".fkst/evolution/intent/product.md"
        ]
    );
}

#[test]
fn a_fully_confined_change_set_has_no_violations() {
    let changed = vec![".fkst/evolution/docs/a.md", ".fkst/evolution/manifest.json"];
    assert!(confinement_violations(changed).is_empty());
}

#[test]
fn an_empty_change_set_has_no_violations() {
    assert!(confinement_violations(Vec::<&str>::new()).is_empty());
}

#[test]
fn both_reserved_prefixes_are_reserved() {
    assert!(is_reserved(".fkst/evolution/docs/a.md"));
    assert!(is_reserved(".fkst/evolution/config.yaml"));
    assert!(is_reserved(".fkst/packages/anything"));
    assert!(!is_reserved("backend/src/main.rs"));
}

#[test]
fn owner_intent_is_exactly_config_and_the_intent_subtree() {
    assert!(is_owner_intent(".fkst/evolution/config.yaml"));
    assert!(is_owner_intent(".fkst/evolution/intent/product.md"));
    assert!(!is_owner_intent(".fkst/evolution/docs/a.md"));
    assert!(!is_owner_intent(".fkst/evolution/manifest.json"));
    assert!(!is_owner_intent("backend/src/main.rs"));
}

#[test]
fn owner_intent_is_reserved_but_not_writable() {
    // The two properties that make the fingerprint rules work: intent is removed
    // from coverage with the rest of the root, added back to productRelevant by
    // the caller, and never written.
    for path in [
        ".fkst/evolution/config.yaml",
        ".fkst/evolution/intent/x.yaml",
    ] {
        assert!(is_reserved(path), "{path}");
        assert!(is_owner_intent(path), "{path}");
        assert!(!is_writable_by_evolution(path), "{path}");
    }
}
