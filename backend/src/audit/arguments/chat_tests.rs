//! Unit tests for the chat-turn safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties,
};
use crate::routes::chat::ChatClientMessage;

const PROMPT_CANARY: &str = "canary-user-prompt-text";
const ANSWER_CANARY: &str = "canary-assistant-answer-text";

fn request() -> ChatRequest {
    ChatRequest {
        messages: vec![
            ChatClientMessage {
                role: ChatClientRole::User,
                content: PROMPT_CANARY.to_string(),
            },
            ChatClientMessage {
                role: ChatClientRole::Assistant,
                content: ANSWER_CANARY.to_string(),
            },
            ChatClientMessage {
                role: ChatClientRole::User,
                content: "canary-final-question".to_string(),
            },
        ],
    }
}

fn safe(broader: bool) -> SafeChatTurn {
    ChatTurnInput {
        request: &request(),
        broader_visibility_requested: broader,
    }
    .to_safe_audit_arguments()
}

#[test]
fn the_chat_dto_is_wired_to_its_declared_policy() {
    assert_policy_matches::<SafeChatTurn>();
}

/// The whole transcript is in the body, and none of it may be recorded.
#[test]
fn a_turn_records_counts_and_sizes_but_no_message_text() {
    let safe = safe(true);
    assert_within_allowlist(&safe);
    assert_no_canary(
        &safe,
        &[PROMPT_CANARY, ANSWER_CANARY, "canary-final-question"],
    );

    let values = properties(&safe);
    assert_eq!(values.len(), 6);
    assert_eq!(
        values
            .get("message_count")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
    assert_eq!(
        values
            .get("user_message_count")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        values
            .get("assistant_message_count")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        values
            .get("total_content_bytes")
            .and_then(serde_json::Value::as_u64),
        Some((PROMPT_CANARY.len() + ANSWER_CANARY.len() + "canary-final-question".len()) as u64)
    );
    assert_eq!(
        values
            .get("last_message_bytes")
            .and_then(serde_json::Value::as_u64),
        Some("canary-final-question".len() as u64),
        "the message being answered is the one whose size matters"
    );
    assert_eq!(
        values
            .get("broader_visibility_requested")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn the_broader_flag_tracks_the_header_and_never_its_value() {
    let values = properties(&safe(false));
    assert_eq!(
        values
            .get("broader_visibility_requested")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

/// The validator rejects an empty history before the handler records, but the
/// projection must still be total rather than panicking on one.
#[test]
fn an_empty_transcript_projects_to_zeroes() {
    let values = properties(
        &ChatTurnInput {
            request: &ChatRequest { messages: vec![] },
            broader_visibility_requested: false,
        }
        .to_safe_audit_arguments(),
    );
    for key in [
        "message_count",
        "user_message_count",
        "assistant_message_count",
        "total_content_bytes",
        "last_message_bytes",
    ] {
        assert_eq!(
            values.get(key).and_then(serde_json::Value::as_u64),
            Some(0),
            "{key}"
        );
    }
}
