//! The concierge's tool layer: what the model is allowed to do, and nothing else.
//!
//! Every capability the model has is a [`ChatTool`] registered in a
//! [`ToolRegistry`]. The orchestrator depends on the trait and the registry only —
//! never on a concrete tool — so later milestones extend the concierge's reach by
//! registering a tool, not by editing the loop.
//!
//! Two rules hold for every tool in this module tree and are the reason the layer
//! is safe by construction:
//!
//! 1. **Data reads go through [`SelfDispatch`], which is GET-only.** A tool cannot
//!    mutate anything, because the only transport it is given cannot.
//! 2. **HTTP error statuses are RESULTS, not failures.** A 403 comes back as data
//!    so the model can say "you don't have log access to that session" truthfully,
//!    instead of the turn dying with an opaque error. Only process-level faults
//!    (unknown tool, malformed arguments, dispatch machinery broken) are
//!    [`ToolError`]s.

use std::sync::Arc;

use async_trait::async_trait;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use secrecy::SecretString;

use super::dispatch::{DispatchError, SelfDispatch};
use super::llm::ToolDef;

pub mod read;

/// Characters escaped inside a single URL path segment or query value.
///
/// Everything except the RFC 3986 *unreserved* set (`ALPHA / DIGIT / - . _ ~`) is
/// escaped, so a session id containing a space or a slash cannot climb out of its
/// segment: `sess 1` becomes `sess%201`, `a/b` becomes `a%2Fb`. This mirrors the
/// encoding the SPA already asserts for the same endpoints.
const URL_COMPONENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// Percent-encode one path segment or query value.
pub(crate) fn encode(component: &str) -> String {
    utf8_percent_encode(component, URL_COMPONENT).to_string()
}

/// Per-turn context every tool call is executed against.
///
/// The bearer is the CALLER's own token — the tool layer holds no credential of its
/// own, which is what keeps chat inside the user's authority.
#[derive(Clone)]
pub struct ToolCtx {
    pub dispatch: SelfDispatch,
    pub bearer: SecretString,
    /// Optional broader-visibility token, forwarded only by the tools whose
    /// endpoint honors it.
    pub broader: Option<SecretString>,
}

/// What one tool call produced.
#[derive(Debug, Clone)]
pub struct ToolOutcome {
    /// The JSON handed back to the model.
    pub result_json: serde_json::Value,
    /// Whether the underlying payload was cut for size.
    pub truncated: bool,
    /// HTTP status for dispatch-backed tools; `None` for in-process tools (the
    /// orchestrator reads `None` as 200). This single field is what the SSE
    /// `tool_result` event and its UI rendering are built on.
    pub status: Option<u16>,
}

/// A tool-invocation fault. Distinct from "the API returned an error status" —
/// which is a successful call whose result the model must interpret.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("invalid tool arguments: {0}")]
    InvalidArgs(String),
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
}

/// One capability exposed to the model.
#[async_trait]
pub trait ChatTool: Send + Sync {
    /// The name, description and JSON-Schema parameters the model sees. The
    /// description is part of the product surface: the model decides whether to
    /// call the tool by reading it.
    fn def(&self) -> ToolDef;

    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutcome, ToolError>;
}

/// The set of tools one runtime exposes. Cheap to clone (the tools sit behind
/// `Arc`), so each turn can carry its own handle.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn ChatTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Arc<dyn ChatTool>) {
        self.tools.push(tool);
    }

    /// Every tool's definition, in registration order — the order the model sees.
    pub fn defs(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|tool| tool.def()).collect()
    }

    /// Whether a tool with this name is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.iter().any(|tool| tool.def().name == name)
    }

    /// Call a tool by name. An unrecognized name is a [`ToolError::UnknownTool`],
    /// which the orchestrator reports to the model as a tool error rather than
    /// failing the turn — models do occasionally hallucinate a tool name, and the
    /// recovery is for them to pick a real one.
    pub async fn invoke(
        &self,
        name: &str,
        ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.def().name == name)
            .ok_or_else(|| ToolError::UnknownTool(name.to_string()))?;
        tool.call(ctx, args).await
    }
}

// ---- argument helpers -----------------------------------------------------
//
// Every tool validates its own arguments through these, so a missing or mistyped
// value becomes a precise `InvalidArgs` message the model can act on ("owner must
// be a string") rather than a panic or a silently-empty path segment.

/// Read a required non-blank string argument.
pub(crate) fn required_str(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    let value = args
        .get(key)
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} is required")))?;
    let text = value
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be a string")))?;
    if text.trim().is_empty() {
        return Err(ToolError::InvalidArgs(format!("{key} must not be blank")));
    }
    Ok(text.to_string())
}

/// Read an optional string argument; `null` and blank both count as absent.
pub(crate) fn optional_str(
    args: &serde_json::Value,
    key: &str,
) -> Result<Option<String>, ToolError> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be a string")))?;
            Ok(Some(text.to_string()).filter(|t| !t.trim().is_empty()))
        }
    }
}

/// Read an optional integer argument and clamp it into `min..=max`.
///
/// Clamping rather than rejecting is deliberate: a model asking for
/// `tail_bytes: 10_000_000` wants "as much as possible", and serving the maximum is
/// a better answer than an error it has to recover from.
pub(crate) fn optional_clamped_u64(
    args: &serde_json::Value,
    key: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, ToolError> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => {
            let number = value
                .as_u64()
                .or_else(|| value.as_i64().map(|n| n.max(0) as u64))
                .or_else(|| value.as_f64().map(|n| n.max(0.0) as u64))
                .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be a number")))?;
            Ok(Some(number.clamp(min, max)))
        }
    }
}

/// Read a required integer argument (e.g. an issue number).
pub(crate) fn required_i64(args: &serde_json::Value, key: &str) -> Result<i64, ToolError> {
    let value = args
        .get(key)
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} is required")))?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|n| n as i64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} must be an integer")))
}

/// The registry the shipped concierge runs with: every read-only tool.
pub fn default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    read::register(&mut registry);
    registry
}
