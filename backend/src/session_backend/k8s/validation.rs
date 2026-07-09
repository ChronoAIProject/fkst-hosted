//! The env-validation verbs (issue #413): run one throwaway isolated validation pod
//! to a verdict, and reap orphaned validation pods a crashed control plane left
//! behind. Moved verbatim from `k8s/env_validator.rs` (the admission/concurrency
//! guards stay there); the pure builders it drives stay in `k8s/env_validator_pod.rs`.
//! Deadline + poll cadence come from the backend's ctor knobs, not a threaded config.

use std::time::Duration;

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::Utc;
use kube::api::{Api, DeleteParams, ListParams, LogParams, PostParams};
use serde::Deserialize;

use crate::k8s::env_validator_pod::{
    build_spec_configmap, build_validation_pod, pod_owner_reference, validation_pod_name,
    COMPONENT_LABEL_VALUE,
};

use super::super::{BackendError, ValidationOutcome, ValidationRequest};
use super::K8sBackend;

/// Wall-clock added beyond the pod's own deadline before the poll loop aborts a
/// pod stuck in `Pending` (unschedulable / ImagePull) that never runs.
const WAIT_BUFFER_SECS: u64 = 30;
/// Age added beyond the deadline before the GC sweep reaps an orphaned pod.
const SWEEP_BUFFER_SECS: i64 = 30;

impl K8sBackend {
    /// The pod lifecycle: create the pod + its spec ConfigMap, wait for a terminal
    /// phase (bounded), and read back the verdict. The caller holds the admission
    /// guard; this verb owns only the runtime.
    pub(super) async fn run_validation_impl(
        &self,
        req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError> {
        let namespace = self.kube.namespace();
        let object_name = validation_pod_name(req.github_user_id, &req.name);
        let pods = self.pods_api();

        // 1. Create the isolated pod (same box as a session pod).
        let pod = build_validation_pod(
            &object_name,
            req.github_user_id,
            &self.pod_config,
            self.validate_deadline_secs,
        )
        .map_err(|e| BackendError::Other(anyhow::anyhow!("{e}")))?;
        let created = pods
            .create(&PostParams::default(), &pod)
            .await
            .map_err(|e| kube_backend("create validation pod", e))?;

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
            req.github_user_id,
            &req.install,
            &req.variables,
            self.validate_deadline_secs,
            owner,
        )
        .map_err(|e| BackendError::Other(anyhow::anyhow!("{e}")))?;
        let configmaps: Api<ConfigMap> = Api::namespaced(self.kube.client().clone(), namespace);
        configmaps
            .create(&PostParams::default(), &configmap)
            .await
            .map_err(|e| kube_backend("create validation spec configmap", e))?;

        // 4. Wait for a terminal phase, wrapped in a hard timeout so a pod stuck in
        //    Pending (unschedulable / ImagePull) still aborts.
        let overall = Duration::from_secs(
            u64::try_from(self.validate_deadline_secs).unwrap_or(0) + WAIT_BUFFER_SECS,
        );
        let poll = Duration::from_secs(self.validate_poll_interval_secs);
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
                        stderr_tail: "validation pod did not complete before the deadline"
                            .to_string(),
                    });
                }
            };

        // 5. Read the verdict from the pod's last stdout line.
        capture_outcome(&pods, &object_name, &phase).await
    }

    /// Reap validation pods left behind by a crashed control plane (a bare Pod has no
    /// `ttlSecondsAfterFinished`, so this backstop is required). Deletes any
    /// `env-validation` pod older than `deadline + buffer`; returns the count.
    pub(super) async fn reap_stale_validations_impl(&self) -> Result<usize, BackendError> {
        let pods = self.pods_api();
        let selector = format!("app.kubernetes.io/component={COMPONENT_LABEL_VALUE}");
        let list = pods
            .list(&ListParams::default().labels(&selector))
            .await
            .map_err(|e| kube_backend("list validation pods", e))?;

        let cutoff = Utc::now()
            - k8s_openapi::chrono::Duration::seconds(
                self.validate_deadline_secs + SWEEP_BUFFER_SECS,
            );
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
}

/// Best-effort, fire-and-forget validation-pod deletion on drop. Background
/// propagation cascades the owner-referenced ConfigMap away with the pod.
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
) -> Result<ValidationOutcome, BackendError> {
    let logs = pods
        .logs(name, &LogParams::default())
        .await
        .map_err(|e| kube_backend("read validation pod logs", e))?;

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

/// A totally-unreadable kube failure, carried opaquely (the detail, which may name
/// cluster objects, is logged/echoed only as a generic 500 at the REST boundary).
fn kube_backend(context: &str, error: kube::Error) -> BackendError {
    BackendError::Other(anyhow::anyhow!("{context}: {error}"))
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
#[path = "validation_tests.rs"]
mod tests;
