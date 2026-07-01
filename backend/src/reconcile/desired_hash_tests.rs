//! Tests for the pure [`super::config_hash`]: stability, per-input sensitivity,
//! and package-order sensitivity. Fixtures live in [`super::desired_test_fixtures`].

use super::config_hash;
use super::desired_test_fixtures::pkg;

#[test]
fn config_hash_is_stable_for_identical_inputs() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let a = config_hash(&pkgs, "wl", Some("env"));
    let b = config_hash(&pkgs, "wl", Some("env"));
    assert_eq!(a, b);
    // A SHA-256 hex digest is 64 chars.
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn config_hash_changes_with_each_input() {
    let pkgs = vec![pkg("acme", "tools", "main", "pkg/a")];
    let base = config_hash(&pkgs, "wl", Some("env"));
    // Different work label.
    assert_ne!(base, config_hash(&pkgs, "other", Some("env")));
    // Different environment (Some vs None).
    assert_ne!(base, config_hash(&pkgs, "wl", None));
    // Different package field.
    let pkgs2 = vec![pkg("acme", "tools", "dev", "pkg/a")];
    assert_ne!(base, config_hash(&pkgs2, "wl", Some("env")));
}

#[test]
fn config_hash_is_order_sensitive_for_packages() {
    // Packages are author-ordered, so their order IS part of the identity.
    let a = vec![pkg("o", "r", "m", "p1"), pkg("o", "r", "m", "p2")];
    let b = vec![pkg("o", "r", "m", "p2"), pkg("o", "r", "m", "p1")];
    assert_ne!(config_hash(&a, "wl", None), config_hash(&b, "wl", None));
}
