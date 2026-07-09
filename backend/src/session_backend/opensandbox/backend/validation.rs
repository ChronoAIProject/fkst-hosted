//! The env-validation verbs (issue #419): run one throwaway isolated validation to a
//! verdict, and reap validation holders a crashed control plane left behind.
//!
//! Mirrors the Kubernetes backend's validation lifecycle
//! ([`crate::session_backend::k8s::validation`]) but over a sandbox instead of a bare
//! Pod: create a HOLDER sandbox (`sleep infinity`), push the validate spec + run the
//! SAME `<entrypoint binary> validate-env` command through execd, poll to completion
//! (bounded), and parse the last log line through the SHARED
//! [`crate::session_backend::verdict`] parser — so a verdict is BYTE-FOR-BYTE identical
//! to the Kubernetes path. A drop-guard deletes the holder on every exit path
//! (success / failure / timeout).
//!
//! The holder is tagged `fkst-validation=true` (so the reaper finds it) but NOT
//! `fkst-managed` (so `list_fleet` / `observe_repo` / `resolve_one` never see it as a
//! session). Its ONE non-null create `timeout` is the server-side GC backstop.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::chrono::Utc;
use secrecy::ExposeSecret;

use crate::install::{ValidateSpec, VALIDATE_ENV_SUBCOMMAND, VALIDATE_SPEC_PATH};
use crate::session_backend::opensandbox::derive_execd_token;
use crate::session_backend::opensandbox::dto::{CreateSandboxRequest, OsbError};
use crate::session_backend::opensandbox::{ExecdClient, OsbLifecycleClient};
use crate::session_backend::verdict::{
    last_non_empty_line, parse_verdict_line, verdict_timed_out, verdict_unparseable,
};
use crate::session_backend::{BackendError, ValidationOutcome, ValidationRequest};

use super::OsbBackend;

/// Metadata key marking a sandbox as a throwaway validation holder (the reaper filters
/// on it). Deliberately NOT `fkst-managed`, so no session-facing filter matches it.
const VALIDATION_METADATA_KEY: &str = "fkst-validation";
/// Metadata key carrying the holder's creation epoch (decimal seconds), the reaper's
/// age signal.
const VALIDATION_CREATED_AT_KEY: &str = "fkst-created-at";
/// Wall-clock added beyond the holder's own deadline before the poll loop aborts a
/// holder whose command never finishes.
const WAIT_BUFFER_SECS: u64 = 30;
/// Age added beyond the deadline before the reaper deletes an orphaned holder.
const SWEEP_BUFFER_SECS: i64 = 30;
/// Extra lifetime added to the holder's server-side GC `timeout` beyond the
/// control-plane poll deadline, so a control-plane crash can't leak the holder forever.
const HOLDER_TIMEOUT_BUFFER_SECS: i64 = 120;
/// Mode the validate spec is uploaded with: world-readable (the in-holder validator
/// reads it), matching the Kubernetes ConfigMap's read-only mount.
const SPEC_FILE_MODE: u32 = 0o444;

impl OsbBackend {
    pub(super) async fn run_validation_impl(
        &self,
        req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError> {
        // A stable run id that (a) derives an execd token matching the create-env and
        // (b) is only ever used for that derivation + the execd factory — it is NOT a
        // sandbox id (the server assigns one) and never rides the correlation metadata.
        let run_id = format!("validation-{}-{}", req.github_user_id, req.name);
        let token = derive_execd_token(&self.config.execd_seed, &run_id);

        // 1. Create the throwaway HOLDER sandbox (a bare `sleep infinity` box) carrying
        //    the execd token on its create env.
        let now = Utc::now().timestamp();
        let mut env = BTreeMap::new();
        // Exposed only to place it on the create env; never logged.
        env.insert(
            self.config.execd_token_env_key.clone(),
            token.expose_secret().to_string(),
        );
        let mut metadata = BTreeMap::new();
        metadata.insert(VALIDATION_METADATA_KEY.to_string(), "true".to_string());
        metadata.insert(VALIDATION_CREATED_AT_KEY.to_string(), now.to_string());
        let request = CreateSandboxRequest {
            image: self.config.image.clone(),
            entrypoint: vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                "sleep infinity".to_string(),
            ],
            env,
            resource_limits: self.config.resource_limits.clone(),
            // The ONE non-null timeout fkst sends: the server-side GC backstop beyond
            // the control-plane poll deadline.
            timeout: Some(self.config.validate_deadline_secs + HOLDER_TIMEOUT_BUFFER_SECS),
            metadata,
            extensions: BTreeMap::new(),
        };
        let holder_id = self.lifecycle.create_sandbox(&request).await?.id;

        // 2. Arm cleanup IMMEDIATELY: from here EVERY exit path (success / failure /
        //    timeout / early error) deletes the holder on drop.
        let _cleanup = HolderCleanup {
            lifecycle: Arc::clone(&self.lifecycle),
            sandbox_id: holder_id.clone(),
        };

        // 3. Upload the validate spec, then run the SAME command the Kubernetes
        //    validation pod runs (`<entrypoint binary> validate-env`) through execd.
        let execd = (self.execd_factory)(&holder_id, &run_id);
        let spec = ValidateSpec {
            install: req.install.clone(),
            variables: req.variables.clone(),
            deadline_secs: u64::try_from(self.config.validate_deadline_secs).unwrap_or(0),
        };
        let spec_bytes = serde_json::to_vec(&spec)
            .map_err(|e| BackendError::Other(anyhow::anyhow!("serialize validate spec: {e}")))?;
        execd
            .upload_file(VALIDATE_SPEC_PATH, &spec_bytes, SPEC_FILE_MODE)
            .await?;

        let command = validator_command(&self.config.entrypoint);
        let timeout_ms = u64::try_from(self.config.validate_deadline_secs)
            .unwrap_or(0)
            .saturating_mul(1000);
        let cmd = execd.run_command(&command, Some(timeout_ms), false).await?;

        // 4. Poll the command to completion, bounded by a hard wall-clock timeout so a
        //    holder whose command never finishes still aborts → conservative timed-out
        //    `Failed`.
        let overall = Duration::from_secs(
            u64::try_from(self.config.validate_deadline_secs).unwrap_or(0) + WAIT_BUFFER_SECS,
        );
        let poll = Duration::from_secs(self.config.validate_poll_interval_secs.max(1));
        if tokio::time::timeout(overall, wait_for_finished(&execd, &cmd.id, poll))
            .await
            .is_err()
        {
            tracing::warn!(
                sandbox_id = %holder_id,
                "opensandbox run_validation: holder command did not finish before the deadline"
            );
            return Ok(verdict_timed_out());
        }

        // 5. Read the command output and parse the LAST line as the verdict. A totally
        //    unreadable log is an infra error (`?`); readable-but-unparseable is the
        //    conservative `Failed` — byte-identical to the Kubernetes `capture_outcome`.
        let (logs, _cursor) = execd.command_logs(&cmd.id, 0).await?;
        let outcome = match last_non_empty_line(&logs).and_then(parse_verdict_line) {
            Some(outcome) => {
                tracing::info!(sandbox_id = %holder_id, "opensandbox run_validation: verdict parsed");
                outcome
            }
            None => {
                tracing::warn!(
                    sandbox_id = %holder_id,
                    "opensandbox run_validation: no parseable verdict; treating as failed"
                );
                verdict_unparseable()
            }
        };
        Ok(outcome)
    }

    pub(super) async fn reap_stale_validations_impl(&self) -> Result<usize, BackendError> {
        let filter = vec![(VALIDATION_METADATA_KEY.to_string(), "true".to_string())];
        let holders = self.lifecycle.list_sandboxes(&filter).await?;
        let cutoff =
            Utc::now().timestamp() - (self.config.validate_deadline_secs + SWEEP_BUFFER_SECS);

        let mut deleted = 0usize;
        for holder in holders {
            let Some(created) = holder
                .metadata
                .get(VALIDATION_CREATED_AT_KEY)
                .and_then(|s| s.parse::<i64>().ok())
            else {
                tracing::warn!(
                    sandbox_id = %holder.id,
                    "opensandbox reap: validation holder has no parseable fkst-created-at; leaving for server GC"
                );
                continue;
            };
            if created >= cutoff {
                continue; // Still within its lifetime; a live run owns it.
            }
            match self.lifecycle.delete_sandbox(&holder.id).await {
                Ok(()) => {
                    deleted += 1;
                    tracing::info!(sandbox_id = %holder.id, "opensandbox reap: deleted orphaned validation holder");
                }
                // Already gone between list + delete: benign.
                Err(OsbError::NotFound) => {}
                Err(error) => {
                    tracing::warn!(sandbox_id = %holder.id, error = %error, "opensandbox reap: delete failed")
                }
            }
        }
        Ok(deleted)
    }
}

/// The command the holder runs: the configured entrypoint binary followed by the
/// shared `validate-env` subcommand — byte-identical to the Kubernetes validation pod's
/// `<image entrypoint> validate-env`. A drift-guard test pins the trailing subcommand.
fn validator_command(entrypoint: &[String]) -> String {
    let binary = entrypoint.first().map(String::as_str).unwrap_or_default();
    format!("{binary} {VALIDATE_ENV_SUBCOMMAND}")
}

/// Poll the command's status until it is finished, returning then. Transient poll
/// errors are logged + retried; the caller's outer `timeout` is the hard backstop
/// (mirrors the Kubernetes `wait_for_terminal_phase`).
async fn wait_for_finished(execd: &ExecdClient, command_id: &str, poll: Duration) {
    loop {
        match execd.command_status(command_id).await {
            Ok(status) if status.is_finished() => return,
            Ok(_) => {}
            Err(error) => {
                tracing::debug!(error = %error, command_id = %command_id, "opensandbox run_validation: status poll error; retrying");
            }
        }
        tokio::time::sleep(poll).await;
    }
}

/// Best-effort, fire-and-forget validation-holder deletion on drop — success, failure,
/// AND timeout all delete the holder (mirrors `k8s/validation.rs`'s `PodCleanup`). Drop
/// cannot be async, so it spawns the delete; if the process is shutting down and the
/// spawn never runs, the holder's server-side `timeout` GC + the reaper are the
/// backstops.
struct HolderCleanup {
    lifecycle: Arc<OsbLifecycleClient>,
    sandbox_id: String,
}

impl Drop for HolderCleanup {
    fn drop(&mut self) {
        let lifecycle = Arc::clone(&self.lifecycle);
        let sandbox_id = std::mem::take(&mut self.sandbox_id);
        tokio::spawn(async move {
            match lifecycle.delete_sandbox(&sandbox_id).await {
                Ok(()) => {
                    tracing::info!(sandbox_id = %sandbox_id, "opensandbox run_validation: cleanup deleted holder")
                }
                Err(OsbError::NotFound) => {}
                Err(error) => {
                    tracing::warn!(sandbox_id = %sandbox_id, error = %error, "opensandbox run_validation: cleanup delete failed")
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
