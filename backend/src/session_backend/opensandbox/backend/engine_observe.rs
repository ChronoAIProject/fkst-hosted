//! The OpenSandbox `engine_observe` verb (issue #473): run the engine's
//! observe read-model through execd and assemble its `--json` output.
//!
//! Execd retrieval is POLL-AND-ASSEMBLE, not streaming: `run_command`
//! deliberately discards the SSE output frames after the init frame, so the
//! output is recovered via `command_status` (until finished) + `command_logs`
//! (from cursor 0). The tail is COMBINED stdout+stderr raw text, while the
//! engine's `--json` snapshot is a pretty-printed, multi-line document. The
//! adapter therefore asks serde_json to find the largest COMPLETE JSON object
//! in the tail. This keeps surrounding tracing out without assuming the JSON is
//! compact or trying to balance braces by hand.

use std::time::Duration;

use serde_json::Value;

use crate::session_backend::k8s::classify_observe_failure;
use crate::session_backend::{BackendError, ObserveError};
use crate::session_pod::supervise::FRAMEWORK_BIN;

use super::spawn::OSB_DURABLE_ROOT;
use super::OsbBackend;

/// execd-side command timeout: the observe snapshot is a fast read-model dump;
/// 30s is generous while still bounding the poll below.
const OBSERVE_TIMEOUT_MS: u64 = 30_000;
/// Overall wall-clock bound on the status poll (timeout + slack).
const OVERALL_DEADLINE: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Match the Kubernetes adapter's output bound. Observe contains counts and
/// digests, never payload bodies, so anything larger is anomalous.
const OUTPUT_BYTE_CAP: usize = 4 * 1024 * 1024;

/// The shell command line execd runs. `limit` arrives pre-clamped (1..=10000);
/// every substituted token is a validated integer or a compile-time constant,
/// so no shell-quoting is needed.
fn observe_command(limit: u32) -> String {
    format!("{FRAMEWORK_BIN} observe --durable-root {OSB_DURABLE_ROOT} --json --limit {limit}")
}

/// Extract the engine snapshot from execd's combined stdout/stderr tail.
///
/// Every `{` is only a candidate start; serde_json decides whether a complete
/// object begins there and reports the exact consumed byte count. Selecting the
/// largest complete object picks the outer snapshot rather than one of its
/// nested queue/source objects. It also ignores compact JSON trace records that
/// may surround the snapshot.
fn extract_json_document(tail: &str) -> Option<&str> {
    tail.match_indices('{')
        .filter_map(|(start, _)| {
            let candidate = &tail[start..];
            let mut values = serde_json::Deserializer::from_str(candidate).into_iter::<Value>();
            match values.next() {
                Some(Ok(Value::Object(_))) => Some(&candidate[..values.byte_offset()]),
                _ => None,
            }
        })
        .max_by_key(|document| document.len())
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
        // Background launch: the status/logs poll below reads by id, which execd
        // permits only for a background command (a foreground command discards
        // its id and streams inline).
        let cmd = execd
            .run_command(&observe_command(limit), Some(OBSERVE_TIMEOUT_MS), true)
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

        if tail.len() > OUTPUT_BYTE_CAP {
            return Err(failed(format!(
                "observe output exceeded the {OUTPUT_BYTE_CAP}-byte limit"
            )));
        }

        if finished.exit_code == Some(0) {
            match extract_json_document(&tail) {
                Some(json) => {
                    tracing::info!(session_id = %session_id, limit, "engine observe: snapshot served");
                    Ok(json.to_string())
                }
                None => Err(failed(
                    "observe exited 0 but produced no complete JSON object".to_string(),
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
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::super::backend_test_support::{backend, list_page, osb_config, sandbox_json};
    use super::*;

    #[test]
    fn observe_command_uses_the_shared_constants_verbatim() {
        assert_eq!(
            observe_command(500),
            "/usr/local/bin/fkst-framework observe --durable-root /var/lib/fkst/durable --json --limit 500"
        );
    }

    #[tokio::test]
    async fn engine_observe_assembles_pretty_json_from_the_execd_tail() {
        // Full poll-and-assemble flow against execd doubles: resolve the
        // sandbox, launch the command (SSE init frame), poll to finished, read
        // the combined tail, and extract the complete multi-line snapshot.
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
            // The status/logs poll below reads by id, valid only for a background
            // command — pin it so a foreground regression fails here.
            .and(body_partial_json(json!({ "background": true })))
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
            .respond_with(ResponseTemplate::new(200).set_body_string(concat!(
                "{\"level\":\"info\",\"message\":\"observe: serving\"}\n",
                "{\n",
                "  \"schema_version\": 1,\n",
                "  \"source\": {\n",
                "    \"database\": \"/var/lib/fkst/durable/delivery.redb\"\n",
                "  },\n",
                "  \"queues\": [{ \"queue\": \"work.ready\", \"depth\": 0 }],\n",
                "  \"deliveries\": [],\n",
                "  \"dead_letters\": []\n",
                "}\n",
            )))
            .mount(&server)
            .await;

        let snapshot = backend(&server.uri(), osb_config())
            .engine_observe_impl("11111111-2222-3333-4444-555555555555", 500)
            .await
            .expect("snapshot assembles");
        let parsed: Value = serde_json::from_str(&snapshot).expect("snapshot is valid JSON");
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(
            parsed["source"]["database"],
            "/var/lib/fkst/durable/delivery.redb"
        );
        assert_eq!(parsed["queues"][0]["queue"], "work.ready");
        assert_eq!(parsed["deliveries"], json!([]));
        assert_eq!(parsed["dead_letters"], json!([]));
    }

    #[test]
    fn extraction_prefers_the_outer_document_over_nested_objects_and_trace_json() {
        let tail = concat!(
            "{\"level\":\"info\",\"message\":\"before\"}\n",
            "{\n",
            "  \"source\": {\"database\": \"delivery.redb\"},\n",
            "  \"queues\": [{\"queue\": \"q\"}],\n",
            "  \"deliveries\": [],\n",
            "  \"dead_letters\": []\n",
            "}\n",
        );
        let document = extract_json_document(tail).expect("complete document");
        let parsed: Value = serde_json::from_str(document).expect("valid JSON");
        assert_eq!(parsed["source"]["database"], "delivery.redb");
        assert_eq!(parsed["queues"][0]["queue"], "q");
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
