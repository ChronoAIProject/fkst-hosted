//! The credential-delivery / self-heal verb (issue #419): `deliver_credential`.
//!
//! Where the Kubernetes backend rewrites one key of a mounted Secret and lets the
//! volume projection propagate it, OpenSandbox has no such projection — the credential
//! files were PUSHED into the container through execd at spawn. So delivery here
//! probes the container first and heals accordingly:
//!
//! - the creds are intact → rewrite JUST the rotated file in place (and keep the cached
//!   bundle's copy fresh for any later full re-push);
//! - the creds were WIPED (container restart) → re-push the WHOLE cached bundle through
//!   the same per-file + sentinel-LAST upload `ensure_session` uses, so the in-pod
//!   engine-start gate can pass again;
//! - the bundle cache is ALSO empty (the control plane restarted too) → deliver only
//!   the rotated file and let the next `ensure_session` repopulate + fully heal.
//!
//! A gone session is the benign [`DeliveryOutcome::SessionGone`] the rotation loop
//! no-ops on; any other failure surfaces so the loop retries next tick (no internal
//! retry). The credential VALUE is never logged.

use secrecy::{ExposeSecret, SecretString};

use crate::session_backend::opensandbox::dto::OsbError;
use crate::session_backend::{BackendError, DeliveryOutcome};
use crate::session_spec::creds::{CredsLayout, DEFAULT_CREDS_DIR};

use super::spawn::{path_str, CREDS_FILE_MODE};
use super::OsbBackend;

impl OsbBackend {
    pub(super) async fn deliver_credential_impl(
        &self,
        session_id: &str,
        file: &str,
        contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        // Resolve the ONE sandbox for this session. A gone session is the benign
        // 404-equivalent the rotation loop treats as a no-op (a deleted runtime needs
        // no fresh token).
        let sandbox_id = match self.resolve_one(session_id).await {
            Ok(view) => view.id,
            Err(BackendError::NotFound) => return Ok(DeliveryOutcome::SessionGone),
            Err(error) => return Err(error),
        };

        let execd = (self.execd_factory)(&sandbox_id, session_id);
        let layout = CredsLayout::new(DEFAULT_CREDS_DIR);
        // Probe a canary credential file (the github token, always present in a healthy
        // bundle) to detect a container restart that wiped the mounted creds dir.
        let probe_path = path_str(&layout.github_token());
        match execd.file_info(&probe_path).await {
            // Files intact → rewrite JUST this file in place (truncate-rewrite) and keep
            // the cached bundle's copy fresh for any later full re-push.
            Ok(_) => {
                let path = path_str(&layout.base().join(file));
                execd
                    .upload_file(&path, contents.expose_secret().as_bytes(), CREDS_FILE_MODE)
                    .await?;
                self.update_cached_file(session_id, file, contents);
                Ok(DeliveryOutcome::Delivered)
            }
            // Canary gone → the container restarted and lost its creds; re-push the FULL
            // bundle so the engine-start gate can pass again.
            Err(OsbError::NotFound) => {
                self.repush_full_bundle(&sandbox_id, session_id, file, contents)
                    .await
            }
            // Any other probe failure: surface it (the rotation loop retries next tick —
            // no internal retry). Logs the session id but NEVER the token contents.
            Err(other) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %other,
                    "opensandbox deliver_credential: creds probe failed; rotation retries next tick"
                );
                Err(other.into())
            }
        }
    }

    /// Re-push the whole cached credential bundle (with the freshly-rotated `file`
    /// overlaid) through the SAME per-file + sentinel-LAST upload `ensure_session` uses.
    /// On a cache MISS (both control plane AND container restarted) deliver only the
    /// single rotated file and self-heal next reconcile once the cache repopulates.
    async fn repush_full_bundle(
        &self,
        sandbox_id: &str,
        session_id: &str,
        file: &str,
        contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        // Take the bundle OUT of the cache: `SecretString` is non-`Clone`, so it is
        // MOVED out, the fresh file overlaid, uploaded via a BORROW, then put back. The
        // lock is released before the upload await (never held across it).
        let bundle = self
            .creds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
        match bundle {
            Some(mut bundle) => {
                bundle.insert(file.to_string(), contents);
                let result = self.upload_creds(sandbox_id, session_id, &bundle).await;
                // Return the overlaid bundle to the cache regardless of the upload
                // outcome — it is the intended latest bundle for the next attempt.
                self.creds
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(session_id.to_string(), bundle);
                result?;
                tracing::warn!(
                    session_id = %session_id,
                    "opensandbox deliver_credential: container creds were wiped; re-pushed the full bundle"
                );
                Ok(DeliveryOutcome::Delivered)
            }
            None => {
                // Both restarted: no bundle to re-push. Deliver only the rotated file;
                // the full set self-heals on the next `ensure_session`.
                let execd = (self.execd_factory)(sandbox_id, session_id);
                let layout = CredsLayout::new(DEFAULT_CREDS_DIR);
                let path = path_str(&layout.base().join(file));
                execd
                    .upload_file(&path, contents.expose_secret().as_bytes(), CREDS_FILE_MODE)
                    .await?;
                tracing::warn!(
                    session_id = %session_id,
                    "opensandbox deliver_credential: container creds wiped AND bundle cache empty \
                     (control plane also restarted); delivered only the rotated file, full bundle \
                     heals next reconcile"
                );
                Ok(DeliveryOutcome::Delivered)
            }
        }
    }

    /// Overlay `contents` onto the cached bundle's `file` key so a later full re-push
    /// carries the freshly-rotated value. A cache MISS is a no-op — the next
    /// `ensure_session` repopulates the whole bundle.
    fn update_cached_file(&self, session_id: &str, file: &str, contents: SecretString) {
        let mut guard = self.creds.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(bundle) = guard.get_mut(session_id) {
            bundle.insert(file.to_string(), contents);
        }
    }
}

#[cfg(test)]
#[path = "rotation_tests.rs"]
mod tests;
