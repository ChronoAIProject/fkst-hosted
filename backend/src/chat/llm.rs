//! Wire-neutral chat-model types and the [`ChatModelClient`] contract.
//!
//! Nothing here mentions a provider or a wire format: the orchestrator builds a
//! [`TurnRequest`], consumes a stream of [`StreamItem`]s, and never learns which
//! implementation produced them. [`crate::chat::llm_openai`] is the one shipped
//! implementation (OpenAI-compatible chat-completions); a future `responses`-wire
//! or vendor-native client plugs in behind the same trait with no caller change.
//!
//! The stream shape is deliberate: text arrives incrementally
//! ([`StreamItem::TextDelta`]) because the SSE endpoint forwards deltas to the
//! browser as they land, while tool calls arrive as ONE fully-assembled
//! [`StreamItem::ToolCalls`] — a partially-decoded tool call is useless to a
//! caller, so reassembling provider fragments is the implementation's job.

use async_trait::async_trait;

/// Who authored a message in the conversation sent to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatRole {
    /// The platform-authored instructions; always first, never user-supplied.
    System,
    /// A message from the human.
    User,
    /// A message the model produced (text and/or tool calls).
    Assistant,
    /// The result of one tool call, keyed back by
    /// [`ChatMessage::tool_call_id`].
    Tool,
}

/// One message in the conversation handed to the model.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    /// Tool calls the model requested. Only ever non-empty on
    /// [`ChatRole::Assistant`] messages.
    pub tool_calls: Vec<ToolCall>,
    /// The [`ToolCall::id`] this message answers. Only ever `Some` on
    /// [`ChatRole::Tool`] messages.
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    /// A plain text message with no tool metadata — the common case.
    pub fn text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    /// An assistant message carrying the tool calls the model requested.
    pub fn assistant_tool_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    /// The result of one tool call, keyed back to the call it answers.
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

/// A tool the model may call, described to it as a JSON Schema.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    /// What the tool answers and how it behaves on denial — the model reads this
    /// verbatim, so it is part of the product surface, not an internal comment.
    pub description: String,
    /// JSON Schema for the arguments object.
    pub parameters: serde_json::Value,
}

/// One tool invocation the model requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// Provider-assigned id; the tool result must carry it back so the model can
    /// pair them.
    pub id: String,
    pub name: String,
    /// The raw arguments JSON as the model emitted it. Kept as a string because a
    /// model can emit syntactically invalid JSON here — parsing is the caller's
    /// job, and a parse failure is reported back to the model as data rather than
    /// failing the turn.
    pub arguments_json: String,
}

/// Everything one model turn needs.
#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub model: String,
    /// Oldest → newest, starting with the system message.
    pub messages: Vec<ChatMessage>,
    /// The tools the model may call this turn. Empty means text-only.
    pub tools: Vec<ToolDef>,
}

/// One item from a streaming model turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    /// An incremental piece of assistant text.
    TextDelta(String),
    /// The turn's fully-assembled tool calls (fragments already reassembled).
    ToolCalls(Vec<ToolCall>),
    /// The turn ended without tool calls.
    Done { finish_reason: String },
}

/// A model-call failure, split by who is at fault so the caller can respond
/// appropriately: [`Api`](LlmError::Api) is the provider rejecting us (bad key,
/// unknown model, rate limit), [`Transport`](LlmError::Transport) is the network,
/// and [`Protocol`](LlmError::Protocol) is a response we cannot interpret.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("llm api error (status {status}): {detail}")]
    Api { status: u16, detail: String },
    #[error("llm transport error: {0}")]
    Transport(String),
    #[error("llm protocol error: {0}")]
    Protocol(String),
}

/// A streaming, tool-call-capable chat model.
///
/// Object-safe (`async_trait`, matching [`crate::session_backend::SessionBackend`])
/// so callers hold `Arc<dyn ChatModelClient>` and can be handed a stub in tests.
#[async_trait]
pub trait ChatModelClient: Send + Sync {
    /// Start one model turn, returning the item stream.
    ///
    /// Errors returned here are pre-stream failures (request build, connect,
    /// non-2xx status). Failures that surface mid-stream arrive as `Err` items
    /// inside the stream instead.
    async fn stream_turn(
        &self,
        req: TurnRequest,
    ) -> Result<futures::stream::BoxStream<'static, Result<StreamItem, LlmError>>, LlmError>;
}
