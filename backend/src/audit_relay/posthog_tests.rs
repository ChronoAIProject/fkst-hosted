//! Verification-query tests: the text is a function of BATCH SIZE only, and the
//! ids ride in the parameter map.

use super::*;
use crate::audit_relay::test_support::now;

#[test]
fn the_query_text_contains_no_caller_value() {
    let hostile = vec![
        "'; DROP TABLE events; --".to_string(),
        "11111111-1111-4111-8111-111111111111".to_string(),
    ];
    let built = build_verification_query(&hostile, now());
    assert!(!built.query.contains("DROP TABLE"));
    assert!(built.query.contains("{event_id_0}"));
    assert!(built.query.contains("{event_id_1}"));
    assert_eq!(
        built.values["event_id_0"],
        serde_json::json!("'; DROP TABLE events; --")
    );
}

#[test]
fn the_query_allowlists_the_three_audit_event_names() {
    let built = build_verification_query(&["ev-1".to_string()], now());
    assert_eq!(
        built.values["event_completed"],
        serde_json::json!(crate::audit::event::EVENT_NAME)
    );
    assert_eq!(
        built.values["event_incomplete"],
        serde_json::json!(crate::audit::event::INCOMPLETE_EVENT_NAME)
    );
    assert_eq!(
        built.values["event_lifecycle"],
        serde_json::json!(crate::audit::lifecycle::LIFECYCLE_EVENT_NAME)
    );
}

#[test]
fn one_query_covers_a_whole_batch() {
    // Verification is BATCHED by contract: one HogQL request per event would
    // make verifying a backlog cost more than producing it.
    let ids: Vec<String> = (0..50).map(|index| format!("ev-{index}")).collect();
    let built = build_verification_query(&ids, now());
    assert_eq!(built.query.matches("{event_id_").count(), 50);
    assert_eq!(built.values["row_limit"], serde_json::json!(50));
}

#[test]
fn the_request_body_is_a_hogql_envelope() {
    let built = build_verification_query(&["ev-1".to_string()], now());
    let body = built.request_body();
    assert_eq!(body["query"]["kind"], "HogQLQuery");
    assert_eq!(body["query"]["query"], serde_json::json!(built.query));
    assert!(body["query"]["values"].is_object());
}

#[tokio::test]
async fn an_empty_batch_asks_posthog_nothing() {
    // No client is even needed: an empty batch must short-circuit, so a quiet
    // relay makes no query requests at all.
    let client = crate::operations::posthog::PosthogQueryClient::new(
        "http://127.0.0.1:1/api/projects/1/query/".to_string(),
        secrecy::SecretString::from("unused".to_string()),
        std::time::Duration::from_millis(50),
    )
    .expect("client builds");
    let visible = verify_visible(&client, &[], now()).await.expect("no call");
    assert!(visible.is_empty());
}
