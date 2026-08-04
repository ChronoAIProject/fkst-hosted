//! Tests for the pure launch-argument assembly: the pod spec built from a
//! registration, the package-side author list, the token JSON, the storage
//! credentials, and the named-environment pre-flight.

use std::time::{Duration, SystemTime};

use k8s_openapi::chrono::DateTime;

use super::*;
use crate::k8s::session_github_token_json;
use crate::reconcile::execute_test_support::*;

fn branch_topology() -> ResolvedBranchTopology {
    ResolvedBranchTopology {
        upstream: "develop".to_string(),
        integration: "fkst-hosted-default".to_string(),
    }
}

#[test]
fn session_pod_spec_is_built_from_the_registration() {
    let reg = registration();
    // A single-label session: the detected set is the one explicit label; the pod work
    // label renders as the bare label (no comma), byte-identical to the pre-I4 value.
    let spec = session_pod_spec_from(
        &reg,
        &["fkst-run".to_string()],
        &branch_topology(),
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
        None,
        None,
    )
    .expect("valid labels");

    assert_eq!(spec.session_id, "sess-abc");
    assert_eq!(spec.installation_id, 42);
    assert_eq!(spec.repo.owner, "acme");
    assert_eq!(spec.trigger_issue_number, 7);
    assert_eq!(spec.work_label, "fkst-run");
    assert_eq!(spec.bot_login, "fkst-bot");
    assert_eq!(spec.creator_login, "author-login");
    assert_eq!(spec.config_hash, "hash123");
    assert_eq!(spec.upstream_branch, "develop");
    assert_eq!(spec.target_branch, "fkst-hosted-default");
    // package_roots are the refs rendered back to `owner/repo@ref:path`, in order.
    assert_eq!(
        spec.package_roots,
        vec![
            "ChronoAIProject/fkst-hosted@packages:packages/github-devloop".to_string(),
            "acme/pkgs@main:packages/proxy".to_string(),
        ]
    );
}

#[test]
fn package_roots_come_from_the_effective_set_not_just_explicit_packages() {
    // Epic #594 I7: `FKST_SESSION_PACKAGE_ROOTS` is built from the EFFECTIVE set
    // (explicit ∪ manifest-expanded), which the reconcile driver stamps onto the reg. A
    // manifest-only package present only in `effective_packages` (not `def.packages`) is
    // therefore cloned into the pod too.
    let mut reg = registration();
    reg.effective_packages
        .push(crate::goals::trigger_parse::PackageRef {
            owner: "acme".to_string(),
            repo: "manifests-pkgs".to_string(),
            git_ref: "main".to_string(),
            path: "packages/from-manifest".to_string(),
        });
    let spec = session_pod_spec_from(
        &reg,
        &["fkst-run".to_string()],
        &branch_topology(),
        None,
        &crate::access_policy::AccessPolicy::default(),
        None,
        None,
    )
    .expect("valid labels");
    assert_eq!(
        spec.package_roots,
        vec![
            "ChronoAIProject/fkst-hosted@packages:packages/github-devloop".to_string(),
            "acme/pkgs@main:packages/proxy".to_string(),
            "acme/manifests-pkgs@main:packages/from-manifest".to_string(),
        ],
        "package_roots reflect effective_packages, including the manifest-only entry"
    );
}

#[test]
fn spec_work_label_is_the_comma_joined_detected_set() {
    // Epic #594 I4: the pod work label is the FULL effective set (explicit ∪
    // package-discovered), comma-joined — github-proxy splits it back on the comma, so
    // the session wakes on ANY of its labels. Order is preserved; blanks/dupes dropped.
    let reg = registration();

    // Discovered-only (no explicit `### Work Label`): the pod still gets a work label.
    let discovered_only = session_pod_spec_from(
        &reg,
        &["pkg-a".to_string(), "pkg-b".to_string()],
        &branch_topology(),
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
        None,
        None,
    )
    .expect("valid labels");
    assert_eq!(discovered_only.work_label, "pkg-a,pkg-b");

    // Explicit + discovered union, comma-joined in the given order, deduped.
    let union = session_pod_spec_from(
        &reg,
        &[
            "fkst-run".to_string(),
            "pkg-a".to_string(),
            "fkst-run".to_string(),
        ],
        &branch_topology(),
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
        None,
        None,
    )
    .expect("valid labels");
    assert_eq!(union.work_label, "fkst-run,pkg-a");
}

#[test]
fn namespaced_spec_uses_only_effective_labels_and_carries_the_mapping() {
    let reg = registration();
    let spec = session_pod_spec_from(
        &reg,
        &["fkst-dev".to_string(), "fkst-security".to_string()],
        &branch_topology(),
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
        None,
        Some("chronoai-fkst"),
    )
    .expect("valid namespaced labels");

    assert_eq!(
        spec.work_label,
        "fkst-dev-chronoai-fkst,fkst-security-chronoai-fkst"
    );
    assert_eq!(
        spec.work_label_map_json.as_deref(),
        Some(
            r#"{"fkst-dev":"fkst-dev-chronoai-fkst","fkst-security":"fkst-security-chronoai-fkst"}"#
        )
    );
    assert_eq!(
        spec.config_hash,
        runtime_config_hash(&reg.config_hash, Some("chronoai-fkst"))
    );
    assert!(!spec.work_label.split(',').any(|label| label == "fkst-dev"));
}

#[test]
fn missing_bot_login_defaults_to_empty() {
    let spec = session_pod_spec_from(
        &registration(),
        &["fkst-run".to_string()],
        &branch_topology(),
        None,
        &crate::access_policy::AccessPolicy::default(),
        None,
        None,
    )
    .expect("valid labels");
    assert_eq!(spec.bot_login, "", "an unset bot login renders as empty");
}

#[test]
fn session_contributors_starts_with_effective_creator_not_app_author() {
    let mut reg = registration();
    reg.trigger_author_login = "fkst-app[bot]".to_string();
    reg.creator_login = "Seed-Owner".to_string();
    reg.creator_id = None;
    reg.log_access = vec!["log-viewer".to_string()];
    reg.collaborators = vec!["seed-owner".to_string(), "reviewer".to_string()];
    let access = crate::access_policy::AccessPolicy::from_vars(&[(
        "FKST_GLOBAL_ADMINS".to_string(),
        "Deploy-Admin,4242,REVIEWER".to_string(),
    )])
    .expect("access");
    assert_eq!(
        session_contributors(&reg, &access),
        vec![
            "Seed-Owner".to_string(),
            "reviewer".to_string(),
            "Deploy-Admin".to_string(),
        ]
    );
}

#[tokio::test]
async fn assignee_derived_creator_resolves_no_environment_without_an_id() {
    let store = crate::k8s::env_store::EnvStore::fake();
    match resolve_named_environment(&store, None, "seed-owner", Some("ignored-selection")).await {
        EnvResolution::Proceed(environment) => {
            assert!(environment.user_env.is_empty());
            assert!(environment.install.is_empty());
            assert!(environment.secret_keys.is_empty());
        }
        EnvResolution::Blocked { comment } => {
            panic!("no-id creator must proceed without an environment: {comment}")
        }
    }
}

#[test]
fn github_token_json_carries_the_token_and_rfc3339_expiry() {
    let token = SecretString::from("ghs_secret".to_string());
    let expires = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let json = session_github_token_json(&token, expires);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    assert_eq!(parsed["token"].as_str().unwrap(), "ghs_secret");
    let expires_at = parsed["expires_at"].as_str().unwrap();
    // A valid RFC3339 instant that round-trips back to the same time.
    let back = DateTime::parse_from_rfc3339(expires_at).expect("rfc3339");
    assert_eq!(
        back.timestamp(),
        1_700_000_000,
        "expiry round-trips as an RFC3339 instant"
    );
}

// ---- Session storage credential assembly (issue #489) -------------------------

/// With chrono-storage configured, the SINGLE NyxID SA is always injected into
/// the session Secret's `storage-*` keys — id and secret sourced from
/// `nyxid_client_id`/`nyxid_client_secret`, unconditionally (no writer pair, no
/// half-enabled state). With no storage config the keys are absent entirely.
#[test]
fn storage_creds_carry_the_single_nyxid_sa_into_the_session_secret() {
    let mut config = Config::default();
    assert!(
        storage_writer_creds(&config).is_none(),
        "no storage config → no storage creds"
    );

    config.storage = Some(crate::storage::ChronoStorageConfig {
        base_url: "https://storage.example/proxy".into(),
        bucket: "fkst-logs".into(),
        nyxid_token_url: "https://nyx.example/oauth/token".into(),
        nyxid_client_id: "sa-client".into(),
        nyxid_client_secret: SecretString::from("sa-secret"),
    });
    let creds = storage_writer_creds(&config)
        .expect("storage configured → uploader creds are unconditional");
    assert_eq!(creds.client_id, "sa-client");
    assert_eq!(creds.client_secret, "sa-secret");
    assert_eq!(creds.token_url, "https://nyx.example/oauth/token");
    assert_eq!(creds.base_url, "https://storage.example/proxy");
    assert_eq!(creds.bucket, "fkst-logs");

    let data = credential_secret_data("ghs", "sk-llm", std::iter::empty(), &[], &[], Some(creds));
    assert_eq!(data["storage-client-id"], "sa-client");
    assert_eq!(data["storage-client-secret"], "sa-secret");
    assert_eq!(data["storage-token-url"], "https://nyx.example/oauth/token");
    assert_eq!(data["storage-base-url"], "https://storage.example/proxy");
    assert_eq!(data["storage-bucket"], "fkst-logs");
}

// ---- Durable attribution + lifecycle records (issue #5673) ------------------

/// Build the launch spec for `reg` with the standard single-label fixture.
fn spec_for(reg: &SessionRegistration) -> SessionPodSpec {
    session_pod_spec_from(
        reg,
        &["fkst-run".to_string()],
        &branch_topology(),
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
        None,
        None,
    )
    .expect("valid labels")
}

#[test]
fn the_launch_spec_threads_every_attribution_field_from_the_registration() {
    let spec = spec_for(&registration());
    assert_eq!(spec.creator_id, Some(583231));
    assert_eq!(spec.creator_login, "author-login");
    assert_eq!(spec.trigger_author_id, 583231);
    assert_eq!(spec.trigger_author_login, "author-login");
}

#[test]
fn an_assignee_derived_creator_survives_into_the_launch_spec_without_an_id() {
    let mut reg = registration();
    reg.creator_id = None;
    reg.creator_login = "assignee".to_string();
    reg.trigger_author_login = "fkst-cloud[bot]".to_string();
    let spec = spec_for(&reg);
    assert_eq!(spec.creator_id, None);
    assert_eq!(spec.creator_login, "assignee");
    assert_eq!(
        spec.trigger_author_login, "fkst-cloud[bot]",
        "the spec carries the raw login; normalization happens at the stamp"
    );
    // The stamp is what both runtimes write, and it is normalized there.
    assert_eq!(spec.identity().trigger_author_login, "fkst-cloud");
    assert_eq!(spec.identity().creator_id, None);
}

#[test]
fn re_attributing_a_trigger_never_moves_the_runtime_config_hash() {
    // The drift check compares this exact value, so if attribution entered it,
    // editing an issue's assignee would delete and respawn a running session.
    let base = spec_for(&registration()).config_hash;

    let mut reg = registration();
    reg.creator_id = Some(999_999);
    reg.creator_login = "someone-else".to_string();
    reg.trigger_author_id = 999_999;
    reg.trigger_author_login = "another-author".to_string();
    assert_eq!(spec_for(&reg).config_hash, base);
}
