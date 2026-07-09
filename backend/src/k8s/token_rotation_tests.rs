//! Tests for the token-rotation sweep's fleet fan-out: every live session handle
//! gets its rotated credential DELIVERED through the backend, and a gone session is
//! a benign no-op. The mint runs against a fake GitHub transport; the delivery runs
//! against the shared recording [`FakeSessionBackend`], so no cluster is touched.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;

use super::rotate_once;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::config::GithubAppConfig;
use crate::github_app::{GithubAppError, GithubAppTokens};
use crate::models::RepoRef;
use crate::reconcile::reconcile_channel;
use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_backend::SessionHandle;
use crate::session_spec::creds::GITHUB_TOKEN_FILE;

/// A GitHub transport that always mints a fake token (the rotation only needs the
/// mint to succeed to reach the delivery step).
#[derive(Default)]
struct OkApi;

#[async_trait]
impl GithubApi for OkApi {
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
        _owner: &str,
        _repo: &str,
        _number: u64,
        _body: &str,
    ) -> Result<(), GithubAppError> {
        Ok(())
    }

    async fn add_issue_labels(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _labels: &[String],
    ) -> Result<(), GithubAppError> {
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
        _label: &str,
    ) -> Result<(), GithubAppError> {
        Ok(())
    }

    async fn get_issue_labels(
        &self,
        _token: &SecretString,
        _owner: &str,
        _repo: &str,
        _number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        Ok(Vec::new())
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

fn tokens() -> GithubAppTokens {
    GithubAppTokens::with_api(&test_config(), Arc::new(OkApi)).expect("tokens")
}

fn handle_for(session_id: &str, name: &str, issue: Option<u64>) -> SessionHandle {
    SessionHandle {
        session_id: session_id.to_string(),
        installation_id: 1,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: name.to_string(),
        },
        trigger_issue: issue,
    }
}

#[tokio::test]
async fn rotation_delivers_a_credential_to_every_fleet_handle() {
    let github = tokens();
    let (handle, _rx) = reconcile_channel(16);
    let fleet = vec![
        handle_for("sess-1", "site", Some(7)),
        handle_for("sess-2", "web", Some(8)),
    ];
    let backend = FakeSessionBackend::default().with_fleet(fleet);

    rotate_once(&backend, &github, &handle)
        .await
        .expect("sweep ok");

    let delivered = backend.delivered.lock().unwrap();
    assert_eq!(delivered.len(), 2, "one delivery per fleet handle");
    // Every delivery targets the rotating github-token credential file.
    assert!(delivered.iter().all(|(_, file)| file == GITHUB_TOKEN_FILE));
    let ids: Vec<&str> = delivered.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"sess-1") && ids.contains(&"sess-2"));
}

#[tokio::test]
async fn rotation_tolerates_a_gone_session() {
    let github = tokens();
    let (handle, _rx) = reconcile_channel(16);
    let fleet = vec![handle_for("sess-gone", "site", Some(7))];
    let backend = FakeSessionBackend::default()
        .with_fleet(fleet)
        .with_gone("sess-gone");

    // Must not panic / error: a gone session's delivery is a benign no-op.
    rotate_once(&backend, &github, &handle)
        .await
        .expect("sweep ok despite a gone session");
    assert_eq!(
        backend.delivered.lock().unwrap().len(),
        1,
        "delivery was still attempted"
    );
}
