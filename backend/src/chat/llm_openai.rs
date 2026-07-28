//! OpenAI-compatible ([chat-completions wire]) streaming implementation of
//! [`ChatModelClient`].
//!
//! Hand-rolled over `reqwest` rather than pulled from an SDK, matching the
//! crate's existing GitHub/OpenSandbox transports (see `Cargo.toml`'s "no GitHub
//! SDK" note): the surface used here is one POST and one SSE body, and an SDK
//! would add a large dependency for it.
//!
//! Two provider behaviours drive the shape of the decoder:
//!
//! * **Text arrives split across frames** — forwarded as [`StreamItem::TextDelta`]
//!   the moment each frame decodes, so the SSE endpoint can push tokens to the
//!   browser while the model is still writing.
//! * **Tool calls arrive fragmented** — `id`/`name` on the first fragment for an
//!   index, `function.arguments` concatenated across later ones. Fragments are
//!   accumulated by index and emitted as ONE [`StreamItem::ToolCalls`], because a
//!   half-decoded call is useless to a caller.
//!
//! [chat-completions wire]: https://platform.openai.com/docs/api-reference/chat/streaming

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use futures::stream::{BoxStream, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;

use super::llm::{ChatModelClient, ChatRole, LlmError, StreamItem, ToolCall, TurnRequest};

/// Connect timeout for the provider. There is deliberately NO request timeout: the
/// response body is a long-lived stream a whole-request budget would sever
/// mid-answer (the same reasoning as the OpenSandbox client in `main.rs`). The
/// per-turn wall clock is enforced by the caller via `FKST_CHAT_TURN_DEADLINE_SECS`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How much of a provider error body is carried into [`LlmError::Api`]. Enough to
/// diagnose ("model not found", "invalid api key"), bounded so a hostile or broken
/// endpoint cannot flood the logs.
const ERROR_DETAIL_MAX_BYTES: usize = 2048;

/// A pooled client shared by every turn. `reqwest::Client` owns a connection pool,
/// so rebuilding per turn would discard keep-alive (the convention in
/// [`crate::github_identity`]).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent("fkst-hosted")
            .build()
            .expect("build chat llm http client")
    })
}

/// An OpenAI-compatible chat-completions client.
pub struct OpenAiCompatClient {
    base_url: reqwest::Url,
    api_key: SecretString,
}

impl OpenAiCompatClient {
    pub fn new(base_url: reqwest::Url, api_key: SecretString) -> Self {
        Self { base_url, api_key }
    }

    /// Resolve `{base}/chat/completions`.
    ///
    /// Deliberately NOT `Url::join`: joining a relative segment onto a base without
    /// a trailing slash discards the base's last path segment, so
    /// `https://host/v1` + `chat/completions` would silently become
    /// `https://host/chat/completions`. Trim-and-append is correct with or without
    /// the trailing slash.
    fn endpoint(&self) -> Result<reqwest::Url, LlmError> {
        let trimmed = self.base_url.as_str().trim_end_matches('/');
        reqwest::Url::parse(&format!("{trimmed}/chat/completions")).map_err(|e| {
            LlmError::Protocol(format!("chat base url does not form a valid endpoint: {e}"))
        })
    }
}

/// Map the neutral request onto the provider's JSON body.
fn request_body(req: &TurnRequest) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                ChatRole::System => "system",
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
                ChatRole::Tool => "tool",
            };
            let mut value = json!({ "role": role, "content": message.content });
            if !message.tool_calls.is_empty() {
                value["tool_calls"] = message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": { "name": call.name, "arguments": call.arguments_json },
                        })
                    })
                    .collect();
            }
            if let Some(id) = &message.tool_call_id {
                value["tool_call_id"] = json!(id);
            }
            value
        })
        .collect();

    let mut body = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    },
                })
            })
            .collect();
    }
    body
}

#[async_trait]
impl ChatModelClient for OpenAiCompatClient {
    async fn stream_turn(
        &self,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, Result<StreamItem, LlmError>>, LlmError> {
        let endpoint = self.endpoint()?;
        // Never the key, never message contents — only shape.
        tracing::debug!(
            model = %req.model,
            messages = req.messages.len(),
            tools = req.tools.len(),
            "chat model turn starting"
        );

        let response = http_client()
            .post(endpoint)
            .bearer_auth(self.api_key.expose_secret())
            .json(&request_body(&req))
            .send()
            .await
            .map_err(|e| LlmError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            // The body is the only place a provider explains itself, so it is
            // carried through (bounded) — it is provider text, not our secret.
            let detail = response.text().await.unwrap_or_default();
            let detail = truncate_on_char_boundary(&detail, ERROR_DETAIL_MAX_BYTES).to_string();
            tracing::warn!(status = status.as_u16(), "chat model api error");
            return Err(LlmError::Api {
                status: status.as_u16(),
                detail,
            });
        }

        let bytes = response.bytes_stream().boxed();
        Ok(decode_sse(bytes))
    }
}

/// Truncate at the last UTF-8 character boundary at or before `max_bytes`.
fn truncate_on_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Accumulator for one tool call's fragments (keyed by the provider's `index`).
#[derive(Default)]
struct ToolFragment {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// Decoder state threaded through [`futures::stream::unfold`].
struct SseState {
    bytes: BoxStream<'static, reqwest::Result<Bytes>>,
    /// Raw, still-undelimited body bytes. Kept as BYTES (not a `String`) so a
    /// multi-byte character split across two network chunks cannot corrupt: only
    /// complete frames are ever decoded as UTF-8.
    buffer: Vec<u8>,
    /// Items decoded from one chunk but not yet yielded (a single chunk commonly
    /// carries several frames).
    pending: std::collections::VecDeque<Result<StreamItem, LlmError>>,
    tools: BTreeMap<u64, ToolFragment>,
    /// Set once the tool calls have been emitted, so an end-of-stream flush cannot
    /// emit them twice.
    tools_flushed: bool,
    done: bool,
}

/// Turn the provider's raw SSE body into neutral [`StreamItem`]s.
fn decode_sse(
    bytes: BoxStream<'static, reqwest::Result<Bytes>>,
) -> BoxStream<'static, Result<StreamItem, LlmError>> {
    let state = SseState {
        bytes,
        buffer: Vec::new(),
        pending: std::collections::VecDeque::new(),
        tools: BTreeMap::new(),
        tools_flushed: false,
        done: false,
    };
    futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(item) = state.pending.pop_front() {
                return Some((item, state));
            }
            if state.done {
                return None;
            }
            match state.bytes.next().await {
                Some(Ok(chunk)) => {
                    state.buffer.extend_from_slice(&chunk);
                    drain_frames(&mut state);
                }
                Some(Err(error)) => {
                    state.done = true;
                    state
                        .pending
                        .push_back(Err(LlmError::Transport(error.to_string())));
                }
                None => {
                    // Body closed. A provider that ended the turn with tool calls
                    // but never sent `finish_reason: "tool_calls"` still gets its
                    // work honored rather than dropped.
                    state.done = true;
                    flush_tool_calls(&mut state);
                }
            }
        }
    })
    .boxed()
}

/// Split every complete `\n\n`-delimited frame out of the buffer and process it.
fn drain_frames(state: &mut SseState) {
    while let Some(position) = find_frame_end(&state.buffer) {
        let frame: Vec<u8> = state.buffer.drain(..position.0).collect();
        state.buffer.drain(..position.1);
        process_frame(state, &frame);
        if state.done {
            return;
        }
    }
}

/// Locate the first frame terminator, returning `(frame_len, terminator_len)`.
/// Both `\n\n` and CRLF-style `\r\n\r\n` are accepted — providers differ.
fn find_frame_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n");
    let crlf = buffer.windows(4).position(|w| w == b"\r\n\r\n");
    match (lf, crlf) {
        (Some(l), Some(c)) if c <= l => Some((c, 4)),
        (Some(l), _) => Some((l, 2)),
        (None, Some(c)) => Some((c, 4)),
        (None, None) => None,
    }
}

/// Interpret one SSE frame: concatenate its `data:` lines and decode the payload.
/// Comment/keep-alive frames (no `data:` line) are skipped.
fn process_frame(state: &mut SseState, frame: &[u8]) {
    let Ok(text) = std::str::from_utf8(frame) else {
        state.done = true;
        state.pending.push_back(Err(LlmError::Protocol(
            "chat model stream frame was not valid utf-8".to_string(),
        )));
        return;
    };

    let mut payload = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            payload.push_str(rest.trim_start());
        }
    }
    if payload.is_empty() {
        return;
    }
    if payload == "[DONE]" {
        state.done = true;
        flush_tool_calls(state);
        return;
    }

    let chunk: StreamChunk = match serde_json::from_str(&payload) {
        Ok(chunk) => chunk,
        Err(error) => {
            // Fail loudly: a payload we cannot parse means we are mis-reading the
            // provider, and silently skipping it would truncate the answer with no
            // trace. The payload itself is not echoed (it can hold model text).
            state.done = true;
            state.pending.push_back(Err(LlmError::Protocol(format!(
                "chat model stream frame was not valid json: {error}"
            ))));
            return;
        }
    };

    let Some(choice) = chunk.choices.into_iter().next() else {
        // Usage-only / heartbeat chunks carry no choice — nothing to emit.
        return;
    };

    if let Some(delta) = choice.delta {
        if let Some(content) = delta.content.filter(|c| !c.is_empty()) {
            state.pending.push_back(Ok(StreamItem::TextDelta(content)));
        }
        for fragment in delta.tool_calls {
            let entry = state.tools.entry(fragment.index).or_default();
            if let Some(id) = fragment.id {
                entry.id = Some(id);
            }
            if let Some(function) = fragment.function {
                if let Some(name) = function.name {
                    entry.name = Some(name);
                }
                if let Some(arguments) = function.arguments {
                    entry.arguments.push_str(&arguments);
                }
            }
        }
    }

    match choice.finish_reason.as_deref() {
        None => {}
        Some("tool_calls") => flush_tool_calls(state),
        Some(reason) => state.pending.push_back(Ok(StreamItem::Done {
            finish_reason: reason.to_string(),
        })),
    }
}

/// Emit the accumulated tool calls as one item (at most once per turn).
///
/// A fragment without a name is dropped: it names no callable tool, so passing it
/// on would only produce an `UnknownTool` further down. Its absence is logged.
fn flush_tool_calls(state: &mut SseState) {
    if state.tools_flushed || state.tools.is_empty() {
        return;
    }
    state.tools_flushed = true;
    let mut calls = Vec::with_capacity(state.tools.len());
    for (index, fragment) in std::mem::take(&mut state.tools) {
        match fragment.name {
            Some(name) => calls.push(ToolCall {
                // A provider that omits the id still gets a stable pairing key.
                id: fragment.id.unwrap_or_else(|| format!("call_{index}")),
                name,
                arguments_json: fragment.arguments,
            }),
            None => tracing::warn!(
                index,
                "chat model tool-call fragment carried no function name; dropped"
            ),
        }
    }
    if !calls.is_empty() {
        state.pending.push_back(Ok(StreamItem::ToolCalls(calls)));
    }
}

// ---- provider wire shapes (deserialize-only) ------------------------------

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<StreamDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallFragment>,
}

#[derive(Debug, Deserialize)]
struct ToolCallFragment {
    /// The call's position in the turn's tool-call list; the ONLY reliable key for
    /// stitching fragments (`id` appears on the first fragment only).
    #[serde(default)]
    index: u64,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionFragment>,
}

#[derive(Debug, Deserialize)]
struct FunctionFragment {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[cfg(test)]
#[path = "llm_openai_tests.rs"]
mod tests;
