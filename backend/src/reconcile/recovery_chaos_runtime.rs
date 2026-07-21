//! Restartable runtime and environment-profile fakes for recovery chaos tests.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use k8s_openapi::chrono::{DateTime, Utc};
use secrecy::SecretString;

use super::{AUTHOR_ID, ENVIRONMENT};
use crate::environment_profile::EnvironmentProfileStore;
use crate::error::AppError;
use crate::github_app::config::GithubAppConfig;
use crate::k8s::env_store::{EnvRecord, EnvSummary};
use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, LivePod, PodLiveness};
use crate::session_backend::{
    BackendError, DeliveryOutcome, EnsureOutcome, ObserveError, RuntimeStatus, SessionBackend,
    SessionHandle, ValidationOutcome, ValidationRequest,
};

#[derive(Default)]
pub(super) struct FixtureEnvironmentStore;

#[async_trait]
impl EnvironmentProfileStore for FixtureEnvironmentStore {
    async fn put_environment(
        &self,
        _id: i64,
        _login: &str,
        _name: &str,
        _install: &[String],
        _variables: &BTreeMap<String, String>,
        _secrets: &BTreeMap<String, String>,
        _validated_at: &str,
        _content_hash: &str,
        _validation_image: &str,
        _expected_version: Option<&str>,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_environment(&self, id: i64, name: &str) -> Result<Option<EnvRecord>, AppError> {
        if id != AUTHOR_ID || name != ENVIRONMENT {
            return Ok(None);
        }
        Ok(Some(EnvRecord {
            name: name.to_string(),
            status: "ready".to_string(),
            validated_at: "2026-01-01T00:00:00Z".to_string(),
            install: vec!["tool install fixture".to_string()],
            variables: BTreeMap::from([("PUBLIC_VALUE".to_string(), "fixture".to_string())]),
            secret_keys: vec!["DEPLOY_KEY".to_string()],
            store_version: None,
            private_content_hash: None,
        }))
    }

    async fn list_environments(&self, _id: i64) -> Result<Vec<EnvSummary>, AppError> {
        Ok(Vec::new())
    }

    async fn count_environments(&self, _id: i64) -> Result<usize, AppError> {
        Ok(1)
    }

    async fn delete_environment(&self, _id: i64, _name: &str) -> Result<bool, AppError> {
        Ok(false)
    }

    async fn load_environment_for_session(
        &self,
        id: i64,
        name: &str,
    ) -> Result<Option<(Vec<String>, BTreeMap<String, String>, Vec<String>)>, AppError> {
        if id != AUTHOR_ID || name != ENVIRONMENT {
            return Ok(None);
        }
        Ok(Some((
            vec!["tool install fixture".to_string()],
            BTreeMap::from([
                ("DEPLOY_KEY".to_string(), "fixture-private".to_string()),
                ("PUBLIC_VALUE".to_string(), "fixture-public".to_string()),
            ]),
            vec!["DEPLOY_KEY".to_string()],
        )))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BackendProfile {
    Kubernetes,
    OpenSandbox,
}

impl BackendProfile {
    pub const ALL: [Self; 2] = [Self::Kubernetes, Self::OpenSandbox];
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EnsureEvent {
    pub session_id: String,
    pub created: bool,
    pub credential_keys: Vec<String>,
}

#[derive(Clone)]
pub(super) struct RuntimeRecord {
    session_id: String,
    installation_id: i64,
    repo: RepoRef,
    trigger_issue: i64,
    config_hash: String,
    work_labels: Vec<String>,
    created_at: DateTime<Utc>,
    last_pending_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
pub(super) struct BackendEvents {
    pub ensures: Vec<EnsureEvent>,
    pub stops: Vec<(String, KillReason)>,
}

pub(super) struct ChaosBackend {
    pub profile: BackendProfile,
    pub runtimes: Arc<Mutex<HashMap<String, RuntimeRecord>>>,
    pub events: Arc<Mutex<BackendEvents>>,
    pub credential_cache: Mutex<HashMap<String, BTreeSet<String>>>,
}

#[async_trait]
impl SessionBackend for ChaosBackend {
    async fn check_reachable(&self) -> Result<String, BackendError> {
        Ok("chaos-fixture".to_string())
    }

    async fn ensure_session(
        &self,
        spec: &SessionPodSpec,
        creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError> {
        let keys: BTreeSet<String> = creds.keys().cloned().collect();
        let mut runtimes = self.runtimes.lock().unwrap();
        let created = !runtimes.contains_key(&spec.session_id);
        if created {
            runtimes.insert(
                spec.session_id.clone(),
                RuntimeRecord {
                    session_id: spec.session_id.clone(),
                    installation_id: spec.installation_id,
                    repo: spec.repo.clone(),
                    trigger_issue: spec.trigger_issue_number,
                    config_hash: spec.config_hash.clone(),
                    work_labels: crate::k8s::work_label_wire::split_work_labels(&spec.work_label),
                    created_at: Utc::now(),
                    last_pending_at: None,
                },
            );
        }
        drop(runtimes);
        if self.profile == BackendProfile::OpenSandbox {
            self.credential_cache
                .lock()
                .unwrap()
                .insert(spec.session_id.clone(), keys.clone());
        }
        self.events.lock().unwrap().ensures.push(EnsureEvent {
            session_id: spec.session_id.clone(),
            created,
            credential_keys: keys.into_iter().collect(),
        });
        Ok(if created {
            EnsureOutcome::Created
        } else {
            EnsureOutcome::AlreadyLive
        })
    }

    async fn credential_recovery_needed(&self, session_id: &str) -> Result<bool, BackendError> {
        Ok(self.profile == BackendProfile::OpenSandbox
            && !self
                .credential_cache
                .lock()
                .unwrap()
                .contains_key(session_id))
    }

    async fn observe_repo(&self, repo: &RepoRef) -> Result<Vec<LivePod>, BackendError> {
        Ok(self
            .runtimes
            .lock()
            .unwrap()
            .values()
            .filter(|runtime| &runtime.repo == repo)
            .map(|runtime| LivePod {
                session_id: runtime.session_id.clone(),
                trigger_issue: runtime.trigger_issue,
                liveness: PodLiveness::Live,
                created_at: runtime.created_at,
                last_pending_at: runtime.last_pending_at,
                config_hash: Some(runtime.config_hash.clone()),
                work_labels: runtime.work_labels.clone(),
            })
            .collect())
    }

    async fn mark_pending(&self, session_id: &str) -> Result<(), BackendError> {
        let mut runtimes = self.runtimes.lock().unwrap();
        let runtime = runtimes.get_mut(session_id).ok_or(BackendError::NotFound)?;
        runtime.last_pending_at = Some(Utc::now());
        Ok(())
    }

    async fn stop_session(&self, session_id: &str, reason: KillReason) -> Result<(), BackendError> {
        if self.runtimes.lock().unwrap().remove(session_id).is_none() {
            return Err(BackendError::NotFound);
        }
        self.events
            .lock()
            .unwrap()
            .stops
            .push((session_id.to_string(), reason));
        Ok(())
    }

    async fn remove_terminal(&self, session_id: &str) -> Result<(), BackendError> {
        self.runtimes
            .lock()
            .unwrap()
            .remove(session_id)
            .map(|_| ())
            .ok_or(BackendError::NotFound)
    }

    async fn list_fleet(&self) -> Result<Vec<SessionHandle>, BackendError> {
        Ok(self
            .runtimes
            .lock()
            .unwrap()
            .values()
            .map(|runtime| SessionHandle {
                session_id: runtime.session_id.clone(),
                installation_id: runtime.installation_id,
                repo: runtime.repo.clone(),
                trigger_issue: Some(runtime.trigger_issue as u64),
            })
            .collect())
    }

    async fn deliver_credential(
        &self,
        _session_id: &str,
        _file: &str,
        _contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        Ok(DeliveryOutcome::Delivered)
    }

    async fn status_summary(&self, _session_id: &str) -> Result<RuntimeStatus, BackendError> {
        Ok(RuntimeStatus::default())
    }

    async fn recent_output(&self, _session_id: &str) -> Option<String> {
        None
    }

    async fn engine_observe(&self, _session_id: &str, _limit: u32) -> Result<String, ObserveError> {
        Ok("{}".to_string())
    }

    async fn run_validation(
        &self,
        _req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError> {
        Ok(ValidationOutcome::Passed { commands: 0 })
    }

    async fn reap_stale_validations(&self) -> Result<usize, BackendError> {
        Ok(0)
    }
}

pub(super) fn test_app_config() -> GithubAppConfig {
    static PRIVATE_KEY: OnceLock<String> = OnceLock::new();
    let pem = PRIVATE_KEY.get_or_init(|| {
        use rand::rngs::OsRng;
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        rsa::RsaPrivateKey::new(&mut OsRng, 2048)
            .expect("RSA key")
            .to_pkcs8_pem(LineEnding::LF)
            .expect("PEM")
            .to_string()
    });
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.clone()),
        app_slug: Some("fkst-chaos".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.invalid".to_string(),
    }
}
