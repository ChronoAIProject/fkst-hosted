//! Transport-layer tests for [`OsbLifecycleClient`] (sibling `#[path]` module,
//! mirrors the repo's `github_app/api_tests.rs` wiremock style). Every mock asserts
//! the `OPEN-SANDBOX-API-KEY` header is present, so no verb can drop it.

use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::super::dto::{ImageSpec, RegistryAuth, ResourceLimits, SandboxState};
use super::*;

const API_KEY: &str = "osb_secret_key_abc123";

fn client(base: &str) -> OsbLifecycleClient {
    OsbLifecycleClient::new(
        reqwest::Url::parse(base).expect("base url"),
        SecretString::from(API_KEY.to_string()),
        reqwest::Client::new(),
    )
}

fn create_req() -> CreateSandboxRequest {
    CreateSandboxRequest {
        image: ImageSpec {
            uri: "python:3.11".to_string(),
            auth: Some(RegistryAuth {
                username: "svc".to_string(),
                password: "pat".to_string(),
            }),
        },
        entrypoint: vec!["python".to_string()],
        env: BTreeMap::new(),
        resource_limits: ResourceLimits(BTreeMap::from([("cpu".to_string(), "500m".to_string())])),
        timeout: None,
        metadata: BTreeMap::new(),
        extensions: BTreeMap::new(),
    }
}

#[tokio::test]
async fn create_sandbox_sends_api_key_and_literal_null_timeout_then_parses_202() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(body_partial_json(serde_json::json!({
            "image": { "uri": "python:3.11" },
            // The load-bearing assertion: `None` reached the wire as a literal null.
            "timeout": serde_json::Value::Null,
        })))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "id": "sbx-1",
            "status": { "state": "Running", "message": "provisioned" },
            "metadata": { "name": "demo" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let view = client(&server.uri())
        .create_sandbox(&create_req())
        .await
        .expect("created");
    assert_eq!(view.id, "sbx-1");
    assert_eq!(view.state, SandboxState::Running);
    assert_eq!(view.message.as_deref(), Some("provisioned"));
    assert_eq!(view.metadata.get("name").map(String::as_str), Some("demo"));
}

#[tokio::test]
async fn get_sandbox_reads_nested_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes/sbx-9"))
        .and(header(API_KEY_HEADER, API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sbx-9",
            "status": { "state": "Paused", "reason": "user_pause" },
            "metadata": {},
            "createdAt": "2026-07-09T00:00:00Z",
            "entrypoint": ["python"]
        })))
        .mount(&server)
        .await;

    let view = client(&server.uri())
        .get_sandbox("sbx-9")
        .await
        .expect("ok");
    assert_eq!(view.id, "sbx-9");
    assert_eq!(view.state, SandboxState::Paused);
    assert_eq!(view.reason.as_deref(), Some("user_pause"));
}

#[tokio::test]
async fn get_sandbox_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes/missing"))
        .and(header(API_KEY_HEADER, API_KEY))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .get_sandbox("missing")
        .await
        .expect_err("404");
    assert!(matches!(err, OsbError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn list_sandboxes_walks_all_pages_and_encodes_metadata_filter() {
    let server = MockServer::start().await;
    // Page 1: one item, another page follows. Also asserts the decoded filter value.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(query_param("page", "1"))
        .and(query_param("metadata", "project=Apollo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [ { "id": "s1", "status": { "state": "Running" } } ],
            "pagination": { "page": 1, "pageSize": 100, "totalItems": 2, "totalPages": 2, "hasNextPage": true }
        })))
        .expect(1)
        .mount(&server)
        .await;
    // Page 2: last page.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(query_param("page", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [ { "id": "s2", "status": { "state": "Paused" } } ],
            "pagination": { "page": 2, "pageSize": 100, "totalItems": 2, "totalPages": 2, "hasNextPage": false }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let filter = vec![("project".to_string(), "Apollo".to_string())];
    let out = client(&server.uri())
        .list_sandboxes(&filter)
        .await
        .expect("listed");
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].id, "s1");
    assert_eq!(out[1].id, "s2");

    // The raw query must url-encode `=` -> `%3D` inside the single `metadata` param.
    let requests = server.received_requests().await.expect("recorded requests");
    let page_one_query = requests
        .iter()
        .filter_map(|r| r.url.query())
        .find(|q| q.contains("page=1"))
        .expect("a page-1 request")
        .to_string();
    assert!(
        page_one_query.contains("metadata=project%3DApollo"),
        "raw query was: {page_one_query}"
    );
}

#[tokio::test]
async fn patch_metadata_sends_json_merge_body() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v1/sandboxes/sbx-3/metadata"))
        .and(header(API_KEY_HEADER, API_KEY))
        // RFC 7396 semantics, but the media type is plain application/json.
        .and(header("content-type", "application/json"))
        .and(body_partial_json(serde_json::json!({ "team": "platform" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "sbx-3",
            "status": { "state": "Running" },
            "metadata": { "team": "platform" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let meta = BTreeMap::from([("team".to_string(), "platform".to_string())]);
    client(&server.uri())
        .patch_metadata("sbx-3", &meta)
        .await
        .expect("patched");
}

#[tokio::test]
async fn patch_metadata_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v1/sandboxes/gone/metadata"))
        .and(header(API_KEY_HEADER, API_KEY))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .patch_metadata("gone", &BTreeMap::new())
        .await
        .expect_err("404");
    assert!(matches!(err, OsbError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn delete_sandbox_204_is_ok() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/sbx-4"))
        .and(header(API_KEY_HEADER, API_KEY))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client(&server.uri())
        .delete_sandbox("sbx-4")
        .await
        .expect("deleted");
}

#[tokio::test]
async fn delete_sandbox_404_is_not_found_not_swallowed() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/sbx-5"))
        .and(header(API_KEY_HEADER, API_KEY))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .delete_sandbox("sbx-5")
        .await
        .expect_err("404 must surface");
    assert!(matches!(err, OsbError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn non_2xx_maps_to_api_error_with_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes/boom"))
        .and(header(API_KEY_HEADER, API_KEY))
        .respond_with(ResponseTemplate::new(500).set_body_string("kaboom"))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .get_sandbox("boom")
        .await
        .expect_err("500");
    match err {
        OsbError::Api { status, message } => {
            assert_eq!(status, 500);
            assert!(message.contains("kaboom"), "message was {message}");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[test]
fn client_debug_never_leaks_the_api_key() {
    let debug = format!("{:?}", client("http://localhost:8080"));
    assert!(
        !debug.contains(API_KEY),
        "api key leaked in Debug output: {debug}"
    );
}
