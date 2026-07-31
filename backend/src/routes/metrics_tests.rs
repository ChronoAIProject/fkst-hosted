//! Exposition-format tests for `GET /metrics`.
//!
//! Every assertion is about one of two properties: the series and their HELP
//! text are present and correctly named, and no unbounded value (a session id,
//! a login, a repository) ever becomes a label or a value.

use super::*;
use crate::audit::lifecycle::LifecycleAction;
use crate::audit::relay::RelayClientMetricsSnapshot;
use crate::operations::{ActivityMetricsSnapshot, SandboxMetricsSnapshot};
use crate::runtime_identity::metrics::{IdentityOperationResult, LifecycleEmitResult};
use crate::runtime_identity::RuntimeBackendKind;
use crate::session_access::{ScopeMetrics, SessionAccessRegistry};

#[test]
fn renders_liveness_and_bounded_recovery_metrics() {
    let monitor = crate::recovery::RecoveryMonitor::new(true);
    monitor.record_attempt(
        crate::recovery::ResyncResult::Partial,
        std::time::Duration::from_millis(250),
        3,
    );
    let body = render_metrics(MetricsSources {
        recovery: &monitor.snapshot(),
        audit: &AuditMetricsSnapshot::default(),
        registry: &SessionAccessRegistry::new(false).snapshot(),
        scope: &ScopeMetrics::new().snapshot(),
        runtime: &RuntimeTelemetrySnapshot::default(),
        activity: &ActivityMetricsSnapshot::default(),
        sandbox: &SandboxMetricsSnapshot::default(),
        relay: &RelayClientMetricsSnapshot::default(),
    });
    assert!(body.contains("# TYPE fkst_up gauge"));
    assert!(body.contains("\nfkst_up 1\n") || body.starts_with("fkst_up 1\n"));
    assert!(body.contains("fkst_startup_resync_attempts_total{result=\"partial\"} 1"));
    assert!(body.contains("fkst_startup_resync_attempts_total{result=\"success\"} 0"));
    assert!(body.contains("fkst_startup_resync_last_duration_seconds 0.25"));
    assert!(body.contains("fkst_startup_resync_complete 0"));
    assert!(body.contains("fkst_recovery_ready 0"));
    assert!(body.contains("fkst_startup_resync_last_repositories_enqueued 3"));
    assert!(body.contains("fkst_leader_state{state=\"disabled\"} 1"));
    assert!(body.contains("fkst_leader_election_enabled 0"));
}

#[test]
fn renders_leader_identity_transitions_and_failure_metrics() {
    let monitor = crate::recovery::RecoveryMonitor::new(true);
    monitor.enable_leader_election("pod-\\\"a".to_string());
    monitor.record_leader_acquired(7);
    monitor.record_leader_api_failure(true);
    monitor.record_attempt(
        crate::recovery::ResyncResult::Success,
        std::time::Duration::from_millis(10),
        2,
    );
    monitor.record_leader_routing(true);
    let body = render_metrics(MetricsSources {
        recovery: &monitor.snapshot(),
        audit: &AuditMetricsSnapshot::default(),
        registry: &SessionAccessRegistry::new(false).snapshot(),
        scope: &ScopeMetrics::new().snapshot(),
        runtime: &RuntimeTelemetrySnapshot::default(),
        activity: &ActivityMetricsSnapshot::default(),
        sandbox: &SandboxMetricsSnapshot::default(),
        relay: &RelayClientMetricsSnapshot::default(),
    });
    assert!(body.contains("fkst_leader 1"));
    assert!(body.contains("fkst_leader_ready 1"));
    assert!(body.contains("fkst_leader_state{state=\"ready\"} 1"));
    assert!(body.contains("fkst_leader_transitions_total{transition=\"acquired\"} 1"));
    assert!(body.contains("fkst_leader_lease_failures_total{operation=\"renew\"} 1"));
    assert!(body.contains("fkst_leader_observed_lease_transitions 7"));
    assert!(body.contains("identity=\"pod-\\\\\\\"a\""));
}

#[test]
fn renders_every_audit_series_with_closed_enum_labels() {
    use crate::audit::metrics::{AuditMetrics, DeliveryResult, DropReason, EnqueueResult};

    let metrics = AuditMetrics::new();
    metrics.set_queue_depth(4);
    metrics.record_enqueued(EnqueueResult::Accepted);
    metrics.record_enqueued(EnqueueResult::Full);
    metrics.record_enqueued(EnqueueResult::Disabled);
    metrics.record_batch(DeliveryResult::Accepted);
    metrics.record_delivery_attempt(
        DeliveryResult::Retryable,
        std::time::Duration::from_millis(250),
    );
    metrics.record_dropped(DropReason::QueueFull, 2);
    metrics.record_dropped(DropReason::Oversized, 1);
    metrics.set_shutdown_remaining(3);
    metrics.record_context_conflicts(5);

    let body = render_metrics(MetricsSources {
        recovery: &crate::recovery::RecoveryMonitor::new(false).snapshot(),
        audit: &metrics.snapshot(),
        registry: &SessionAccessRegistry::new(false).snapshot(),
        scope: &ScopeMetrics::new().snapshot(),
        runtime: &RuntimeTelemetrySnapshot::default(),
        activity: &ActivityMetricsSnapshot::default(),
        sandbox: &SandboxMetricsSnapshot::default(),
        relay: &RelayClientMetricsSnapshot::default(),
    });
    assert!(body.contains("fkst_audit_queue_depth 4"), "{body}");
    assert!(body.contains("fkst_audit_events_enqueued_total{result=\"accepted\"} 1"));
    assert!(body.contains("fkst_audit_events_enqueued_total{result=\"full\"} 1"));
    assert!(body.contains("fkst_audit_events_enqueued_total{result=\"disabled\"} 1"));
    assert!(body.contains("fkst_audit_batches_total{result=\"accepted\"} 1"));
    assert!(body.contains("fkst_audit_batches_total{result=\"permanent\"} 0"));
    assert!(body.contains("fkst_audit_delivery_attempts_total{result=\"retryable\"} 1"));
    assert!(body.contains("fkst_audit_delivery_duration_seconds_sum 0.25"));
    assert!(body.contains("fkst_audit_delivery_duration_seconds_count 1"));
    assert!(body.contains("fkst_audit_events_dropped_total{reason=\"queue_full\"} 2"));
    assert!(body.contains("fkst_audit_events_dropped_total{reason=\"oversized\"} 1"));
    assert!(body.contains("fkst_audit_shutdown_remaining_events 3"));
    // Unlabelled on purpose: the offending FIELD is named in the structured
    // log, never turned into an unbounded Prometheus label.
    assert!(body.contains("fkst_audit_context_conflicts_total 5"));
    // Capture success is named `accepted`, never `delivered`/`persisted`:
    // a PostHog 200 is acceptance, not proof of query visibility. Only the
    // series lines are checked — the HELP prose may still say "undelivered"
    // when that is the accurate description.
    for line in body.lines().filter(|line| !line.starts_with('#')) {
        assert!(!line.contains("delivered"), "{line}");
        assert!(!line.contains("persisted"), "{line}");
    }
}

#[test]
fn renders_the_runtime_attribution_and_lifecycle_series() {
    use crate::runtime_identity::metrics::RuntimeTelemetry;

    let telemetry = RuntimeTelemetry::new();
    telemetry.record_identity(
        RuntimeBackendKind::Kubernetes,
        IdentityOperationResult::Backfilled,
    );
    telemetry.record_identity(
        RuntimeBackendKind::OpenSandbox,
        IdentityOperationResult::Conflict,
    );
    telemetry.record_lifecycle(
        RuntimeBackendKind::Kubernetes,
        LifecycleAction::Created,
        LifecycleEmitResult::Emitted,
    );
    telemetry.record_lifecycle(
        RuntimeBackendKind::OpenSandbox,
        LifecycleAction::DeleteFailed,
        LifecycleEmitResult::Dropped,
    );

    let body = render_metrics(MetricsSources {
        recovery: &crate::recovery::RecoveryMonitor::new(false).snapshot(),
        audit: &AuditMetricsSnapshot::default(),
        registry: &SessionAccessRegistry::new(false).snapshot(),
        scope: &ScopeMetrics::new().snapshot(),
        runtime: &telemetry.snapshot(),
        activity: &ActivityMetricsSnapshot::default(),
        sandbox: &SandboxMetricsSnapshot::default(),
        relay: &RelayClientMetricsSnapshot::default(),
    });
    assert!(body.contains(
        "fkst_runtime_identity_operations_total{backend=\"kubernetes\",result=\"backfilled\"} 1"
    ), "{body}");
    assert!(body.contains(
        "fkst_runtime_identity_operations_total{backend=\"opensandbox\",result=\"conflict\"} 1"
    ));
    assert!(body.contains(
        "fkst_sandbox_lifecycle_events_total{backend=\"kubernetes\",action=\"created\",result=\"emitted\"} 1"
    ));
    assert!(body.contains(
        "fkst_sandbox_lifecycle_events_total{backend=\"opensandbox\",action=\"delete_failed\",result=\"dropped\"} 1"
    ));
    // Every label tuple is rendered, so a zero is a series that exists and
    // is quiet rather than a series that silently disappeared.
    assert!(body.contains(
        "fkst_sandbox_lifecycle_events_total{backend=\"kubernetes\",action=\"identity_conflict\",result=\"emitted\"} 0"
    ));
    // The label set is closed: no id, login, session, or repository value
    // may appear anywhere in the exposition.
    for line in body.lines().filter(|line| {
        line.contains("fkst_runtime_identity") || line.contains("fkst_sandbox_lifecycle")
    }) {
        assert!(
            !line.contains("session") && !line.contains("sess-"),
            "{line}"
        );
    }
}

#[test]
fn renders_the_session_access_projection_and_scope_series() {
    use crate::models::RepoRef;
    use crate::session_access::{ScopeOutcome, SessionAccessContext};

    let registry = SessionAccessRegistry::new(true);
    registry.begin_generation(
        [(
            1,
            RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
        )]
        .into_iter()
        .collect(),
    );
    let scope = ScopeMetrics::new();
    scope.record(ScopeOutcome::MineDefault);
    scope.record(ScopeOutcome::AllForbidden);

    // Mid-generation: recovering, nothing published.
    let body = render_metrics(MetricsSources {
        recovery: &crate::recovery::RecoveryMonitor::new(false).snapshot(),
        audit: &AuditMetricsSnapshot::default(),
        registry: &registry.snapshot(),
        scope: &scope.snapshot(),
        runtime: &RuntimeTelemetrySnapshot::default(),
        activity: &ActivityMetricsSnapshot::default(),
        sandbox: &SandboxMetricsSnapshot::default(),
        relay: &RelayClientMetricsSnapshot::default(),
    });
    assert!(
        body.contains("fkst_session_access_registry_sessions 0"),
        "{body}"
    );
    assert!(body.contains("fkst_session_access_registry_pending_repositories 1"));
    assert!(body.contains("fkst_session_access_registry_generation 1"));
    assert!(body.contains("fkst_session_access_registry_generation_state{state=\"recovering\"} 1"));
    assert!(body.contains("fkst_session_access_registry_generation_state{state=\"ready\"} 0"));
    assert!(body.contains(
        "fkst_operations_scope_decisions_total{scope=\"mine\",result=\"allowed\",reason=\"resolved_default\"} 1"
    ));
    assert!(body.contains(
        "fkst_operations_scope_decisions_total{scope=\"all\",result=\"forbidden\",reason=\"global_scope_forbidden\"} 1"
    ));

    // After publication: ready, one session, and still no unbounded label.
    registry.replace_repo(
        1,
        &RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        vec![(
            "sess-secret".to_string(),
            SessionAccessContext {
                installation_id: 1,
                repo: RepoRef {
                    owner: "acme".to_string(),
                    name: "site".to_string(),
                },
                trigger_issue: 7,
                creator: crate::reconcile::creator::SessionCreator {
                    login: "alice".to_string(),
                    id: Some(42),
                },
                collaborators: vec!["bob".to_string()],
                log_access: vec!["carol".to_string()],
            },
        )],
    );
    let body = render_metrics(MetricsSources {
        recovery: &crate::recovery::RecoveryMonitor::new(false).snapshot(),
        audit: &AuditMetricsSnapshot::default(),
        registry: &registry.snapshot(),
        scope: &scope.snapshot(),
        runtime: &RuntimeTelemetrySnapshot::default(),
        activity: &ActivityMetricsSnapshot::default(),
        sandbox: &SandboxMetricsSnapshot::default(),
        relay: &RelayClientMetricsSnapshot::default(),
    });
    assert!(
        body.contains("fkst_session_access_registry_sessions 1"),
        "{body}"
    );
    assert!(body.contains("fkst_session_access_registry_generation_state{state=\"ready\"} 1"));
    for leak in ["sess-secret", "alice", "bob", "carol", "acme"] {
        assert!(!body.contains(leak), "{leak} leaked into /metrics: {body}");
    }
}
