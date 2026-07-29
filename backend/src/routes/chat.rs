//! `POST /api/v1/chat` — the concierge's single public surface.
//!
//! One authenticated request carries the visible conversation; the response is an
//! SSE stream of [`ChatStreamEvent`] frames produced by the orchestration loop.
//!
//! **The server is stateless across turns.** The client sends the history it is
//! displaying on every request and nothing is persisted — no conversation store, no
//! per-user transcript, nothing to leak, expire, or migrate. The transcript the user
//! sees IS the conversation.
//!
//! Mounting is conditional on the chat runtime being configured, so a deployment
//! with the feature off does not serve (or document) the route at all — the same
//! shape as the GitHub App webhook, whose presence likewise tracks live config.
//!
//! The wire types live HERE rather than in `crate::chat` because they ARE the
//! HTTP contract: they carry the `ToSchema` derives that put them in
//! `/openapi.json`, and the orchestrator emits them for this route to serialize.

use axum::extract::{DefaultBodyLimit, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::Json;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_stream::wrappers::ReceiverStream;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::chat::actions::ActionProposal;
use crate::chat::dispatch::SelfDispatch;
use crate::chat::orchestrator;
use crate::chat::tools::ToolCtx;
use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::routes::dashboard::bearer_token;
use crate::state::AppState;

/// SSE keep-alive interval. Emits a comment frame so an idle proxy does not drop a
/// stream while the model is still thinking; the SPA parser skips comment frames.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Buffered events between the orchestrator task and the response stream. Small on
/// purpose: back-pressure here means a slow client slows the loop rather than
/// letting an unbounded queue grow behind it.
const EVENT_BUFFER: usize = 32;

/// Who authored one message in the client-supplied history.
///
/// There is no `system` variant: the system prompt is the platform's and is never
/// accepted from a client, which is the first line of defence against a caller
/// rewriting the concierge's instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatClientRole {
    User,
    Assistant,
}

/// One message of the visible transcript.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ChatClientMessage {
    pub role: ChatClientRole,
    pub content: String,
}

/// One turn's request: the visible transcript, oldest → newest.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ChatRequest {
    /// Oldest → newest. The last message must be `role: "user"` — that is the
    /// message being answered.
    pub messages: Vec<ChatClientMessage>,
}

/// A session the turn's tool results identified, so the SPA can render a card that
/// deep-links to it.
///
/// Collected from STRUCTURED tool results, never parsed out of the model's prose:
/// a card that navigates somewhere must not be steerable by generated text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SessionRef {
    pub owner: String,
    pub name: String,
    /// Null until the session's runtime has been identified.
    pub session_id: Option<String>,
    pub trigger_number: i64,
    pub title: Option<String>,
    /// The first `fkst-*` status label, if any.
    pub status_label: Option<String>,
}

/// One frame of the response stream.
///
/// Serialized as `{"type": "...", ...}`; each frame is one SSE `data:` line. The
/// stream ALWAYS terminates with `done` or `error`, so a client never has to infer
/// completion from the socket closing.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatStreamEvent {
    /// An incremental piece of assistant text.
    Delta { text: String },
    /// A tool call is starting. `args_preview` is truncated — it exists to show the
    /// user what is being looked up, not to be re-parsed.
    ToolCall {
        id: String,
        name: String,
        args_preview: String,
    },
    /// A tool call finished. `status` is the underlying HTTP status (200 for
    /// in-process tools), so the UI can distinguish "denied" from "failed".
    ToolResult {
        id: String,
        name: String,
        status: u16,
        truncated: bool,
    },
    /// A confirm-gated action the user may review and execute.
    ///
    /// The chat backend never performs the action: this frame carries the complete,
    /// server-validated payload, the user confirms it, and the SPA calls the
    /// pre-existing REST endpoint with its own token. Nothing is stored server-side.
    ///
    /// Boxed because a proposal is far larger than any other frame, and every `Delta`
    /// would otherwise pay that size. It serializes transparently, so the wire shape is
    /// unchanged.
    ActionProposal { proposal: Box<ActionProposal> },
    /// A structured rendering of the tool result that just landed.
    ///
    /// Projected from the RESULT, never from the model's prose: a card that links
    /// somewhere, or states that a pull request merged, must not be steerable by
    /// generated text. Boxed for the same reason as a proposal — it is far larger than
    /// a `Delta`, which every token would otherwise pay for.
    DataCard {
        card: Box<crate::chat::cards::DataCard>,
    },
    /// The turn completed normally.
    Done {
        finish_reason: String,
        session_refs: Vec<SessionRef>,
    },
    /// The turn ended abnormally. `code` is stable and machine-readable
    /// (`deadline_exceeded`, `tool_budget_exhausted`, `llm_error`, `internal`);
    /// `message` is human-facing and never carries provider or credential detail.
    Error { code: String, message: String },
}

/// Validate the client-supplied history.
///
/// Over-long histories are deliberately NOT rejected — the orchestrator truncates
/// them oldest-first. The frontend sends the full visible transcript and should not
/// have to mirror a server-side cap to keep a long conversation working.
fn validate(request: &ChatRequest) -> Result<(), AppError> {
    if request.messages.is_empty() {
        return Err(AppError::Unprocessable(
            "messages must not be empty".to_string(),
        ));
    }
    if request
        .messages
        .last()
        .is_some_and(|m| m.role != ChatClientRole::User)
    {
        return Err(AppError::Unprocessable(
            "the last message must have role \"user\"".to_string(),
        ));
    }
    if let Some(index) = request
        .messages
        .iter()
        .position(|m| m.content.trim().is_empty())
    {
        return Err(AppError::Unprocessable(format!(
            "messages[{index}].content must not be empty"
        )));
    }
    Ok(())
}

/// `POST /api/v1/chat` — run one conversation turn, streaming the result.
#[utoipa::path(
    post,
    path = "/chat",
    tag = "chat",
    operation_id = "chat_turn",
    params(
        ("X-Github-Broader-Token" = Option<String>, Header, description = "Optional broader-visibility OAuth token, forwarded to the concierge's overview tool exactly as the dashboard forwards it, so chat sees the same repository set."),
    ),
    request_body = ChatRequest,
    responses(
        (status = 200, description = "SSE stream of ChatStreamEvent frames; always terminates with a `done` or `error` frame", content_type = "text/event-stream", body = ChatStreamEvent),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not admitted by the deployment access policy", body = ErrorEnvelope),
        (status = 413, description = "Request body exceeds FKST_CHAT_REQUEST_MAX_BYTES (a plain body-limit rejection, not an error envelope)"),
        (status = 422, description = "Empty history, an empty message, or a last message that is not role=user", body = ErrorEnvelope),
        (status = 429, description = "A turn is already in flight for this account, or global chat capacity is saturated", body = ErrorEnvelope),
        (status = 503, description = "Chat is not configured on this deployment", body = ErrorEnvelope),
    )
)]
pub(super) async fn post_chat(
    State(state): State<AppState>,
    user: GithubUser,
    headers: HeaderMap,
    Json(request): Json<ChatRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Unreachable in practice — the route is only mounted when the runtime exists —
    // but a 503 is the honest answer if that ever changes, rather than a panic.
    let runtime = state.chat.clone().ok_or_else(|| {
        AppError::Unavailable("chat is not configured on this deployment".to_string())
    })?;

    validate(&request)?;

    // The extractor verified the identity; the RAW token is what the tool layer
    // dispatches with, so chat acts strictly as this user.
    let bearer = bearer_token(&headers)?;
    let broader = headers
        .get(crate::routes::canvas::BROADER_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.strip_prefix("Bearer ").unwrap_or(value).trim())
        .filter(|value| !value.is_empty())
        .map(|value| secrecy::SecretString::from(value.to_string()));

    // Admitted BEFORE the stream starts, so saturation is a normal JSON 429 the SPA
    // can act on rather than an error frame inside a stream it already opened.
    let admission = runtime.limits().admit(user.id).await?;

    let (tx, rx) = tokio::sync::mpsc::channel(EVENT_BUFFER);
    let ctx = ToolCtx {
        dispatch: SelfDispatch::new(state.self_router.clone()),
        bearer,
        broader,
    };
    tracing::info!(
        github_user_id = user.id,
        messages = request.messages.len(),
        "chat turn accepted"
    );
    // The admission guard moves into the task: whether the turn completes, fails,
    // times out, or the browser disconnects, ending the task releases the slot.
    tokio::spawn(async move {
        orchestrator::run_turn(runtime, ctx, request.messages, tx).await;
        drop(admission);
    });

    Ok(Sse::new(event_stream(rx)).keep_alive(KeepAlive::new().interval(KEEPALIVE_INTERVAL)))
}

/// Adapt the orchestrator's event channel into SSE frames.
fn event_stream(
    rx: tokio::sync::mpsc::Receiver<ChatStreamEvent>,
) -> impl Stream<Item = Result<Event, std::convert::Infallible>> {
    use futures::StreamExt;
    ReceiverStream::new(rx).map(|event| Ok(to_sse_event(&event)))
}

/// Render one event as an SSE frame.
///
/// A serialization failure cannot be sent as the event it failed on, so it degrades
/// to a well-formed `error` frame — the stream's contract ("always ends with done or
/// error") holds even then.
fn to_sse_event(event: &ChatStreamEvent) -> Event {
    match serde_json::to_string(event) {
        Ok(json) => Event::default().data(json),
        Err(error) => {
            tracing::error!(error = %error, "chat event failed to serialize");
            Event::default().data(
                r#"{"type":"error","code":"internal","message":"the server could not encode an event"}"#,
            )
        }
    }
}

/// The chat router (nested under `/api/v1`).
///
/// Open at the app layer like every other `/api/v1` route: the per-request GitHub
/// token IS the auth (the [`GithubUser`] extractor), so there is no middleware and
/// no documented security scheme. The body limit is route-scoped because it is
/// derived from this feature's own config.
pub fn router(request_max_bytes: usize) -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(post_chat))
        .layer(DefaultBodyLimit::max(request_max_bytes))
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
