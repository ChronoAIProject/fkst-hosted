//! The best-effort recent-output read (issue #419): `recent_output`, feeding the
//! package-agnostic session-health scrape's log-severity signal.
//!
//! ## Why this lives in its OWN module
//! It is the ONE verb that leans on a DEPRECATED upstream endpoint — the plain-text
//! `GET /v1/sandboxes/{id}/diagnostics/logs` (the structured `scope=`-param JSON
//! diagnostics endpoint currently answers `501`). Isolating it here keeps that
//! dependency in a single, clearly-labelled place: when the structured endpoint ships,
//! only [`OsbLifecycleClient::diagnostics_logs`](crate::session_backend::opensandbox::OsbLifecycleClient::diagnostics_logs)
//! and this module change; the 3-state taxonomy this verb feeds the scrape stays fixed.
//!
//! ## The 3-state WITHHOLD taxonomy (preserved from the Kubernetes backend)
//! `Some(text)` = read OK; `Some("")` = the sandbox is gone (a benign empty window);
//! `None` = the output could NOT be read at all (a transport/5xx error). The scrape
//! WITHHOLDS a health-clear on `None` — a `5xx` must map to `None`, NEVER `Some("")`,
//! so a transient read failure can never clear a legitimately-degraded flag.

use k8s_openapi::chrono::DateTime;

use crate::session_backend::BackendError;

use super::OsbBackend;

/// Trailing log lines requested per scrape. Matches the Kubernetes backend's 600 so
/// both see the same recurrence window for a periodic warning.
const TAIL_LINES: u32 = 600;
/// How far back the window reaches. `"20m"` mirrors the Kubernetes backend's 1200s
/// bound, keeping the scan to recent behaviour ("does this recur") on either runtime.
const SINCE: &str = "20m";
/// Client-side cap on the returned lines: even if the server ignores `tail`, the
/// scrape never processes more than the intended window.
const MAX_LINES: usize = 600;

impl OsbBackend {
    pub(super) async fn recent_output_impl(&self, session_id: &str) -> Option<String> {
        // Resolve the sandbox id first. A gone session is a benign empty window
        // (`Some("")`); any other resolve error is unreadable (`None`) so the scrape
        // withholds a clear it cannot justify.
        let view = match self.resolve_one(session_id).await {
            Ok(view) => view,
            Err(BackendError::NotFound) => return Some(String::new()),
            Err(_) => return None,
        };
        match self
            .lifecycle
            .diagnostics_logs(&view.id, TAIL_LINES, SINCE)
            .await
        {
            // Readable: strip each line's RFC3339 prefix and cap to the recent window.
            Ok(Some(text)) => Some(strip_and_cap(&text)),
            // 404: the sandbox / its logs are gone — a benign empty window.
            Ok(None) => Some(String::new()),
            // A transport/5xx error: unreadable → WITHHOLD (never a benign `Some("")`).
            Err(_) => None,
        }
    }
}

/// Strip each line's RFC3339 timestamp prefix, then keep only the last [`MAX_LINES`]
/// lines. Line-based (the scrape parses per-line), so the join separator is `\n`.
fn strip_and_cap(text: &str) -> String {
    let stripped: Vec<&str> = text.lines().map(strip_rfc3339_prefix).collect();
    let start = stripped.len().saturating_sub(MAX_LINES);
    stripped[start..].join("\n")
}

/// Strip a leading `"<RFC3339-timestamp> "` prefix from one log line, if present.
///
/// Only strips when the prefix GENUINELY parses as an RFC3339 timestamp; otherwise the
/// WHOLE line is returned untouched — a line that merely contains a space (real engine
/// output) is never destroyed.
pub(super) fn strip_rfc3339_prefix(line: &str) -> &str {
    match line.split_once(' ') {
        Some((prefix, rest)) if DateTime::parse_from_rfc3339(prefix).is_ok() => rest,
        _ => line,
    }
}

#[cfg(test)]
#[path = "logs_tests.rs"]
mod tests;
