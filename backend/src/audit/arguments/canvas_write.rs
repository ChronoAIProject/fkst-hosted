//! Safe arguments for the two canvas mutations that carry substantial input:
//! opening a session and queuing a work item.
//!
//! ## Why references are re-parsed here
//!
//! `package_refs` / `manifest_refs` are the one place a record keeps a
//! caller-supplied STRING rather than a count. That is only safe because each
//! entry is put back through the product's own strict grammar
//! ([`parse_package_ref`]) and re-rendered from the PARSED parts — so what lands
//! in the record is a value the parser accepted, in the canonical
//! `owner/repo@ref:path` form, and never the caller's raw line. An entry the
//! parser rejects is dropped; the list's true `count` still reports it.
//!
//! Re-parsing is deliberate duplication of work the create handler already did.
//! It keeps the projection independent: the audit path can never mutate, reorder,
//! or short-circuit the business request in order to describe it.
//!
//! ## What is never recorded
//!
//! The session name (free text), the rendered trigger-issue body, the work
//! item's title and body, and every disposable-environment key, value, and
//! install command. Those are reduced to presence flags, counts, and byte sizes.

use serde::Serialize;

use super::bounds::{
    bounded_list, byte_len, safe_branch, safe_environment_name, safe_output_lang, safe_owner,
    safe_repo, safe_work_label, MAX_REF_ENTRIES, MAX_REF_LEN,
};
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};
use crate::audit::event::ArgumentsParseStatus;
use crate::goals::trigger_parse::parse_package_ref;
use crate::routes::canvas::trigger_body::CreateSessionRequest;

/// Re-render one reference through the strict parser, or drop it.
///
/// Returns the canonical form only when the grammar accepts the entry AND the
/// canonical form fits the documented per-entry cap.
fn accepted_reference(value: &str) -> Option<String> {
    let parsed = parse_package_ref(value.trim()).ok()?;
    let rendered = format!(
        "{}/{}@{}:{}",
        parsed.owner, parsed.repo, parsed.git_ref, parsed.path
    );
    (rendered.len() <= MAX_REF_LEN).then_some(rendered)
}

/// `canvas_create_session` — opening a session's trigger issue.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasCreateSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    package_refs: Vec<String>,
    package_count: usize,
    /// Emitted when `package_refs` is not the whole request: the list hit its
    /// cap, or the strict parser refused an entry. Either way `package_count`
    /// still reports what the caller sent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    package_refs_truncated: bool,
    manifest_refs: Vec<String>,
    manifest_count: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    manifest_refs_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    work_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment_name: Option<String>,
    disposable_environment_present: bool,
    disposable_variable_count: usize,
    disposable_secret_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_branch: Option<String>,
    auto_merge: bool,
    log_access_count: usize,
    collaborator_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_language: Option<String>,
}

impl BoundedAuditArguments for SafeCanvasCreateSession {
    const OPERATION_ID: &'static str = "canvas_create_session";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_CREATE_SESSION_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.owner.is_some() && self.repo.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// The input view for `canvas_create_session`: the validated path segments plus
/// the request whose rendered body already round-tripped through the trigger
/// parser.
pub struct CreateSessionInput<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub request: &'a CreateSessionRequest,
}

impl Sealed for CreateSessionInput<'_> {}

impl ToSafeAuditArguments for CreateSessionInput<'_> {
    type Safe = SafeCanvasCreateSession;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        let request = self.request;
        let packages = bounded_list(
            request.packages.iter().map(String::as_str),
            MAX_REF_ENTRIES,
            accepted_reference,
        );
        let manifests = bounded_list(
            request.manifests.iter().map(String::as_str),
            MAX_REF_ENTRIES,
            accepted_reference,
        );
        let disposable = request.disposable_environment.as_ref();
        SafeCanvasCreateSession {
            owner: safe_owner(self.owner),
            repo: safe_repo(self.repo),
            package_refs: packages.items,
            package_count: packages.count,
            package_refs_truncated: packages.truncated,
            manifest_refs: manifests.items,
            manifest_count: manifests.count,
            manifest_refs_truncated: manifests.truncated,
            work_label: request.work_label.as_deref().and_then(safe_work_label),
            environment_name: request
                .environment
                .as_deref()
                .and_then(safe_environment_name),
            disposable_environment_present: disposable.is_some(),
            disposable_variable_count: disposable.map(|env| env.variables.len()).unwrap_or(0),
            disposable_secret_count: disposable.map(|env| env.secrets.len()).unwrap_or(0),
            source_branch: request.source_branch.as_deref().and_then(safe_branch),
            target_branch: request.target_branch.as_deref().and_then(safe_branch),
            auto_merge: request.auto_merge == Some(true),
            log_access_count: request
                .log_access
                .iter()
                .filter(|entry| !entry.trim().is_empty())
                .count(),
            collaborator_count: request
                .collaborators
                .iter()
                .filter(|entry| !entry.trim().is_empty())
                .count(),
            output_language: request.output_lang.as_deref().and_then(safe_output_lang),
        }
    }
}

/// `canvas_create_work_item` — queuing a work issue against a live session.
#[derive(Clone, Debug, Serialize)]
pub struct SafeCanvasCreateWorkItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    trigger_issue: i64,
    /// The label the handler RESOLVED against the session's applicable set —
    /// never the caller's requested string, which may name nothing at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_label: Option<String>,
    title_bytes: u64,
    body_present: bool,
    body_bytes: u64,
}

impl BoundedAuditArguments for SafeCanvasCreateWorkItem {
    const OPERATION_ID: &'static str = "canvas_create_work_item";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::CANVAS_CREATE_WORK_ITEM_FIELDS;

    fn parse_status(&self) -> ArgumentsParseStatus {
        if self.owner.is_some() && self.repo.is_some() {
            ArgumentsParseStatus::Parsed
        } else {
            ArgumentsParseStatus::Invalid
        }
    }
}

/// The input view for `canvas_create_work_item`.
pub struct CreateWorkItemInput<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub trigger_issue: i64,
    /// The effective work label the handler resolved, immediately before the
    /// GitHub write.
    pub selected_label: &'a str,
    /// The trimmed issue title. Only its byte length is recorded.
    pub title: &'a str,
    /// The issue body, empty when the request opened a body-less issue.
    pub body: &'a str,
}

impl Sealed for CreateWorkItemInput<'_> {}

impl ToSafeAuditArguments for CreateWorkItemInput<'_> {
    type Safe = SafeCanvasCreateWorkItem;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafeCanvasCreateWorkItem {
            owner: safe_owner(self.owner),
            repo: safe_repo(self.repo),
            trigger_issue: self.trigger_issue,
            selected_label: safe_work_label(self.selected_label),
            title_bytes: byte_len(self.title),
            body_present: !self.body.is_empty(),
            body_bytes: byte_len(self.body),
        }
    }
}

#[cfg(test)]
#[path = "canvas_write_tests.rs"]
mod tests;
