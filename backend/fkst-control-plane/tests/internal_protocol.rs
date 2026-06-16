//! Integration tests for the internal controller<->worker protocol (#134):
//! the internal router's auth + behaviour, registry liveness/expiry, and a real
//! `WorkerAgent` driven against an in-process controller.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use secrecy::SecretString;
use tower::ServiceExt;

use fkst_control_plane::controller::{
    internal_router, ClaimMap, InternalAuth, SessionTokenMinter, WorkerRegistry,
};
use fkst_shared::protocol::{
    Draining, Heartbeat, HeartbeatResponse, LifecycleState, PullRequest, PullResponse,
    RegisterRequest, RegisterResponse, Released, INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};

const TOKEN: &str = "test-internal-token";

fn auth() -> InternalAuth {
    InternalAuth::new(SecretString::from(TOKEN.to_string()))
}

/// A fresh empty claim map for the controller's mid-run channels (#151). These
/// #134 transport tests do not exercise credential-refresh / status-report, so
/// an empty map + no minter preserves their original behaviour exactly.
fn empty_claims() -> Arc<ClaimMap> {
    Arc::new(ClaimMap::new())
}

/// No token minter — the credential-refresh route is not exercised here.
fn no_minter() -> Option<Arc<dyn SessionTokenMinter>> {
    None
}

/// No reassignment driver — these #134 transport tests do not exercise the
/// graceful-drain reassignment (#140), so the drain handlers log only, exactly
/// as before dispatch mode.
fn no_reassign() -> Option<Arc<fkst_control_plane::controller::ReassignDriver>> {
    None
}

/// Build a signed (auth-header-bearing) POST request to an internal route.
fn post<T: serde::Serialize>(uri: &str, body: &T) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(INTERNAL_AUTH_HEADER, TOKEN)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap()
}

async fn body_json<T: serde::de::DeserializeOwned>(resp: axum::response::Response) -> T {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn register_req(id: &str) -> RegisterRequest {
    RegisterRequest {
        worker_id: id.to_string(),
        protocol_version: PROTOCOL_VERSION,
        capacity: 4,
        engine_temp_root: "/tmp/e".to_string(),
    }
}

fn heartbeat(id: &str) -> Heartbeat {
    Heartbeat {
        worker_id: id.to_string(),
        protocol_version: PROTOCOL_VERSION,
        lifecycle_state: LifecycleState::Active,
        running_sessions: vec![],
        timestamp_unix_ms: 0,
    }
}

#[tokio::test]
async fn register_then_heartbeat_marks_worker_live() {
    let registry = WorkerRegistry::new(Duration::from_secs(10));
    let router = internal_router(
        registry.clone(),
        auth(),
        5,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );

    let resp = router
        .clone()
        .oneshot(post("/internal/v1/register", &register_req("w1")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: RegisterResponse = body_json(resp).await;
    assert!(body.accepted);
    assert_eq!(body.heartbeat_interval_secs, 5);
    assert_eq!(body.controller_protocol_version, PROTOCOL_VERSION);

    let resp = router
        .clone()
        .oneshot(post("/internal/v1/heartbeat", &heartbeat("w1")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let hb: HeartbeatResponse = body_json(resp).await;
    assert!(hb.acknowledged);
    assert!(hb.control.is_empty());

    let live = registry.live_workers().await;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].worker_id, "w1");
}

#[tokio::test]
async fn internal_route_without_auth_header_is_401() {
    let router = internal_router(
        WorkerRegistry::new(Duration::from_secs(10)),
        auth(),
        5,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );
    let req = Request::builder()
        .method("POST")
        .uri("/internal/v1/register")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&register_req("w1")).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_auth_header_is_401() {
    let router = internal_router(
        WorkerRegistry::new(Duration::from_secs(10)),
        auth(),
        5,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );
    let req = Request::builder()
        .method("POST")
        .uri("/internal/v1/register")
        .header(INTERNAL_AUTH_HEADER, "wrong-token")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&register_req("w1")).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn stale_worker_expires() {
    let registry = WorkerRegistry::new(Duration::from_millis(1));
    let router = internal_router(
        registry.clone(),
        auth(),
        5,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );
    router
        .oneshot(post("/internal/v1/register", &register_req("w1")))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let expired = registry.expire_stale().await;
    assert_eq!(expired, vec!["w1".to_string()]);
    assert!(registry.live_workers().await.is_empty());
}

#[tokio::test]
async fn controller_receives_draining_and_released() {
    let registry = WorkerRegistry::new(Duration::from_secs(10));
    let router = internal_router(
        registry.clone(),
        auth(),
        5,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );
    router
        .clone()
        .oneshot(post("/internal/v1/register", &register_req("w1")))
        .await
        .unwrap();

    let resp = router
        .clone()
        .oneshot(post(
            "/internal/v1/draining",
            &Draining {
                worker_id: "w1".to_string(),
                sessions: vec![],
                checkpoint_done: false,
            },
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        registry.live_workers().await[0].lifecycle_state,
        LifecycleState::Draining
    );

    let resp = router
        .oneshot(post(
            "/internal/v1/released",
            &Released {
                worker_id: "w1".to_string(),
                session_id: "s1".to_string(),
            },
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn pull_returns_no_assignments() {
    let router = internal_router(
        WorkerRegistry::new(Duration::from_secs(10)),
        auth(),
        5,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );
    let resp = router
        .oneshot(post(
            "/internal/v1/pull",
            &PullRequest {
                worker_id: "w1".to_string(),
                protocol_version: PROTOCOL_VERSION,
                available_capacity: 4,
            },
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: PullResponse = body_json(resp).await;
    assert!(body.assignments.is_empty(), "no claim authority yet (#135)");
}

#[tokio::test]
async fn worker_agent_register_and_heartbeat_against_in_process_controller() {
    let registry = WorkerRegistry::new(Duration::from_secs(10));
    let router = internal_router(
        registry.clone(),
        auth(),
        7,
        empty_claims(),
        no_minter(),
        no_reassign(),
    );

    // Serve the internal router on an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // Point a REAL WorkerAgent at it. `heartbeat` takes `&Arc<Self>` (a dispatch
    // spawns a supervise loop that holds an `Arc<WorkerAgent>`), so the agent is
    // an `Arc`; `Arc` derefs to `WorkerAgent` for the `&self` methods.
    let agent = Arc::new(fkst_worker::WorkerAgent::new(
        format!("http://{addr}"),
        SecretString::from(TOKEN.to_string()),
        "w-agent".to_string(),
        4,
        "/tmp/e".to_string(),
    ));

    let reg = agent.register().await.expect("register");
    assert!(reg.accepted);
    assert_eq!(reg.heartbeat_interval_secs, 7);

    let hb = agent
        .heartbeat(LifecycleState::Active)
        .await
        .expect("heartbeat");
    assert!(hb.acknowledged);

    let live = registry.live_workers().await;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].worker_id, "w-agent");
}
