//! Admission for the ephemeral, fully-isolated env-validation Pod (issue #338
//! §3.1/§3.3/§3.4).
//!
//! A named environment carries an ordered list of install commands. Before it is
//! persisted, those commands are run once inside a throwaway, hard-isolated Pod;
//! the pod prints a single-line JSON verdict as its last stdout line, which the
//! [`SessionBackend`] reads back into a [`ValidationOutcome`]. The REST layer calls
//! [`validate_environment`]: on `Passed` it persists the environment, on `Failed`
//! it renders the detailed 422.
//!
//! This module owns only the ADMISSION concerns (kept off `AppState` so the REST
//! layer need not thread anything new); the pod lifecycle itself lives on the
//! backend (`crate::session_backend::k8s::K8sBackend::run_validation`).
//!
//! ## Concurrency
//!
//! Two module-level guards bound the blast radius: a global [`Semaphore`] caps how
//! many validation pods run at once ([`crate::config::EnvConfig::validate_max_concurrent`]),
//! and a per-`(id, name)` in-flight set rejects a duplicate validation of the SAME
//! environment. Both release on every exit path via a drop-guard.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::{Semaphore, SemaphorePermit};

use crate::config::Config;
use crate::error::AppError;
use crate::session_backend::{BackendError, SessionBackend, ValidationOutcome, ValidationRequest};

/// How long we wait for a free validation slot before declaring capacity busy.
const ADMISSION_GRACE: Duration = Duration::from_secs(2);
/// `Retry-After` (seconds) advertised when the concurrency ceiling is saturated.
const CAPACITY_RETRY_AFTER_SECS: u64 = 5;
/// `Retry-After` (seconds) advertised when the same env is already validating.
const INFLIGHT_RETRY_AFTER_SECS: u64 = 15;

/// Global concurrency ceiling, sized from config on first use. `OnceLock` (not
/// `AppState`) so the REST layer needs no new construction to be threaded.
static SEM: OnceLock<Semaphore> = OnceLock::new();
/// The set of `(github_user_id, env_name)` pairs currently validating, so a
/// duplicate validation of the same environment is rejected rather than doubled.
static INFLIGHT: OnceLock<Mutex<HashSet<(i64, String)>>> = OnceLock::new();

/// Releases the admission resources (semaphore permit + in-flight entry) on every
/// exit path — success, error, early return, or panic.
struct AdmissionGuard {
    /// Held only to release the semaphore slot on drop.
    _permit: SemaphorePermit<'static>,
    /// The in-flight key to remove on drop.
    key: (i64, String),
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        if let Some(lock) = INFLIGHT.get() {
            // A poisoned lock still lets us clear our own key (we only remove).
            let mut set = match lock.lock() {
                Ok(set) => set,
                Err(poisoned) => poisoned.into_inner(),
            };
            set.remove(&self.key);
        }
    }
}

/// Validate a named environment's install commands in a throwaway isolated pod.
///
/// Admits the call under the concurrency ceiling + the per-env in-flight guard
/// (429 when either is saturated), then runs the validation runtime through the
/// backend and returns the parsed verdict. Every exit path releases the admission
/// resources; the backend best-effort deletes the pod.
pub async fn validate_environment(
    backend: &dyn SessionBackend,
    config: &Config,
    id: i64,
    login: &str,
    name: &str,
    install: &[String],
    variables: &BTreeMap<String, String>,
) -> Result<ValidationOutcome, AppError> {
    // Admission 1: bound global concurrency. A short grace (not an instant fail)
    // smooths a burst, but a saturated ceiling maps to 429 "capacity busy".
    let sem: &'static Semaphore =
        SEM.get_or_init(|| Semaphore::new(config.env.validate_max_concurrent));
    let permit = match tokio::time::timeout(ADMISSION_GRACE, sem.acquire()).await {
        Ok(Ok(permit)) => permit,
        // Elapsed grace, or a (never-closed) semaphore: both are "no slot".
        Ok(Err(_)) | Err(_) => {
            return Err(AppError::RateLimited {
                message: "validation capacity busy, retry".to_string(),
                retry_after_secs: CAPACITY_RETRY_AFTER_SECS,
            });
        }
    };

    // Admission 2: reject a duplicate in-flight validation of the SAME env. On
    // rejection the permit drops here, releasing the slot we just took.
    let key = (id, name.to_string());
    {
        let inflight = INFLIGHT.get_or_init(|| Mutex::new(HashSet::new()));
        let mut set = match inflight.lock() {
            Ok(set) => set,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !set.insert(key.clone()) {
            return Err(AppError::RateLimited {
                message: "a validation for this environment is already in flight".to_string(),
                retry_after_secs: INFLIGHT_RETRY_AFTER_SECS,
            });
        }
    }
    // From here every exit path releases both resources.
    let _admission = AdmissionGuard {
        _permit: permit,
        key,
    };

    tracing::info!(
        github_user_id = id,
        login = %login,
        env = %name,
        commands = install.len(),
        "env validation: admitted; launching isolated pod"
    );

    // The `_admission` guard stays alive across this await and drops when this
    // function returns, so the slot is held for the whole pod lifecycle.
    let req = ValidationRequest {
        github_user_id: id,
        name: name.to_string(),
        install: install.to_vec(),
        variables: variables.clone(),
    };
    backend.run_validation(&req).await.map_err(|e| match e {
        // Carry the backend's opaque detail through as a 500 (never echoed to the
        // client); a `NotFound` here is not a real code path but must map to Err.
        BackendError::Other(err) => AppError::Internal(err),
        BackendError::NotFound => {
            AppError::Internal(anyhow::anyhow!("env validation backend resource not found"))
        }
    })
}

/// The spawned GC sweep loop: reap orphaned validation pods every `interval` through
/// the backend. Modeled on the Job watcher's run loop; runs for the process lifetime.
pub async fn run_sweep_loop(backend: Arc<dyn SessionBackend>, interval: Duration) {
    tracing::info!(?interval, "env validation gc sweep: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        match backend.reap_stale_validations().await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                deleted = n,
                "env validation gc sweep: removed orphaned pods"
            ),
            Err(error) => tracing::warn!(error = %error, "env validation gc sweep: failed"),
        }
    }
}
