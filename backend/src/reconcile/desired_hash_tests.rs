//! Tests for the pure [`super::config_hash`]: stability, per-input sensitivity,
//! and package-order sensitivity. Fixtures live in [`super::desired_test_fixtures`].

use super::config_hash;
use super::desired_test_fixtures::pkg;

#[test]
fn config_hash_is_stable_for_identical_inputs() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let a = config_hash(&pkgs, "wl", Some("env"), None);
    let b = config_hash(&pkgs, "wl", Some("env"), None);
    assert_eq!(a, b);
    // A SHA-256 hex digest is 64 chars.
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn config_hash_changes_with_each_input() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let base = config_hash(&pkgs, "wl", Some("env"), None);
    // Different work label.
    assert_ne!(base, config_hash(&pkgs, "other", Some("env"), None));
    // Different environment (Some vs None).
    assert_ne!(base, config_hash(&pkgs, "wl", None, None));
    // Different package field.
    let pkgs2 = vec![pkg("acme", "tools", "dev", "pkg/a")];
    assert_ne!(base, config_hash(&pkgs2, "wl", Some("env"), None));
}

#[test]
fn config_hash_is_order_sensitive_for_packages() {
    // Packages are author-ordered, so their order IS part of the identity.
    let a = vec![pkg("o", "r", "m", "p1"), pkg("o", "r", "m", "p2")];
    let b = vec![pkg("o", "r", "m", "p2"), pkg("o", "r", "m", "p1")];
    assert_ne!(
        config_hash(&a, "wl", None, None),
        config_hash(&b, "wl", None, None)
    );
}

#[test]
fn config_hash_is_digest_stable_for_old_configs() {
    // PINNED pre-`output_lang` digests (captured before the field existed).
    // If either assertion ever fails, a shape change broke cross-deploy hash
    // stability: every live session's recomputed hash would differ from the
    // one latched at announce, tripping the immutability check fleet-wide
    // (false `fkst-config-rejected` + spawn suppression). Fields added to the
    // canonical struct MUST skip serialization when unset.
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    assert_eq!(
        config_hash(&pkgs, "wl", Some("env"), None),
        "7a039ccf53042416ee9ae7127e168806f353fa7472e49eb24d39e7994ef9dfea"
    );
    assert_eq!(
        config_hash(&pkgs, "wl", None, None),
        "326cafd0f2a4d7e2d4e4bc54f8f6a958e12c84c9bc409862912f31c96c5fbf6d"
    );
}

#[test]
fn config_hash_moves_with_the_output_language() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let without = config_hash(&pkgs, "wl", Some("env"), None);
    let with = config_hash(&pkgs, "wl", Some("env"), Some("zh"));
    assert_ne!(without, with, "output_lang is pod-affecting config");
    assert_ne!(
        with,
        config_hash(&pkgs, "wl", Some("env"), Some("en")),
        "different locales must hash differently"
    );
}
