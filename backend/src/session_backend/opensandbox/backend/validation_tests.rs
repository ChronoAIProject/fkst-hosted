//! Wiremock tests for the env-validation verbs: the happy `ok` verdict, the
//! unparseable → conservative `Failed`, holder teardown on BOTH the success and the
//! failure paths, the reaper age filter, and the drift-guard pinning the validator
//! command to the shared `validate-env` subcommand.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::install::VALIDATE_ENV_SUBCOMMAND;
use crate::session_backend::{ValidationOutcome, ValidationRequest};

use super::super::backend_test_support::{backend, list_page, osb_config, sandbox_json};
use super::validator_command;

const HOLDER_ID: &str = "holder-1";

fn req() -> ValidationRequest {
    ValidationRequest {
        github_user_id: 42,
        name: "web".to_string(),
        install: vec!["apt-get update".to_string()],
        variables: BTreeMap::from([("FOO".to_string(), "bar".to_string())]),
    }
}

/// Mount the full holder run: create → spec upload → run command → status (finished) →
/// logs (`logs_body`) → delete (the drop-guard cleanup).
async fn mount_holder_flow(server: &MockServer, logs_body: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            HOLDER_ID,
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .mount(server)
        .await;
    // The exec-plane security gate: the probe's WRONG token is rejected (401) —
    // the enforced answer that lets the run proceed.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes/holder-1/proxy/44772/files/info"))
        .respond_with(ResponseTemplate::new(401))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes/holder-1/proxy/44772/files/upload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes/holder-1/proxy/44772/command"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("{\"type\":\"init\",\"text\":\"cmd-1\"}\n\n"),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/v1/sandboxes/holder-1/proxy/44772/command/status/cmd-1",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({ "id": "cmd-1", "running": false, "exit_code": 0 })),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/v1/sandboxes/holder-1/proxy/44772/command/cmd-1/logs",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(logs_body))
        .mount(server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/holder-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
}

/// Poll the recorded requests for the drop-guard's (spawned) holder DELETE. The exact
/// path `/v1/sandboxes/holder-1` is unique to the delete (create is `/v1/sandboxes`, the
/// exec calls ride `/proxy/44772`).
async fn holder_deleted(server: &MockServer) -> bool {
    for _ in 0..50 {
        let reqs = server.received_requests().await.unwrap_or_default();
        if reqs
            .iter()
            .any(|r| r.url.path() == "/v1/sandboxes/holder-1")
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn run_validation_parses_the_ok_verdict_and_tears_down() {
    let server = MockServer::start().await;
    mount_holder_flow(&server, "starting\n{\"status\":\"ok\",\"commands\":2}\n").await;

    let outcome = backend(&server.uri(), osb_config())
        .run_validation_impl(&req())
        .await
        .expect("validated");
    assert_eq!(outcome, ValidationOutcome::Passed { commands: 2 });
    assert!(
        holder_deleted(&server).await,
        "the holder is deleted on the success path"
    );
}

#[tokio::test]
async fn run_validation_treats_unparseable_output_as_failed_and_tears_down() {
    let server = MockServer::start().await;
    mount_holder_flow(&server, "starting up\nno verdict frame here\n").await;

    let outcome = backend(&server.uri(), osb_config())
        .run_validation_impl(&req())
        .await
        .expect("validated");
    // Readable-but-unparseable → the SHARED conservative Failed (byte-identical to K8s).
    assert_eq!(
        outcome,
        ValidationOutcome::Failed {
            failed_command_index: 0,
            failed_command: String::new(),
            exit_code: -1,
            timed_out: false,
            stderr_tail: "validation pod exceeded its limits".to_string(),
        }
    );
    assert!(
        holder_deleted(&server).await,
        "the holder is deleted on the failure path too"
    );
}

#[tokio::test]
async fn run_validation_fails_and_tears_down_when_execd_accepts_a_wrong_token() {
    // The security gate: a holder whose execd ACCEPTS the wrong-token probe (2xx)
    // is an unauthenticated exec surface — the run refuses to proceed with a
    // conservative Failed verdict naming the property, and the drop-guard still
    // deletes the holder. Nothing secret-adjacent (spec upload / command) runs.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            HOLDER_ID,
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes/holder-1/proxy/44772/files/info"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/holder-1"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let backend = backend(&server.uri(), osb_config());
    let outcome = backend
        .run_validation_impl(&req())
        .await
        .expect("a security refusal is a verdict, not an infra error");
    match outcome {
        ValidationOutcome::Failed { stderr_tail, .. } => {
            assert!(
                stderr_tail.contains("execd accepted an invalid access token"),
                "{stderr_tail}"
            );
        }
        other => panic!("expected the security Failed verdict, got {other:?}"),
    }

    // The drop-guard cleanup fires on this early-exit path too.
    assert!(
        holder_deleted(&server).await,
        "the holder is deleted after the security refusal"
    );
}

#[tokio::test]
async fn reap_deletes_only_holders_older_than_the_deadline() {
    let server = MockServer::start().await;
    let now = k8s_openapi::chrono::Utc::now().timestamp();
    let old = (now - 1000).to_string();
    let fresh = now.to_string();
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([
            sandbox_json(
                "holder-old",
                "Running",
                "2026-07-09T00:00:00Z",
                json!({ "fkst-validation": "true", "fkst-created-at": old }),
            ),
            sandbox_json(
                "holder-fresh",
                "Running",
                "2026-07-09T00:00:00Z",
                json!({ "fkst-validation": "true", "fkst-created-at": fresh }),
            ),
        ]))))
        .mount(&server)
        .await;
    // Only the STALE holder is deleted.
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/holder-old"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let deleted = backend(&server.uri(), osb_config())
        .reap_stale_validations_impl()
        .await
        .expect("reaped");
    assert_eq!(deleted, 1, "only the stale holder is reaped");

    // The fresh holder is never touched.
    let reqs = server.received_requests().await.expect("requests");
    assert!(
        !reqs.iter().any(|r| r.url.path().contains("holder-fresh")),
        "the fresh holder is left for its owning run"
    );
}

#[test]
fn validator_command_ends_with_the_shared_subcommand() {
    // The drift guard: the OSB validator command is `<entrypoint binary> validate-env`,
    // pinned to the SAME const the K8s validation pod arg + main.rs dispatch use.
    let cmd = validator_command(&["run-substrate".to_string()]);
    assert_eq!(cmd, format!("run-substrate {VALIDATE_ENV_SUBCOMMAND}"));
    assert!(cmd.ends_with(VALIDATE_ENV_SUBCOMMAND));
    // An empty entrypoint still yields the bare subcommand (never panics).
    assert_eq!(
        validator_command(&[]),
        format!(" {VALIDATE_ENV_SUBCOMMAND}")
    );
}
