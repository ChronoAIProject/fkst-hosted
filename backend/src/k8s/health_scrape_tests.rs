//! Unit tests for the session-health scrape DECISION (flag/clear/no-op), the
//! fleet-driven `scrape_one` clear-withholding, and the pure comment bodies. The
//! GitHub effects run against a recording fake [`GithubApi`] and the runtime reads
//! against the shared [`FakeSessionBackend`], so no network / cluster is touched.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::*;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::k8s::health_eval::HealthVerdict;
use crate::models::RepoRef;
use crate::reconcile::reconcile_channel;
use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_backend::SessionHandle;

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

fn session(session_id: &str) -> SessionHandle {
    SessionHandle {
        session_id: session_id.to_string(),
        installation_id: 1,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue: Some(7),
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

// ---- fleet-driven scrape_one: the clear-withholding gate on recent_output ---

#[tokio::test]
async fn scrape_one_withholds_clear_when_recent_output_is_none() {
    // Already-flagged issue + a Healthy verdict (default status, no logs) BUT an
    // unreadable log window (`None`) must NOT clear the flag.
    let api = Arc::new(RecordingApi {
        issue_labels: vec![SUBSTRATE_DEGRADED_LABEL.to_string()],
        ..Default::default()
    });
    let github = tokens(api.clone());
    let (handle, _rx) = reconcile_channel(16);
    let backend = FakeSessionBackend::default().with_recent("sess-1", None);

    scrape_one(&backend, &github, &handle, &session("sess-1")).await;

    assert!(
        api.labels_removed.lock().unwrap().is_empty(),
        "unreadable logs → flag retained"
    );
    assert!(
        api.comments.lock().unwrap().is_empty(),
        "no recovery comment"
    );
}

#[tokio::test]
async fn scrape_one_clears_when_recent_output_is_readable() {
    // Already-flagged issue + a Healthy verdict + a READABLE (empty) window clears.
    let api = Arc::new(RecordingApi {
        issue_labels: vec![SUBSTRATE_DEGRADED_LABEL.to_string()],
        ..Default::default()
    });
    let github = tokens(api.clone());
    let (handle, _rx) = reconcile_channel(16);
    let backend = FakeSessionBackend::default().with_recent("sess-1", Some(String::new()));

    scrape_one(&backend, &github, &handle, &session("sess-1")).await;

    let removed = api.labels_removed.lock().unwrap();
    assert_eq!(removed.len(), 1, "readable healthy logs → flag cleared");
    assert_eq!(removed[0].3, SUBSTRATE_DEGRADED_LABEL);
}

#[tokio::test]
async fn structured_bootstrap_failure_flags_and_explains_the_trigger() {
    let api = Arc::new(RecordingApi::default());
    let github = tokens(api.clone());
    let (handle, _rx) = reconcile_channel(16);
    let line = "TIMESTAMP=2026-07-21T14:18:05Z LEVEL=error MSG=github-devloop dept=ensure_repo proposal_id=unknown tag=FAILURE error_class=gh-command-failed fingerprint=fp-123 queue=devloop_ensure_repo_tick error=github-devloop: gh-command-failed: bootstrap failed";
    let backend = FakeSessionBackend::default().with_recent("sess-1", Some(line.to_string()));

    scrape_one(&backend, &github, &handle, &session("sess-1")).await;

    let added = api.labels_added.lock().unwrap();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].3, vec![SUBSTRATE_DEGRADED_LABEL.to_string()]);
    let comments = api.comments.lock().unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].3.contains("dept=ensure_repo"));
    assert!(comments[0].3.contains("error_class=gh-command-failed"));
    assert!(comments[0].3.contains("queue=devloop_ensure_repo_tick"));
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
