//! The model↔tools loop: one chat turn, driven to completion, emitting wire events
//! as they happen.
//!
//! Runs in its own task so the HTTP handler can return the SSE response
//! immediately — which is also why the `/api/v1` nest's response timeout never
//! severs a stream: the handler's future finishes at once and the body outlives it.
//!
//! Every exit path emits a terminal frame (`done` or `error`), so a client never has
//! to infer completion from a closed socket. Three bounds keep a turn finite:
//! the whole-turn deadline, the tool-iteration budget, and the caller's admission
//! guard (dropped when this task ends).

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::mpsc::Sender;

use super::llm::{ChatMessage, ChatRole, LlmError, StreamItem, ToolCall, TurnRequest};
use super::tools::{ToolCtx, ToolError};
use super::ChatRuntime;
use crate::routes::chat::{ChatClientMessage, ChatClientRole, ChatStreamEvent, SessionRef};

/// How much of a tool call's arguments the `tool_call` event carries. Enough to show
/// the user WHAT is being looked up; not a re-parsable payload.
const ARGS_PREVIEW_MAX_CHARS: usize = 200;

/// Maximum session cards one turn advertises. A turn that touched dozens of sessions
/// should surface the first few, not bury the answer under a wall of cards.
const MAX_SESSION_REFS: usize = 6;

/// Why a turn stopped early.
enum TurnFailure {
    /// The whole-turn deadline elapsed.
    DeadlineExceeded,
    /// The model kept calling tools past `max_tool_iterations`.
    ToolBudgetExhausted,
    /// The provider failed. Details are logged, never streamed.
    Llm(LlmError),
    /// The client disconnected — there is nobody left to tell.
    ClientGone,
}

impl TurnFailure {
    /// The frame to stream, or `None` when there is no client to receive it.
    fn event(&self) -> Option<ChatStreamEvent> {
        let (code, message) = match self {
            Self::DeadlineExceeded => (
                "deadline_exceeded",
                "the assistant took too long to answer; please try again",
            ),
            Self::ToolBudgetExhausted => (
                "tool_budget_exhausted",
                "the assistant needed too many lookups to answer; try a narrower question",
            ),
            Self::Llm(_) => (
                "llm_error",
                "the language model provider could not be reached; please try again",
            ),
            Self::ClientGone => return None,
        };
        Some(ChatStreamEvent::Error {
            code: code.to_string(),
            message: message.to_string(),
        })
    }
}

/// Run one conversation turn to completion.
///
/// Never returns an error: a turn's outcome is always communicated as a frame on
/// `tx`, because the HTTP response has already been handed to the client by the time
/// this runs.
pub async fn run_turn(
    runtime: Arc<ChatRuntime>,
    ctx: ToolCtx,
    client_messages: Vec<ChatClientMessage>,
    tx: Sender<ChatStreamEvent>,
) {
    let deadline = std::time::Duration::from_secs(runtime.config().turn_deadline_secs);
    let outcome = tokio::time::timeout(deadline, drive(&runtime, &ctx, client_messages, &tx)).await;

    let failure = match outcome {
        Ok(Ok(())) => return,
        Ok(Err(failure)) => failure,
        Err(_) => TurnFailure::DeadlineExceeded,
    };
    if let TurnFailure::Llm(error) = &failure {
        // The provider's own words can name a model or quote a request; keep them in
        // the log and send the client a generic message.
        tracing::warn!(error = %error, "chat turn failed at the model provider");
    }
    if let Some(event) = failure.event() {
        let _ = tx.send(event).await;
    }
}

/// The loop proper.
async fn drive(
    runtime: &ChatRuntime,
    ctx: &ToolCtx,
    client_messages: Vec<ChatClientMessage>,
    tx: &Sender<ChatStreamEvent>,
) -> Result<(), TurnFailure> {
    let config = runtime.config();
    let mut messages = vec![ChatMessage::text(ChatRole::System, runtime.system_prompt())];
    messages.extend(
        truncate_history(client_messages, config.history_max_messages)
            .into_iter()
            .map(to_chat_message),
    );

    let tools = runtime.registry().defs();
    let mut session_refs: Vec<SessionRef> = Vec::new();

    for iteration in 0..config.max_tool_iterations {
        let request = TurnRequest {
            model: config.model.clone(),
            messages: messages.clone(),
            tools: tools.clone(),
        };
        let mut stream = runtime
            .client()
            .stream_turn(request)
            .await
            .map_err(TurnFailure::Llm)?;

        let mut assistant_text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut finish_reason: Option<String> = None;

        while let Some(item) = stream.next().await {
            match item.map_err(TurnFailure::Llm)? {
                StreamItem::TextDelta(text) => {
                    assistant_text.push_str(&text);
                    send(tx, ChatStreamEvent::Delta { text }).await?;
                }
                StreamItem::ToolCalls(requested) => calls = requested,
                StreamItem::Done { finish_reason: r } => finish_reason = Some(r),
            }
        }

        // No tool calls means the model answered. A stream that ended without an
        // explicit finish reason is still a completed answer — some providers just
        // close the body — so it is reported as a normal stop rather than an error.
        if calls.is_empty() {
            send(
                tx,
                ChatStreamEvent::Done {
                    finish_reason: finish_reason.unwrap_or_else(|| "stop".to_string()),
                    session_refs,
                },
            )
            .await?;
            return Ok(());
        }

        tracing::debug!(
            iteration,
            calls = calls.len(),
            "chat turn running tool calls"
        );
        messages.push(ChatMessage::assistant_tool_calls(
            assistant_text,
            calls.clone(),
        ));
        for call in &calls {
            run_one_call(runtime, ctx, tx, call, &mut messages, &mut session_refs).await?;
        }
    }

    Err(TurnFailure::ToolBudgetExhausted)
}

/// Execute one tool call, emit its two events, and append its result to the
/// conversation.
async fn run_one_call(
    runtime: &ChatRuntime,
    ctx: &ToolCtx,
    tx: &Sender<ChatStreamEvent>,
    call: &ToolCall,
    messages: &mut Vec<ChatMessage>,
    session_refs: &mut Vec<SessionRef>,
) -> Result<(), TurnFailure> {
    send(
        tx,
        ChatStreamEvent::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            args_preview: truncate_chars(&call.arguments_json, ARGS_PREVIEW_MAX_CHARS),
        },
    )
    .await?;

    // Every failure below becomes a tool RESULT the model can react to, not a dead
    // turn: a model that emitted invalid arguments or an unknown tool name can
    // recover within the same turn if it is told what went wrong.
    let mut proposal = None;
    let (result_json, status, truncated) = match serde_json::from_str::<serde_json::Value>(
        arguments_or_empty(&call.arguments_json),
    ) {
        Ok(args) => match runtime
            .registry()
            .invoke(&call.name, ctx, args.clone())
            .await
        {
            Ok(outcome) => {
                collect_session_refs(session_refs, &call.name, &args, &outcome.result_json);
                proposal = outcome.proposal;
                (outcome.result_json, outcome.status, outcome.truncated)
            }
            Err(error) => {
                // A dispatch fault is a real server problem; the other two are the
                // model's mistakes. Both are surfaced to the model identically, but
                // only the former is worth a warning.
                if matches!(error, ToolError::Dispatch(_)) {
                    tracing::warn!(tool = %call.name, error = %error, "chat tool dispatch failed");
                }
                (
                    serde_json::json!({ "error": error.to_string() }),
                    None,
                    false,
                )
            }
        },
        Err(_) => (
            serde_json::json!({
                "error": "invalid tool arguments: the arguments were not valid JSON",
            }),
            None,
            false,
        ),
    };

    send(
        tx,
        ChatStreamEvent::ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            // In-process tools carry no HTTP status; 200 is the honest rendering of
            // "it ran and produced a result".
            status: status.unwrap_or(200),
            truncated,
        },
    )
    .await?;

    // A drafted action follows its tool result immediately, so the card lands next to
    // the sentence introducing it. Several proposals in one turn each get their own
    // frame; the server keeps none of them — the frame carries everything the SPA needs.
    if let Some(proposal) = proposal {
        send(
            tx,
            ChatStreamEvent::ActionProposal {
                proposal: Box::new(proposal),
            },
        )
        .await?;
    }

    messages.push(ChatMessage::tool_result(&call.id, result_json.to_string()));
    Ok(())
}

/// An absent/blank arguments payload means "no arguments" — providers emit `""` for
/// a zero-argument call, which is not valid JSON.
fn arguments_or_empty(arguments_json: &str) -> &str {
    if arguments_json.trim().is_empty() {
        "{}"
    } else {
        arguments_json
    }
}

/// Send one frame, mapping a closed channel to [`TurnFailure::ClientGone`].
///
/// A closed channel means the browser went away; aborting promptly stops us paying a
/// provider to write into a void.
async fn send(tx: &Sender<ChatStreamEvent>, event: ChatStreamEvent) -> Result<(), TurnFailure> {
    tx.send(event).await.map_err(|_| {
        tracing::debug!("chat client disconnected mid-turn; abandoning the turn");
        TurnFailure::ClientGone
    })
}

/// Trim a history down to `max` messages, dropping the OLDEST first.
///
/// Truncating rather than rejecting keeps long conversations working, and dropping
/// from the front preserves both the most recent context and the invariant that the
/// last message is the user's question.
fn truncate_history(mut messages: Vec<ChatClientMessage>, max: usize) -> Vec<ChatClientMessage> {
    if messages.len() > max {
        messages.drain(..messages.len() - max);
    }
    messages
}

fn to_chat_message(message: ChatClientMessage) -> ChatMessage {
    let role = match message.role {
        ChatClientRole::User => ChatRole::User,
        ChatClientRole::Assistant => ChatRole::Assistant,
    };
    ChatMessage::text(role, message.content)
}

/// Cut a string to `max` CHARACTERS (not bytes), so a preview never splits a
/// multi-byte character.
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

/// Harvest session references from a successful, structured tool result.
///
/// Deliberately narrow: only tools whose response shape is known contribute, and
/// only on a 200. The model's prose is NEVER parsed for card data — a card that
/// navigates the user somewhere must not be steerable by generated text.
fn collect_session_refs(
    refs: &mut Vec<SessionRef>,
    tool_name: &str,
    args: &serde_json::Value,
    result: &serde_json::Value,
) {
    if refs.len() >= MAX_SESSION_REFS || result["status"].as_u64() != Some(200) {
        return;
    }
    if tool_name != "list_repo_sessions" {
        // `observe_session` knows only a session id — no repo, no trigger number —
        // so it cannot form a card that links anywhere. Skipping it beats emitting a
        // card with a dead link.
        return;
    }
    let body = &result["body"];
    // Prefer the response's canonical casing over the caller's raw arguments.
    let owner = body["owner"]
        .as_str()
        .or_else(|| args["owner"].as_str())
        .unwrap_or_default()
        .to_string();
    let name = body["name"]
        .as_str()
        .or_else(|| args["name"].as_str())
        .unwrap_or_default()
        .to_string();
    if owner.is_empty() || name.is_empty() {
        return;
    }
    let Some(sessions) = body["sessions"].as_array() else {
        return;
    };
    for session in sessions {
        if refs.len() >= MAX_SESSION_REFS {
            return;
        }
        let Some(trigger_number) = session["trigger"]["number"].as_i64() else {
            continue;
        };
        let candidate = SessionRef {
            owner: owner.clone(),
            name: name.clone(),
            session_id: session["session_id"].as_str().map(str::to_string),
            trigger_number,
            title: session["name"].as_str().map(str::to_string),
            status_label: session["status_labels"]
                .as_array()
                .and_then(|labels| labels.first())
                .and_then(|label| label.as_str())
                .map(str::to_string),
        };
        // One card per session even if several tool calls returned it.
        let duplicate = refs.iter().any(|existing| {
            existing.owner == candidate.owner
                && existing.name == candidate.name
                && existing.trigger_number == candidate.trigger_number
        });
        if !duplicate {
            refs.push(candidate);
        }
    }
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod tests;
