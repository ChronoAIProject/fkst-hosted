//! The chat concierge: the conversational surface through which users ask about
//! and monitor their fkst sessions.
//!
//! Architecture (each layer plugs in behind an abstraction, none reaches past its
//! neighbour):
//!
//! * [`config`] — the fail-closed `FKST_CHAT_*` block. The whole feature is dark
//!   unless an operator enables it, so a deployment that never wants a chat
//!   surface carries no runtime cost and validates no chat variable.
//! * [`llm`] — wire-neutral message/tool/stream types plus the
//!   [`ChatModelClient`](llm::ChatModelClient) contract. Callers depend only on
//!   this trait, so swapping the provider wire is a new implementation, not a
//!   refactor.
//! * [`llm_openai`] — the one shipped implementation: OpenAI-compatible
//!   chat-completions, streaming, with tool-call fragment reassembly.
//! * [`dispatch`] — GET-only, in-process dispatch of data reads through this
//!   deployment's own router, carrying the caller's token.
//! * [`tools`] — the [`ChatTool`](tools::ChatTool) registry: every capability the
//!   model has, and nothing else. The orchestrator depends on the trait, never on a
//!   concrete tool, so later milestones extend the concierge by registering a tool.
//! * [`knowledge`] — the compiled-in operator manual, split into searchable sections,
//!   with a drift guard tying it to the backend's own label and heading constants.
//! * [`prompt`] — the system prompt: grounding, injection resistance, and the manual's
//!   table of contents.
//! * [`limits`] — per-user and process-wide admission control for turns.
//! * [`orchestrator`] — the model↔tools loop, emitting wire events as they happen.
//!
//! [`ChatRuntime`] ties them together and is what the router holds.
//!
//! Security posture, stated once here because every later layer inherits it: the
//! chat backend is a **client of the public API acting with the calling user's own
//! token**. It gains no authority of its own — no service account, no elevated
//! path — so a user can never see through chat what they could not see on the
//! dashboard.

pub mod config;
pub mod dispatch;
pub mod knowledge;
pub mod limits;
pub mod llm;
pub mod llm_openai;
pub mod orchestrator;
pub mod prompt;
pub mod tools;

// Shared stub model client + config/context builders, driven by BOTH the
// orchestrator suite and the route suite so the loop and the endpoint that
// serves it cannot drift apart behind two different stubs.
#[cfg(test)]
pub(crate) mod test_support;

use std::sync::Arc;

use config::ChatConfig;
use limits::ChatLimits;
use llm::ChatModelClient;
use tools::ToolRegistry;

/// Everything one deployment's chat feature needs at runtime.
///
/// Fields are private so `state.chat` can stay `pub` without exposing
/// `Arc<dyn ChatModelClient>` in a public interface, and so the two constructors
/// remain the only ways to build one.
pub struct ChatRuntime {
    config: ChatConfig,
    client: Arc<dyn ChatModelClient>,
    registry: ToolRegistry,
    limits: ChatLimits,
    /// Computed once at construction — the prompt is a pure function of the
    /// deployment's knowledge base, so recomputing it per turn would only burn CPU.
    system_prompt: String,
}

impl ChatRuntime {
    /// The production runtime: the OpenAI-compatible client and the shipped tools.
    pub fn from_config(config: ChatConfig) -> Self {
        let client = Arc::new(llm_openai::OpenAiCompatClient::new(
            config.base_url.clone(),
            config.api_key.clone(),
        ));
        Self::with_client(config, client, tools::default_registry())
    }

    /// Test seam: inject a scripted model client and/or a stub registry.
    pub(crate) fn with_client(
        config: ChatConfig,
        client: Arc<dyn ChatModelClient>,
        registry: ToolRegistry,
    ) -> Self {
        let limits = ChatLimits::new(config.max_concurrent_turns);
        Self {
            config,
            client,
            registry,
            limits,
            // Computed once here: the prompt is a pure function of the compiled-in
            // manual, so a per-turn rebuild would only burn CPU.
            system_prompt: prompt::system_prompt(&knowledge::toc()),
        }
    }

    pub fn config(&self) -> &ChatConfig {
        &self.config
    }

    pub fn limits(&self) -> &ChatLimits {
        &self.limits
    }

    pub(crate) fn client(&self) -> &Arc<dyn ChatModelClient> {
        &self.client
    }

    pub(crate) fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub(crate) fn system_prompt(&self) -> &str {
        &self.system_prompt
    }
}
