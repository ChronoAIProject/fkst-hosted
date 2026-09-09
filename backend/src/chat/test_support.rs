//! Shared chat test fixtures: a scripted model client and config/context builders.
//!
//! Split out so the orchestrator's own suite and the route suite drive the SAME stub
//! client — a second copy would let the two drift and hide a contract mismatch
//! between the loop and the endpoint that serves it.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};

use super::config::ChatConfig;
use super::dispatch::SelfDispatch;
use super::llm::{ChatModelClient, LlmError, StreamItem, ToolCall, TurnRequest};
use super::tools::ToolCtx;
use crate::state::empty_self_router;

/// A model client that replays scripted turns in order.
///
/// Each entry is one `stream_turn` response; the recorded requests let a test assert
/// what the loop actually sent back to the model.
pub(crate) struct ScriptedClient {
    turns: Mutex<std::collections::VecDeque<Result<Vec<StreamItem>, LlmError>>>,
    seen: Mutex<Vec<TurnRequest>>,
}

impl ScriptedClient {
    pub(crate) fn new(turns: Vec<Result<Vec<StreamItem>, LlmError>>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into()),
            seen: Mutex::new(Vec::new()),
        })
    }

    /// One scripted turn that streams `text` and stops — the simplest useful script.
    pub(crate) fn text_turn(text: &str) -> Arc<Self> {
        Self::new(vec![Ok(vec![
            StreamItem::TextDelta(text.to_string()),
            StreamItem::Done {
                finish_reason: "stop".to_string(),
            },
        ])])
    }

    pub(crate) fn requests(&self) -> Vec<TurnRequest> {
        self.seen.lock().expect("requests").clone()
    }
}

#[async_trait]
impl ChatModelClient for ScriptedClient {
    async fn stream_turn(
        &self,
        req: TurnRequest,
    ) -> Result<BoxStream<'static, Result<StreamItem, LlmError>>, LlmError> {
        self.seen.lock().expect("requests").push(req);
        let scripted = self
            .turns
            .lock()
            .expect("turns")
            .pop_front()
            .expect("the loop asked for more turns than the test scripted");
        match scripted {
            Err(error) => Err(error),
            Ok(items) => Ok(futures::stream::iter(items.into_iter().map(Ok)).boxed()),
        }
    }
}

/// A model client whose stream never ends — for deadline and in-flight cases.
pub(crate) struct HangingClient;

#[async_trait]
impl ChatModelClient for HangingClient {
    async fn stream_turn(
        &self,
        _req: TurnRequest,
    ) -> Result<BoxStream<'static, Result<StreamItem, LlmError>>, LlmError> {
        Ok(futures::stream::pending().boxed())
    }
}

/// An enabled [`ChatConfig`] built through the real parser (so a test never
/// constructs one the parser would have rejected).
pub(crate) fn config() -> ChatConfig {
    config_with(&[])
}

/// [`config`] with extra `FKST_CHAT_*` overrides applied.
pub(crate) fn config_with(overrides: &[(&str, &str)]) -> ChatConfig {
    let mut vars: Vec<(String, String)> = vec![
        ("FKST_CHAT_ENABLED".to_string(), "true".to_string()),
        (
            "FKST_LLM_BASE_URL".to_string(),
            "https://llm.example/v1".to_string(),
        ),
        ("FKST_LLM_API_KEY".to_string(), "k".to_string()),
        ("FKST_LLM_MODEL".to_string(), "test-model".to_string()),
    ];
    vars.extend(
        overrides
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string())),
    );
    super::config::from_vars(&vars)
        .expect("test chat config parses")
        .expect("test chat config is enabled")
}

/// A [`ToolCtx`] whose router handle is empty: any dispatch fails immediately, which
/// is what loop-level tests want (they use stub tools, never real dispatch).
pub(crate) fn ctx() -> ToolCtx {
    ToolCtx {
        dispatch: SelfDispatch::new(empty_self_router()),
        bearer: secrecy::SecretString::from("gho_test".to_string()),
        broader: None,
    }
}

/// A tool call as a provider would emit it.
pub(crate) fn call(id: &str, name: &str, arguments_json: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments_json: arguments_json.to_string(),
    }
}
