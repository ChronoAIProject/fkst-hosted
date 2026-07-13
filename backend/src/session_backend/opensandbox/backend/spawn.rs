//! The create-side lifecycle verbs (issue #418): `check_reachable` + `ensure_session`.
//!
//! `ensure_session` is the load-bearing verb. It (a) list-guards so an already-live
//! session is an idempotent [`EnsureOutcome::AlreadyLive`] no-op; (b) creates the
//! sandbox with the derived execd token RIDING THE CREATE ENV (execd gates every
//! `/files/upload` on `X-EXECD-ACCESS-TOKEN`, read from env `EXECD_ACCESS_TOKEN`, so it
//! must exist before the first push); (c) pushes each credential file THROUGH execd,
//! writing the completeness sentinel LAST; and (d) ROLLS BACK (best-effort delete) on
//! any post-create failure so a half-provisioned sandbox never lingers.

use std::collections::BTreeMap;
use std::path::Path;

use secrecy::{ExposeSecret, SecretString};

use crate::k8s::session_launcher::session_env_pairs;
use crate::k8s::SessionPodSpec;
use crate::session_backend::opensandbox::derive_execd_token;
use crate::session_backend::opensandbox::dto::CreateSandboxRequest;
use crate::session_backend::{BackendError, EnsureOutcome};
use crate::session_pod::log_stream::{ENV_POD_NAME, ENV_POD_UID};
use crate::session_spec::creds::{CredsLayout, DEFAULT_CREDS_DIR};

use super::{correlate, managed_session_filter, OsbBackend};

/// The mode every credential file (and the sentinel) is uploaded with: owner-read-only
/// (0o400), matching the Kubernetes backend's Secret mount. `pub(super)` so the
/// rotation heal path ([`super::rotation`]) rewrites a single file with the SAME mode.
pub(super) const CREDS_FILE_MODE: u32 = 0o400;

/// The never-matching session id the reachability probe filters on (one empty page).
const REACHABILITY_PROBE_ID: &str = "__reachability_probe__";

impl OsbBackend {
    pub(super) async fn check_reachable_impl(&self) -> Result<String, BackendError> {
        // A never-matching filter returns one empty page — the cheapest reachability
        // probe that needs no new client method. Any transport/API error surfaces as
        // a `BackendError` via the blanket `OsbError` conversion.
        let filter = vec![(
            correlate::KEY_SESSION_ID.to_string(),
            REACHABILITY_PROBE_ID.to_string(),
        )];
        self.lifecycle.list_sandboxes(&filter).await?;
        Ok("opensandbox".to_string())
    }

    pub(super) async fn ensure_session_impl(
        &self,
        spec: &SessionPodSpec,
        creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError> {
        // (a) List-guard: a deterministically-correlated sandbox already exists → the
        // session is already live (idempotent no-op), mirroring the K8s 409 create.
        // This guard is ALSO the orphan absorber for a create whose CLIENT-side
        // request budget elapsed while the server still materialized the sandbox:
        // the orphan correlates by metadata and turns the next ensure into a no-op.
        let existing = self
            .lifecycle
            .list_sandboxes(&managed_session_filter(&spec.session_id))
            .await?;
        if !existing.is_empty() {
            tracing::info!(
                session_id = %spec.session_id,
                "opensandbox ensure_session: sandbox already exists; already-live no-op"
            );
            return Ok(EnsureOutcome::AlreadyLive);
        }

        // (b) Create. The execd token rides the create env so it is present before any
        // push; `timeout: None` serialises as a literal null ("no auto-expiry").
        let metadata = correlate::stamp(spec)?;
        let request = CreateSandboxRequest {
            image: self.config.image.clone(),
            entrypoint: self.config.entrypoint.clone(),
            env: self.create_env(spec),
            resource_limits: self.config.resource_limits.clone(),
            timeout: None,
            metadata,
            extensions: BTreeMap::new(),
        };
        let sandbox_id = self.lifecycle.create_sandbox(&request).await?.id;
        tracing::info!(
            session_id = %spec.session_id,
            sandbox_id = %sandbox_id,
            "opensandbox ensure_session: sandbox created; uploading credentials"
        );

        // (c)+(d) Upload creds (sentinel LAST); roll back on any failure. `creds` is
        // BORROWED for the upload so it can be MOVED into the cache after (a non-`Clone`
        // `SecretString` bundle is never cloned).
        if let Err(error) = self
            .upload_creds(&sandbox_id, &spec.session_id, &creds)
            .await
        {
            tracing::error!(
                session_id = %spec.session_id,
                sandbox_id = %sandbox_id,
                error = %error,
                "opensandbox ensure_session: credential upload failed; rolling back sandbox"
            );
            if let Err(rollback) = self.lifecycle.delete_sandbox(&sandbox_id).await {
                tracing::warn!(
                    session_id = %spec.session_id,
                    sandbox_id = %sandbox_id,
                    error = %rollback,
                    "opensandbox ensure_session: rollback delete failed (reaper will GC the leak)"
                );
            }
            return Err(error);
        }

        // Cache the full bundle so the rotation heal path can re-push it wholesale if a
        // container restart later wipes the creds dir. Only on a fresh create — NOT the
        // AlreadyLive early-return, which IS the documented control-plane-restart empty
        // state the next reconcile repopulates.
        self.creds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(spec.session_id.clone(), creds);
        Ok(EnsureOutcome::Created)
    }

    /// Assemble the sandbox create env: the shared [`session_env_pairs`] (identical to
    /// the pod path), then the two downward-API vars supplied as PLAIN values (a
    /// sandbox has no downward API, so both take the session id), then the derived
    /// execd token under the configured env key.
    fn create_env(&self, spec: &SessionPodSpec) -> BTreeMap<String, String> {
        let mut env: BTreeMap<String, String> = session_env_pairs(spec, &self.pod_config)
            .into_iter()
            .collect();
        env.insert(ENV_POD_NAME.to_string(), spec.session_id.clone());
        env.insert(ENV_POD_UID.to_string(), spec.session_id.clone());
        let token = derive_execd_token(&self.config.execd_seed, &spec.session_id);
        // Exposed only to place it on the create env; never logged.
        env.insert(
            self.config.execd_token_env_key.clone(),
            token.expose_secret().to_string(),
        );
        env
    }

    /// Push each credential file into the sandbox through execd, then the completeness
    /// sentinel LAST. Each byte value is exposed only to upload it and is NEVER logged.
    /// `pub(super)` so the rotation heal path ([`super::rotation`]) reuses the exact
    /// per-file + sentinel-LAST upload for its full-bundle re-push.
    pub(super) async fn upload_creds(
        &self,
        sandbox_id: &str,
        session_id: &str,
        creds: &BTreeMap<String, SecretString>,
    ) -> Result<(), BackendError> {
        let execd = (self.execd_factory)(sandbox_id, session_id);
        let layout = CredsLayout::new(DEFAULT_CREDS_DIR);
        for (file, secret) in creds {
            let path = path_str(&layout.base().join(file));
            execd
                .upload_file(&path, secret.expose_secret().as_bytes(), CREDS_FILE_MODE)
                .await?;
        }
        // The completeness sentinel LAST: only once every credential is on disk does
        // the in-pod engine-start gate pass.
        let sentinel = path_str(&layout.creds_complete());
        execd.upload_file(&sentinel, b"1", CREDS_FILE_MODE).await?;
        Ok(())
    }
}

/// Render a credential path as the string execd's `/files/upload` expects. Credential
/// paths are ASCII under [`DEFAULT_CREDS_DIR`]; the lossy fallback never triggers.
/// `pub(super)` so the rotation heal path composes single-file paths the same way.
pub(super) fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;
