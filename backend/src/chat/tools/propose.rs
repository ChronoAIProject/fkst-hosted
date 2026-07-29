//! The session-lifecycle proposal tools, plus the shaping and argument helpers the
//! sibling [`super::propose_resources`] tools reuse.
//!
//! Proposal tools are the model's only way to suggest a mutation.
//!
//! Each one validates a draft and hands the typed proposal to the orchestrator
//! **out of band** via [`ToolOutcome::proposal`], while returning the model only a lean
//! acknowledgement. That split matters twice over: the model's context does not fill up
//! with a payload it already produced, and the payload the user reviews is the one the
//! server validated rather than whatever the model would restate.
//!
//! A validation failure is returned to the model as tool-result DATA, so it can fix the
//! draft and retry within the same turn instead of losing the turn.

use std::sync::Arc;

use async_trait::async_trait;

use super::super::actions::{self, DraftSessionRequest};
use super::super::llm::ToolDef;
use super::{
    optional_str, required_i64, required_str, ChatTool, ToolCtx, ToolError, ToolOutcome,
    ToolRegistry,
};

/// Shape a successful proposal for the model: an acknowledgement plus the same one-line
/// summary the card shows, so the model can refer to it in prose without restating the
/// whole draft.
pub(super) fn presented(proposal: actions::ActionProposal) -> ToolOutcome {
    let summary = proposal.summary().to_string();
    ToolOutcome {
        result_json: serde_json::json!({
            "proposal_presented": true,
            "summary": summary,
            "note": "The user must review and confirm this card. Do not claim it has happened.",
        }),
        truncated: false,
        status: None,
        proposal: Some(proposal),
    }
}

/// Shape a rejected draft as data the model can act on.
pub(super) fn rejected(error: actions::ProposalError) -> ToolOutcome {
    ToolOutcome {
        result_json: serde_json::json!({
            "proposal_presented": false,
            "error": error.to_string(),
        }),
        truncated: false,
        status: None,
        proposal: None,
    }
}

/// Read an optional string array argument, dropping blank entries.
pub(super) fn optional_list(args: &serde_json::Value, key: &str) -> Result<Vec<String>, ToolError> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(|s| s.trim().to_string()).ok_or_else(|| {
                    ToolError::InvalidArgs(format!("{key} must be an array of strings"))
                })
            })
            .filter(|entry| !matches!(entry, Ok(value) if value.is_empty()))
            .collect(),
        // A model that emits a single string where an array is expected is being helpful,
        // not wrong; accept it rather than burning a retry.
        Some(serde_json::Value::String(single)) => Ok(vec![single.trim().to_string()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect()),
        Some(_) => Err(ToolError::InvalidArgs(format!(
            "{key} must be an array of strings"
        ))),
    }
}

/// Read an optional boolean argument.
pub(super) fn optional_bool(
    args: &serde_json::Value,
    key: &str,
) -> Result<Option<bool>, ToolError> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Bool(value)) => Ok(Some(*value)),
        Some(serde_json::Value::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "enabled" | "1" => Ok(Some(true)),
            "false" | "no" | "off" | "disabled" | "0" => Ok(Some(false)),
            other => Err(ToolError::InvalidArgs(format!(
                "{key} must be a boolean (got {other:?})"
            ))),
        },
        Some(_) => Err(ToolError::InvalidArgs(format!("{key} must be a boolean"))),
    }
}

pub(super) fn string_array(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": { "type": "string" },
        "description": description,
    })
}

// ---- draft_trigger_session ------------------------------------------------

struct DraftTriggerSession;

#[async_trait]
impl ChatTool for DraftTriggerSession {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "draft_trigger_session".to_string(),
            description:
                "Draft a new substrate session for the user to review and confirm. This does \
                 NOT create anything — it presents a card showing the exact trigger issue that \
                 would be filed, which the user must confirm. Never claim the session exists \
                 after calling this. Requires at least one of `packages` or `manifests`. \
                 `environment` names a SAVED profile only: never put commands, variable values \
                 or secrets in a draft."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "owner": { "type": "string", "description": "Repository owner." },
                    "name": { "type": "string", "description": "Repository name." },
                    "session_name": {
                        "type": "string",
                        "description": "Session name: lowercase letters, digits and inner dashes, 1-40 chars.",
                    },
                    "packages": string_array("Package references in `owner/repo@ref:path` form."),
                    "manifests": string_array("fkst-manifest references, same grammar as packages."),
                    "work_label": { "type": "string", "description": "One work label, max 50 chars, no comma. Omit to auto-discover from packages." },
                    "environment": { "type": "string", "description": "Name of a saved environment profile the user owns." },
                    "source_branch": { "type": "string", "description": "Upstream branch; omit for the repository default." },
                    "target_branch": { "type": "string", "description": "Integration branch; omit for fkst-hosted-default." },
                    "auto_merge": { "type": "boolean", "description": "Auto-merge the bot's PRs. This bypasses review — only draft it when the user asked for it." },
                    "log_access": string_array("Extra GitHub logins granted log access."),
                    "collaborators": string_array("GitHub logins granted work-item authority."),
                    "output_lang": { "type": "string", "description": "Output locale tag, e.g. `en` or `zh-CN`." },
                },
                "required": ["owner", "name", "session_name"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let owner = required_str(&args, "owner")?;
        let repo = required_str(&args, "name")?;
        let draft = DraftSessionRequest {
            name: required_str(&args, "session_name")?,
            packages: optional_list(&args, "packages")?,
            manifests: optional_list(&args, "manifests")?,
            work_label: optional_str(&args, "work_label")?,
            environment: optional_str(&args, "environment")?,
            source_branch: optional_str(&args, "source_branch")?,
            target_branch: optional_str(&args, "target_branch")?,
            auto_merge: optional_bool(&args, "auto_merge")?,
            log_access: optional_list(&args, "log_access")?,
            collaborators: optional_list(&args, "collaborators")?,
            output_lang: optional_str(&args, "output_lang")?,
        };
        Ok(
            match actions::propose_create_session(&owner, &repo, draft) {
                Ok(proposal) => presented(proposal),
                Err(error) => rejected(error),
            },
        )
    }
}

// ---- draft_work_item -----------------------------------------------------

struct DraftWorkItem;

#[async_trait]
impl ChatTool for DraftWorkItem {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "draft_work_item".to_string(),
            description:
                "Draft one work item (a task) for an existing session, for the user to review \
                 and confirm. This does NOT create the issue — it presents a card the user must \
                 confirm. Identify the session by its repository plus its TRIGGER issue number. \
                 Write a specific title and a body naming the exact files and acceptance \
                 criteria: the agent sees that one issue plus the repository, not the sibling \
                 backlog."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "owner": { "type": "string", "description": "Repository owner." },
                    "name": { "type": "string", "description": "Repository name." },
                    "trigger_issue_number": { "type": "integer", "description": "The session's trigger issue number." },
                    "title": { "type": "string", "description": "The work-item title, max 200 chars." },
                    "label": { "type": "string", "description": "A work label; omit to use the session's explicit `### Work Label`." },
                    "body": { "type": "string", "description": "Markdown details: exact files, acceptance criteria, context." },
                },
                "required": ["owner", "name", "trigger_issue_number", "title"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let owner = required_str(&args, "owner")?;
        let repo = required_str(&args, "name")?;
        let trigger = required_i64(&args, "trigger_issue_number")?;
        let title = required_str(&args, "title")?;
        let label = optional_str(&args, "label")?;
        let body = optional_str(&args, "body")?;
        Ok(
            match actions::propose_work_item(&owner, &repo, trigger, &title, label, body) {
                Ok(proposal) => presented(proposal),
                Err(error) => rejected(error),
            },
        )
    }
}

// ---- propose_stop_session ------------------------------------------------

struct ProposeStopSession;

#[async_trait]
impl ChatTool for ProposeStopSession {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "propose_stop_session".to_string(),
            description:
                "Propose stopping a session, for the user to review and confirm. This does NOT \
                 stop anything. Stopping is PERMANENT: closing the trigger retires the session \
                 and it never revives. Only draft this when the user clearly asked to stop a \
                 session, and always state the reason so they can see why."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "owner": { "type": "string", "description": "Repository owner." },
                    "name": { "type": "string", "description": "Repository name." },
                    "trigger_issue_number": { "type": "integer", "description": "The session's trigger issue number." },
                    "reason": { "type": "string", "description": "Why stopping is being proposed; shown on the card." },
                },
                "required": ["owner", "name", "trigger_issue_number", "reason"],
                "additionalProperties": false,
            }),
        }
    }

    async fn call(
        &self,
        _ctx: &ToolCtx,
        args: serde_json::Value,
    ) -> Result<ToolOutcome, ToolError> {
        let owner = required_str(&args, "owner")?;
        let repo = required_str(&args, "name")?;
        let trigger = required_i64(&args, "trigger_issue_number")?;
        let reason = required_str(&args, "reason")?;
        Ok(
            match actions::propose_stop_session(&owner, &repo, trigger, &reason) {
                Ok(proposal) => presented(proposal),
                Err(error) => rejected(error),
            },
        )
    }
}

/// Register the three session-lifecycle proposal tools.
pub(super) fn register(registry: &mut ToolRegistry) {
    registry.register(Arc::new(DraftTriggerSession));
    registry.register(Arc::new(DraftWorkItem));
    registry.register(Arc::new(ProposeStopSession));
}

#[cfg(test)]
#[path = "propose_tests.rs"]
mod tests;
