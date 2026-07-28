//! Confirm-gated action proposals: the concierge may DRAFT a mutation, never perform one.
//!
//! The security architecture is fixed and is the whole reason this module is shaped the
//! way it is:
//!
//! 1. The model calls a proposal tool, which VALIDATES and RENDERS a draft.
//! 2. The draft is streamed to the SPA as a structured `action_proposal` event.
//! 3. A human reviews the exact payload and confirms.
//! 4. The **SPA** then calls the pre-existing REST endpoint with the user's own token —
//!    the same code path the dashboard's own buttons use.
//!
//! So prompt-injection blast radius for mutations is **zero**: a hijacked model can at
//! worst present a strange proposal card. No new write surface exists, and the chat
//! backend never holds write capability.
//!
//! The module is split by the RESOURCE a proposal acts on, because that is the axis
//! along which the validation rules differ:
//!
//! * [`session`] — the session lifecycle: start, queue work, stop.
//! * [`resources`] — everything a session runs *on*: repositories, named environment
//!   profiles, and the App installation itself.
//!
//! Both halves build the one [`ActionProposal`] union defined here, so the SPA has a
//! single wire contract to match and the orchestrator one type to stream.
//!
//! ## The secrets rule, enforced structurally
//!
//! No proposal variant has a field that can hold a secret VALUE. A session draft mirrors
//! `CreateSessionRequest` minus its `disposable_environment`; an environment draft carries
//! secret KEY NAMES only, and the user types the values into the confirmation card. A
//! secret therefore has nowhere to go in a draft — that is a type-level guarantee rather
//! than a rule someone has to remember.

pub mod resources;
pub mod session;

use serde::Serialize;
use utoipa::ToSchema;

pub use resources::{
    propose_create_repository, propose_delete_environment_profile,
    propose_save_environment_profile, propose_uninstall_app,
};
pub use session::{
    propose_create_session, propose_stop_session, propose_work_item, DraftSessionRequest,
};

/// Why a draft was rejected. The message is returned to the MODEL as tool-result data,
/// so it must be precise enough for the model to fix the draft and retry in the same
/// turn.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProposalError(pub String);

impl ProposalError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Descriptive metadata for the preview card: which endpoint a confirmation will reach.
///
/// **Display only.** The SPA maps `kind` to its own typed API function and must never
/// blindly fetch `path` — a generic method/path executor driven by model output would
/// reintroduce exactly the write capability this design removes.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct ActionTarget {
    pub method: String,
    pub path: String,
}

/// One non-secret environment variable in an environment draft.
///
/// A list of pairs rather than a map so the card renders them in the order the model
/// wrote them, which is the order the user reasoned about.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct EnvVarDraft {
    pub key: String,
    pub value: String,
}

/// A confirm-gated action the user may review and execute.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionProposal {
    /// Open a trigger issue — start a session.
    CreateSession {
        owner: String,
        name: String,
        request: DraftSessionRequest,
        /// The EXACT issue body a confirmation will file, rendered by the same function
        /// the real endpoint uses. Preview equals reality.
        rendered_issue_body: String,
        /// One line the card shows above the preview.
        summary: String,
        target: ActionTarget,
    },
    /// Open a labeled, assigned work issue on an existing session.
    CreateWorkItem {
        owner: String,
        name: String,
        trigger_issue_number: i64,
        title: String,
        label: Option<String>,
        body: String,
        summary: String,
        target: ActionTarget,
    },
    /// Close a trigger issue — retire a session permanently.
    StopSession {
        owner: String,
        name: String,
        trigger_issue_number: i64,
        /// Shown on the card so the user sees why the assistant suggested it. Not sent
        /// anywhere on confirmation.
        reason: String,
        summary: String,
        target: ActionTarget,
    },
    /// Create a repository as the signed-in user.
    CreateRepository {
        /// Organization to create under; `None` means the viewer's personal account.
        owner: Option<String>,
        name: String,
        private: bool,
        description: Option<String>,
        summary: String,
        target: ActionTarget,
    },
    /// Create or replace a named environment profile.
    ///
    /// `secret_keys` carries NAMES only. The card collects the values, which therefore
    /// never transit the model, this event, or any server-side log.
    SaveEnvironmentProfile {
        profile_name: String,
        /// Whether a profile with this name already exists, so the card can say
        /// "replace" rather than "create". `None` when the check could not run.
        replaces_existing: Option<bool>,
        install: Vec<String>,
        variables: Vec<EnvVarDraft>,
        secret_keys: Vec<String>,
        summary: String,
        target: ActionTarget,
    },
    /// Delete a named environment profile.
    DeleteEnvironmentProfile {
        profile_name: String,
        summary: String,
        target: ActionTarget,
    },
    /// Uninstall the fkst GitHub App from one account.
    UninstallApp {
        owner: String,
        /// Shown on the card: this stops every session on every repository of that
        /// account, so the user must see why it is being suggested.
        reason: String,
        summary: String,
        target: ActionTarget,
    },
}

impl ActionProposal {
    /// The one-line summary, whatever the variant. Used by the tool layer to hand the
    /// model the same sentence the card shows, so it can refer to the draft in prose
    /// without restating the payload.
    pub fn summary(&self) -> &str {
        match self {
            Self::CreateSession { summary, .. }
            | Self::CreateWorkItem { summary, .. }
            | Self::StopSession { summary, .. }
            | Self::CreateRepository { summary, .. }
            | Self::SaveEnvironmentProfile { summary, .. }
            | Self::DeleteEnvironmentProfile { summary, .. }
            | Self::UninstallApp { summary, .. } => summary,
        }
    }
}

// ---- shared validation helpers -------------------------------------------
//
// Child modules reach these through `super::`; they are deliberately not `pub`,
// because a caller outside the proposal layer validating a draft would mean two
// implementations that can disagree.

/// Trim a value, rejecting a blank one.
pub(crate) fn required(value: &str, field: &str) -> Result<String, ProposalError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ProposalError::new(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

/// Trim an optional value; blank counts as absent.
pub(crate) fn optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Drop blank entries from a list.
pub(crate) fn clean_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Reject a positive-integer field that is not positive.
pub(crate) fn positive_issue_number(value: i64) -> Result<i64, ProposalError> {
    if value <= 0 {
        return Err(ProposalError::new(
            "the trigger issue number must be positive",
        ));
    }
    Ok(value)
}
