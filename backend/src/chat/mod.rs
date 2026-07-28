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
//!
//! Security posture, stated once here because every later layer inherits it: the
//! chat backend is a **client of the public API acting with the calling user's own
//! token**. It gains no authority of its own — no service account, no elevated
//! path — so a user can never see through chat what they could not see on the
//! dashboard.

pub mod config;
pub mod dispatch;
pub mod llm;
pub mod llm_openai;
pub mod tools;
