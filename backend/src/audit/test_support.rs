//! Shared fixtures for the audit module's unit tests.
//!
//! One place for the canonical "valid record", so a contract change surfaces as
//! one compile error here rather than as a dozen subtly-diverging local copies.

use k8s_openapi::chrono::{DateTime, TimeZone, Utc};

use super::event::{
    Actor, ActorKind, ApiRequestCompletedV1, AuditOutcome, AuthenticationMethod, Correlation,
    Principal, PrincipalKind, RequestIdentity, RequestResult, RequestTiming, ServiceIdentity,
};

/// A fixed instant, so every projection snapshot is byte-stable.
pub(crate) fn instant(secs: i64, millis: u32) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, millis * 1_000_000)
        .single()
        .expect("fixture timestamp is representable")
}

pub(crate) fn identity() -> RequestIdentity {
    RequestIdentity {
        request_id: "req-0001".to_string(),
        method: "GET".to_string(),
        route_template: "/api/v1/logs/{session_id}".to_string(),
        operation_id: "logs_download".to_string(),
    }
}

/// 1 700 000 000.000 → 1 700 000 000.250 (a 250 ms request).
pub(crate) fn timing() -> RequestTiming {
    RequestTiming {
        started_at: instant(1_700_000_000, 0),
        completed_at: instant(1_700_000_000, 250),
    }
}

pub(crate) fn service() -> ServiceIdentity {
    ServiceIdentity {
        version: "9.9.9".to_string(),
        environment: "test".to_string(),
    }
}

pub(crate) fn ok() -> RequestResult {
    RequestResult {
        status_code: Some(200),
        outcome: AuditOutcome::Success,
        error_code: None,
    }
}

pub(crate) fn correlation() -> Correlation {
    Correlation {
        session_id: Some("sess-abc".to_string()),
        repo_full_name: Some("acme/site".to_string()),
        installation_id: Some(4242),
        trigger_issue: Some(77),
        webhook_delivery_id: Some("11111111-2222-3333-4444-555555555555".to_string()),
    }
}

/// A verified GitHub human's successful request, fully correlated.
pub(crate) fn human_event() -> ApiRequestCompletedV1 {
    ApiRequestCompletedV1::new(
        identity(),
        timing(),
        Actor::github_user(583_231, "octocat", AuthenticationMethod::Bearer),
        Principal::new(
            PrincipalKind::GithubUserToken,
            Some("github_user_token".to_string()),
        ),
        ok(),
        service(),
    )
    .with_correlation(correlation())
}

/// An unauthenticated caller rejected by the identity gate.
pub(crate) fn anonymous_event() -> ApiRequestCompletedV1 {
    ApiRequestCompletedV1::new(
        identity(),
        timing(),
        Actor::anonymous(),
        Principal::new(PrincipalKind::Anonymous, None),
        RequestResult {
            status_code: Some(401),
            outcome: AuditOutcome::Rejected,
            error_code: Some("unauthorized".to_string()),
        },
        service(),
    )
}

/// The control plane acting on its own behalf.
pub(crate) fn system_event() -> ApiRequestCompletedV1 {
    ApiRequestCompletedV1::new(
        identity(),
        timing(),
        Actor::system(),
        Principal::new(PrincipalKind::Reconciler, Some("reconciler".to_string())),
        ok(),
        service(),
    )
}

/// A credentialed machine caller that is not a GitHub person (deployment
/// tooling, a monitored probe). It carries a LABEL, never an identity: the label
/// must not become the distinct id and must not become an actor id.
pub(crate) fn service_event() -> ApiRequestCompletedV1 {
    ApiRequestCompletedV1::new(
        identity(),
        timing(),
        Actor {
            kind: ActorKind::Service,
            id: None,
            login: Some("fkst-probe".to_string()),
            authentication: AuthenticationMethod::Internal,
        },
        Principal::new(PrincipalKind::None, Some("deployment-probe".to_string())),
        ok(),
        service(),
    )
}

/// A signature-verified webhook. `sender_id` is `None` when GitHub's payload
/// carried no resolvable numeric id.
pub(crate) fn webhook_event(sender_id: Option<i64>) -> ApiRequestCompletedV1 {
    let actor = Actor {
        kind: ActorKind::GithubWebhookSender,
        id: sender_id,
        login: sender_id.map(|_| "octocat".to_string()),
        authentication: AuthenticationMethod::WebhookHmac,
    };
    ApiRequestCompletedV1::new(
        identity(),
        timing(),
        actor,
        Principal::new(PrincipalKind::WebhookHmac, None),
        RequestResult {
            status_code: Some(202),
            outcome: AuditOutcome::Success,
            error_code: None,
        },
        service(),
    )
}

/// Merge `extra` over `base`, replacing any key already present.
///
/// envy rejects a duplicated variable outright, so a test that overrides one
/// knob must overwrite the base entry rather than append a second one.
pub(crate) fn merge_vars(base: &[(&str, &str)], extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = base
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    for (key, value) in extra {
        match pairs.iter_mut().find(|(existing, _)| existing == key) {
            Some(entry) => entry.1 = value.to_string(),
            None => pairs.push((key.to_string(), value.to_string())),
        }
    }
    pairs
}
