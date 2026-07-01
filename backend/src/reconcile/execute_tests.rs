//! Unit tests for the executor. The GitHub issue effects (flag/clear) run against
//! a recording fake [`GithubApi`] so no network is touched; the Kubernetes effects
//! need a live cluster, so those are covered by their PURE argument-assembly
//! (`kill_delete_params`, `last_pending_patch`, `session_pod_spec_from`, the token
//! JSON) and the wiring itself is live-verified.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::goals::trigger_parse::PackageRef;
use crate::k8s::session_github_token_json;
use crate::reconcile::desired::SessionDef;

// ---- recording fake GitHub transport ---------------------------------------

/// A recorded issue call: `(owner, repo, issue_number, payload)`.
type Call = (String, String, u64, String);
/// A recorded label-add call: `(owner, repo, issue_number, labels)`.
type LabelCall = (String, String, u64, Vec<String>);

#[derive(Default)]
struct RecordingApi {
    comments: Mutex<Vec<Call>>,
    labels_added: Mutex<Vec<LabelCall>>,
    labels_removed: Mutex<Vec<Call>>,
}

#[async_trait]
impl GithubApi for RecordingApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(1))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        Ok(InstallationToken {
            token: SecretString::from("ghs_fake".to_string()),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }

    async fn create_issue_comment(
        &self,
        _token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), GithubAppError> {
        self.comments.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            body.to_string(),
        ));
        Ok(())
    }

    async fn add_issue_labels(
        &self,
        _token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<(), GithubAppError> {
        self.labels_added.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            labels.to_vec(),
        ));
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        _token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        label: &str,
    ) -> Result<(), GithubAppError> {
        self.labels_removed.lock().unwrap().push((
            owner.to_string(),
            repo.to_string(),
            number,
            label.to_string(),
        ));
        Ok(())
    }
}

fn test_config() -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let mut rng = OsRng;
    let private = RsaPrivateKey::new(&mut rng, 2048).expect("key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst-test".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.com".to_string(),
    }
}

fn tokens(api: Arc<RecordingApi>) -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), api).expect("tokens")
}

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

// ---- pure argument assembly (Kubernetes effects) ----------------------------

#[test]
fn kill_delete_params_carries_the_grace_period() {
    let params = kill_delete_params(60);
    assert_eq!(params.grace_period_seconds, Some(60));
    // A zero grace is legitimate (immediate SIGKILL) and must be honoured.
    assert_eq!(kill_delete_params(0).grace_period_seconds, Some(0));
}

#[test]
fn last_pending_patch_sets_the_annotation_key_to_now() {
    let now = DateTime::parse_from_rfc3339("2026-07-01T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let patch = last_pending_patch(now);
    let value = &patch["metadata"]["annotations"][ANNOTATION_LAST_PENDING_AT];
    assert_eq!(value.as_str().unwrap(), now.to_rfc3339());
}

fn registration() -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue: 7,
        trigger_author_id: 583231,
        def: SessionDef {
            name: "site".to_string(),
            packages: vec![
                PackageRef {
                    owner: "ChronoAIProject".to_string(),
                    repo: "fkst-packages".to_string(),
                    git_ref: "dev".to_string(),
                    path: "packages/github-devloop".to_string(),
                },
                PackageRef {
                    owner: "acme".to_string(),
                    repo: "pkgs".to_string(),
                    git_ref: "main".to_string(),
                    path: "packages/proxy".to_string(),
                },
            ],
            work_label: "fkst-run".to_string(),
            environment: None,
        },
        session_id: "sess-abc".to_string(),
        config_hash: "hash123".to_string(),
    }
}

#[test]
fn session_pod_spec_is_built_from_the_registration() {
    let reg = registration();
    let spec = session_pod_spec_from(&reg, Some("fkst-bot".to_string()));

    assert_eq!(spec.session_id, "sess-abc");
    assert_eq!(spec.installation_id, 42);
    assert_eq!(spec.repo.owner, "acme");
    assert_eq!(spec.trigger_issue_number, 7);
    assert_eq!(spec.work_label, "fkst-run");
    assert_eq!(spec.bot_login, "fkst-bot");
    assert_eq!(spec.config_hash, "hash123");
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
fn missing_bot_login_defaults_to_empty() {
    let spec = session_pod_spec_from(&registration(), None);
    assert_eq!(spec.bot_login, "", "an unset bot login renders as empty");
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
