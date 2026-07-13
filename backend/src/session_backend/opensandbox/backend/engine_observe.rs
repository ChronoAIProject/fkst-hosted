//! The OpenSandbox `engine_observe` verb (issue #473): run the engine's
//! observe read-model through execd and assemble its `--json` output.
//!
//! Execd retrieval is POLL-AND-ASSEMBLE, not streaming: `run_command`
//! deliberately discards the SSE output frames after the init frame, so the
//! output is recovered via `command_status` (until finished) + `command_logs`
//! (from cursor 0). The tail is COMBINED stdout+stderr raw text; the engine's
//! `--json` snapshot is one serde-compact line, so the LAST line that starts
//! with `{` is the snapshot (any interleaved tracing lines are their own
//! lines) — extraction, not parsing heuristics on partial fragments.

use std::time::Duration;

use crate::k8s::session_launcher::DURABLE_ROOT_DIR;
use crate::session_backend::k8s::classify_observe_failure;
use crate::session_backend::{BackendError, ObserveError};
use crate::session_pod::supervise::FRAMEWORK_BIN;

use super::OsbBackend;

/// execd-side command timeout: the observe snapshot is a fast read-model dump;
/// 30s is generous while still bounding the poll below.
const OBSERVE_TIMEOUT_MS: u64 = 30_000;
/// Overall wall-clock bound on the status poll (timeout + slack).
const OVERALL_DEADLINE: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The shell command line execd runs. `limit` arrives pre-clamped (1..=10000);
/// every substituted token is a validated integer or a compile-time constant,
/// so no shell-quoting is needed.
fn observe_command(limit: u32) -> String {
    format!("{FRAMEWORK_BIN} observe --durable-root {DURABLE_ROOT_DIR} --json --limit {limit}")
}

impl OsbBackend {
    pub(super) async fn engine_observe_impl(
        &self,
        session_id: &str,
        limit: u32,
    ) -> Result<String, ObserveError> {
        let view = self.resolve_one(session_id).await.map_err(|e| match e {
            BackendError::NotFound => ObserveError::SessionNotFound,
            BackendError::Other(other) => ObserveError::Failed(other.to_string()),
        })?;
        let execd = (self.execd_factory)(&view.id, session_id);

        let failed = |detail: String| ObserveError::Failed(detail);
        let cmd = execd
            .run_command(&observe_command(limit), Some(OBSERVE_TIMEOUT_MS), false)
            .await
            .map_err(|e| failed(format!("observe run_command: {e}")))?;

        // Bounded poll to a terminal status; a command that never finishes
        // inside the deadline is a failure, never a hang.
        let finished = tokio::time::timeout(OVERALL_DEADLINE, async {
            loop {
                match execd.command_status(&cmd.id).await {
                    Ok(status) if !status.running => return status,
                    Ok(_) => {}
                    Err(error) => {
                        tracing::debug!(error = %error, "engine observe: status poll error; retrying");
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        })
        .await
        .map_err(|_| failed("observe command did not finish before the deadline".to_string()))?;

        let (tail, _cursor) = execd
            .command_logs(&cmd.id, 0)
            .await
            .map_err(|e| failed(format!("observe command_logs: {e}")))?;

        if finished.exit_code == Some(0) {
            // The snapshot is the LAST `{`-prefixed line of the combined tail.
            match tail
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with('{'))
            {
                Some(json) => {
                    tracing::info!(session_id = %session_id, limit, "engine observe: snapshot served");
                    Ok(json.to_string())
                }
                None => Err(failed(
                    "observe exited 0 but produced no JSON line".to_string(),
                )),
            }
        } else {
            Err(classify_observe_failure(session_id, &tail))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::backend_test_support::{backend, list_page, osb_config, sandbox_json};
    use super::*;

    #[test]
    fn observe_command_uses_the_shared_constants_verbatim() {
        assert_eq!(
            observe_command(500),
            "/usr/local/bin/fkst-framework observe --durable-root /var/run/fkst/durable --json --limit 500"
        );
    }

    #[tokio::test]
    async fn engine_observe_assembles_the_json_line_from_the_execd_tail() {
        // Full poll-and-assemble flow against execd doubles: resolve the
        // sandbox, launch the command (SSE init frame), poll to finished, read
        // the combined tail, and extract the LAST `{`-prefixed line.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/sandboxes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([
                sandbox_json("sbx-1", "Running", "2026-07-09T00:00:00Z", json!({}))
            ]))))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/sandboxes/sbx-1/proxy/44772/command"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("data: {\"type\":\"init\",\"text\":\"cmd-9\"}\n\n"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/sandboxes/sbx-1/proxy/44772/command/status/cmd-9"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "cmd-9", "running": false, "exit_code": 0
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/sandboxes/sbx-1/proxy/44772/command/cmd-9/logs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    "2026-07-13T00:00:00Z INFO observe: serving\n{\"queues\":[]}\n",
                ),
            )
            .mount(&server)
            .await;

        let snapshot = backend(&server.uri(), osb_config())
            .engine_observe_impl("11111111-2222-3333-4444-555555555555", 500)
            .await
            .expect("snapshot assembles");
        assert_eq!(snapshot, "{\"queues\":[]}");
    }

    #[tokio::test]
    async fn engine_observe_maps_an_absent_sandbox_to_session_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/sandboxes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
            .mount(&server)
            .await;
        let err = backend(&server.uri(), osb_config())
            .engine_observe_impl("11111111-2222-3333-4444-555555555555", 500)
            .await
            .expect_err("absent sandbox");
        assert!(matches!(err, ObserveError::SessionNotFound));
    }
}
