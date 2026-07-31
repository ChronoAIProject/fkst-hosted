//! Safe arguments for the chat concierge turn.
//!
//! `chat_turn` was added after issue #5671's catalog was written, so its policy
//! is stated here in the same terms: the request body is the visible transcript —
//! the user's prompt in full, plus every assistant answer the client is
//! displaying. That is the single most content-bearing body on the whole
//! surface, and none of it is a valid audit property. The model's streamed
//! response, the tools it chose, and the arguments it passed them are likewise
//! absent (a response is never captured at all).
//!
//! What remains is shape: how many messages the client sent, how they split
//! between roles, how large the transcript and the message being answered were,
//! and whether the optional broader-visibility credential was forwarded. That is
//! enough to spot an abusive or malfunctioning client without the analytics store
//! holding one word a user typed.

use serde::Serialize;

use super::bounds::byte_len;
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};
use crate::routes::chat::{ChatClientRole, ChatRequest};

/// `chat_turn` — one conversation turn.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct SafeChatTurn {
    message_count: usize,
    user_message_count: usize,
    assistant_message_count: usize,
    /// Total UTF-8 bytes across the whole submitted transcript.
    total_content_bytes: u64,
    /// Bytes of the last message — the one actually being answered.
    last_message_bytes: u64,
    broader_visibility_requested: bool,
}

impl BoundedAuditArguments for SafeChatTurn {
    const OPERATION_ID: &'static str = "chat_turn";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CHAT_TURN_FIELDS;
}

/// The input view for `chat_turn`: the validated request plus the one header
/// flag. The request is borrowed, never serialized.
pub struct ChatTurnInput<'a> {
    pub request: &'a ChatRequest,
    pub broader_visibility_requested: bool,
}

impl Sealed for ChatTurnInput<'_> {}

impl ToSafeAuditArguments for ChatTurnInput<'_> {
    type Safe = SafeChatTurn;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        let messages = &self.request.messages;
        let user_message_count = messages
            .iter()
            .filter(|message| message.role == ChatClientRole::User)
            .count();
        SafeChatTurn {
            message_count: messages.len(),
            user_message_count,
            assistant_message_count: messages.len().saturating_sub(user_message_count),
            total_content_bytes: messages
                .iter()
                .map(|message| byte_len(&message.content))
                .fold(0u64, u64::saturating_add),
            last_message_bytes: messages
                .last()
                .map(|message| byte_len(&message.content))
                .unwrap_or(0),
            broader_visibility_requested: self.broader_visibility_requested,
        }
    }
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
