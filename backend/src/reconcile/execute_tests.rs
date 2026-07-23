//! Unit tests for the executor's GitHub issue effects (flag/clear/announce/reject)
//! and the pure argument assembly (`session_pod_spec_from`, the token JSON). These
//! run against a recording fake [`GithubApi`] so no network is touched; the action
//! routing through the session backend lives in the sibling [`super::routing_tests`],
//! and the shared fakes/builders live in [`super::execute_test_support`].

use std::time::{Duration, SystemTime};

use k8s_openapi::chrono::DateTime;

use super::*;
use crate::k8s::session_github_token_json;
use crate::reconcile::announce::announce_session_comment_with_defaults;
use crate::reconcile::execute_test_support::*;

// ---- GitHub issue effects ---------------------------------------------------

#[tokio::test]
async fn flag_invalid_posts_a_comment_and_latches_the_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    flag_invalid(&github, "acme/site", 7, "bad body: fix it").await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(
        comments[0],
        ("acme".into(), "site".into(), 7, "bad body: fix it".into())
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 7);
    assert_eq!(added[0].3, vec![SUBSTRATE_INVALID_LABEL.to_string()]);
}

#[tokio::test]
async fn announce_session_posts_a_comment_and_latches_the_announced_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    let body = announce_session_comment_with_defaults(
        "demo",
        Some("fkst-run"),
        &["fkst-run".to_string()],
        &[],
        None,
        false,
        None,
        "cfg99",
    );
    announce_session(&github, "acme/site", 11, &body).await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 11);
    assert!(
        comments[0].3.contains("fkst session `demo` registered."),
        "the posted body is the rendered announcement"
    );
    assert!(
        comments[0].3.contains("<!-- fkst-config-hash: cfg99 -->"),
        "the posted body latches the config-hash marker"
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 11);
    assert_eq!(added[0].3, vec![SUBSTRATE_ANNOUNCED_LABEL.to_string()]);
}

#[tokio::test]
async fn reject_config_change_posts_a_comment_and_latches_the_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    reject_config_change(&github, "acme/site", 13).await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "exactly one comment");
    assert_eq!(comments[0].2, 13);
    assert!(
        comments[0]
            .3
            .contains("Config changes are not allowed after a session trigger exists."),
        "the posted body is the rejection feedback"
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1, "exactly one label add");
    assert_eq!(added[0].2, 13);
    assert_eq!(
        added[0].3,
        vec![SUBSTRATE_CONFIG_REJECTED_LABEL.to_string()]
    );
}

#[tokio::test]
async fn clear_invalid_removes_the_label() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    clear_invalid(&github, "acme/site", 9).await;

    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0],
        (
            "acme".into(),
            "site".into(),
            9,
            SUBSTRATE_INVALID_LABEL.into()
        )
    );
}

#[tokio::test]
async fn trigger_unauthorized_latches_before_posting_the_comment() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    flag_trigger_unauthorized(
        &github,
        "acme/site",
        17,
        &trigger_unauthorized_comment("@alice lacks maintain permission"),
    )
    .await;

    assert_eq!(*api.events.lock().unwrap(), ["label-add", "comment"]);
    assert_eq!(
        api.labels_added.lock().unwrap()[0].3,
        vec![TRIGGER_UNAUTHORIZED_LABEL.to_string()]
    );
    assert!(api.comments.lock().unwrap()[0]
        .3
        .contains("The issue body has not been read"));
}

#[tokio::test]
async fn clear_trigger_unauthorized_removes_the_latch() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    clear_trigger_unauthorized(&github, "acme/site", 19).await;

    assert_eq!(
        api.labels_removed.lock().unwrap()[0],
        (
            "acme".into(),
            "site".into(),
            19,
            TRIGGER_UNAUTHORIZED_LABEL.into()
        )
    );
}

fn target_branch_ctx(api: Arc<RecordingApi>) -> ReconcileCtx {
    let backend = Arc::new(FakeSessionBackend::default());
    let mut ctx = test_ctx(backend);
    ctx.github = tokens(api);
    ctx
}

#[tokio::test]
async fn missing_target_is_created_at_the_current_source_head() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "main".to_string(),
        "source-head".to_string(),
    )]));
    let ctx = target_branch_ctx(api.clone());

    assert!(ensure_target_branch(&registration(), &ctx).await);
    assert_eq!(
        *api.create_refs.lock().unwrap(),
        [("fkst-hosted-default".to_string(), "source-head".to_string())]
    );
}

#[tokio::test]
async fn target_create_lost_race_is_successful() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "main".to_string(),
        "source-head".to_string(),
    )]));
    *api.create_ref_error.lock().unwrap() = Some(GithubAppError::RefExists);
    let ctx = target_branch_ctx(api);

    assert!(ensure_target_branch(&registration(), &ctx).await);
}

#[tokio::test]
async fn existing_target_is_never_reset_or_recreated() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "fkst-hosted-default".to_string(),
        "existing-head".to_string(),
    )]));
    let ctx = target_branch_ctx(api.clone());

    assert!(ensure_target_branch(&registration(), &ctx).await);
    assert!(api.create_refs.lock().unwrap().is_empty());
}

#[tokio::test]
async fn target_create_failure_skips_spawn_without_issue_feedback() {
    let api = Arc::new(RecordingApi::default());
    *api.branch_heads.lock().unwrap() = Some(std::collections::HashMap::from([(
        "main".to_string(),
        "source-head".to_string(),
    )]));
    *api.create_ref_error.lock().unwrap() = Some(GithubAppError::Http(
        "branch protection denied creation".to_string(),
    ));
    let ctx = target_branch_ctx(api.clone());

    assert!(!ensure_target_branch(&registration(), &ctx).await);
    assert!(api.comments.lock().unwrap().is_empty());
    assert!(api.labels_added.lock().unwrap().is_empty());
}

// ---- pure argument assembly -------------------------------------------------

#[test]
fn session_pod_spec_is_built_from_the_registration() {
    let reg = registration();
    // A single-label session: the detected set is the one explicit label; the pod work
    // label renders as the bare label (no comma), byte-identical to the pre-I4 value.
    let spec = session_pod_spec_from(
        &reg,
        &["fkst-run".to_string()],
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
    );

    assert_eq!(spec.session_id, "sess-abc");
    assert_eq!(spec.installation_id, 42);
    assert_eq!(spec.repo.owner, "acme");
    assert_eq!(spec.trigger_issue_number, 7);
    assert_eq!(spec.work_label, "fkst-run");
    assert_eq!(spec.bot_login, "fkst-bot");
    assert_eq!(spec.creator_login, "author-login");
    assert_eq!(spec.config_hash, "hash123");
    assert_eq!(spec.target_branch, "fkst-hosted-default");
    // package_roots are the refs rendered back to `owner/repo@ref:path`, in order.
    assert_eq!(
        spec.package_roots,
        vec![
            "ChronoAIProject/fkst-packages@dev:packages/github-devloop".to_string(),
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
        None,
        &crate::access_policy::AccessPolicy::default(),
    );
    assert_eq!(
        spec.package_roots,
        vec![
            "ChronoAIProject/fkst-packages@dev:packages/github-devloop".to_string(),
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
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
    );
    assert_eq!(discovered_only.work_label, "pkg-a,pkg-b");

    // Explicit + discovered union, comma-joined in the given order, deduped.
    let union = session_pod_spec_from(
        &reg,
        &[
            "fkst-run".to_string(),
            "pkg-a".to_string(),
            "fkst-run".to_string(),
        ],
        Some("fkst-bot".to_string()),
        &crate::access_policy::AccessPolicy::default(),
    );
    assert_eq!(union.work_label, "fkst-run,pkg-a");
}

#[test]
fn missing_bot_login_defaults_to_empty() {
    let spec = session_pod_spec_from(
        &registration(),
        &["fkst-run".to_string()],
        None,
        &crate::access_policy::AccessPolicy::default(),
    );
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
