//! Integration tests for the unified `POST /api/v1/goals/submit` endpoint (#178)
//! driven through the REAL `build_router(AppState)` via `tower::ServiceExt`.
//!
//! These exercise the end-to-end inline-source path (which reaches
//! `SessionService::create_for_goal`, hence needs a Mongo container) and so
//! self-skip when Docker is unavailable — keeping `cargo test` green on runners
//! without a daemon. The Docker-FREE behaviour (parsers, DTOs, `adopt_issue`,
//! the permission gate, secret redaction) is covered by the in-crate unit tests
//! in `src/routes/goals_submit.rs`, `src/goals/issue_parse.rs`, and
//! `src/goals/issue_store.rs`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::db::Db;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::error::AppError;
use fkst_control_plane::goals::issue_store::{IssueApi, IssuePatch};
use fkst_control_plane::goals::{GoalIssueStore, RepoRef};
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;

mod support;

const MONGO_TAG: &str = "7";

/// True when a Docker daemon answers `docker info`.
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// One recorded `create_issue` call: `(title, body, labels)`.
type Created = (String, String, Vec<String>);

/// A recording [`IssueApi`] that returns a fixed issue number — lets the test
/// assert the FILED body (which `GoalIssueStore::new(None)`'s noop cannot).
#[derive(Default)]
struct RecordingIssueApi {
    created: Mutex<Vec<Created>>,
    patched: Mutex<Vec<(u64, IssuePatch)>>,
}

#[async_trait]
impl IssueApi for RecordingIssueApi {
    async fn create_issue(
        &self,
        _repo: &RepoRef,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<u64, AppError> {
        self.created
            .lock()
            .unwrap()
            .push((title.to_string(), body.to_string(), labels.to_vec()));
        Ok(101)
    }

    async fn patch_issue(
        &self,
        _repo: &RepoRef,
        number: u64,
        patch: IssuePatch,
    ) -> Result<(), AppError> {
        self.patched.lock().unwrap().push((number, patch));
        Ok(())
    }
}

struct TestApp {
    _container: ContainerAsync<Mongo>,
    router: axum::Router,
    issue_api: Arc<RecordingIssueApi>,
}

/// A scripted Ornn transport for the pre-flight (#179) gate test: FIFO replies
/// keyed by a path substring, mirroring the in-crate `ornn` fakes.
struct FakeOrnnTransport {
    replies: Mutex<Vec<(String, u16, Value)>>,
}

impl FakeOrnnTransport {
    fn new(replies: Vec<(&str, u16, Value)>) -> Self {
        Self {
            replies: Mutex::new(
                replies
                    .into_iter()
                    .map(|(n, s, b)| (n.to_string(), s, b))
                    .collect(),
            ),
        }
    }
}

#[async_trait]
impl fkst_control_plane::ornn::OrnnTransport for FakeOrnnTransport {
    async fn proxy_get(
        &self,
        path: &str,
        _query: &[(&str, &str)],
        _user_token: &secrecy::SecretString,
    ) -> Result<fkst_control_plane::nyxid::ProxyResponse, AppError> {
        let mut queue = self.replies.lock().unwrap();
        let idx = queue
            .iter()
            .position(|(needle, _, _)| path.contains(needle.as_str()))
            .unwrap_or_else(|| panic!("no fake ornn reply for {path}"));
        let (_, status, body) = queue.remove(idx);
        Ok(fkst_control_plane::nyxid::ProxyResponse {
            status: axum::http::StatusCode::from_u16(status).unwrap(),
            headers: axum::http::HeaderMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    async fn download_direct(&self, _url: &str) -> Result<Vec<u8>, AppError> {
        unreachable!("preflight never downloads")
    }
}

async fn app() -> TestApp {
    app_with_ornn(None).await
}

/// Build the test app, optionally wiring a fake [`OrnnClient`] so the submit
/// pre-flight (#179) Ornn-availability gate can be exercised end-to-end.
async fn app_with_ornn(ornn: Option<fkst_control_plane::ornn::OrnnClient>) -> TestApp {
    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .expect("container port");
    let config = Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");

    let issue_api = Arc::new(RecordingIssueApi::default());
    let goals = GoalIssueStore::with_api(issue_api.clone());
    let sessions = SessionService::new(SessionRepo::new(&db), EngineConfig::default());
    let vault = support::test_vault(&db);
    let router = build_router(AppState {
        config,
        db: db.clone(),
        sessions,
        // Disabled => the dev AuthContext carries `fkst:admin`, which bypasses
        // both action gates and the object check, so the 202 path is reachable
        // without a live NyxID. (The 403/422 cases are unit-tested in-crate.)
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn,
    })
    .expect("router");

    TestApp {
        _container: container,
        router,
        issue_api,
    }
}

async fn post_submit(router: &axum::Router, body: &Value) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/goals/submit")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    (status, String::from_utf8(bytes.to_vec()).expect("utf-8"))
}

/// Inline source: 202, exactly one issue filed, the response carries the issue
/// locator, and the FILED issue body is the non-sensitive summary + marker —
/// never the goal prompt (`description`) and never any secret value.
#[tokio::test]
async fn inline_source_returns_202_and_files_one_non_sensitive_issue() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let app = app().await;

    let body = json!({
        "source": "inline",
        "goal": "SUPER-SECRET-PROMPT do the thing",
        "repo": { "owner": "acme", "name": "site" },
        "package_names": ["pkg-a", "pkg-b"],
        "secrets": [{ "key": "OPENAI_API_KEY", "value": "sk-LEAKY-SECRET" }]
    });

    let (status, response_body) = post_submit(&app.router, &body).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "inline submit returns 202: {response_body}"
    );

    let resp: Value = serde_json::from_str(&response_body).expect("json response");
    assert_eq!(resp["issue_number"], 101, "carries the filed issue number");
    assert_eq!(
        resp["issue_url"], "https://github.com/acme/site/issues/101",
        "composes the issue url"
    );
    assert_eq!(resp["session_status"], "pending");
    assert_eq!(resp["goal_status"], "triggered");
    assert!(resp["session_id"].is_string());

    // Exactly one issue filed; its body never leaks the prompt or the secret.
    let created = app.issue_api.created.lock().unwrap();
    assert_eq!(created.len(), 1, "inline files exactly one issue");
    let (_title, filed_body, labels) = &created[0];
    assert!(
        !filed_body.contains("SUPER-SECRET-PROMPT"),
        "the prompt must NEVER appear in the filed issue body: {filed_body}"
    );
    assert!(
        !filed_body.contains("sk-LEAKY-SECRET"),
        "a secret value must NEVER appear in the filed issue body: {filed_body}"
    );
    assert!(
        !filed_body.contains("OPENAI_API_KEY"),
        "a secret key must NEVER appear in the filed issue body: {filed_body}"
    );
    assert!(
        filed_body.contains("fkst-hosted:goal"),
        "the filed body carries the hidden server marker: {filed_body}"
    );
    assert!(
        labels.iter().any(|l| l == "fkst-hosted:goal"),
        "the filed issue carries the goal label: {labels:?}"
    );
}

/// A malformed inline repo URL is a 422 (the new parsers' contract), not a 400.
#[tokio::test]
async fn inline_malformed_repo_url_is_422() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let app = app().await;
    let body = json!({
        "source": "inline",
        "goal": "g",
        "repo": { "url": "https://gitlab.com/acme/site" },
        "package_names": ["pkg-a"]
    });
    let (status, response_body) = post_submit(&app.router, &body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "wrong-host repo url is 422: {response_body}"
    );
    // No issue is filed on a rejected request.
    assert!(app.issue_api.created.lock().unwrap().is_empty());
}

/// A malformed issue URL is a 422 before any GitHub fetch.
#[tokio::test]
async fn issue_source_malformed_url_is_422() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    let app = app().await;
    let body = json!({
        "source": "issue",
        "issue": { "url": "https://github.com/acme/site/pull/3" }
    });
    let (status, response_body) = post_submit(&app.router, &body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a /pull/ url is not a valid issue ref: {response_body}"
    );
}

/// Submit-time pre-flight gate (#179): an inline submission whose Ornn pin is
/// available only at a DIFFERENT version is rejected with an aggregated 422 that
/// names the bad pin — the gate runs before `create_for_goal`, so the session
/// spawn is never reached (the response is the 422, never the 202). With
/// `github_app: None` the package check is skipped, isolating the Ornn gate.
#[tokio::test]
async fn submit_with_unavailable_ornn_version_is_gated_with_422() {
    if !docker_available() {
        eprintln!("SKIP: docker unavailable");
        return;
    }
    // The catalog only offers `fmt@2.0`; the submission pins `fmt@1.0`.
    let transport = Arc::new(FakeOrnnTransport::new(vec![(
        "/skills/fmt/versions",
        200,
        json!({ "data": { "items": [ { "version": "2.0" } ] } }),
    )]));
    let ornn = fkst_control_plane::ornn::OrnnClient::new(transport);
    let app = app_with_ornn(Some(ornn)).await;

    let body = json!({
        "source": "inline",
        "goal": "do the thing",
        "repo": { "owner": "acme", "name": "site" },
        "package_names": ["pkg-a"],
        "ornn_skills": [{ "kind": "skill", "name": "fmt", "version": "1.0" }]
    });

    let (status, response_body) = post_submit(&app.router, &body).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an unavailable pin version must gate the submission with a 422 (not a 202): \
         {response_body}"
    );
    let resp: Value = serde_json::from_str(&response_body).expect("json");
    assert_eq!(resp["error"], "submission_invalid");
    assert_eq!(resp["ornn"][0]["name"], "fmt", "names the bad pin");
    assert_eq!(resp["ornn"][0]["version"], "1.0");
    // The response is the gate's 422, never the success body — proving the spawn
    // (`create_for_goal`) below the gate was never reached.
    assert!(
        resp.get("session_id").is_none(),
        "a gated submission must not carry a session id"
    );
}
