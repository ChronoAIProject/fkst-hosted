//! Orchestrates the ephemeral, fully-isolated env-validation Pod (issue #338
//! §3.1/§3.3/§3.4).
//!
//! A named environment carries an ordered list of install commands. Before it is
//! persisted, those commands are run once inside a throwaway, hard-isolated Pod
//! (the SAME box as a session pod, see [`crate::k8s::isolation`]); the pod prints
//! a single-line JSON verdict as its last stdout line, which this module reads
//! back into a [`ValidationOutcome`]. The REST layer (a later PR) calls
//! [`validate_environment`]: on `Passed` it persists the environment, on `Failed`
//! it renders the detailed 422.
//!
//! ## Concurrency
//!
//! Two module-level guards bound the blast radius (kept off `AppState` so the
//! REST layer need not thread anything new): a global [`Semaphore`] caps how many
//! validation pods run at once ([`crate::config::EnvConfig::validate_max_concurrent`]),
//! and a per-`(id, name)` in-flight set rejects a duplicate validation of the
//! SAME environment. Both release on every exit path via a drop-guard.
//!
//! ## Cleanup
//!
//! A drop-guard best-effort deletes the pod on every exit (background
//! propagation cascades the owner-referenced ConfigMap). `activeDeadlineSeconds`
//! and [`sweep_orphans`] are the backstops if the control plane crashes mid-run.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::Utc;
use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams};
use serde::Deserialize;
use tokio::sync::{Semaphore, SemaphorePermit};

use crate::config::Config;
use crate::error::AppError;
use crate::k8s::client::KubeClient;

#[path = "env_validator_pod.rs"]
mod pod;

use pod::{
    build_spec_configmap, build_validation_pod, pod_owner_reference, validation_pod_name,
    COMPONENT_LABEL_VALUE,
};

/// How long we wait for a free validation slot before declaring capacity busy.
const ADMISSION_GRACE: Duration = Duration::from_secs(2);
/// Wall-clock added beyond the pod's own deadline before the poll loop aborts a
/// pod stuck in `Pending` (unschedulable / ImagePull) that never runs.
const WAIT_BUFFER_SECS: u64 = 30;
/// Age added beyond the deadline before the GC sweep reaps an orphaned pod.
const SWEEP_BUFFER_SECS: i64 = 30;
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

/// The result of validating an environment's install commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// Every install command exited zero. `commands` is how many ran.
    Passed { commands: usize },
    /// A command failed, the sequence timed out, or the pod produced no trusted
    /// verdict. Carries the detail the REST layer renders in its 422.
    Failed {
        failed_command_index: u32,
        failed_command: String,
        exit_code: i32,
        timed_out: bool,
        stderr_tail: String,
    },
}

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

/// Best-effort, fire-and-forget pod deletion on drop. Background propagation
/// cascades the owner-referenced ConfigMap away with the pod.
struct PodCleanup {
    api: Api<Pod>,
    name: String,
}

impl Drop for PodCleanup {
    fn drop(&mut self) {
        let api = self.api.clone();
        let name = std::mem::take(&mut self.name);
        // Drop cannot be async: spawn the delete. If the process is shutting
        // down and this never runs, activeDeadlineSeconds + the GC sweep reap it.
        tokio::spawn(async move {
            match api.delete(&name, &DeleteParams::background()).await {
                Ok(_) => tracing::info!(pod = %name, "env validation: cleanup deleted pod"),
                Err(kube::Error::Api(e)) if e.code == 404 => {}
                Err(error) => {
                    tracing::warn!(error = %error, pod = %name, "env validation: cleanup delete failed")
                }
            }
        });
    }
}

/// Validate a named environment's install commands in a throwaway isolated pod.
///
/// Admits the call under the concurrency ceiling + the per-env in-flight guard
/// (429 when either is saturated), launches the pod, waits for it to terminate
/// (bounded so a stuck pod still aborts), and returns the parsed verdict. Every
/// exit path releases the admission resources and best-effort deletes the pod.
pub async fn validate_environment(
    kube: &KubeClient,
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
    run_validation(kube, config, id, name, install, variables).await
}

/// The pod lifecycle: create the pod + its spec ConfigMap, wait for a terminal
/// phase (bounded), and read back the verdict. Split from [`validate_environment`]
/// so the admission logic stays small; the caller holds the admission guard.
async fn run_validation(
    kube: &KubeClient,
    config: &Config,
    id: i64,
    name: &str,
    install: &[String],
    variables: &BTreeMap<String, String>,
) -> Result<ValidationOutcome, AppError> {
    let namespace = &config.pod.namespace;
    let object_name = validation_pod_name(id, name);
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), namespace);

    // 1. Create the isolated pod (same box as a session pod).
    let pod = build_validation_pod(
        &object_name,
        id,
        &config.pod,
        config.env.validate_deadline_secs,
    )?;
    let created = pods
        .create(&PostParams::default(), &pod)
        .await
        .map_err(|e| kube_internal("create validation pod", e))?;

    // 2. Arm cleanup: from here EVERY exit path deletes the pod (its
    //    owner-referenced ConfigMap cascades away with it).
    let _cleanup = PodCleanup {
        api: pods.clone(),
        name: object_name.clone(),
    };

    // 3. Create the spec ConfigMap AFTER the pod (it needs the pod UID for its
    //    owner reference); the kubelet retries the mount until it exists.
    let owner = pod_owner_reference(&created);
    let configmap = build_spec_configmap(
        &object_name,
        namespace,
        id,
        install,
        variables,
        config.env.validate_deadline_secs,
        owner,
    )?;
    let configmaps: Api<ConfigMap> = Api::namespaced(kube.client().clone(), namespace);
    configmaps
        .create(&PostParams::default(), &configmap)
        .await
        .map_err(|e| kube_internal("create validation spec configmap", e))?;

    // 4. Wait for a terminal phase, wrapped in a hard timeout so a pod stuck in
    //    Pending (unschedulable / ImagePull) still aborts.
    let overall = Duration::from_secs(
        u64::try_from(config.env.validate_deadline_secs).unwrap_or(0) + WAIT_BUFFER_SECS,
    );
    let poll = Duration::from_secs(config.env.validate_poll_interval_secs);
    let phase =
        match tokio::time::timeout(overall, wait_for_terminal_phase(&pods, &object_name, poll))
            .await
        {
            Ok(phase) => phase,
            Err(_elapsed) => {
                tracing::warn!(
                    pod = %object_name,
                    "env validation: pod did not reach a terminal phase before the deadline"
                );
                return Ok(ValidationOutcome::Failed {
                    failed_command_index: 0,
                    failed_command: String::new(),
                    exit_code: -1,
                    timed_out: true,
                    stderr_tail: "validation pod did not complete before the deadline".to_string(),
                });
            }
        };

    // 5. Read the verdict from the pod's last stdout line.
    capture_outcome(&pods, &object_name, &phase).await
}

/// Poll the pod's status until its phase is terminal (`Succeeded`/`Failed`),
/// returning that phase. Transient poll errors are logged and retried; the
/// caller's outer timeout is the hard backstop.
async fn wait_for_terminal_phase(pods: &Api<Pod>, name: &str, poll: Duration) -> String {
    loop {
        match pods.get_status(name).await {
            Ok(p) => {
                if let Some(phase) = p.status.and_then(|s| s.phase) {
                    if phase == "Succeeded" || phase == "Failed" {
                        return phase;
                    }
                }
            }
            Err(error) => {
                // Eventual consistency right after create, or an API blip: keep
                // polling. The outer timeout bounds the total wait.
                tracing::debug!(error = %error, pod = %name, "env validation: status poll error; retrying");
            }
        }
        tokio::time::sleep(poll).await;
    }
}

/// Read the pod logs and parse the LAST non-empty line as the verdict frame.
/// Readable-but-unparseable logs (OOM / deadline-kill / anomaly) map to a
/// conservative `Failed` — NOT an infra error, since the environment must never
/// be persisted on an untrusted result. Only totally-unreadable logs `Err`.
async fn capture_outcome(
    pods: &Api<Pod>,
    name: &str,
    phase: &str,
) -> Result<ValidationOutcome, AppError> {
    let logs = pods
        .logs(name, &LogParams::default())
        .await
        .map_err(|e| kube_internal("read validation pod logs", e))?;

    match last_non_empty_line(&logs).and_then(parse_verdict_line) {
        Some(outcome) => {
            tracing::info!(pod = %name, phase = %phase, "env validation: verdict parsed");
            Ok(outcome)
        }
        None => {
            tracing::warn!(
                pod = %name,
                phase = %phase,
                "env validation: no parseable verdict; treating as failed"
            );
            Ok(ValidationOutcome::Failed {
                failed_command_index: 0,
                failed_command: String::new(),
                exit_code: -1,
                timed_out: false,
                stderr_tail: "validation pod exceeded its limits".to_string(),
            })
        }
    }
}

/// Reap validation pods left behind by a crashed control plane (a bare Pod has no
/// `ttlSecondsAfterFinished`, so this backstop is required). Deletes any
/// `env-validation` pod older than `deadline_secs + buffer`; returns the count.
pub async fn sweep_orphans(kube: &KubeClient, deadline_secs: i64) -> Result<usize, AppError> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    let selector = format!("app.kubernetes.io/component={COMPONENT_LABEL_VALUE}");
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(|e| kube_internal("list validation pods", e))?;

    let cutoff =
        Utc::now() - k8s_openapi::chrono::Duration::seconds(deadline_secs + SWEEP_BUFFER_SECS);
    let mut deleted = 0usize;
    for pod in list.items {
        let Some(name) = pod.metadata.name.clone() else {
            continue;
        };
        let Some(Time(created)) = pod.metadata.creation_timestamp else {
            continue;
        };
        if created >= cutoff {
            continue; // Still within its lifetime; the run owns it.
        }
        match pods.delete(&name, &DeleteParams::background()).await {
            Ok(_) => {
                deleted += 1;
                tracing::info!(pod = %name, "env validation gc: reaped orphaned pod");
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {}
            Err(error) => {
                tracing::warn!(error = %error, pod = %name, "env validation gc: reap failed")
            }
        }
    }
    Ok(deleted)
}

/// The spawned GC sweep loop: reap orphaned validation pods every `interval`.
/// Modeled on the Job watcher's run loop; runs for the process lifetime.
pub async fn run_sweep_loop(kube: KubeClient, deadline_secs: i64, interval: Duration) {
    tracing::info!(?interval, "env validation gc sweep: started");
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        match sweep_orphans(&kube, deadline_secs).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                deleted = n,
                "env validation gc sweep: removed orphaned pods"
            ),
            Err(error) => tracing::warn!(error = %error, "env validation gc sweep: failed"),
        }
    }
}

/// A totally-unreadable kube failure, mapped to a generic 500. The detail (which
/// may name cluster objects) is logged, never echoed to the client.
fn kube_internal(context: &str, error: kube::Error) -> AppError {
    AppError::Internal(anyhow::anyhow!("{context}: {error}"))
}

/// The verdict frame the pod prints as its last stdout line (see
/// [`crate::install::verdict_frame`]). Optional fields let both the `ok` and
/// `failed` shapes deserialize into one struct.
#[derive(Deserialize)]
struct VerdictFrame {
    status: String,
    #[serde(default)]
    commands: Option<usize>,
    #[serde(default)]
    index: Option<u64>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    timed_out: Option<bool>,
    #[serde(default)]
    stderr_tail: Option<String>,
}

/// Parse a single verdict JSON line into a [`ValidationOutcome`]. `None` for a
/// non-JSON / empty / unrecognized-status line (pure + unit-tested).
fn parse_verdict_line(line: &str) -> Option<ValidationOutcome> {
    let frame: VerdictFrame = serde_json::from_str(line.trim()).ok()?;
    match frame.status.as_str() {
        "ok" => Some(ValidationOutcome::Passed {
            commands: frame.commands?,
        }),
        "failed" => Some(ValidationOutcome::Failed {
            failed_command_index: u32::try_from(frame.index?).unwrap_or(0),
            failed_command: frame.command.unwrap_or_default(),
            exit_code: frame.exit_code.unwrap_or(-1),
            timed_out: frame.timed_out.unwrap_or(false),
            stderr_tail: frame.stderr_tail.unwrap_or_default(),
        }),
        _ => None,
    }
}

/// The last non-empty (trimmed) line of `text`, or `None` if there is none. The
/// pod may emit tracing chatter before the frame, so only the final line counts.
fn last_non_empty_line(text: &str) -> Option<&str> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
}

#[cfg(test)]
#[path = "env_validator_tests.rs"]
mod tests;
