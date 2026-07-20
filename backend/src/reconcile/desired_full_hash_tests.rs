//! Tests for the pure [`super::full_config_hash`]: the FULL-config superset of
//! [`super::config_hash`]. Verifies stability, per-config-field sensitivity, that
//! identity fields are excluded, and that the `auto_merge` opt-in + the `log_access`
//! allow-list — which `config_hash` ignores — DO move the full hash. Fixtures live in
//! [`super::desired_test_fixtures`].

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
        trigger_author_login: "author-login".to_string(),
        def: SessionDef {
            name: "sess".to_string(),
            packages: vec![pkg("acme", "tools", "main", "pkg/a")],
            manifest_refs: vec![],
            work_label: Some("wl".to_string()),
            environment: Some("env".to_string()),
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
        },
        effective_packages: vec![pkg("acme", "tools", "main", "pkg/a")],
        session_id: "sid".to_string(),
        config_hash: "ignored".to_string(),
        auto_merge: false,
        log_access: vec!["alice".to_string()],
        collaborators: vec![],
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
    reg.def.work_label = Some("other".to_string());
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

    // log_access — the allow-list is FROZEN by config-immutability, so editing it
    // MUST move the full hash (that is how the immutability check catches a widening).
    let mut reg = base_reg();
    reg.log_access = vec!["alice".to_string(), "bob".to_string()];
    assert_ne!(
        base,
        full_config_hash(&reg),
        "log_access must move the full hash (it is frozen by config-immutability)"
    );

    // A DIFFERENT allow-list (order/content) hashes differently again.
    let mut reg = base_reg();
    reg.log_access = vec![];
    assert_ne!(
        base,
        full_config_hash(&reg),
        "clearing log_access must also move the hash"
    );

    // collaborators — like log_access, FROZEN by config-immutability, so adding a
    // non-empty list MUST move the full hash (base_reg has none).
    let mut reg = base_reg();
    reg.collaborators = vec!["worker".to_string()];
    assert_ne!(
        base,
        full_config_hash(&reg),
        "collaborators must move the full hash (it is frozen by config-immutability)"
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
        base.def.work_label.as_deref(),
        base.def.environment.as_deref(),
        base.def.output_lang.as_deref(),
        &base.def.engine_config,
        &base.def.manifest_refs,
    );
    let base_full = full_config_hash(&base);

    let mut toggled = base_reg();
    toggled.auto_merge = true;
    let toggled_config = config_hash(
        &toggled.def.packages,
        toggled.def.work_label.as_deref(),
        toggled.def.environment.as_deref(),
        toggled.def.output_lang.as_deref(),
        &toggled.def.engine_config,
        &toggled.def.manifest_refs,
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

#[test]
fn full_config_hash_is_digest_stable_for_old_configs() {
    // PINNED pre-`output_lang` digest (captured before that field existed): a
    // registration that never set the new sections must hash byte-identically
    // across the deploy, or every announced session's latched marker would
    // mismatch and the immutability check would fire fleet-wide. base_reg carries
    // NO output_lang / engine_config / collaborators, so every skip-if-empty
    // trailing field is omitted and the digest is unchanged — this same pin also
    // guards the later `collaborators` addition (issue #572 F3).
    assert_eq!(
        full_config_hash(&base_reg()),
        "0a97c6a26cf3dc2ae4326ea253f7ee33e8bf8e28bd3416b8e33443e7999fb091"
    );
}

#[test]
fn full_config_hash_empty_collaborators_matches_pre_field_baseline() {
    // (a) A session WITHOUT collaborators must hash byte-identically to the pinned
    // pre-`collaborators` baseline — the skip-if-empty guard keeps old configs
    // stable across the deploy (no fleet-wide `fkst-config-rejected`).
    assert!(base_reg().collaborators.is_empty());
    assert_eq!(
        full_config_hash(&base_reg()),
        "0a97c6a26cf3dc2ae4326ea253f7ee33e8bf8e28bd3416b8e33443e7999fb091",
        "an empty collaborators list must leave the full hash at the pre-field baseline"
    );
}

#[test]
fn full_config_hash_non_empty_collaborators_moves_full_but_not_config_hash() {
    // (b) A non-empty collaborators list flips the FULL hash (freezing it) while
    // leaving the pod-subset `config_hash` untouched — collaborators live on the
    // registration, outside the SessionDef that `config_hash` covers.
    let base = base_reg();
    let base_full = full_config_hash(&base);
    let base_config = config_hash(
        &base.def.packages,
        base.def.work_label.as_deref(),
        base.def.environment.as_deref(),
        base.def.output_lang.as_deref(),
        &base.def.engine_config,
        &base.def.manifest_refs,
    );

    let mut reg = base_reg();
    reg.collaborators = vec!["worker".to_string()];
    let reg_config = config_hash(
        &reg.def.packages,
        reg.def.work_label.as_deref(),
        reg.def.environment.as_deref(),
        reg.def.output_lang.as_deref(),
        &reg.def.engine_config,
        &reg.def.manifest_refs,
    );

    assert_ne!(
        base_full,
        full_config_hash(&reg),
        "collaborators are inside full_config_hash"
    );
    assert_eq!(
        base_config, reg_config,
        "collaborators are outside config_hash (the pod is unaffected)"
    );
}

#[test]
fn full_config_hash_empty_manifest_matches_pre_field_baseline() {
    // (a) A session WITHOUT manifest references must hash byte-identically to the
    // pinned pre-`manifest_refs` baseline — the skip-if-empty guard keeps old configs
    // stable across the deploy (no fleet-wide `fkst-config-rejected`). base_reg carries
    // no manifest_refs, so the trailing skip-if-empty field is omitted and the digest
    // is unchanged (the same pin `full_config_hash_is_digest_stable_for_old_configs`
    // and the collaborators addition also guard).
    assert!(base_reg().def.manifest_refs.is_empty());
    assert_eq!(
        full_config_hash(&base_reg()),
        "0a97c6a26cf3dc2ae4326ea253f7ee33e8bf8e28bd3416b8e33443e7999fb091",
        "an empty manifest list must leave the full hash at the pre-field baseline"
    );
}

#[test]
fn full_config_hash_moves_with_the_manifest_refs() {
    // (b) A non-empty manifest reference flips the FULL hash, FREEZING it under
    // config-immutability. Unlike the opt-ins and collaborators, manifest_refs is ALSO
    // part of config_hash (it is pod-affecting), so it moves the pod-subset hash too —
    // that relationship is asserted in `desired_hash_tests`.
    let base = full_config_hash(&base_reg());
    let mut reg = base_reg();
    reg.def.manifest_refs = vec![pkg("acme", "manifests", "main", "manifests/team.json")];
    assert_ne!(
        base,
        full_config_hash(&reg),
        "manifest_refs must move the full hash (it is frozen by config-immutability)"
    );
}

#[test]
fn full_config_hash_moves_with_the_output_language() {
    let base = full_config_hash(&base_reg());
    let mut reg = base_reg();
    reg.def.output_lang = Some("zh".to_string());
    assert_ne!(
        base,
        full_config_hash(&reg),
        "output_lang must move the hash"
    );
}

#[test]
fn full_config_hash_moves_with_the_engine_config() {
    let base = full_config_hash(&base_reg());
    let mut reg = base_reg();
    reg.def.engine_config = std::collections::BTreeMap::from([(
        "FKST_CODEX_PERMIT_SLOTS".to_string(),
        "8".to_string(),
    )]);
    assert_ne!(
        base,
        full_config_hash(&reg),
        "engine_config must move the hash"
    );
}
