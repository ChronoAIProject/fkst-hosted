//! Unit tests for the GitHub App webhook safe arguments.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

const CANARIES: &[&str] = &[
    "canary-issue-title",
    "canary-issue-body",
    "canary-signature",
    "canary-repository-list",
];

fn verified() -> SafeGithubAppWebhook {
    VerifiedDeliveryInput {
        event: "issues",
        action: Some("opened"),
        installation_id: Some(146_704_012),
        repo_owner: Some("acme"),
        repo_name: Some("site"),
        issue_number: Some(42),
        delivery_id: Some("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e"),
        handling: WebhookHandling::Reconciled,
    }
    .to_safe_audit_arguments()
}

#[test]
fn the_webhook_dto_is_wired_to_its_declared_policy() {
    assert_policy_matches::<SafeGithubAppWebhook>();
}

/// The security property: a delivery whose HMAC did not verify contributes ONE
/// property. Its claimed sender, installation, repository, and issue are
/// attacker-controlled, so none of them may be recorded.
#[test]
fn a_rejected_delivery_records_only_that_the_signature_failed() {
    let safe = SafeGithubAppWebhook::rejected();
    assert_within_allowlist(&safe);
    let values = properties(&safe);
    assert_eq!(values.len(), 1);
    assert_eq!(
        values.get("signature_valid").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_no_canary(&safe, CANARIES);
}

#[test]
fn a_verified_delivery_records_its_closed_event_action_and_correlation() {
    let safe = verified();
    assert_within_allowlist(&safe);
    assert_no_canary(&safe, CANARIES);

    let values = properties(&safe);
    assert_eq!(string(&values, "event_type").as_deref(), Some("issues"));
    assert_eq!(string(&values, "action").as_deref(), Some("opened"));
    assert_eq!(
        values
            .get("installation_id")
            .and_then(serde_json::Value::as_i64),
        Some(146_704_012)
    );
    assert_eq!(
        string(&values, "repo_full_name").as_deref(),
        Some("acme/site")
    );
    assert_eq!(
        values
            .get("trigger_issue")
            .and_then(serde_json::Value::as_i64),
        Some(42)
    );
    assert_eq!(string(&values, "handling").as_deref(), Some("reconciled"));
    assert!(
        !values.contains_key("signature_valid"),
        "only a rejection states it explicitly"
    );
}

/// GitHub adds event and action names regularly; an unknown one must collapse
/// rather than turn an unbounded upstream string into a dashboard facet.
#[test]
fn unknown_events_and_actions_collapse_to_other() {
    let values = properties(
        &VerifiedDeliveryInput {
            event: "canary_brand_new_event",
            action: Some("canary_brand_new_action"),
            installation_id: None,
            repo_owner: None,
            repo_name: None,
            issue_number: None,
            delivery_id: None,
            handling: WebhookHandling::Ignored,
        }
        .to_safe_audit_arguments(),
    );
    assert_eq!(string(&values, "event_type").as_deref(), Some("other"));
    assert_eq!(string(&values, "action").as_deref(), Some("other"));
    let rendered = serde_json::to_string(&values).expect("serializes");
    assert!(!rendered.contains("canary_brand_new"), "{rendered}");
}

#[test]
fn every_recognized_event_and_action_maps_to_its_own_value() {
    for (header, expected) in [
        ("installation", WebhookEventType::Installation),
        (
            "installation_repositories",
            WebhookEventType::InstallationRepositories,
        ),
        ("issues", WebhookEventType::Issues),
        ("issue_comment", WebhookEventType::IssueComment),
        ("ping", WebhookEventType::Ping),
        ("", WebhookEventType::Other),
    ] {
        assert_eq!(WebhookEventType::from_header(header), expected);
    }
    for (payload, expected) in [
        ("created", WebhookAction::Created),
        ("deleted", WebhookAction::Deleted),
        ("added", WebhookAction::Added),
        ("removed", WebhookAction::Removed),
        ("suspend", WebhookAction::Suspend),
        ("unsuspend", WebhookAction::Unsuspend),
        (
            "new_permissions_accepted",
            WebhookAction::NewPermissionsAccepted,
        ),
        ("opened", WebhookAction::Opened),
        ("reopened", WebhookAction::Reopened),
        ("closed", WebhookAction::Closed),
        ("edited", WebhookAction::Edited),
        ("labeled", WebhookAction::Labeled),
        ("unlabeled", WebhookAction::Unlabeled),
        ("assigned", WebhookAction::Assigned),
        ("unassigned", WebhookAction::Unassigned),
        ("anything else", WebhookAction::Other),
    ] {
        assert_eq!(WebhookAction::from_payload(payload), expected);
    }
}

#[test]
fn a_delivery_without_an_action_still_records_a_bounded_value() {
    let values = properties(
        &VerifiedDeliveryInput {
            event: "ping",
            action: None,
            installation_id: None,
            repo_owner: None,
            repo_name: None,
            issue_number: None,
            delivery_id: None,
            handling: WebhookHandling::Ignored,
        }
        .to_safe_audit_arguments(),
    );
    assert_eq!(string(&values, "action").as_deref(), Some("other"));
}

/// A repository whose halves do not validate is dropped rather than half
/// rendered: `repo_full_name` is a correlation key on the read side.
#[test]
fn an_unvalidated_repository_is_dropped() {
    let values = properties(
        &VerifiedDeliveryInput {
            event: "issues",
            action: Some("opened"),
            installation_id: None,
            repo_owner: Some("acme"),
            repo_name: Some("canary site"),
            issue_number: None,
            delivery_id: None,
            handling: WebhookHandling::ParseFailed,
        }
        .to_safe_audit_arguments(),
    );
    assert!(!values.contains_key("repo_full_name"));
    assert_eq!(string(&values, "handling").as_deref(), Some("parse_failed"));
}

#[test]
fn every_handling_outcome_renders_its_closed_wire_value() {
    for (handling, expected) in [
        (WebhookHandling::Reconciled, "reconciled"),
        (WebhookHandling::CacheBusted, "cache_busted"),
        (WebhookHandling::Ignored, "ignored"),
        (WebhookHandling::ParseFailed, "parse_failed"),
    ] {
        let values = properties(
            &VerifiedDeliveryInput {
                event: "installation",
                action: Some("created"),
                installation_id: None,
                repo_owner: None,
                repo_name: None,
                issue_number: None,
                delivery_id: None,
                handling,
            }
            .to_safe_audit_arguments(),
        );
        assert_eq!(string(&values, "handling").as_deref(), Some(expected));
    }
}
