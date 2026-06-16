//! Unit suite for [`super::WorkerAgent`]. Kept in its own file (included via
//! `#[path]` from `agent.rs`) so the agent source stays under the 500-line
//! budget. `super::*` resolves to the `agent` module, so its imports are in
//! scope here.

use super::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `heartbeat` takes `&Arc<Self>` (a dispatch spawns a supervise loop holding an
/// `Arc<WorkerAgent>`), so the test agent is an `Arc`; `Arc` derefs to
/// `WorkerAgent` so the `&self` methods (`register`, `running_session_ids`, …)
/// still resolve through it.
fn agent(uri: String) -> Arc<WorkerAgent> {
    Arc::new(WorkerAgent::new(
        uri,
        SecretString::from("tok".to_string()),
        "w1".into(),
        4,
        "/tmp/e".into(),
    ))
}

#[tokio::test]
async fn register_sends_auth_header_and_parses_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/register"))
        .and(header(INTERNAL_AUTH_HEADER, "tok"))
        .respond_with(ResponseTemplate::new(200).set_body_json(RegisterResponse {
            accepted: true,
            heartbeat_interval_secs: 10,
            controller_protocol_version: PROTOCOL_VERSION,
        }))
        .mount(&server)
        .await;

    let resp = agent(server.uri()).register().await.expect("register");
    assert!(resp.accepted);
    assert_eq!(resp.heartbeat_interval_secs, 10);
}

#[tokio::test]
async fn register_fails_closed_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/register"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = agent(server.uri())
        .register()
        .await
        .expect_err("must fail closed");
    assert!(matches!(err, AgentError::Unauthorized));
}

#[tokio::test]
async fn heartbeat_releases_on_stop_session_control() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/heartbeat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(HeartbeatResponse {
            acknowledged: true,
            control: vec![ControlMessage::StopSession {
                session_id: "s1".into(),
                reason: "drain".into(),
            }],
        }))
        .mount(&server)
        .await;
    // The released call is best-effort; mount it so it succeeds.
    Mock::given(method("POST"))
        .and(path("/internal/v1/released"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let resp = agent(server.uri())
        .heartbeat(LifecycleState::Active)
        .await
        .expect("heartbeat");
    assert!(resp.acknowledged);
}

/// A `ResolvedDispatch` whose clone hits the hardcoded github.com URL fails the
/// spawn, but the worker must NOT crash: the heartbeat returns Ok, no session is
/// registered, and `running_sessions` stays empty. (The full spawn-success path
/// is covered offline in `engine::executor`'s tests via the injected fake
/// cloner.)
#[tokio::test]
async fn heartbeat_swallows_a_failing_dispatch_and_stays_up() {
    use fkst_shared::models::RepoRef;
    use fkst_shared::protocol::{CloneSpec, DispatchGoal, ResolvedDispatch};

    let dispatch = ResolvedDispatch {
        session_id: "s-dispatch".into(),
        worker_id: "w1".into(),
        fencing_id: 1,
        goal: DispatchGoal {
            goal_id: "00000000-0000-0000-0000-000000000000".into(),
            title: "t".into(),
            description: SecretString::from("p".to_string()),
            repo: RepoRef {
                owner: "acme".into(),
                name: "does-not-resolve-locally".into(),
            },
        },
        clone_spec: CloneSpec {
            repo: RepoRef {
                owner: "acme".into(),
                name: "does-not-resolve-locally".into(),
            },
            git_ref: "main".into(),
            package_roots: vec!["demo".into()],
        },
        github_token: SecretString::from("ghs_x".to_string()),
        github_token_expires_at_unix_ms: 1,
        env_profile: Default::default(),
        codex_config_toml: None,
        ornn: None,
        mint_nonce: SecretString::from("n".to_string()),
    };

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/heartbeat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(HeartbeatResponse {
            acknowledged: true,
            control: vec![ControlMessage::ResolvedDispatch(Box::new(dispatch))],
        }))
        .mount(&server)
        .await;

    let a = agent(server.uri());
    let resp = a
        .heartbeat(LifecycleState::Active)
        .await
        .expect("heartbeat returns Ok despite a failing dispatch");
    assert!(resp.acknowledged);
    // The failing clone left the registry empty; the worker is still alive.
    assert!(
        a.running_session_ids().is_empty(),
        "no session registered for a failed dispatch"
    );
}
