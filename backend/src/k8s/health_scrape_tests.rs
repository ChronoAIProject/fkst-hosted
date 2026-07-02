//! Unit tests for the session-health scrape DECISION (flag/clear/no-op) and its
//! pure helpers. The GitHub effects run against a recording fake [`GithubApi`]
//! (mirroring `reconcile::execute_tests`) so no network is touched; the pod
//! LIST + log read need a live cluster and are live-verified, as with the token
//! rotation loop.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::k8s::health_eval::HealthVerdict;

// ---- recording fake GitHub transport ---------------------------------------

type Call = (String, String, u64, String);
type LabelCall = (String, String, u64, Vec<String>);

#[derive(Default)]
struct RecordingApi {
    /// Labels `get_issue_labels` reports for the issue.
    issue_labels: Vec<String>,
    /// When set, `get_issue_labels` fails (to exercise the read-error path).
    fail_label_read: AtomicBool,
    /// When set, `create_issue_comment` fails (to exercise failure-swallowing).
    fail_comment: AtomicBool,
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
        if self.fail_comment.load(Ordering::SeqCst) {
            return Err(GithubAppError::Http("boom".to_string()));
        }
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

    async fn get_issue_labels(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        if self.fail_label_read.load(Ordering::SeqCst) {
            return Err(GithubAppError::AppAuth);
        }
        Ok(self.issue_labels.clone())
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

fn degraded() -> HealthVerdict {
    HealthVerdict::Degraded {
        reason_verbatim: "codex-triage/score_dedup: no issue mirror at .fkst/mirror/42".to_string(),
        detail: "logged 12× in the recent window; the pod is up but keeps reporting this."
            .to_string(),
    }
}

// ---- flag / clear / no-op decision -----------------------------------------

#[tokio::test]
async fn flags_on_degraded_when_not_yet_labelled() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    apply_verdict(&github, "acme/site", 7, &degraded(), &[], true).await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1, "one degraded comment");
    assert!(comments[0].3.contains("Session health: degraded"));
    assert!(
        comments[0].3.contains("no issue mirror at .fkst/mirror/42"),
        "verbatim line quoted"
    );

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].3, vec![SUBSTRATE_DEGRADED_LABEL.to_string()]);
    assert!(api.labels_removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_op_when_degraded_and_already_flagged() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    apply_verdict(
        &github,
        "acme/site",
        7,
        &degraded(),
        &[SUBSTRATE_DEGRADED_LABEL.to_string()],
        true,
    )
    .await;

    assert!(api.comments.lock().unwrap().is_empty(), "no re-comment");
    assert!(api.labels_added.lock().unwrap().is_empty());
    assert!(api.labels_removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn clears_on_recovery_when_flagged() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    apply_verdict(
        &github,
        "acme/site",
        7,
        &HealthVerdict::Healthy,
        &[SUBSTRATE_DEGRADED_LABEL.to_string()],
        true,
    )
    .await;

    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].3.contains("Session health: recovered"));

    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].3, SUBSTRATE_DEGRADED_LABEL);
    assert!(api.labels_added.lock().unwrap().is_empty());
}

#[tokio::test]
async fn no_op_when_healthy_and_not_flagged() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    apply_verdict(&github, "acme/site", 7, &HealthVerdict::Healthy, &[], true).await;

    assert!(api.comments.lock().unwrap().is_empty());
    assert!(api.labels_added.lock().unwrap().is_empty());
    assert!(api.labels_removed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn withholds_clear_when_logs_were_unreadable() {
    // Healthy verdict computed on an UNREADABLE log window is inconclusive: the flag
    // must stay so a transient log-read failure never clears a real degradation.
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());

    apply_verdict(
        &github,
        "acme/site",
        7,
        &HealthVerdict::Healthy,
        &[SUBSTRATE_DEGRADED_LABEL.to_string()],
        false,
    )
    .await;

    assert!(
        api.comments.lock().unwrap().is_empty(),
        "no recovery comment"
    );
    assert!(
        api.labels_removed.lock().unwrap().is_empty(),
        "flag retained"
    );
}

#[tokio::test]
async fn comment_failure_is_swallowed_and_flag_still_attempted() {
    let api = Arc::new(RecordingApi::default());
    api.fail_comment.store(true, Ordering::SeqCst);
    let github = tokens(api.clone());

    // Must not panic even though the comment POST errors.
    apply_verdict(&github, "acme/site", 7, &degraded(), &[], true).await;

    assert!(api.comments.lock().unwrap().is_empty(), "comment failed");
    // The label latch is still attempted so the transition is not lost.
    assert_eq!(api.labels_added.lock().unwrap().len(), 1);
}

// ---- trigger-issue annotation reader ----------------------------------------

fn pod_with_annotations(pairs: &[(&str, &str)]) -> Pod {
    let annotations = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<BTreeMap<_, _>>();
    Pod {
        metadata: ObjectMeta {
            annotations: Some(annotations),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn trigger_issue_reads_the_stamped_annotation() {
    let pod = pod_with_annotations(&[(ANNOTATION_TRIGGER_ISSUE, "123")]);
    assert_eq!(trigger_issue_from_pod(&pod), Some(123));
}

#[test]
fn trigger_issue_is_none_when_missing_zero_or_unparseable() {
    assert_eq!(trigger_issue_from_pod(&pod_with_annotations(&[])), None);
    assert_eq!(
        trigger_issue_from_pod(&pod_with_annotations(&[(ANNOTATION_TRIGGER_ISSUE, "0")])),
        None
    );
    assert_eq!(
        trigger_issue_from_pod(&pod_with_annotations(&[(ANNOTATION_TRIGGER_ISSUE, "nan")])),
        None
    );
}

// ---- comment bodies ---------------------------------------------------------

#[test]
fn degraded_comment_quotes_verbatim_and_disclaims_judgment() {
    let body = degraded_comment("LEVEL=warn MSG=no mirror", "logged 3×");
    assert!(body.contains("Session health: degraded"));
    assert!(
        body.contains("```\nLEVEL=warn MSG=no mirror\n```"),
        "fenced verbatim"
    );
    assert!(body.contains("logged 3×"), "detail relayed");
    assert!(
        body.contains("not a fkst-hosted judgment"),
        "package-agnostic disclaimer present"
    );
}

#[test]
fn recovered_comment_announces_recovery() {
    assert!(recovered_comment().contains("Session health: recovered"));
}
