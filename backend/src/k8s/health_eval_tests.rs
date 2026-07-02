//! Exhaustive unit tests for the PURE session-health evaluator: the two log
//! formats, message normalization/grouping, the pod-status projection, and the full
//! `evaluate_health` decision matrix (error / recurring-warn / status / clean, plus
//! priority and threshold edges).

use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateWaiting, ContainerStatus, Pod, PodStatus,
};

use super::*;

// ---- helpers ----------------------------------------------------------------

fn running() -> PodStatusSummary {
    PodStatusSummary {
        phase: Some("Running".to_string()),
        restart_count: 0,
        waiting_reason: None,
    }
}

fn warn_stat(sample: &str, count: usize) -> MessageStat {
    MessageStat {
        sample_verbatim: sample.to_string(),
        level: Severity::Warn,
        count,
    }
}

fn error_stat(sample: &str, count: usize) -> MessageStat {
    MessageStat {
        sample_verbatim: sample.to_string(),
        level: Severity::Error,
        count,
    }
}

/// The concrete motivating line: a codex-triage pod that is Running yet warns every
/// cycle that it has no useful work to do.
const CODEX_WARN: &str = "2026-07-01T12:00:00Z LEVEL=warn target=codex MSG=codex-triage/score_dedup: no issue mirror at .fkst/mirror/42 - run scripts/reconcile";

// ---- log parsing: space-kv format -------------------------------------------

#[test]
fn parses_space_kv_warn_line_message_to_end_of_line() {
    let stats = parse_severity_lines(CODEX_WARN);
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].level, Severity::Warn);
    assert_eq!(stats[0].count, 1);
    assert_eq!(
        stats[0].sample_verbatim,
        "codex-triage/score_dedup: no issue mirror at .fkst/mirror/42 - run scripts/reconcile"
    );
}

#[test]
fn parses_space_kv_error_line() {
    let logs = "TS=1 LEVEL=ERROR MSG=engine crashed: boom";
    let stats = parse_severity_lines(logs);
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].level, Severity::Error);
    assert_eq!(stats[0].sample_verbatim, "engine crashed: boom");
}

#[test]
fn ignores_info_and_debug_severity_lines() {
    let logs = "TS=1 LEVEL=info MSG=all good\nTS=2 LEVEL=debug MSG=noisy\nTS=3 LEVEL=trace MSG=x";
    assert!(parse_severity_lines(logs).is_empty());
}

// ---- log parsing: JSON format -----------------------------------------------

#[test]
fn parses_json_warn_line_case_insensitively() {
    let logs = r#"{"level":"WARN","fields":{"message":"low disk"}}"#;
    let stats = parse_severity_lines(logs);
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].level, Severity::Warn);
    assert_eq!(stats[0].sample_verbatim, "low disk");
}

#[test]
fn parses_json_error_line_and_top_level_message_fallback() {
    let nested = r#"{"level":"ERROR","fields":{"message":"nested boom"}}"#;
    let top = r#"{"level":"error","message":"top boom"}"#;
    let stats = parse_severity_lines(&format!("{nested}\n{top}"));
    assert_eq!(stats.len(), 2);
    assert!(stats.iter().all(|s| s.level == Severity::Error));
    assert_eq!(stats[0].sample_verbatim, "nested boom");
    assert_eq!(stats[1].sample_verbatim, "top boom");
}

#[test]
fn ignores_json_info_lines_and_non_severity_json() {
    let logs =
        "{\"level\":\"INFO\",\"fields\":{\"message\":\"ok\"}}\n{\"unrelated\":true}\n{not json}";
    assert!(parse_severity_lines(logs).is_empty());
}

// ---- normalization / grouping ----------------------------------------------

#[test]
fn recurring_warning_with_drifting_ids_collapses_to_one_bucket() {
    // Same warning three cycles, each with a different path number + timestamp.
    let logs = "\
2026-07-01T12:00:00Z LEVEL=warn MSG=no issue mirror at .fkst/mirror/42 - run reconcile
2026-07-01T12:05:00Z LEVEL=warn MSG=no issue mirror at .fkst/mirror/97 - run reconcile
2026-07-01T12:10:00Z LEVEL=warn MSG=no issue mirror at .fkst/mirror/1234 - run reconcile";
    let stats = parse_severity_lines(logs);
    assert_eq!(
        stats.len(),
        1,
        "the drifting id must not fragment the bucket"
    );
    assert_eq!(stats[0].count, 3);
    // The verbatim sample is the FIRST raw line, unedited.
    assert_eq!(
        stats[0].sample_verbatim,
        "no issue mirror at .fkst/mirror/42 - run reconcile"
    );
}

#[test]
fn hex_ids_are_normalized_for_grouping() {
    let logs = "\
TS=1 LEVEL=warn MSG=commit deadbeefcafe failed to push
TS=2 LEVEL=warn MSG=commit 0123456789ab failed to push";
    let stats = parse_severity_lines(logs);
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].count, 2);
}

#[test]
fn distinct_messages_stay_in_separate_buckets() {
    let logs = "\
TS=1 LEVEL=warn MSG=first kind of warning
TS=2 LEVEL=warn MSG=totally different warning";
    let stats = parse_severity_lines(logs);
    assert_eq!(stats.len(), 2);
    assert!(stats.iter().all(|s| s.count == 1));
}

#[test]
fn warn_and_error_with_same_text_are_distinct_buckets() {
    let logs = "TS=1 LEVEL=warn MSG=same text\nTS=2 LEVEL=error MSG=same text";
    let stats = parse_severity_lines(logs);
    assert_eq!(stats.len(), 2);
}

// ---- pod-status projection --------------------------------------------------

#[test]
fn summarize_pod_status_reads_phase_restarts_and_waiting_reason() {
    let pod = Pod {
        status: Some(PodStatus {
            phase: Some("Running".to_string()),
            container_statuses: Some(vec![ContainerStatus {
                name: "session".to_string(),
                restart_count: 4,
                state: Some(ContainerState {
                    waiting: Some(ContainerStateWaiting {
                        reason: Some("CrashLoopBackOff".to_string()),
                        message: None,
                    }),
                    ..Default::default()
                }),
                image: String::new(),
                image_id: String::new(),
                ready: false,
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };
    let summary = summarize_pod_status(&pod);
    assert_eq!(summary.phase.as_deref(), Some("Running"));
    assert_eq!(summary.restart_count, 4);
    assert_eq!(summary.waiting_reason.as_deref(), Some("CrashLoopBackOff"));
}

#[test]
fn summarize_pod_status_defaults_when_status_absent() {
    let summary = summarize_pod_status(&Pod::default());
    assert_eq!(summary, PodStatusSummary::default());
}

// ---- evaluate_health: the decision matrix -----------------------------------

#[test]
fn clean_running_pod_is_healthy() {
    assert_eq!(evaluate_health(&running(), &[]), HealthVerdict::Healthy);
}

#[test]
fn error_line_is_degraded_and_quotes_it_verbatim() {
    let parsed = vec![error_stat("engine crashed: boom", 1)];
    match evaluate_health(&running(), &parsed) {
        HealthVerdict::Degraded {
            reason_verbatim, ..
        } => assert_eq!(reason_verbatim, "engine crashed: boom"),
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn recurring_warn_at_threshold_is_degraded() {
    let parsed = vec![warn_stat(CODEX_WARN, WARN_RECUR_THRESHOLD)];
    match evaluate_health(&running(), &parsed) {
        HealthVerdict::Degraded { detail, .. } => {
            assert!(
                detail.contains(&format!("{WARN_RECUR_THRESHOLD}×")),
                "detail relays the recurrence: {detail}"
            );
        }
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn single_warn_is_healthy() {
    let parsed = vec![warn_stat("one-off warning", 1)];
    assert_eq!(evaluate_health(&running(), &parsed), HealthVerdict::Healthy);
}

#[test]
fn warn_below_threshold_is_healthy() {
    let parsed = vec![warn_stat("twice", WARN_RECUR_THRESHOLD - 1)];
    assert_eq!(evaluate_health(&running(), &parsed), HealthVerdict::Healthy);
}

#[test]
fn json_error_drives_degraded() {
    let parsed = parse_severity_lines(r#"{"level":"error","fields":{"message":"disk full"}}"#);
    match evaluate_health(&running(), &parsed) {
        HealthVerdict::Degraded {
            reason_verbatim, ..
        } => assert_eq!(reason_verbatim, "disk full"),
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn pod_restart_with_clean_logs_is_degraded_by_status() {
    let status = PodStatusSummary {
        phase: Some("Running".to_string()),
        restart_count: 2,
        waiting_reason: None,
    };
    match evaluate_health(&status, &[]) {
        HealthVerdict::Degraded {
            reason_verbatim,
            detail,
        } => {
            assert!(reason_verbatim.contains("restarted"), "{reason_verbatim}");
            assert!(detail.contains("restartCount=2"), "{detail}");
        }
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn crashloop_waiting_reason_is_degraded() {
    let status = PodStatusSummary {
        phase: Some("Running".to_string()),
        restart_count: 0,
        waiting_reason: Some("CrashLoopBackOff".to_string()),
    };
    match evaluate_health(&status, &[]) {
        HealthVerdict::Degraded {
            reason_verbatim, ..
        } => assert!(
            reason_verbatim.contains("CrashLoopBackOff"),
            "{reason_verbatim}"
        ),
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn benign_container_creating_is_healthy() {
    let status = PodStatusSummary {
        phase: Some("Pending".to_string()),
        restart_count: 0,
        waiting_reason: Some("ContainerCreating".to_string()),
    };
    assert_eq!(evaluate_health(&status, &[]), HealthVerdict::Healthy);
}

#[test]
fn failed_phase_is_degraded_but_pending_and_succeeded_are_not() {
    let failed = PodStatusSummary {
        phase: Some("Failed".to_string()),
        ..Default::default()
    };
    assert!(matches!(
        evaluate_health(&failed, &[]),
        HealthVerdict::Degraded { .. }
    ));
    for ok in ["Pending", "Succeeded", "Running"] {
        let status = PodStatusSummary {
            phase: Some(ok.to_string()),
            ..Default::default()
        };
        assert_eq!(
            evaluate_health(&status, &[]),
            HealthVerdict::Healthy,
            "phase {ok} must not be status-degraded"
        );
    }
}

#[test]
fn error_outranks_recurring_warn_and_status() {
    let status = PodStatusSummary {
        phase: Some("Running".to_string()),
        restart_count: 5,
        waiting_reason: None,
    };
    let parsed = vec![
        warn_stat("recurring warn", 9),
        error_stat("the real error", 1),
    ];
    match evaluate_health(&status, &parsed) {
        HealthVerdict::Degraded {
            reason_verbatim, ..
        } => assert_eq!(reason_verbatim, "the real error"),
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn recurring_warn_outranks_status_when_no_error() {
    let status = PodStatusSummary {
        phase: Some("Running".to_string()),
        restart_count: 5,
        waiting_reason: None,
    };
    let parsed = vec![warn_stat("recurring warn", 4)];
    match evaluate_health(&status, &parsed) {
        HealthVerdict::Degraded {
            reason_verbatim, ..
        } => assert_eq!(reason_verbatim, "recurring warn"),
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn most_significant_error_wins_by_count() {
    let parsed = vec![error_stat("rare error", 1), error_stat("frequent error", 8)];
    match evaluate_health(&running(), &parsed) {
        HealthVerdict::Degraded {
            reason_verbatim, ..
        } => assert_eq!(reason_verbatim, "frequent error"),
        other => panic!("expected degraded, got {other:?}"),
    }
}
