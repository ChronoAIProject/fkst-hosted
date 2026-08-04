//! The PRESENCE half of the safe-argument contract, driven through the real
//! router.
//!
//! Its sibling `audit_redaction_canary` proves nothing forbidden reaches a
//! record. On its own that is satisfiable by recording nothing at all, which is
//! why these cases assert the exact counts, flags, normalized identifiers, and
//! parse states each operation's catalog entry promises — including the three
//! non-`parsed` states, whose distinctions are the whole point of having four.

mod audit_canary;

use audit_canary::{arguments, create_session_body, rendered, sign, Canary, BOGUS_SIGNATURE};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::audit::ArgumentsParseStatus;

/// The presence half for the create-session record: the counts, flags, and
/// normalized identifiers the catalog promises are all there.
#[tokio::test]
async fn the_create_session_record_keeps_its_safe_counts_and_flags() {
    let canary = Canary::start().await;
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions"),
            create_session_body(),
        ))
        .await;

    let event = canary.event("canvas_create_session");
    assert_eq!(event.arguments_parse_status, ArgumentsParseStatus::Parsed);
    let arguments = arguments(&event);
    assert_eq!(arguments["owner"], serde_json::json!("acme"));
    assert_eq!(arguments["repo"], serde_json::json!("site"));
    assert_eq!(
        arguments["package_refs"],
        serde_json::json!(["acme/pkgs@main:packages/devloop"])
    );
    assert_eq!(arguments["package_count"], serde_json::json!(1));
    assert_eq!(arguments["manifest_count"], serde_json::json!(1));
    assert_eq!(arguments["work_label"], serde_json::json!("fkst:work"));
    assert_eq!(arguments["source_branch"], serde_json::json!("main"));
    assert_eq!(
        arguments["target_branch"],
        serde_json::json!("fkst-hosted-default")
    );
    assert_eq!(arguments["output_language"], serde_json::json!("zh-CN"));
    assert_eq!(arguments["auto_merge"], serde_json::json!(true));
    assert_eq!(
        arguments["disposable_environment_present"],
        serde_json::json!(true)
    );
    assert_eq!(arguments["disposable_variable_count"], serde_json::json!(1));
    assert_eq!(arguments["disposable_secret_count"], serde_json::json!(1));
    assert_eq!(arguments["log_access_count"], serde_json::json!(2));
    assert_eq!(arguments["collaborator_count"], serde_json::json!(1));
    // …and the correlation the epic requires alongside them.
    assert_eq!(
        event.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    assert!(
        !rendered(&event).contains("canary-session-name"),
        "the session name must not be recorded"
    );
}

/// The presence half for the environment PUT: three counts and a validated name,
/// with every command, key, and value absent.
#[tokio::test]
async fn the_environment_put_record_keeps_its_counts_and_name() {
    let canary = Canary::start().await;
    canary
        .call(canary.authenticated_json(
            Request::put("/api/v1/users/me/environment-profiles/node-20"),
            serde_json::json!({
                "install": ["canary-install-command", "echo two"],
                "variables": { "CANARY_VARIABLE_KEY": "canary-variable-value" },
                "secrets": {
                    "CANARY_SECRET_KEY": "canary-secret-value",
                    "SECOND_SECRET": "another",
                },
            }),
        ))
        .await;

    let event = canary.event("put_user_environment_profile");
    assert_eq!(event.arguments_parse_status, ArgumentsParseStatus::Parsed);
    let arguments = arguments(&event);
    assert_eq!(arguments["environment_name"], serde_json::json!("node-20"));
    assert_eq!(arguments["install_command_count"], serde_json::json!(2));
    assert_eq!(arguments["variable_count"], serde_json::json!(1));
    assert_eq!(arguments["secret_count"], serde_json::json!(2));
    assert_eq!(
        arguments.as_object().map(serde_json::Map::len),
        Some(4),
        "nothing beyond the documented four properties"
    );
}

/// The presence half for the log surfaces: the session, the run, the file CLASS
/// (never the path), and the tail size — plus the top-level session correlation.
#[tokio::test]
async fn the_log_file_record_keeps_its_class_and_session_correlation() {
    let canary = Canary::start().await;
    let session = "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e";
    canary
        .call(canary.authenticated(Request::get(format!(
            "/api/v1/logs/{session}/file?path=fkst-substrate/codex/codex.log&tail_bytes=2048"
        ))))
        .await;

    let event = canary.event("session_log_file");
    let arguments = arguments(&event);
    assert_eq!(arguments["session_id"], serde_json::json!(session));
    assert_eq!(arguments["run_id_or_latest"], serde_json::json!("latest"));
    assert_eq!(arguments["file_class"], serde_json::json!("codex"));
    assert_eq!(arguments["tail_bytes"], serde_json::json!(2048));
    assert_eq!(event.session_id.as_deref(), Some(session));
    assert!(
        !rendered(&event).contains("codex.log"),
        "the requested path is replaced by its class, never recorded"
    );
}

/// A work item refused before its label could resolve still describes the
/// request: the arguments are recorded up front and only the label — which the
/// refused GitHub reads were what resolved — is missing.
#[tokio::test]
async fn a_refused_work_item_still_records_its_safe_arguments() {
    let canary = Canary::start().await;
    // The canary GitHub answers every non-`/user` call with a 500, so the
    // trigger pre-flight fails and the handler never reaches its write.
    let response = canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos/acme/site/sessions/42/work-items"),
            serde_json::json!({
                "title": "canary-work-item-title",
                "body": "canary-work-item-body",
                "label": "fkst:work",
            }),
        ))
        .await;
    assert!(
        response.status().is_client_error() || response.status().is_server_error(),
        "the fixture must refuse the call, got {}",
        response.status()
    );

    let event = canary.event("canvas_create_work_item");
    assert_eq!(event.arguments_parse_status, ArgumentsParseStatus::Parsed);
    let arguments = arguments(&event);
    assert_eq!(arguments["owner"], serde_json::json!("acme"));
    assert_eq!(arguments["repo"], serde_json::json!("site"));
    assert_eq!(arguments["trigger_issue"], serde_json::json!(42));
    assert_eq!(
        arguments["title_bytes"],
        serde_json::json!("canary-work-item-title".len())
    );
    assert_eq!(arguments["body_present"], serde_json::json!(true));
    assert_eq!(
        arguments["body_bytes"],
        serde_json::json!("canary-work-item-body".len())
    );
    assert!(
        arguments.get("selected_label").is_none(),
        "the caller's requested label is never recorded as the selected one"
    );
    assert_eq!(
        event.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    assert_eq!(event.correlation.trigger_issue, Some(42));
    let rendered = rendered(&event);
    assert!(!rendered.contains("canary-work-item-title"), "{rendered}");
    assert!(!rendered.contains("canary-work-item-body"), "{rendered}");
}

/// Creating a repository correlates by repository, not only inside its
/// arguments: `repo_full_name` is a query key on the read side.
#[tokio::test]
async fn the_create_repo_record_correlates_by_repository() {
    let canary = Canary::start().await;
    canary
        .call(canary.authenticated_json(
            Request::post("/api/v1/repos"),
            serde_json::json!({
                "owner": null,
                "name": "site",
                "private": true,
                "description": "canary-repository-description",
            }),
        ))
        .await;

    let event = canary.event("create_repo");
    assert_eq!(arguments(&event)["name"], serde_json::json!("site"));
    assert_eq!(
        event.correlation.repo_full_name.as_deref(),
        Some("octocat/site"),
        "the owner defaults to the verified caller's own account"
    );
}

/// A path segment that does not fit its type is `invalid` — the class of
/// rejection an operator can act on — and never `unavailable`, which is what an
/// authentication failure means.
#[tokio::test]
async fn a_malformed_path_segment_records_invalid_arguments() {
    let canary = Canary::start().await;
    let response = canary
        .call(canary.authenticated(Request::get(
            "/api/v1/repos/acme/site/sessions/canary-bad-issue/outcomes",
        )))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let event = canary.event("canvas_session_outcomes");
    assert_eq!(event.arguments_parse_status, ArgumentsParseStatus::Invalid);
    assert_eq!(event.error_code.as_deref(), Some("invalid_request"));
    assert!(!rendered(&event).contains("canary-bad-issue"));
}

/// A verified delivery records the closed event/action, the correlation handles,
/// and how it was handled — and none of the payload's free text.
#[tokio::test]
async fn the_verified_webhook_record_keeps_its_closed_metadata() {
    let canary = Canary::start().await;
    let body = serde_json::json!({
        "action": "opened",
        "issue": { "number": 9, "title": "canary-issue-title", "body": "canary-issue-body" },
        "repository": { "owner": { "login": "acme" }, "name": "site" },
        "installation": { "id": 7 },
        "sender": { "login": "octocat", "id": 583_231 },
    })
    .to_string();
    let response = canary
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("x-hub-signature-256", sign(&body))
                .header("x-github-event", "issues")
                .header("x-github-delivery", "8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let event = canary.event("github_app_webhook");
    let arguments = arguments(&event);
    assert_eq!(arguments["event_type"], serde_json::json!("issues"));
    assert_eq!(arguments["action"], serde_json::json!("opened"));
    assert_eq!(arguments["installation_id"], serde_json::json!(7));
    assert_eq!(arguments["repo_full_name"], serde_json::json!("acme/site"));
    assert_eq!(arguments["trigger_issue"], serde_json::json!(9));
    assert_eq!(arguments["handling"], serde_json::json!("ignored"));
    assert!(arguments.get("signature_valid").is_none());
    // Correlation is populated from the same verified body.
    assert_eq!(event.correlation.installation_id, Some(7));
    assert_eq!(
        event.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    assert_eq!(event.correlation.trigger_issue, Some(9));
}

/// A rejected delivery says one thing about itself and nothing else — the
/// sender, installation, repository, and issue it CLAIMS are unverified.
#[tokio::test]
async fn a_rejected_webhook_record_says_only_that_the_signature_failed() {
    let canary = Canary::start().await;
    let response = canary
        .call(
            Request::post("/api/v1/github/app/webhook")
                .header("x-hub-signature-256", BOGUS_SIGNATURE)
                .header("x-github-event", "issues")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "action": "opened",
                        "issue": { "number": 9, "title": "canary-issue-title" },
                        "repository": { "owner": { "login": "acme" }, "name": "site" },
                        "installation": { "id": 7 },
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let event = canary.event("github_app_webhook");
    let arguments = arguments(&event);
    assert_eq!(arguments["signature_valid"], serde_json::json!(false));
    assert_eq!(arguments.as_object().map(serde_json::Map::len), Some(1));
    assert_eq!(event.correlation.installation_id, None);
    assert_eq!(event.correlation.repo_full_name, None);
    assert_eq!(event.correlation.trigger_issue, None);
}

/// A malformed body records the four documented metadata fields and never a
/// byte, a lossy string, or the parser's message.
#[tokio::test]
async fn a_malformed_body_records_only_bounded_transport_metadata() {
    let canary = Canary::start().await;
    let body = r#"{"name": canary-log-content}"#;
    let response = canary
        .call(
            Request::post("/api/v1/repos")
                .header("authorization", "Bearer canary-bearer-token")
                .header("content-type", "application/json")
                .header("content-length", body.len().to_string())
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let event = canary.event("create_repo");
    assert_eq!(event.arguments_parse_status, ArgumentsParseStatus::Invalid);
    let arguments = arguments(&event);
    assert_eq!(
        arguments["content_type"],
        serde_json::json!("application/json")
    );
    assert_eq!(
        arguments["content_length_declared"],
        serde_json::json!(body.len())
    );
    assert_eq!(
        arguments["body_bytes_observed"],
        serde_json::json!(body.len())
    );
    assert_eq!(arguments.as_object().map(serde_json::Map::len), Some(3));
    assert_eq!(event.error_code.as_deref(), Some("invalid_request"));
    assert!(!rendered(&event).contains("canary-log-content"));
}

/// A request rejected before its safe parse could run is `unavailable`, not
/// `not_applicable`: the two answer very different operational questions.
#[tokio::test]
async fn a_pre_parse_rejection_records_unavailable_arguments() {
    let canary = Canary::start().await;
    // No bearer token at all: the identity extractor rejects before the handler.
    let response = canary
        .call(
            Request::get("/api/v1/users/me/environment-profiles/node-20")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let event = canary.event("get_user_environment_profile");
    assert_eq!(
        event.arguments_parse_status,
        ArgumentsParseStatus::Unavailable
    );
    assert!(event.arguments.is_empty());
}

/// The operation that genuinely takes no arguments says so, which is what makes
/// `unavailable` meaningful elsewhere.
#[tokio::test]
async fn an_argument_free_operation_records_not_applicable() {
    let canary = Canary::start().await;
    canary
        .call(canary.authenticated(Request::get("/api/v1/users/me/environment-profiles")))
        .await;

    let event = canary.event("list_user_environment_profiles");
    assert_eq!(
        event.arguments_parse_status,
        ArgumentsParseStatus::NotApplicable
    );
    assert!(event.arguments.is_empty());
}

/// The OAuth callback records the flow and the closed outcome — never the code,
/// the state, or GitHub's error slug.
#[tokio::test]
async fn the_oauth_callback_record_keeps_only_its_flow_and_outcome() {
    let canary = Canary::start().await;
    canary
        .call(
            Request::get(
                "/api/v1/auth/github/callback\
                 ?code=canary-oauth-code&state=canary-oauth-state&error=canary-oauth-error",
            )
            .body(Body::empty())
            .expect("request builds"),
        )
        .await;

    let event = canary.event("github_login_callback");
    let arguments = arguments(&event);
    assert_eq!(arguments["flow"], serde_json::json!("login"));
    assert_eq!(arguments["result"], serde_json::json!("denied"));
    assert_eq!(arguments.as_object().map(serde_json::Map::len), Some(2));
}

/// An unrouted path keeps neither its raw path nor its query: both are exactly
/// the material an OAuth redirect leaves lying around.
#[tokio::test]
async fn an_unrouted_request_records_the_sentinels_and_no_arguments() {
    let canary = Canary::start().await;
    canary
        .call(
            Request::get("/api/v1/canary-unrouted-path?token=canary-query-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    let event = canary.event("<unmatched>");
    assert_eq!(event.route_template, "<unmatched>");
    assert!(event.arguments.is_empty());
    assert!(!rendered(&event).contains("canary-unrouted-path"));
    assert!(!rendered(&event).contains("canary-query-token"));
}
