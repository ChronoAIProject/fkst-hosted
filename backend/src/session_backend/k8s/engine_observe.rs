//! The Kubernetes `engine_observe` verb (issue #473): exec the engine's
//! observe read-model inside the session pod's `runner` container over
//! pods/exec (kube `ws` feature) and return its `--json` stdout.
//!
//! The argv references the launcher's OWN constants ([`FRAMEWORK_BIN`],
//! [`DURABLE_ROOT_DIR`], [`RUNNER_CONTAINER`]) — never second literals: the
//! engine derives its live-observe socket path from an FNV hash of the
//! durable-root string AS GIVEN, so any drift (a trailing slash) silently
//! degrades every call into a redb-lock error against the live supervise.
//!
//! RBAC: `pods/exec` (create) rides the CONTROL-PLANE Role only; the session
//! pod stays zero-RBAC and `automountServiceAccountToken: false` does not
//! block API-server-driven exec.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams};
use tokio::io::AsyncReadExt;

use crate::k8s::session_launcher::{session_object_name, DURABLE_ROOT_DIR, RUNNER_CONTAINER};
use crate::session_backend::{ObserveError, ENGINE_OBSERVE_NO_STORE_MARKER};
use crate::session_pod::supervise::FRAMEWORK_BIN;

use super::K8sBackend;

/// Cap on the collected stdout/stderr (the engine's observe snapshot is small —
/// counts + digests, never payload bodies — so 4 MiB is a generous ceiling that
/// still bounds a runaway stream).
const OUTPUT_BYTE_CAP: usize = 4 * 1024 * 1024;

/// The exec argv. `limit` arrives pre-clamped to the engine's 1..=10000.
fn observe_argv(limit: u32) -> Vec<String> {
    vec![
        FRAMEWORK_BIN.to_string(),
        "observe".to_string(),
        "--durable-root".to_string(),
        DURABLE_ROOT_DIR.to_string(),
        "--json".to_string(),
        "--limit".to_string(),
        limit.to_string(),
    ]
}

impl K8sBackend {
    pub(super) async fn engine_observe_impl(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<String, ObserveError> {
        let pods: Api<Pod> = self.pods_api();
        let pod_name = session_object_name(session_id);
        let params = AttachParams::default()
            .container(RUNNER_CONTAINER)
            .stdout(true)
            .stderr(true);

        let mut attached = pods
            .exec(&pod_name, observe_argv(limit), &params)
            .await
            .map_err(|error| match &error {
                kube::Error::Api(api) if api.code == 404 => ObserveError::SessionNotFound,
                other => ObserveError::Failed(format!("pods/exec {pod_name}: {other}")),
            })?;

        let stdout = read_capped(attached.stdout()).await;
        let stderr = read_capped(attached.stderr()).await;
        // The exec status rides a kube Status object on the channel: `Success`
        // means exit 0; anything else is a non-zero exit or transport failure.
        let status = match attached.take_status() {
            Some(status) => status.await,
            None => None,
        };
        let succeeded = status
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .is_some_and(|s| s == "Success");

        if succeeded {
            tracing::info!(session_id = %session_id, limit, "engine observe: snapshot served");
            return Ok(stdout);
        }
        Err(classify_failure(session_id, &stderr))
    }
}

/// Read an optional exec stream to the byte cap, lossily UTF-8 decoded.
async fn read_capped(stream: Option<impl tokio::io::AsyncRead + Unpin>) -> String {
    let Some(stream) = stream else {
        return String::new();
    };
    let mut buf = Vec::new();
    let mut capped = stream.take(OUTPUT_BYTE_CAP as u64);
    if let Err(error) = capped.read_to_end(&mut buf).await {
        tracing::warn!(error = %error, "engine observe: exec stream read failed");
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Classify a non-zero observe exit: the engine's no-durable-store message is
/// a distinct, expected state (→ 409 at the route); anything else surfaces as
/// a generic failure carrying a TRUNCATED, token-free stderr excerpt for logs.
pub(crate) fn classify_failure(session_id: &str, stderr: &str) -> ObserveError {
    if stderr.contains(ENGINE_OBSERVE_NO_STORE_MARKER) {
        tracing::info!(session_id = %session_id, "engine observe: session has no durable store");
        return ObserveError::NoDurableStore;
    }
    let excerpt: String = stderr.chars().take(300).collect();
    tracing::warn!(session_id = %session_id, stderr = %excerpt, "engine observe: exec failed");
    ObserveError::Failed(format!("engine observe exited non-zero: {excerpt}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_argv_uses_the_launcher_constants_verbatim() {
        // Load-bearing: the durable-root string must byte-match the launcher's
        // (the engine socket path is an FNV hash of the string AS GIVEN).
        assert_eq!(
            observe_argv(500),
            vec![
                "/usr/local/bin/fkst-framework",
                "observe",
                "--durable-root",
                "/var/run/fkst/durable",
                "--json",
                "--limit",
                "500",
            ]
        );
    }

    #[test]
    fn classify_failure_maps_the_no_store_marker_to_its_own_variant() {
        let err = classify_failure(
            "sid",
            "error: open existing durable delivery database /var/run/fkst/durable/delivery.redb",
        );
        assert!(matches!(err, ObserveError::NoDurableStore));
        let err = classify_failure("sid", "some other engine failure");
        assert!(matches!(err, ObserveError::Failed(_)));
    }
}
