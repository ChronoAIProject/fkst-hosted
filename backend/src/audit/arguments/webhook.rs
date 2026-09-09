//! Safe arguments for the signature-verified GitHub App webhook.
//!
//! ## Two shapes, one operation
//!
//! A delivery whose HMAC did NOT verify contributes exactly one property:
//! `signature_valid: false`. Everything a webhook payload claims — sender,
//! installation, repository, issue — is attacker-controlled until the signature
//! over the exact bytes verifies, so recording any of it on a rejected delivery
//! would let anyone who can reach the endpoint write arbitrary correlation into
//! the audit trail.
//!
//! A verified delivery contributes the closed-enum event and action, the
//! delivery's correlation handles, and how this deployment handled it. The
//! payload's issue TITLE and BODY, the repository list of an
//! `installation_repositories` event, the raw headers, and the signature itself
//! are all absent.
//!
//! ## Why the enums are closed
//!
//! `X-GitHub-Event` and `payload.action` are strings GitHub controls, and GitHub
//! adds new values regularly. Copying them through would make an unbounded set
//! of caller-influenced strings a first-class property (and, downstream, a
//! dashboard facet). Unknown-but-valid values collapse to `other`, which keeps
//! the property bounded while still saying honestly that something arrived.

use serde::Serialize;

use super::bounds::safe_repo_full_name;
use super::catalog;
use super::{sealed::Sealed, BoundedAuditArguments, ToSafeAuditArguments};

/// The delivery event types this deployment recognizes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    Installation,
    InstallationRepositories,
    Issues,
    IssueComment,
    Ping,
    /// A validly signed delivery of an event this deployment does not act on.
    Other,
}

impl WebhookEventType {
    /// Narrow the `X-GitHub-Event` header onto the closed set.
    pub fn from_header(event: &str) -> Self {
        match event {
            "installation" => Self::Installation,
            "installation_repositories" => Self::InstallationRepositories,
            "issues" => Self::Issues,
            "issue_comment" => Self::IssueComment,
            "ping" => Self::Ping,
            _ => Self::Other,
        }
    }
}

/// The payload actions this deployment recognizes across those events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookAction {
    Created,
    Deleted,
    Added,
    Removed,
    Suspend,
    Unsuspend,
    NewPermissionsAccepted,
    Opened,
    Reopened,
    Closed,
    Edited,
    Labeled,
    Unlabeled,
    Assigned,
    Unassigned,
    /// A validly signed action outside the recognized set, or none at all.
    Other,
}

impl WebhookAction {
    /// Narrow the payload's `action` onto the closed set.
    pub fn from_payload(action: &str) -> Self {
        match action {
            "created" => Self::Created,
            "deleted" => Self::Deleted,
            "added" => Self::Added,
            "removed" => Self::Removed,
            "suspend" => Self::Suspend,
            "unsuspend" => Self::Unsuspend,
            "new_permissions_accepted" => Self::NewPermissionsAccepted,
            "opened" => Self::Opened,
            "reopened" => Self::Reopened,
            "closed" => Self::Closed,
            "edited" => Self::Edited,
            "labeled" => Self::Labeled,
            "unlabeled" => Self::Unlabeled,
            "assigned" => Self::Assigned,
            "unassigned" => Self::Unassigned,
            _ => Self::Other,
        }
    }
}

/// What this deployment did with a verified delivery.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookHandling {
    /// Enqueued onto the reconcile queue as a level-based nudge.
    Reconciled,
    /// Installation caches were evicted.
    CacheBusted,
    /// Acknowledged, no action required.
    Ignored,
    /// The verified body could not be parsed into the event's shape (answered
    /// `202` so GitHub does not hammer redeliveries).
    ParseFailed,
}

/// `github_app_webhook` — one inbound delivery.
#[derive(Clone, Debug, Serialize)]
pub struct SafeGithubAppWebhook {
    /// Emitted only on the rejection shape; a verified record's other fields
    /// already imply the signature verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    signature_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_type: Option<WebhookEventType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<WebhookAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    installation_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_full_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trigger_issue: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    handling: Option<WebhookHandling>,
}

impl SafeGithubAppWebhook {
    /// The pre-verification shape: the ONLY thing an unverified delivery may say
    /// about itself.
    pub fn rejected() -> Self {
        Self {
            signature_valid: Some(false),
            event_type: None,
            action: None,
            installation_id: None,
            repo_full_name: None,
            trigger_issue: None,
            delivery_id: None,
            handling: None,
        }
    }
}

impl BoundedAuditArguments for SafeGithubAppWebhook {
    const OPERATION_ID: &'static str = "github_app_webhook";
    const ALLOWED_FIELDS: &'static [&'static str] = catalog::GITHUB_APP_WEBHOOK_FIELDS;
}

/// The input view for a VERIFIED delivery.
///
/// Constructing one is the assertion that the HMAC over the exact raw bytes
/// already passed — every field below is read from that verified body or from a
/// header accepted alongside it.
pub struct VerifiedDeliveryInput<'a> {
    /// The `X-GitHub-Event` header value.
    pub event: &'a str,
    /// The verified payload's `action`, when it carries one.
    pub action: Option<&'a str>,
    pub installation_id: Option<i64>,
    /// The verified payload's repository owner login.
    pub repo_owner: Option<&'a str>,
    /// The verified payload's repository name.
    pub repo_name: Option<&'a str>,
    /// The verified payload's issue number, for an `issues` delivery.
    pub issue_number: Option<i64>,
    /// GitHub's `X-GitHub-Delivery`, already accepted as a safe token.
    pub delivery_id: Option<&'a str>,
    pub handling: WebhookHandling,
}

impl Sealed for VerifiedDeliveryInput<'_> {}

impl ToSafeAuditArguments for VerifiedDeliveryInput<'_> {
    type Safe = SafeGithubAppWebhook;

    fn to_safe_audit_arguments(&self) -> Self::Safe {
        SafeGithubAppWebhook {
            // Absent rather than `true`: a rejected delivery is the only case a
            // reader has to distinguish, and it says so explicitly.
            signature_valid: None,
            event_type: Some(WebhookEventType::from_header(self.event)),
            action: Some(
                self.action
                    .map(WebhookAction::from_payload)
                    .unwrap_or(WebhookAction::Other),
            ),
            installation_id: self.installation_id,
            repo_full_name: self
                .repo_owner
                .zip(self.repo_name)
                .and_then(|(owner, name)| safe_repo_full_name(owner, name)),
            trigger_issue: self.issue_number,
            delivery_id: self.delivery_id.map(str::to_string),
            handling: Some(self.handling),
        }
    }
}

#[cfg(test)]
#[path = "webhook_tests.rs"]
mod tests;
