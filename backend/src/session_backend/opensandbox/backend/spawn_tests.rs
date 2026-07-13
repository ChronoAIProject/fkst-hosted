//! Wiremock tests for the create-side verbs: `check_reachable` and `ensure_session`
//! (happy path with the literal-null timeout + stamped metadata + create-env execd
//! token, sentinel-LAST creds ordering, rollback-on-upload-failure, and the
//! list-guard AlreadyLive short-circuit).

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::session_backend::opensandbox::derive_execd_token;
use crate::session_backend::{BackendError, EnsureOutcome};

use super::super::backend_test_support::{
    backend, backend_with_pod_config, list_page, osb_config, sandbox_json, spec, API_KEY,
    EXECD_SEED, SESSION_ID,
};

const UPLOAD_PATH: &str = "/v1/sandboxes/sbx-1/proxy/44772/files/upload";

/// The create-env execd token the backend derives for [`SESSION_ID`].
fn expected_execd_token() -> String {
    derive_execd_token(&SecretString::from(EXECD_SEED.to_string()), SESSION_ID)
        .expose_secret()
        .to_string()
}

fn one_cred() -> BTreeMap<String, SecretString> {
    BTreeMap::from([(
        "github-token".to_string(),
        SecretString::from("ghs_secret_value".to_string()),
    )])
}

#[tokio::test]
async fn check_reachable_reports_opensandbox_on_an_empty_page() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(header("OPEN-SANDBOX-API-KEY", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let status = backend(&server.uri(), osb_config())
        .check_reachable_impl()
        .await
        .expect("reachable");
    assert_eq!(status, "opensandbox");
}

#[tokio::test]
async fn check_reachable_errors_on_a_non_success_status() {
    for code in [401u16, 500] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/sandboxes"))
            .respond_with(ResponseTemplate::new(code))
            .mount(&server)
            .await;

        let err = backend(&server.uri(), osb_config())
            .check_reachable_impl()
            .await
            .expect_err("unreachable");
        assert!(
            matches!(err, BackendError::Other(_)),
            "code {code}: {err:?}"
        );
    }
}

#[tokio::test]
async fn ensure_session_creates_with_null_timeout_stamped_metadata_and_execd_token() {
    let server = MockServer::start().await;
    // The list-guard finds nothing → proceed to create.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;
    // Create must carry the explicit entrypoint, a literal-null timeout, the stamped
    // correlation metadata, and the derived execd token on the create env.
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .and(header("OPEN-SANDBOX-API-KEY", API_KEY))
        .and(body_partial_json(json!({
            "entrypoint": ["run-substrate"],
            "timeout": serde_json::Value::Null,
            "metadata": {
                "fkst-managed": "true",
                "fkst-session-id": SESSION_ID,
                "fkst-owner": "acme",
                "fkst-repo": "site",
                "fkst-config-hash": "a".repeat(32),
                "fkst-config-hash-2": "a".repeat(32),
            },
            "env": {
                "EXECD_ACCESS_TOKEN": expected_execd_token(),
                // The shared `session_env_pairs` source must reach the sandbox
                // create env too — proven here via the engine HostFact pair.
                "FKST_CANDIDATE_PREFIX": "fkst-cand",
                "FKST_CANDIDATE_FROM_SEP": "--from--",
            },
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            "sbx-1",
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .expect(1)
        .mount(&server)
        .await;
    // Both credential uploads (the file + the sentinel) succeed.
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .and(header("X-EXECD-ACCESS-TOKEN", "execd-token"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let outcome = backend(&server.uri(), osb_config())
        .ensure_session_impl(&spec(), one_cred())
        .await
        .expect("created");
    assert_eq!(outcome, EnsureOutcome::Created);
}

#[tokio::test]
async fn ensure_session_renders_operator_rate_pools_on_the_create_env() {
    // The shared env source's rate-pool rendering must reach the sandbox create
    // env too (pool definition + the pinned ledger root).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .and(body_partial_json(json!({
            "env": {
                "FKST_RATE_POOL_GH": "50,50",
                "FKST_RATE_POOL_ROOT": "/var/run/fkst/rate-pools",
            },
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            "sbx-1",
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let pod_config = crate::config::PodConfig {
        rate_pools: BTreeMap::from([(
            "GH".to_string(),
            crate::config::RatePool {
                burst: 50,
                refill_per_minute: 50,
            },
        )]),
        ..crate::config::PodConfig::default()
    };
    let outcome = backend_with_pod_config(&server.uri(), osb_config(), pod_config)
        .ensure_session_impl(&spec(), one_cred())
        .await
        .expect("created");
    assert_eq!(outcome, EnsureOutcome::Created);
}

#[tokio::test]
async fn ensure_session_uploads_the_completeness_sentinel_last() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            "sbx-1",
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    backend(&server.uri(), osb_config())
        .ensure_session_impl(&spec(), one_cred())
        .await
        .expect("created");

    // Inspect the upload bodies in the order they arrived: the credential file first,
    // the completeness sentinel LAST (each body embeds its target path).
    let requests = server.received_requests().await.expect("recorded requests");
    let uploads: Vec<String> = requests
        .iter()
        .filter(|r| r.url.path() == UPLOAD_PATH)
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect();
    assert_eq!(uploads.len(), 2, "one credential file + the sentinel");
    assert!(
        uploads[0].contains("github-token"),
        "first upload is the cred"
    );
    assert!(
        !uploads[0].contains(".creds-complete"),
        "the sentinel must not be first"
    );
    assert!(
        uploads[1].contains(".creds-complete"),
        "the sentinel is uploaded LAST"
    );
}

#[tokio::test]
async fn ensure_session_rolls_back_the_sandbox_on_a_credential_upload_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            "sbx-1",
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .mount(&server)
        .await;
    // The first credential upload fails hard.
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    // The rollback DELETE of the half-provisioned sandbox is REQUIRED (expect 1).
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/sbx-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let err = backend(&server.uri(), osb_config())
        .ensure_session_impl(&spec(), one_cred())
        .await
        .expect_err("upload failure must surface");
    assert!(matches!(err, BackendError::Other(_)), "got {err:?}");
    // The `.expect(1)` on the DELETE mock is verified when `server` drops.
}

#[tokio::test]
async fn ensure_session_is_already_live_when_a_sandbox_exists() {
    let server = MockServer::start().await;
    // A managed sandbox already exists for this session → AlreadyLive, no create.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(list_page(json!([sandbox_json(
                "sbx-existing",
                "Running",
                "2026-07-09T00:00:00Z",
                json!({ "fkst-session-id": SESSION_ID }),
            )]))),
        )
        .mount(&server)
        .await;
    // No create mock: a create call would 404 (no match) and error, so reaching
    // AlreadyLive proves the create was short-circuited.

    let outcome = backend(&server.uri(), osb_config())
        .ensure_session_impl(&spec(), one_cred())
        .await
        .expect("already live");
    assert_eq!(outcome, EnsureOutcome::AlreadyLive);
}
