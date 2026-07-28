//! Tests for the read-only tools (sibling `#[path]` module).
//!
//! The URLs a tool builds are the security-relevant part — they are assembled from
//! MODEL-supplied arguments, so a value carrying a slash or a space must not be able
//! to reach a different endpoint. Those assertions run against the real router
//! through a recording probe, so the asserted path is the one that would actually be
//! dispatched.

use super::super::default_registry;
use super::*;
use crate::chat::dispatch::SelfDispatch;
use crate::routes::logs::test_support::{
    github_user_401, log_config, registry, state, storage_server,
};
use crate::state::empty_self_router;

/// A `ToolCtx` over a router that rejects every token.
///
/// Every request 401s, which is exactly what these tests want: the assertion is on
/// the URL the tool built and on the status arriving as DATA, not on endpoint
/// behaviour (that belongs to each endpoint's own suite).
async fn rejecting_ctx() -> (ToolCtx, wiremock::MockServer) {
    let gh = github_user_401().await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );
    crate::routes::logs::identity::clear_cache();
    let _router = crate::router::build_router(st.clone()).expect("router builds");
    (
        ToolCtx {
            dispatch: SelfDispatch::new(st.self_router.clone()),
            bearer: secrecy::SecretString::from("gho_rejected".to_string()),
            broader: None,
        },
        gh,
    )
}

/// A `ToolCtx` whose router handle is empty, so every dispatch fails immediately.
///
/// Used for pure argument-validation cases: they must be rejected BEFORE any
/// dispatch is attempted, so reaching the transport at all is itself a failure.
fn undispatchable_ctx() -> ToolCtx {
    ToolCtx {
        dispatch: SelfDispatch::new(empty_self_router()),
        bearer: secrecy::SecretString::from("gho_x".to_string()),
        broader: None,
    }
}

/// Invoke a tool on the shipped registry.
async fn invoke(
    ctx: &ToolCtx,
    name: &str,
    args: serde_json::Value,
) -> Result<ToolOutcome, ToolError> {
    default_registry().invoke(name, ctx, args).await
}

// ---- registry surface -----------------------------------------------------

#[test]
fn the_default_registry_exposes_exactly_the_eight_read_tools() {
    let registry = default_registry();
    let names: Vec<String> = registry.defs().into_iter().map(|d| d.name).collect();
    assert_eq!(
        names,
        vec![
            "get_overview",
            "list_repo_sessions",
            "get_session_outcomes",
            "observe_session",
            "list_log_runs",
            "get_log_manifest",
            "tail_log_file",
            "list_environment_profiles",
        ]
    );
}

#[test]
fn every_tool_declares_a_closed_object_schema_and_a_description() {
    for def in default_registry().defs() {
        assert_eq!(def.parameters["type"], "object", "{}", def.name);
        assert_eq!(
            def.parameters["additionalProperties"], false,
            "{} must reject unknown arguments",
            def.name
        );
        assert!(
            def.description.len() > 40,
            "{} needs a description the model can route on",
            def.name
        );
    }
}

#[tokio::test]
async fn an_unknown_tool_name_is_reported_not_fatal() {
    let ctx = undispatchable_ctx();
    let error = invoke(&ctx, "delete_everything", serde_json::json!({}))
        .await
        .expect_err("an unregistered name must be rejected");
    match error {
        ToolError::UnknownTool(name) => assert_eq!(name, "delete_everything"),
        other => panic!("expected UnknownTool, got {other:?}"),
    }
}

// ---- argument validation --------------------------------------------------

#[tokio::test]
async fn a_missing_required_argument_names_the_argument() {
    let ctx = undispatchable_ctx();
    let error = invoke(
        &ctx,
        "list_repo_sessions",
        serde_json::json!({"owner": "acme"}),
    )
    .await
    .expect_err("a missing name must be rejected");
    match error {
        ToolError::InvalidArgs(message) => assert!(
            message.contains("name"),
            "the message must name the argument: {message}"
        ),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}

#[tokio::test]
async fn a_mistyped_required_argument_is_rejected() {
    let ctx = undispatchable_ctx();
    let error = invoke(
        &ctx,
        "list_repo_sessions",
        serde_json::json!({"owner": 7, "name": "site"}),
    )
    .await
    .expect_err("a numeric owner must be rejected");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}

#[tokio::test]
async fn a_blank_required_argument_is_rejected() {
    let ctx = undispatchable_ctx();
    let error = invoke(
        &ctx,
        "list_log_runs",
        serde_json::json!({"session_id": "   "}),
    )
    .await
    .expect_err("a blank session id must be rejected");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}

#[test]
fn optional_arguments_treat_null_and_blank_as_absent() {
    let args = serde_json::json!({ "run": serde_json::Value::Null, "other": "  " });
    assert_eq!(optional_str(&args, "run").expect("parses"), None);
    assert_eq!(optional_str(&args, "other").expect("parses"), None);
    assert_eq!(optional_str(&args, "missing").expect("parses"), None);
}

#[test]
fn clamped_numbers_are_pulled_into_range_rather_than_rejected() {
    // A model asking for everything wants the maximum, not an error to recover from.
    let args = serde_json::json!({ "big": 10_000_000, "small": 0, "float": 42.9, "negative": -5 });
    assert_eq!(
        optional_clamped_u64(&args, "big", 1, 65_536).expect("parses"),
        Some(65_536)
    );
    assert_eq!(
        optional_clamped_u64(&args, "small", 1, 65_536).expect("parses"),
        Some(1)
    );
    assert_eq!(
        optional_clamped_u64(&args, "float", 1, 65_536).expect("parses"),
        Some(42)
    );
    assert_eq!(
        optional_clamped_u64(&args, "negative", 1, 65_536).expect("parses"),
        Some(1)
    );
    assert!(
        optional_clamped_u64(&serde_json::json!({"n": "abc"}), "n", 1, 10).is_err(),
        "a non-numeric value is still an error"
    );
}

#[test]
fn required_integers_accept_a_numeric_string() {
    // Models routinely quote numbers; refusing that would be a needless retry.
    assert_eq!(
        required_i64(&serde_json::json!({"issue_number": "42"}), "issue_number").expect("parses"),
        42
    );
    assert_eq!(
        required_i64(&serde_json::json!({"issue_number": 42}), "issue_number").expect("parses"),
        42
    );
    assert!(required_i64(&serde_json::json!({"issue_number": "x"}), "issue_number").is_err());
}

// ---- url encoding ---------------------------------------------------------

#[test]
fn dynamic_components_are_percent_encoded() {
    // The load-bearing cases: a space must not break the URL, and a slash must not
    // let a model-supplied value escape its path segment.
    assert_eq!(encode("sess 1"), "sess%201");
    assert_eq!(encode("a/b"), "a%2Fb");
    assert_eq!(encode("../../admin"), "..%2F..%2Fadmin");
    assert_eq!(encode("run?x=1"), "run%3Fx%3D1");
    assert_eq!(encode("a&b=c"), "a%26b%3Dc");
    // RFC 3986 unreserved characters stay legible.
    assert_eq!(encode("run-1_2.3~4"), "run-1_2.3~4");
}

/// A recorded dispatched request: what a tool actually put on the wire.
type Recorded = std::sync::Arc<std::sync::Mutex<Vec<(String, Option<String>)>>>;

/// A `ToolCtx` whose "router" records every request instead of serving it.
///
/// This asserts the exact thing that matters and cannot be observed through the
/// real router (whose handlers reject before echoing anything back): the URI a tool
/// built from model-supplied arguments, and whether it forwarded the
/// broader-visibility header.
fn probe_ctx(broader: Option<&str>) -> (ToolCtx, Recorded) {
    use axum::body::Body;
    use axum::http::Request;

    let recorded: Recorded = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = recorded.clone();
    let router = axum::Router::new().fallback(move |request: Request<Body>| {
        let sink = sink.clone();
        async move {
            let broader = request
                .headers()
                .get(crate::routes::canvas::BROADER_TOKEN_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            sink.lock()
                .expect("probe sink is not poisoned")
                .push((request.uri().to_string(), broader));
            axum::Json(serde_json::json!({ "recorded": true }))
        }
    });
    let handle = empty_self_router();
    handle.set(router).expect("fresh handle");
    (
        ToolCtx {
            dispatch: SelfDispatch::new(handle),
            bearer: secrecy::SecretString::from("gho_probe".to_string()),
            broader: broader.map(|t| secrecy::SecretString::from(t.to_string())),
        },
        recorded,
    )
}

/// Invoke one tool against the probe and return the URI it dispatched.
async fn dispatched_uri(name: &str, args: serde_json::Value) -> String {
    let (ctx, recorded) = probe_ctx(None);
    invoke(&ctx, name, args)
        .await
        .unwrap_or_else(|e| panic!("{name} must dispatch: {e:?}"));
    let entries = recorded.lock().expect("sink");
    assert_eq!(entries.len(), 1, "{name} must issue exactly one request");
    entries[0].0.clone()
}

#[tokio::test]
async fn each_tool_dispatches_the_documented_url() {
    assert_eq!(
        dispatched_uri("get_overview", serde_json::json!({})).await,
        "/api/v1/overview"
    );
    assert_eq!(
        dispatched_uri(
            "list_repo_sessions",
            serde_json::json!({"owner": "acme", "name": "site"})
        )
        .await,
        "/api/v1/repos/acme/site/sessions"
    );
    assert_eq!(
        dispatched_uri(
            "get_session_outcomes",
            serde_json::json!({"owner": "acme", "name": "site", "issue_number": 7})
        )
        .await,
        "/api/v1/repos/acme/site/sessions/7/outcomes"
    );
    assert_eq!(
        dispatched_uri("observe_session", serde_json::json!({"session_id": "s1"})).await,
        "/api/v1/sessions/s1/observe",
        "an absent limit must not append an empty query"
    );
    assert_eq!(
        dispatched_uri(
            "observe_session",
            serde_json::json!({"session_id": "s1", "limit": 25})
        )
        .await,
        "/api/v1/sessions/s1/observe?limit=25"
    );
    assert_eq!(
        dispatched_uri("list_log_runs", serde_json::json!({"session_id": "s1"})).await,
        "/api/v1/logs/s1/runs"
    );
    assert_eq!(
        dispatched_uri("get_log_manifest", serde_json::json!({"session_id": "s1"})).await,
        "/api/v1/logs/s1/manifest"
    );
    assert_eq!(
        dispatched_uri(
            "get_log_manifest",
            serde_json::json!({"session_id": "s1", "run": "r 1"})
        )
        .await,
        "/api/v1/logs/s1/manifest?run=r%201"
    );
    assert_eq!(
        dispatched_uri("list_environment_profiles", serde_json::json!({})).await,
        "/api/v1/users/me/environment-profiles"
    );
}

#[tokio::test]
async fn model_supplied_values_cannot_escape_their_url_component() {
    // A space becomes %20 (the encoding the SPA asserts for the same endpoint), and
    // a slash becomes %2F so a session id or file path stays one component.
    assert_eq!(
        dispatched_uri("list_log_runs", serde_json::json!({"session_id": "sess 1"})).await,
        "/api/v1/logs/sess%201/runs"
    );
    assert_eq!(
        dispatched_uri(
            "list_log_runs",
            serde_json::json!({"session_id": "a/b/../admin"})
        )
        .await,
        "/api/v1/logs/a%2Fb%2F..%2Fadmin/runs"
    );
    assert_eq!(
        dispatched_uri(
            "tail_log_file",
            serde_json::json!({"session_id": "s1", "path": "codex/run 1.log"})
        )
        .await,
        "/api/v1/logs/s1/file?path=codex%2Frun%201.log&tail_bytes=16384",
        "an absent tail_bytes takes the documented default"
    );
    assert_eq!(
        dispatched_uri(
            "list_repo_sessions",
            serde_json::json!({"owner": "acme", "name": "site?x=1"})
        )
        .await,
        "/api/v1/repos/acme/site%3Fx%3D1/sessions",
        "a query character in a path segment must not start a query string"
    );
}

#[tokio::test]
async fn tail_bytes_is_clamped_into_range_on_the_wire() {
    assert_eq!(
        dispatched_uri(
            "tail_log_file",
            serde_json::json!({"session_id": "s1", "path": "run.log", "tail_bytes": 999_999})
        )
        .await,
        "/api/v1/logs/s1/file?path=run.log&tail_bytes=65536"
    );
    assert_eq!(
        dispatched_uri(
            "observe_session",
            serde_json::json!({"session_id": "s1", "limit": 999_999})
        )
        .await,
        "/api/v1/sessions/s1/observe?limit=10000"
    );
}

// ---- dispatch shape -------------------------------------------------------

#[tokio::test]
async fn read_tools_return_the_http_status_as_data() {
    let (ctx, _gh) = rejecting_ctx().await;
    let outcome = invoke(
        &ctx,
        "list_log_runs",
        serde_json::json!({"session_id": "sess-abc-123"}),
    )
    .await
    .expect("a 401 is a successful tool call whose result explains itself");
    assert_eq!(outcome.status, Some(401));
    assert_eq!(outcome.result_json["status"], 401);
    assert!(
        outcome.result_json["body"]["error"].is_string(),
        "the endpoint's error envelope must reach the model: {:?}",
        outcome.result_json
    );
}

#[tokio::test]
async fn every_dispatch_backed_tool_populates_its_status() {
    let (ctx, _gh) = rejecting_ctx().await;
    let calls: Vec<(&str, serde_json::Value)> = vec![
        ("get_overview", serde_json::json!({})),
        (
            "list_repo_sessions",
            serde_json::json!({"owner": "acme", "name": "site"}),
        ),
        (
            "get_session_outcomes",
            serde_json::json!({"owner": "acme", "name": "site", "issue_number": 7}),
        ),
        (
            "observe_session",
            serde_json::json!({"session_id": "s1", "limit": 5}),
        ),
        ("list_log_runs", serde_json::json!({"session_id": "s1"})),
        (
            "get_log_manifest",
            serde_json::json!({"session_id": "s1", "run": "r1"}),
        ),
        (
            "tail_log_file",
            serde_json::json!({"session_id": "s1", "path": "run.log"}),
        ),
        ("list_environment_profiles", serde_json::json!({})),
    ];
    for (name, args) in calls {
        let outcome = invoke(&ctx, name, args)
            .await
            .unwrap_or_else(|e| panic!("{name} must dispatch: {e:?}"));
        assert!(
            outcome.status.is_some(),
            "{name} must carry its HTTP status for the tool_result event"
        );
    }
}

// ---- broader-token forwarding --------------------------------------------

#[tokio::test]
async fn get_overview_forwards_the_broader_token_and_nothing_else_does() {
    // `/overview` is the only endpoint that honors the broader-visibility token, so
    // it must be forwarded there (or the concierge sees fewer repositories than the
    // dashboard) and nowhere else (needless credential exposure).
    let (ctx, recorded) = probe_ctx(Some("gho_broader"));
    invoke(&ctx, "get_overview", serde_json::json!({}))
        .await
        .expect("get_overview dispatches");
    invoke(
        &ctx,
        "list_repo_sessions",
        serde_json::json!({"owner": "acme", "name": "site"}),
    )
    .await
    .expect("list_repo_sessions dispatches");
    invoke(
        &ctx,
        "list_log_runs",
        serde_json::json!({"session_id": "s1"}),
    )
    .await
    .expect("list_log_runs dispatches");

    let entries = recorded.lock().expect("sink");
    assert_eq!(
        entries[0].1.as_deref(),
        Some("gho_broader"),
        "get_overview must forward the broader token"
    );
    assert_eq!(entries[1].1, None, "list_repo_sessions must not forward it");
    assert_eq!(entries[2].1, None, "list_log_runs must not forward it");
}

#[tokio::test]
async fn no_broader_token_means_no_header_at_all() {
    let (ctx, recorded) = probe_ctx(None);
    invoke(&ctx, "get_overview", serde_json::json!({}))
        .await
        .expect("get_overview dispatches");
    assert_eq!(recorded.lock().expect("sink")[0].1, None);
}
