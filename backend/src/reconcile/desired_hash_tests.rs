//! Tests for the pure [`super::config_hash`]: stability, per-input sensitivity,
//! and package-order sensitivity. Fixtures live in [`super::desired_test_fixtures`].

use std::collections::BTreeMap;

use super::config_hash;
use super::desired_test_fixtures::pkg;
use crate::goals::trigger_parse::PackageRef;

/// The no-`### Engine Config` case shared by most tests.
fn no_engine_config() -> BTreeMap<String, String> {
    BTreeMap::new()
}

/// The no-`### Manifest` case shared by most tests (an empty manifest list must be
/// skip-serialized, so it never perturbs the digest — see the digest-stability tests).
fn no_manifest() -> Vec<PackageRef> {
    Vec::new()
}

#[test]
fn config_hash_is_stable_for_identical_inputs() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let a = config_hash(
        &pkgs,
        Some("wl"),
        Some("env"),
        None,
        &no_engine_config(),
        &no_manifest(),
    );
    let b = config_hash(
        &pkgs,
        Some("wl"),
        Some("env"),
        None,
        &no_engine_config(),
        &no_manifest(),
    );
    assert_eq!(a, b);
    // A SHA-256 hex digest is 64 chars.
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn config_hash_changes_with_each_input() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let base = config_hash(
        &pkgs,
        Some("wl"),
        Some("env"),
        None,
        &no_engine_config(),
        &no_manifest(),
    );
    // Different work label.
    assert_ne!(
        base,
        config_hash(
            &pkgs,
            Some("other"),
            Some("env"),
            None,
            &no_engine_config(),
            &no_manifest()
        )
    );
    // Different environment (Some vs None).
    assert_ne!(
        base,
        config_hash(
            &pkgs,
            Some("wl"),
            None,
            None,
            &no_engine_config(),
            &no_manifest()
        )
    );
    // Different package field.
    let pkgs2 = vec![pkg("acme", "tools", "dev", "pkg/a")];
    assert_ne!(
        base,
        config_hash(
            &pkgs2,
            Some("wl"),
            Some("env"),
            None,
            &no_engine_config(),
            &no_manifest()
        )
    );
}

#[test]
fn config_hash_is_order_sensitive_for_packages() {
    // Packages are author-ordered, so their order IS part of the identity.
    let a = vec![pkg("o", "r", "m", "p1"), pkg("o", "r", "m", "p2")];
    let b = vec![pkg("o", "r", "m", "p2"), pkg("o", "r", "m", "p1")];
    assert_ne!(
        config_hash(
            &a,
            Some("wl"),
            None,
            None,
            &no_engine_config(),
            &no_manifest()
        ),
        config_hash(
            &b,
            Some("wl"),
            None,
            None,
            &no_engine_config(),
            &no_manifest()
        )
    );
}

#[test]
fn config_hash_is_digest_stable_for_old_configs() {
    // PINNED pre-`output_lang` digests (captured before the field existed).
    // If either assertion ever fails, a shape change broke cross-deploy hash
    // stability: every live session's recomputed hash would differ from the
    // one latched at announce, tripping the immutability check fleet-wide
    // (false `fkst-config-rejected` + spawn suppression). Fields added to the
    // canonical struct MUST skip serialization when unset. The trailing empty
    // `manifest_refs` (epic #594 I3) is skip-if-empty, so it too leaves these
    // pre-field digests untouched.
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    assert_eq!(
        config_hash(
            &pkgs,
            Some("wl"),
            Some("env"),
            None,
            &no_engine_config(),
            &no_manifest()
        ),
        "7a039ccf53042416ee9ae7127e168806f353fa7472e49eb24d39e7994ef9dfea"
    );
    assert_eq!(
        config_hash(
            &pkgs,
            Some("wl"),
            None,
            None,
            &no_engine_config(),
            &no_manifest()
        ),
        "326cafd0f2a4d7e2d4e4bc54f8f6a958e12c84c9bc409862912f31c96c5fbf6d"
    );
}

#[test]
fn config_hash_empty_manifest_matches_pre_field_baseline() {
    // (a) A session WITHOUT a manifest must hash byte-identically to the pinned
    // pre-`manifest_refs` baseline — the skip-if-empty guard keeps old configs stable
    // across the deploy (no fleet-wide `fkst-config-rejected`). Same pinned digest as
    // `config_hash_is_digest_stable_for_old_configs`, asserted here explicitly against
    // an EMPTY manifest list to lock in the empty-list invariant.
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    assert!(no_manifest().is_empty());
    assert_eq!(
        config_hash(
            &pkgs,
            Some("wl"),
            Some("env"),
            None,
            &no_engine_config(),
            &no_manifest()
        ),
        "7a039ccf53042416ee9ae7127e168806f353fa7472e49eb24d39e7994ef9dfea",
        "an empty manifest list must leave the config hash at the pre-field baseline"
    );
}

#[test]
fn config_hash_moves_with_the_manifest_refs() {
    // (b) A non-empty manifest reference flips the config hash — a manifest expands
    // into packages downstream, so it is pod-affecting config.
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let without = config_hash(
        &pkgs,
        Some("wl"),
        None,
        None,
        &no_engine_config(),
        &no_manifest(),
    );
    let manifest = vec![pkg("acme", "manifests", "main", "manifests/team.json")];
    let with = config_hash(
        &pkgs,
        Some("wl"),
        None,
        None,
        &no_engine_config(),
        &manifest,
    );
    assert_ne!(without, with, "manifest_refs is pod-affecting config");
    // A DIFFERENT manifest reference hashes differently again.
    let other = vec![pkg("acme", "manifests", "dev", "manifests/team.json")];
    assert_ne!(
        with,
        config_hash(&pkgs, Some("wl"), None, None, &no_engine_config(), &other),
        "a different manifest ref must hash differently"
    );
}

#[test]
fn config_hash_is_by_manifest_reference_not_contents() {
    // (c) HASH-BY-REFERENCE INVARIANT: the manifest is hashed as its author-written
    // REFERENCE string (owner/repo@ref:path), never its (unfetched) file contents —
    // there is no expansion in this PR. So two hashes over the SAME manifest ref are
    // equal regardless of what the referenced JSON file holds now vs after any
    // upstream edit: the file's bytes are never an input. We assert this by hashing
    // the ref twice (not by fetching anything). Hashing a mutable file's contents
    // would flip the hash on any upstream edit; hashing the ref does not.
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let manifest = vec![pkg("acme", "manifests", "main", "manifests/team.json")];
    let a = config_hash(
        &pkgs,
        Some("wl"),
        None,
        None,
        &no_engine_config(),
        &manifest,
    );
    // The SAME reference, imagined at a later time when the file's JSON body differs —
    // the ref (the only input) is unchanged, so the hash is unchanged.
    let b = config_hash(
        &pkgs,
        Some("wl"),
        None,
        None,
        &no_engine_config(),
        &manifest,
    );
    assert_eq!(
        a, b,
        "the same manifest ref hashes identically — only the reference string is an input"
    );
}

#[test]
fn config_hash_moves_with_the_output_language() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let without = config_hash(
        &pkgs,
        Some("wl"),
        Some("env"),
        None,
        &no_engine_config(),
        &no_manifest(),
    );
    let with = config_hash(
        &pkgs,
        Some("wl"),
        Some("env"),
        Some("zh"),
        &no_engine_config(),
        &no_manifest(),
    );
    assert_ne!(without, with, "output_lang is pod-affecting config");
    assert_ne!(
        with,
        config_hash(
            &pkgs,
            Some("wl"),
            Some("env"),
            Some("en"),
            &no_engine_config(),
            &no_manifest()
        ),
        "different locales must hash differently"
    );
}

#[test]
fn config_hash_moves_with_the_engine_config() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let empty = config_hash(
        &pkgs,
        Some("wl"),
        None,
        None,
        &no_engine_config(),
        &no_manifest(),
    );
    let cfg = BTreeMap::from([("FKST_CODEX_PERMIT_SLOTS".to_string(), "8".to_string())]);
    assert_ne!(
        empty,
        config_hash(&pkgs, Some("wl"), None, None, &cfg, &no_manifest()),
        "engine_config is pod-affecting config"
    );
}
