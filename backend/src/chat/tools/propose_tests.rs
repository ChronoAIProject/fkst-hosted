//! Tests for the proposal tools (sibling `#[path]` module).

use super::super::default_registry;
use super::*;
use crate::chat::actions::ActionProposal;
use crate::chat::dispatch::SelfDispatch;
use crate::state::empty_self_router;

/// A context with no usable router: a proposal tool must never dispatch, so a broken
/// transport is exactly what it should be handed.
fn ctx() -> ToolCtx {
    ToolCtx {
        dispatch: SelfDispatch::new(empty_self_router()),
        bearer: secrecy::SecretString::from("gho_unused".to_string()),
        broader: None,
    }
}

async fn call_tool(name: &str, args: serde_json::Value) -> Result<ToolOutcome, ToolError> {
    default_registry().invoke(name, &ctx(), args).await
}

fn session_args() -> serde_json::Value {
    serde_json::json!({
        "owner": "acme",
        "name": "site",
        "session_name": "sitebuilder",
        "packages": ["acme/pkgs@main:packages/site"],
        "work_label": "site-build",
    })
}

// ---- registration + schemas ----------------------------------------------

#[test]
fn the_three_proposal_tools_are_registered() {
    let registry = default_registry();
    for name in [
        "draft_trigger_session",
        "draft_work_item",
        "propose_stop_session",
    ] {
        assert!(registry.contains(name), "{name} must be registered");
    }
}

#[test]
fn every_proposal_schema_is_closed_and_warns_that_nothing_is_executed() {
    let defs = default_registry().defs();
    for name in [
        "draft_trigger_session",
        "draft_work_item",
        "propose_stop_session",
    ] {
        let def = defs.iter().find(|d| d.name == name).expect("registered");
        assert_eq!(def.parameters["additionalProperties"], false, "{name}");
        assert!(
            def.parameters["required"].is_array(),
            "{name} must declare required arguments"
        );
        // The model must not be able to read these descriptions and conclude it acted.
        assert!(
            def.description.contains("does NOT") || def.description.contains("only present a card"),
            "{name} must state that it does not perform the action: {}",
            def.description
        );
    }
}

#[test]
fn no_proposal_tool_accepts_secret_bearing_arguments() {
    // Secrets are excluded structurally; this guards the schema surface too.
    let defs = default_registry().defs();
    for def in defs {
        let properties = def.parameters["properties"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        for key in properties.keys() {
            assert!(
                !key.contains("secret") && !key.contains("disposable") && !key.contains("token"),
                "{} must not accept {key}",
                def.name
            );
        }
    }
}

// ---- success paths -------------------------------------------------------

#[tokio::test]
async fn a_valid_session_draft_carries_the_proposal_out_of_band() {
    let outcome = call_tool("draft_trigger_session", session_args())
        .await
        .expect("the tool runs");

    // The model sees only an acknowledgement — not the payload it already produced.
    assert_eq!(outcome.result_json["proposal_presented"], true);
    assert!(outcome.result_json["summary"].is_string());
    assert!(
        outcome.result_json.get("rendered_issue_body").is_none(),
        "the full draft must not be re-sent to the model"
    );
    assert!(
        outcome.result_json["note"]
            .as_str()
            .expect("note")
            .contains("Do not claim"),
        "the result must warn the model not to claim the action happened"
    );
    assert_eq!(
        outcome.status, None,
        "an in-process tool has no HTTP status"
    );

    let proposal = outcome.proposal.expect("the proposal travels out of band");
    let ActionProposal::CreateSession {
        rendered_issue_body,
        ..
    } = &proposal
    else {
        panic!("expected a create-session proposal");
    };
    assert!(rendered_issue_body.contains("### Session Name"));
    assert!(rendered_issue_body.contains("sitebuilder"));
}

#[tokio::test]
async fn a_valid_work_item_draft_produces_a_proposal() {
    let outcome = call_tool(
        "draft_work_item",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "trigger_issue_number": 7,
            "title": "Add the footer",
            "body": "Edit src/footer.tsx",
        }),
    )
    .await
    .expect("the tool runs");
    assert_eq!(outcome.result_json["proposal_presented"], true);
    assert!(matches!(
        outcome.proposal,
        Some(ActionProposal::CreateWorkItem { .. })
    ));
}

#[tokio::test]
async fn a_valid_stop_draft_produces_a_proposal() {
    let outcome = call_tool(
        "propose_stop_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "trigger_issue_number": 7,
            "reason": "the work is finished",
        }),
    )
    .await
    .expect("the tool runs");
    assert!(matches!(
        outcome.proposal,
        Some(ActionProposal::StopSession { .. })
    ));
}

// ---- argument coercion ---------------------------------------------------

#[tokio::test]
async fn a_single_string_is_accepted_where_a_list_is_expected() {
    // Models routinely pass one value instead of a one-element array; refusing that
    // would burn a retry for nothing.
    let outcome = call_tool(
        "draft_trigger_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "session_name": "sitebuilder",
            "packages": "acme/pkgs@main:packages/site",
        }),
    )
    .await
    .expect("the tool runs");
    assert_eq!(outcome.result_json["proposal_presented"], true);
}

#[tokio::test]
async fn a_stringly_typed_boolean_is_accepted() {
    let outcome = call_tool(
        "draft_trigger_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "session_name": "sitebuilder",
            "packages": ["acme/pkgs@main:packages/site"],
            "auto_merge": "true",
        }),
    )
    .await
    .expect("the tool runs");
    let Some(ActionProposal::CreateSession { request, .. }) = outcome.proposal else {
        panic!("expected a create-session proposal");
    };
    assert_eq!(request.auto_merge, Some(true));
}

#[tokio::test]
async fn a_nonsense_boolean_is_an_argument_error() {
    let error = call_tool(
        "draft_trigger_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "session_name": "sitebuilder",
            "packages": ["acme/pkgs@main:packages/site"],
            "auto_merge": "maybe",
        }),
    )
    .await
    .expect_err("an unparseable boolean must be rejected");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}

#[tokio::test]
async fn a_non_string_list_entry_is_an_argument_error() {
    let error = call_tool(
        "draft_trigger_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "session_name": "sitebuilder",
            "packages": [7],
        }),
    )
    .await
    .expect_err("a numeric package entry must be rejected");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}

// ---- rejection-as-data ---------------------------------------------------

#[tokio::test]
async fn an_invalid_draft_is_rejected_as_data_with_no_proposal() {
    // The model must be able to fix the draft and retry inside the same turn, so this is
    // a successful tool call whose RESULT explains the problem.
    let outcome = call_tool(
        "draft_trigger_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "session_name": "sitebuilder",
            "packages": ["not-a-reference"],
        }),
    )
    .await
    .expect("a rejected draft is still a successful call");
    assert_eq!(outcome.result_json["proposal_presented"], false);
    assert!(outcome.result_json["error"]
        .as_str()
        .expect("error text")
        .contains("packages"));
    assert!(
        outcome.proposal.is_none(),
        "a rejected draft must not present a card"
    );
}

#[tokio::test]
async fn a_draft_without_a_package_source_is_rejected_as_data() {
    let outcome = call_tool(
        "draft_trigger_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "session_name": "sitebuilder",
        }),
    )
    .await
    .expect("a rejected draft is still a successful call");
    assert_eq!(outcome.result_json["proposal_presented"], false);
    assert!(outcome.proposal.is_none());
}

#[tokio::test]
async fn a_reasonless_stop_draft_is_rejected_as_data() {
    let error = call_tool(
        "propose_stop_session",
        serde_json::json!({
            "owner": "acme",
            "name": "site",
            "trigger_issue_number": 7,
        }),
    )
    .await
    .expect_err("reason is a required argument");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}

#[tokio::test]
async fn a_missing_required_argument_names_the_argument() {
    let error = call_tool("draft_work_item", serde_json::json!({"owner": "acme"}))
        .await
        .expect_err("name is required");
    match error {
        ToolError::InvalidArgs(message) => assert!(message.contains("name"), "got {message}"),
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}
