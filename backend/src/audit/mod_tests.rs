//! The application-facing handle: admission accounting, the disabled default,
//! and the secret canary that must never escape any serialized surface.

use super::*;
use crate::audit::metrics::EnqueueResult;
use crate::audit::test_support::human_event;

#[tokio::test]
async fn the_default_handle_is_disabled_and_does_nothing() {
    let handle = AuditHandle::default();
    assert!(!handle.is_delivering());
    handle.submit(human_event()).expect("admission succeeds");

    let snapshot = handle.metrics_snapshot();
    assert_eq!(snapshot.enqueued_disabled, 1);
    assert_eq!(snapshot.enqueued_accepted, 0);
    assert_eq!(snapshot.queue_depth, 0);
    assert_eq!(handle.drain().await, DrainReport::default());
}

#[test]
fn a_disabled_configuration_starts_no_worker() {
    // The default (feature off) path must never need a host, a token, or a task.
    let config = AuditConfig::default();
    let handle = AuditHandle::from_config(&config).expect("disabled config builds");
    assert!(!handle.is_delivering());
}

#[test]
fn an_enabled_configuration_without_a_host_is_a_startup_error() {
    // `AuditConfig::from_vars` already refuses this; the constructor is the
    // second gate for a hand-built config, so the failure can never become a
    // silently disabled audit trail.
    let config = AuditConfig {
        enabled: true,
        ..AuditConfig::default()
    };
    let error = AuditHandle::from_config(&config).expect_err("no host must fail closed");
    assert!(error.to_string().contains("FKST_POSTHOG_HOST"), "{error}");
}

#[test]
fn admitted_events_are_counted_and_reach_the_sink() {
    let (handle, recorder) = AuditHandle::recording();
    assert!(handle.is_delivering());
    handle.submit(human_event()).expect("admitted");

    assert_eq!(recorder.len(), 1);
    let snapshot = handle.metrics_snapshot();
    assert_eq!(snapshot.enqueued_accepted, 1);
    assert_eq!(snapshot.enqueued_disabled, 0);
    // The gauge is read straight from the sink, so it is exact at scrape time.
    assert_eq!(snapshot.queue_depth, 1);
}

#[test]
fn a_full_queue_is_counted_as_both_a_refusal_and_a_drop() {
    // "Never silently discarded": overflow shows up in the admission counter AND
    // in the drop counter, so an alert can fire on either.
    let handle = AuditHandle::new(
        std::sync::Arc::new(RecordingSink::new(1)),
        AuditMetrics::new(),
    );
    handle.submit(human_event()).expect("first fits");
    assert_eq!(
        handle.submit(human_event()),
        Err(SubmitError::QueueFull),
        "the caller sees the refusal"
    );

    let snapshot = handle.metrics_snapshot();
    assert_eq!(snapshot.enqueued_accepted, 1);
    assert_eq!(snapshot.enqueued_full, 1);
    assert_eq!(snapshot.dropped_queue_full, 1);
}

#[test]
fn the_handle_is_cheaply_cloneable_and_shares_one_sink() {
    let (handle, recorder) = AuditHandle::recording();
    let clone = handle.clone();
    clone.submit(human_event()).expect("admitted");
    assert_eq!(recorder.len(), 1);
    assert_eq!(handle.metrics_snapshot().enqueued_accepted, 1);
}

#[test]
fn a_secret_canary_never_escapes_through_any_serialized_surface() {
    const CANARY: &str = "phc_canary_a1b2c3d4e5f6";

    let config = AuditConfig::from_vars(&[
        ("FKST_POSTHOG_ENABLED".to_string(), "true".to_string()),
        (
            "FKST_POSTHOG_HOST".to_string(),
            "https://posthog.example".to_string(),
        ),
        ("FKST_POSTHOG_PROJECT_TOKEN".to_string(), CANARY.to_string()),
        (
            "FKST_DEPLOYMENT_ENVIRONMENT".to_string(),
            "production".to_string(),
        ),
    ])
    .expect("config is valid");

    // 1. Config debug output (and therefore anything embedding the config).
    let config_debug = format!("{config:?}");
    assert!(!config_debug.contains(CANARY), "{config_debug}");

    // 2. The transport client's debug output.
    let client = posthog::PostHogClient::from_config(&config).expect("client builds");
    let client_debug = format!("{client:?}");
    assert!(!client_debug.contains(CANARY), "{client_debug}");

    // 3. The serialized event itself. The token is added by the transport, so a
    //    projected event can be logged or relayed without carrying a credential.
    let captured = human_event()
        .to_capture_event(EventLimits::new(65_536))
        .expect("projects");
    let encoded = serde_json::to_string(&captured).expect("serializes");
    assert!(!encoded.contains(CANARY), "{encoded}");
    let event_debug = format!("{captured:?}");
    assert!(!event_debug.contains(CANARY), "{event_debug}");

    // 4. The metrics surface: bounded labels only, no credential and no ids.
    let metrics = AuditMetrics::new();
    metrics.record_enqueued(EnqueueResult::Accepted);
    let metrics_debug = format!("{:?}", metrics.snapshot());
    assert!(!metrics_debug.contains(CANARY), "{metrics_debug}");

    // 5. The whole `Config` this rides on, since a `{:?}` there is the most
    //    likely accidental leak.
    let whole = crate::config::Config {
        audit: config,
        ..crate::config::Config::default()
    };
    let whole_debug = format!("{whole:?}");
    assert!(!whole_debug.contains(CANARY), "{whole_debug}");
}
