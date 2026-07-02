//! Best-effort recent-log capture for a running session pod (session health, PR).
//!
//! The package-agnostic session-health scrape ([`crate::k8s::health_scrape`]) needs
//! a bounded window of a pod's OWN framework logs to relay their severity verbatim.
//! This is the ONLY I/O seam for that: a thin wrapper over `kube`'s
//! `Api::<Pod>::logs` (RBAC already grants `pods/log` `get`).
//!
//! Discipline: never fatal. A `NotFound` (the pod was deleted between the LIST and
//! the read) is a benign EMPTY window (`Some("")`); any other transport error is a
//! `None` — the caller distinguishes "read clean, nothing to see" from "could not
//! read" so a transient failure never CLEARS a legitimately-degraded flag. Logs may
//! carry a package's own output, so nothing here is logged at a level that would
//! echo their content, and no credential is ever involved.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, LogParams};

use crate::k8s::client::KubeClient;

/// How many trailing log lines to pull per scrape. Enough to see several cycles of
/// a periodic warning (the recurrence signal) without paying for the whole history.
const TAIL_LINES: i64 = 600;

/// How far back (seconds) to bound the window. Pairs with [`TAIL_LINES`] so the
/// scan reflects only recent behaviour (~20 min), matching the "does this recur"
/// question rather than a pod's entire lifetime.
const SINCE_SECONDS: i64 = 1200;

/// Read a bounded tail of `name`'s recent logs in the client's namespace.
///
/// Returns `Some(logs)` on success (including `Some("")` for a pod with no output
/// or a `NotFound` deleted pod) and `None` when the logs could not be read at all
/// (a real transport error) — so the caller can withhold a health CLEAR it cannot
/// justify. Best-effort: never panics, never propagates.
pub async fn pod_recent_logs(kube: &KubeClient, name: &str) -> Option<String> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    let params = LogParams {
        tail_lines: Some(TAIL_LINES),
        since_seconds: Some(SINCE_SECONDS),
        ..Default::default()
    };
    match pods.logs(name, &params).await {
        Ok(logs) => Some(logs),
        // The pod vanished between the LIST and the read — an empty window, benign.
        Err(kube::Error::Api(e)) if e.code == 404 => Some(String::new()),
        Err(error) => {
            tracing::warn!(pod = %name, error = %error, "session health: pod log read failed (skipping this pod)");
            None
        }
    }
}
