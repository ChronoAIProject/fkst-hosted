//! Tests for the `search_manual` tool (sibling `#[path]` module).

use super::super::default_registry;
use super::*;
use crate::chat::dispatch::SelfDispatch;
use crate::state::empty_self_router;

/// A context with no usable router: this tool must never dispatch, so a broken
/// transport is exactly the right thing to hand it.
fn ctx() -> ToolCtx {
    ToolCtx {
        dispatch: SelfDispatch::new(empty_self_router()),
        bearer: secrecy::SecretString::from("gho_unused".to_string()),
        broader: None,
    }
}

async fn search(query: serde_json::Value) -> Result<ToolOutcome, ToolError> {
    default_registry()
        .invoke("search_manual", &ctx(), query)
        .await
}

#[test]
fn the_tool_is_registered_in_the_default_registry() {
    assert!(default_registry().contains("search_manual"));
}

#[test]
fn the_schema_takes_one_required_query_string() {
    let def = default_registry()
        .defs()
        .into_iter()
        .find(|d| d.name == "search_manual")
        .expect("registered");
    assert_eq!(def.parameters["type"], "object");
    assert_eq!(def.parameters["properties"]["query"]["type"], "string");
    assert_eq!(def.parameters["required"][0], "query");
    assert_eq!(def.parameters["additionalProperties"], false);
    assert!(
        def.description.contains("how the platform WORKS"),
        "the description must tell the model when to prefer this over a live tool"
    );
}

#[tokio::test]
async fn a_hit_returns_the_matching_sections_and_no_toc() {
    let outcome = search(serde_json::json!({ "query": "fkst-unrouted assignee" }))
        .await
        .expect("the lookup succeeds");
    let sections = outcome.result_json["sections"]
        .as_array()
        .expect("sections array");
    assert!(!sections.is_empty());
    assert!(sections[0]["id"].is_string());
    assert!(sections[0]["title"].is_string());
    assert!(
        sections[0]["content"]
            .as_str()
            .expect("content")
            .contains("exactly one assignee"),
        "the section content must carry the actual rule"
    );
    assert!(
        outcome.result_json.get("toc").is_none(),
        "the table of contents is noise on a hit"
    );
}

#[tokio::test]
async fn a_miss_returns_the_table_of_contents_so_the_model_can_retry() {
    let outcome = search(serde_json::json!({ "query": "zzzznotatopic" }))
        .await
        .expect("a miss is still a successful call");
    assert!(outcome.result_json["sections"]
        .as_array()
        .expect("sections array")
        .is_empty());
    let toc = outcome.result_json["toc"]
        .as_array()
        .expect("toc on a miss");
    assert!(!toc.is_empty());
    assert!(toc[0]["id"].is_string() && toc[0]["title"].is_string());
}

#[tokio::test]
async fn an_in_process_tool_reports_no_http_status() {
    let outcome = search(serde_json::json!({ "query": "logs" }))
        .await
        .expect("the lookup succeeds");
    assert_eq!(
        outcome.status, None,
        "there is no HTTP status for a compiled-in lookup"
    );
    assert!(!outcome.truncated);
}

#[tokio::test]
async fn a_missing_query_is_rejected_as_invalid_arguments() {
    let error = search(serde_json::json!({}))
        .await
        .expect_err("query is required");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}

#[tokio::test]
async fn a_blank_query_is_rejected() {
    let error = search(serde_json::json!({ "query": "   " }))
        .await
        .expect_err("a blank query is not a search");
    assert!(matches!(error, ToolError::InvalidArgs(_)), "got {error:?}");
}
