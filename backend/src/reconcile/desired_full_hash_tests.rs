//! Tests for the pure [`super::full_config_hash`]: the FULL-config superset of
//! [`super::config_hash`]. Verifies stability, per-config-field sensitivity, that
//! identity fields are excluded, and that the two opt-ins (`auto_merge`,
//! `log_streaming`) — which `config_hash` ignores — DO move the full hash. Fixtures
//! live in [`super::desired_test_fixtures`].

use super::desired_test_fixtures::pkg;
use super::{config_hash, full_config_hash, SessionDef, SessionRegistration};
use crate::models::RepoRef;

/// A fully-populated registration whose every config field is set to a non-default
/// value, so a test can flip exactly one field and observe the hash change.
fn base_reg() -> SessionRegistration {
    SessionRegistration {
        installation_id: 1,
        repo: RepoRef {
            owner: "o".to_string(),
            name: "r".to_string(),
        },
        trigger_issue: 5,
        trigger_author_id: 9,
        def: SessionDef {
            name: "sess".to_string(),
            packages: vec![pkg("acme", "tools", "main", "pkg/a")],
            work_label: "wl".to_string(),
            environment: Some("env".to_string()),
        },
        session_id: "sid".to_string(),
        config_hash: "ignored".to_string(),
        auto_merge: false,
        log_streaming: false,
    }
}

#[test]
fn full_config_hash_is_stable_for_identical_inputs() {
    let a = full_config_hash(&base_reg());
    let b = full_config_hash(&base_reg());
    assert_eq!(a, b);
    // A SHA-256 hex digest is 64 lowercase hex chars.
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn full_config_hash_changes_with_each_config_field() {
    let base = full_config_hash(&base_reg());

    // name
    let mut reg = base_reg();
    reg.def.name = "other".to_string();
    assert_ne!(base, full_config_hash(&reg), "name must move the hash");

    // packages (a field of the ref)
    let mut reg = base_reg();
    reg.def.packages = vec![pkg("acme", "tools", "dev", "pkg/a")];
    assert_ne!(base, full_config_hash(&reg), "packages must move the hash");

    // work_label
    let mut reg = base_reg();
    reg.def.work_label = "other".to_string();
    assert_ne!(
        base,
        full_config_hash(&reg),
        "work_label must move the hash"
    );

    // environment (Some -> None)
    let mut reg = base_reg();
    reg.def.environment = None;
    assert_ne!(
        base,
        full_config_hash(&reg),
        "environment must move the hash"
    );

    // auto_merge
    let mut reg = base_reg();
    reg.auto_merge = true;
    assert_ne!(
        base,
        full_config_hash(&reg),
        "auto_merge must move the hash"
    );

    // log_streaming
    let mut reg = base_reg();
    reg.log_streaming = true;
    assert_ne!(
        base,
        full_config_hash(&reg),
        "log_streaming must move the hash"
    );
}

#[test]
fn full_config_hash_is_order_sensitive_for_packages() {
    // Packages are author-ordered, so their order IS part of the identity.
    let mut a = base_reg();
    a.def.packages = vec![pkg("o", "r", "m", "p1"), pkg("o", "r", "m", "p2")];
    let mut b = base_reg();
    b.def.packages = vec![pkg("o", "r", "m", "p2"), pkg("o", "r", "m", "p1")];
    assert_ne!(full_config_hash(&a), full_config_hash(&b));
}

#[test]
fn full_config_hash_ignores_identity_fields() {
    // The hash is over the CONFIG only; the identity keys (installation, repo,
    // issue, author, session id, and the pod-subset config_hash) never affect it.
    let base = full_config_hash(&base_reg());

    let mut reg = base_reg();
    reg.installation_id = 999;
    reg.repo = RepoRef {
        owner: "x".to_string(),
        name: "y".to_string(),
    };
    reg.trigger_issue = 4242;
    reg.trigger_author_id = 4242;
    reg.session_id = "different".to_string();
    reg.config_hash = "different".to_string();
    assert_eq!(
        base,
        full_config_hash(&reg),
        "identity fields must not move the full-config hash"
    );
}

#[test]
fn full_config_hash_is_a_strict_superset_of_config_hash() {
    // Toggling an opt-in moves the FULL hash but leaves the pod-subset
    // `config_hash` untouched — the defining superset relationship.
    let base = base_reg();
    let base_config = config_hash(
        &base.def.packages,
        &base.def.work_label,
        base.def.environment.as_deref(),
    );
    let base_full = full_config_hash(&base);

    let mut toggled = base_reg();
    toggled.auto_merge = true;
    toggled.log_streaming = true;
    let toggled_config = config_hash(
        &toggled.def.packages,
        &toggled.def.work_label,
        toggled.def.environment.as_deref(),
    );

    assert_eq!(
        base_config, toggled_config,
        "the opt-ins are outside config_hash (pod is unaffected)"
    );
    assert_ne!(
        base_full,
        full_config_hash(&toggled),
        "the opt-ins are inside full_config_hash"
    );
    assert_ne!(
        base_config, base_full,
        "the full hash covers a larger field set than config_hash"
    );
}
