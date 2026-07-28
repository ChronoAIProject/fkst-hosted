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
//! ## Why a separate draft DTO instead of `CreateSessionRequest`
//!
//! The proposal has to be `Serialize` (it is an SSE event). `CreateSessionRequest`
//! deliberately is not, because it holds a `DisposableEnvironmentRequest` whose
//! documented invariant is that it implements neither `Debug` nor `Serialize` — the
//! server accepts that shape and never echoes it. Rather than weaken that invariant,
//! [`DraftSessionRequest`] mirrors the request MINUS the disposable field. That also
//! enforces the secrets rule structurally: a chat draft has nowhere to put a secret.

use serde::Serialize;
use utoipa::ToSchema;

use crate::goals::trigger_parse::parse_package_ref;
use crate::routes::canvas::trigger_body::{validated_trigger_body, CreateSessionRequest};

/// Maximum work-item title length. Matches what a GitHub issue title usefully holds.
const MAX_WORK_ITEM_TITLE_CHARS: usize = 200;
/// Maximum work-item body size. Generous, but bounded so a runaway draft cannot be
/// streamed to the browser.
const MAX_WORK_ITEM_BODY_BYTES: usize = 20 * 1024;
/// Maximum explicit work-label length, mirroring the trigger parser's own cap.
const MAX_WORK_LABEL_CHARS: usize = 50;
/// Maximum stop-reason length; it is display-only on the card.
const MAX_STOP_REASON_CHARS: usize = 500;

/// Why a draft was rejected. The message is returned to the MODEL as tool-result data,
/// so it must be precise enough for the model to fix the draft and retry in the same
/// turn.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ProposalError(pub String);

impl ProposalError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The subset of a create-session request a chat draft may carry.
///
/// Mirrors `CreateSessionRequest` MINUS `disposable_environment`: secrets never transit
/// the LLM conversation or the SSE event, and with no field for them that is a type-level
/// guarantee rather than a rule someone has to remember.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct DraftSessionRequest {
    /// The session name (`### Session Name`, also the issue title).
    pub name: String,
    /// Package references (`owner/repo@ref:path`).
    pub packages: Vec<String>,
    /// fkst-manifest references, same grammar as `packages`.
    pub manifests: Vec<String>,
    /// The explicit work label (`### Work Label`).
    pub work_label: Option<String>,
    /// A NAMED environment profile (`### Environment`). A reference only — never inline
    /// commands, variables or secrets.
    pub environment: Option<String>,
    /// `### Source Branch`.
    pub source_branch: Option<String>,
    /// `### Target Branch`.
    pub target_branch: Option<String>,
    /// `### Auto-merge`.
    pub auto_merge: Option<bool>,
    /// `### Log Access Allowlist` / `### FKST Contributors` grantees.
    pub log_access: Vec<String>,
    /// `### Session Collaborators`.
    pub collaborators: Vec<String>,
    /// `### Output Language`.
    pub output_lang: Option<String>,
}

impl DraftSessionRequest {
    /// Map the draft onto the real request type the renderer and the endpoint use.
    ///
    /// `disposable_environment` is always `None` — the draft has no such field, which is
    /// the point.
    fn to_create_request(&self) -> CreateSessionRequest {
        CreateSessionRequest {
            name: self.name.clone(),
            packages: self.packages.clone(),
            manifests: self.manifests.clone(),
            work_label: self.work_label.clone(),
            environment: self.environment.clone(),
            disposable_environment: None,
            source_branch: self.source_branch.clone(),
            target_branch: self.target_branch.clone(),
            auto_merge: self.auto_merge,
            log_access: self.log_access.clone(),
            collaborators: self.collaborators.clone(),
            output_lang: self.output_lang.clone(),
        }
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
}

/// Trim a value, rejecting a blank one.
fn required(value: &str, field: &str) -> Result<String, ProposalError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ProposalError::new(format!("{field} must not be empty")));
    }
    Ok(trimmed.to_string())
}

/// Trim an optional value; blank counts as absent.
fn optional(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Drop blank entries from a list.
fn clean_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// A session draft that passed validation, with the body a confirmation will file.
#[derive(Debug, Clone)]
pub struct ValidatedSessionDraft {
    pub owner: String,
    pub repo: String,
    pub request: DraftSessionRequest,
    /// Rendered by the same function the real endpoint uses, so preview equals reality.
    pub rendered_issue_body: String,
}

/// Validate a session draft and render the exact issue body a confirmation will file.
///
/// What is deliberately NOT checked here: repository admin/maintain authority, work-label
/// collisions, and whether the repository or referenced packages exist. Those gates live
/// in the real endpoint and run at CONFIRMATION with the user's own token — duplicating
/// them here would mean two implementations that can disagree, and would let a proposal
/// look pre-approved when it is not. The card says final checks run on confirm.
pub fn validate_create_session(
    owner: &str,
    repo: &str,
    draft: DraftSessionRequest,
) -> Result<ValidatedSessionDraft, ProposalError> {
    let owner = required(owner, "owner")?;
    let repo = required(repo, "name")?;

    let draft = DraftSessionRequest {
        name: required(&draft.name, "the session name")?,
        packages: clean_list(draft.packages),
        manifests: clean_list(draft.manifests),
        work_label: optional(draft.work_label),
        environment: optional(draft.environment),
        source_branch: optional(draft.source_branch),
        target_branch: optional(draft.target_branch),
        auto_merge: draft.auto_merge,
        log_access: clean_list(draft.log_access),
        collaborators: clean_list(draft.collaborators),
        output_lang: optional(draft.output_lang),
    };

    if draft.packages.is_empty() && draft.manifests.is_empty() {
        return Err(ProposalError::new(
            "the draft must list at least one package source: fill `packages` or `manifests`",
        ));
    }
    // Reuse the trigger parser's grammar rather than re-implementing it, so a draft can
    // never be accepted here and rejected by the endpoint.
    for (field, refs) in [
        ("packages", &draft.packages),
        ("manifests", &draft.manifests),
    ] {
        for reference in refs {
            parse_package_ref(reference).map_err(|error| {
                ProposalError::new(format!(
                    "{field} entry {reference:?} is not a valid reference: {error}"
                ))
            })?;
        }
    }
    if let Some(label) = &draft.work_label {
        if label.chars().count() > MAX_WORK_LABEL_CHARS {
            return Err(ProposalError::new(format!(
                "the work label must be at most {MAX_WORK_LABEL_CHARS} characters"
            )));
        }
        if label.contains(',') {
            return Err(ProposalError::new(
                "the work label must be a single label with no comma",
            ));
        }
    }

    // Rendered by the SAME function the real `create_session` handler uses, so the
    // preview the user approves is byte-for-byte what gets filed. It also round-trips
    // the body through the trigger parser, which is where a structural mistake surfaces.
    let rendered_issue_body = validated_trigger_body(&draft.to_create_request())
        .map_err(|error| ProposalError::new(error.to_string()))?;

    Ok(ValidatedSessionDraft {
        owner,
        repo,
        request: draft,
        rendered_issue_body,
    })
}

/// Build a validated create-session proposal.
pub fn propose_create_session(
    owner: &str,
    repo: &str,
    draft: DraftSessionRequest,
) -> Result<ActionProposal, ProposalError> {
    let ValidatedSessionDraft {
        owner,
        repo,
        request,
        rendered_issue_body,
    } = validate_create_session(owner, repo, draft)?;
    let summary = format!(
        "Start session `{}` on {}/{}{}",
        request.name,
        owner,
        repo,
        match &request.work_label {
            Some(label) => format!(" watching `{label}`"),
            None => " with auto-discovered work labels".to_string(),
        }
    );
    Ok(ActionProposal::CreateSession {
        target: ActionTarget {
            method: "POST".to_string(),
            path: format!("/api/v1/repos/{owner}/{repo}/sessions"),
        },
        owner,
        name: repo,
        request,
        rendered_issue_body,
        summary,
    })
}

/// Validate and build a work-item proposal.
///
/// The label is optional: when omitted the endpoint falls back to the trigger's explicit
/// `### Work Label`, which is the right default for the common case.
pub fn propose_work_item(
    owner: &str,
    repo: &str,
    trigger_issue_number: i64,
    title: &str,
    label: Option<String>,
    body: Option<String>,
) -> Result<ActionProposal, ProposalError> {
    let owner = required(owner, "owner")?;
    let repo = required(repo, "name")?;
    if trigger_issue_number <= 0 {
        return Err(ProposalError::new(
            "the trigger issue number must be positive",
        ));
    }
    let title = required(title, "the work-item title")?;
    if title.chars().count() > MAX_WORK_ITEM_TITLE_CHARS {
        return Err(ProposalError::new(format!(
            "the work-item title must be at most {MAX_WORK_ITEM_TITLE_CHARS} characters"
        )));
    }
    let body = body.unwrap_or_default();
    if body.len() > MAX_WORK_ITEM_BODY_BYTES {
        return Err(ProposalError::new(format!(
            "the work-item body must be at most {MAX_WORK_ITEM_BODY_BYTES} bytes"
        )));
    }
    let label = optional(label);
    if let Some(label) = &label {
        if label.chars().count() > MAX_WORK_LABEL_CHARS {
            return Err(ProposalError::new(format!(
                "the work label must be at most {MAX_WORK_LABEL_CHARS} characters"
            )));
        }
    }

    let summary = format!("Queue work item “{title}” on {owner}/{repo} #{trigger_issue_number}");
    Ok(ActionProposal::CreateWorkItem {
        target: ActionTarget {
            method: "POST".to_string(),
            path: format!(
                "/api/v1/repos/{owner}/{repo}/sessions/{trigger_issue_number}/work-items"
            ),
        },
        owner,
        name: repo,
        trigger_issue_number,
        title,
        label,
        body,
        summary,
    })
}

/// Validate and build a stop-session proposal.
pub fn propose_stop_session(
    owner: &str,
    repo: &str,
    trigger_issue_number: i64,
    reason: &str,
) -> Result<ActionProposal, ProposalError> {
    let owner = required(owner, "owner")?;
    let repo = required(repo, "name")?;
    if trigger_issue_number <= 0 {
        return Err(ProposalError::new(
            "the trigger issue number must be positive",
        ));
    }
    // Required because stopping is irreversible: the user deserves to see why the
    // assistant is suggesting it before confirming.
    let reason = required(reason, "the stop reason")?;
    if reason.chars().count() > MAX_STOP_REASON_CHARS {
        return Err(ProposalError::new(format!(
            "the stop reason must be at most {MAX_STOP_REASON_CHARS} characters"
        )));
    }

    let summary =
        format!("Stop the session on {owner}/{repo} by closing trigger #{trigger_issue_number}");
    Ok(ActionProposal::StopSession {
        target: ActionTarget {
            method: "DELETE".to_string(),
            path: format!("/api/v1/repos/{owner}/{repo}/sessions/{trigger_issue_number}"),
        },
        owner,
        name: repo,
        trigger_issue_number,
        reason,
        summary,
    })
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
