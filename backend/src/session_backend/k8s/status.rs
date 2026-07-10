//! The status + recent-output verbs (issue #413), feeding the package-agnostic
//! health scrape. `status_summary` GETs a session pod and reuses the UNCHANGED
//! [`summarize_pod_status`] projection; `recent_output` is the best-effort log tail
//! moved verbatim from `k8s/pod_logs.rs` — its EXACT 3-state taxonomy (read /
//! gone-empty / unreadable) and warn log are preserved so a transient failure never
//! clears a legitimately-degraded flag.

use kube::api::LogParams;

use crate::k8s::health_eval::summarize_pod_status;
use crate::k8s::session_object_name;

use super::super::{BackendError, RuntimeStatus};
use super::K8sBackend;

/// How many trailing log lines to pull per scrape. Enough to see several cycles of
/// a periodic warning (the recurrence signal) without paying for the whole history.
const TAIL_LINES: i64 = 600;

/// How far back (seconds) to bound the window. Pairs with [`TAIL_LINES`] so the
/// scan reflects only recent behaviour (~20 min), matching the "does this recur"
/// question rather than a pod's entire lifetime.
const SINCE_SECONDS: i64 = 1200;

impl K8sBackend {
    pub(super) async fn status_summary_impl(
        &self,
        session_id: &str,
    ) -> Result<RuntimeStatus, BackendError> {
        let name = session_object_name(session_id);
        match self.pods_api().get(&name).await {
            Ok(pod) => {
                let summary = summarize_pod_status(&pod);
                // Adapt the signed pod restart count to the kube-free `Option<u32>`
                // OUTSIDE the pure health evaluator (a real pod count is never
                // negative, so the conversion never loses information).
                Ok(RuntimeStatus {
                    phase: summary.phase,
                    restart_count: Some(u32::try_from(summary.restart_count).unwrap_or(0)),
                    stall_reason: summary.waiting_reason,
                })
            }
            // The pod vanished between the fleet LIST and this GET — treat it as an
            // empty status (nothing to see), matching the old "pod gone" handling.
            Err(kube::Error::Api(e)) if e.code == 404 => Ok(RuntimeStatus::default()),
            Err(error) => Err(BackendError::Other(anyhow::Error::new(error))),
        }
    }

    pub(super) async fn recent_output_impl(&self, session_id: &str) -> Option<String> {
        let name = session_object_name(session_id);
        let params = LogParams {
            tail_lines: Some(TAIL_LINES),
            since_seconds: Some(SINCE_SECONDS),
            ..Default::default()
        };
        match self.pods_api().logs(&name, &params).await {
            Ok(logs) => Some(logs),
            // The pod vanished between the LIST and the read — an empty window, benign.
            Err(kube::Error::Api(e)) if e.code == 404 => Some(String::new()),
            Err(error) => {
                tracing::warn!(pod = %name, error = %error, "session health: pod log read failed (skipping this pod)");
                None
            }
        }
    }
}
