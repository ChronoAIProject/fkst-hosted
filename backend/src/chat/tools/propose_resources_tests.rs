//! Tests for the resource proposal tools (sibling `#[path]` module).
//!
//! The context these run against has a deliberately broken router. That is the point for
//! three of the four tools — they must never dispatch — and it also pins the fourth's
//! degradation: `draft_environment_profile`'s existence lookup must fail to `None` and
//! still produce a card, rather than losing the draft because a cosmetic read failed.

use super::super::default_registry;
use super::*;
use crate::chat::actions::ActionProposal;
use crate::chat::dispatch::SelfDispatch;
use crate::state::empty_self_router;

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

/// The proposal a successful draft produced.
fn proposal_of(outcome: &ToolOutcome) -> &ActionProposal {
    assert_eq!(
        outcome.result_json["proposal_presented"],
        serde_json::json!(true),
        "expected a presented proposal, got {}",
        outcome.result_json
    );
    outcome.proposal.as_ref().expect("a proposal")
}

/// The rejection message a refused draft returned to the model.
fn rejection_of(outcome: &ToolOutcome) -> String {
    assert_eq!(
        outcome.result_json["proposal_presented"],
        serde_json::json!(false),
        "expected a rejection, got {}",
        outcome.result_json
    );
    assert!(
        outcome.proposal.is_none(),
        "a rejected draft must carry no proposal"
    );
    outcome.result_json["error"].as_str().unwrap().to_string()
}

// ---- registration + schemas ----------------------------------------------

#[test]
fn the_resource_proposal_tools_are_registered() {
    let registry = default_registry();
    for name in [
        "propose_create_repository",
        "draft_environment_profile",
        "propose_delete_environment_profile",
        "propose_uninstall_app",
    ] {
        assert!(registry.contains(name), "{name} is not registered");
    }
}

#[test]
fn every_resource_proposal_schema_is_closed() {
    // `additionalProperties: false` keeps a model from smuggling an unmodelled field —
    // in particular a secret value — into a draft.
    let registry = default_registry();
    for def in registry.defs().into_iter().filter(|def| {
        matches!(
            def.name.as_str(),
            "propose_create_repository"
                | "draft_environment_profile"
                | "propose_delete_environment_profile"
                | "propose_uninstall_app"
        )
    }) {
        assert_eq!(
            def.parameters["additionalProperties"],
            serde_json::json!(false),
            "{} has an open schema",
            def.name
        );
    }
}

#[test]
fn the_environment_draft_schema_has_no_field_for_a_secret_value() {
    let registry = default_registry();
    let def = registry
        .defs()
        .into_iter()
        .find(|def| def.name == "draft_environment_profile")
        .expect("the tool");
    let properties = def.parameters["properties"]
        .as_object()
        .expect("properties");
    // Names only. A `secrets` object keyed by name would be a place for a value to live.
    assert!(properties.contains_key("secret_names"));
    assert!(
        !properties.contains_key("secrets"),
        "a secret VALUE must have nowhere to go in a draft"
    );
    assert_eq!(
        def.parameters["properties"]["secret_names"]["type"],
        "array"
    );
}

// ---- propose_create_repository -------------------------------------------

#[tokio::test]
async fn create_repository_drafts_a_private_repo_by_default() {
    let outcome = call_tool(
        "propose_create_repository",
        serde_json::json!({ "name": "site-builder" }),
    )
    .await
    .expect("tool ran");
    let ActionProposal::CreateRepository { private, name, .. } = proposal_of(&outcome) else {
        panic!("expected a CreateRepository proposal");
    };
    // Defaulting to public would be the one mistake a user cannot undo by flipping the flag.
    assert!(*private, "a drafted repository must default to private");
    assert_eq!(name, "site-builder");
}

#[tokio::test]
async fn create_repository_honors_an_explicit_public_request() {
    let outcome = call_tool(
        "propose_create_repository",
        serde_json::json!({ "name": "docs", "private": false, "owner": "acme" }),
    )
    .await
    .expect("tool ran");
    let ActionProposal::CreateRepository { private, owner, .. } = proposal_of(&outcome) else {
        panic!("expected a CreateRepository proposal");
    };
    assert!(!*private);
    assert_eq!(owner.as_deref(), Some("acme"));
}

#[tokio::test]
async fn create_repository_returns_a_bad_name_to_the_model_as_data() {
    // A rejection must be recoverable INSIDE the turn, so it is tool-result data rather
    // than a `ToolError` that would end the call.
    let outcome = call_tool(
        "propose_create_repository",
        serde_json::json!({ "name": "not a repo name" }),
    )
    .await
    .expect("tool ran");
    assert!(rejection_of(&outcome).contains("[A-Za-z0-9._-]"));
}

#[tokio::test]
async fn create_repository_requires_a_name() {
    let error = call_tool("propose_create_repository", serde_json::json!({}))
        .await
        .expect_err("missing name");
    assert!(matches!(error, ToolError::InvalidArgs(_)));
}

// ---- draft_environment_profile -------------------------------------------

#[tokio::test]
async fn environment_draft_carries_install_variables_and_secret_names() {
    let outcome = call_tool(
        "draft_environment_profile",
        serde_json::json!({
            "profile_name": "node-ci",
            "install": ["npm ci"],
            "variables": { "NODE_ENV": "production" },
            "secret_names": ["NPM_TOKEN"],
        }),
    )
    .await
    .expect("tool ran");
    let ActionProposal::SaveEnvironmentProfile {
        profile_name,
        install,
        variables,
        secret_keys,
        replaces_existing,
        ..
    } = proposal_of(&outcome)
    else {
        panic!("expected a SaveEnvironmentProfile proposal");
    };
    assert_eq!(profile_name, "node-ci");
    assert_eq!(install, &vec!["npm ci".to_string()]);
    assert_eq!(variables.len(), 1);
    assert_eq!(variables[0].key, "NODE_ENV");
    assert_eq!(secret_keys, &vec!["NPM_TOKEN".to_string()]);
    // The router is broken here, so the existence lookup could not answer — and the card
    // must say so rather than claim either outcome.
    assert_eq!(*replaces_existing, None);
}

#[tokio::test]
async fn environment_draft_survives_a_failed_existence_lookup() {
    // The same broken-router context, asserted as its own behaviour: a cosmetic lookup
    // failing must not cost the user their draft.
    let outcome = call_tool(
        "draft_environment_profile",
        serde_json::json!({ "profile_name": "node-ci", "install": ["npm ci"] }),
    )
    .await
    .expect("tool ran");
    assert!(outcome.proposal.is_some());
}

#[tokio::test]
async fn environment_draft_rejects_a_non_string_variable_value() {
    // A model emitting `{"PORT": 8080}` gets a precise message it can fix, not a panic.
    let error = call_tool(
        "draft_environment_profile",
        serde_json::json!({
            "profile_name": "node-ci",
            "install": ["npm ci"],
            "variables": { "PORT": 8080 },
        }),
    )
    .await
    .expect_err("non-string value");
    let ToolError::InvalidArgs(message) = error else {
        panic!("expected InvalidArgs");
    };
    assert!(message.contains("PORT"), "message was {message}");
}

#[tokio::test]
async fn environment_draft_rejects_a_reserved_variable_as_data() {
    let outcome = call_tool(
        "draft_environment_profile",
        serde_json::json!({
            "profile_name": "node-ci",
            "install": ["npm ci"],
            "variables": { "LLM_API_KEY": "sk-x" },
        }),
    )
    .await
    .expect("tool ran");
    assert!(rejection_of(&outcome).contains("reserved"));
}

#[tokio::test]
async fn environment_draft_requires_an_install_command() {
    let outcome = call_tool(
        "draft_environment_profile",
        serde_json::json!({ "profile_name": "node-ci", "install": [] }),
    )
    .await
    .expect("tool ran");
    assert!(rejection_of(&outcome).contains("at least one install command"));
}

// ---- propose_delete_environment_profile ----------------------------------

#[tokio::test]
async fn delete_environment_profile_drafts_the_delete() {
    let outcome = call_tool(
        "propose_delete_environment_profile",
        serde_json::json!({ "profile_name": "node-ci" }),
    )
    .await
    .expect("tool ran");
    let ActionProposal::DeleteEnvironmentProfile { profile_name, .. } = proposal_of(&outcome)
    else {
        panic!("expected a DeleteEnvironmentProfile proposal");
    };
    assert_eq!(profile_name, "node-ci");
}

#[tokio::test]
async fn delete_environment_profile_rejects_an_invalid_name_as_data() {
    let outcome = call_tool(
        "propose_delete_environment_profile",
        serde_json::json!({ "profile_name": "Node CI" }),
    )
    .await
    .expect("tool ran");
    assert!(rejection_of(&outcome).contains("must match"));
}

// ---- propose_uninstall_app -----------------------------------------------

#[tokio::test]
async fn uninstall_app_drafts_with_its_reason() {
    let outcome = call_tool(
        "propose_uninstall_app",
        serde_json::json!({ "owner": "acme", "reason": "consolidating accounts" }),
    )
    .await
    .expect("tool ran");
    let ActionProposal::UninstallApp { owner, reason, .. } = proposal_of(&outcome) else {
        panic!("expected an UninstallApp proposal");
    };
    assert_eq!(owner, "acme");
    assert_eq!(reason, "consolidating accounts");
}

#[tokio::test]
async fn uninstall_app_requires_a_reason() {
    let error = call_tool(
        "propose_uninstall_app",
        serde_json::json!({ "owner": "acme" }),
    )
    .await
    .expect_err("missing reason");
    assert!(matches!(error, ToolError::InvalidArgs(_)));
}
