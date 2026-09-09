//! Table tests for the heartbeat verdict, including the boundary and the false-alarm
//! regression that protects every idle session in the fleet.

use super::*;

fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("fixture timestamp")
        .with_timezone(&Utc)
}

const NOW: &str = "2026-07-30T15:00:00Z";

fn entry(generated_at: &str, interval: u64) -> HealthIndexEntry {
    HealthIndexEntry {
        id: "report-id".to_string(),
        key: "health/sid/report-id.md".to_string(),
        generated_at: generated_at.to_string(),
        expected_interval_secs: interval,
        status: "working".to_string(),
        headline: "headline".to_string(),
        producer: "fkst-health@0.1.0".to_string(),
    }
}

#[test]
fn no_live_runtime_and_no_reports_is_not_running() {
    let verdict = evaluate(None, false, at(NOW));
    assert_eq!(verdict.state, StalenessState::NotRunning);
    assert_eq!(verdict.expected_interval_secs, None);
    assert_eq!(verdict.age_secs, None);
}

#[test]
fn a_live_runtime_with_no_reports_is_never_reported() {
    let verdict = evaluate(None, true, at(NOW));
    assert_eq!(verdict.state, StalenessState::NeverReported);
    assert_eq!(verdict.age_secs, None);
}

#[test]
fn a_recent_report_on_a_live_runtime_is_fresh() {
    // 10 minutes old, 10-minute cadence.
    let verdict = evaluate(Some(&entry("2026-07-30T14:50:00Z", 600)), true, at(NOW));
    assert_eq!(verdict.state, StalenessState::Fresh);
    assert_eq!(verdict.expected_interval_secs, Some(600));
    assert_eq!(verdict.age_secs, Some(600));
}

#[test]
fn the_stale_boundary_is_exactly_two_intervals() {
    // Exactly 2x — still fresh. One tolerated missed tick plus flush/upload latency.
    let boundary = evaluate(Some(&entry("2026-07-30T14:40:00Z", 600)), true, at(NOW));
    assert_eq!(boundary.state, StalenessState::Fresh);
    assert_eq!(boundary.age_secs, Some(1200));

    // One second past — stale.
    let past = evaluate(Some(&entry("2026-07-30T14:39:59Z", 600)), true, at(NOW));
    assert_eq!(past.state, StalenessState::Stale);
    assert_eq!(past.age_secs, Some(1201));
}

/// THE false-alarm regression. A reaped pod is the normal end of a session's work; if
/// silence from one read as "stale", every idle session in the fleet would carry an
/// alarm — the exact failure this design exists to avoid.
#[test]
fn an_ancient_report_with_no_live_runtime_is_not_running_never_stale() {
    let verdict = evaluate(Some(&entry("2026-07-30T09:00:00Z", 600)), false, at(NOW));
    assert_eq!(verdict.state, StalenessState::NotRunning);
    assert_ne!(verdict.state, StalenessState::Stale);
    // The numbers are still reported, so a client can render "last report 6h ago"
    // without implying anything is wrong.
    assert_eq!(verdict.age_secs, Some(21_600));
    assert_eq!(verdict.expected_interval_secs, Some(600));
}

#[test]
fn the_producers_own_interval_is_honoured_not_a_control_plane_constant() {
    // A 30-minute cadence: 35 minutes old is well past 2x ten minutes, but comfortably
    // inside 2x thirty.
    let generous = evaluate(Some(&entry("2026-07-30T14:25:00Z", 1800)), true, at(NOW));
    assert_eq!(generous.state, StalenessState::Fresh);
    assert_eq!(generous.expected_interval_secs, Some(1800));

    // The same report age under a 1-minute declared cadence IS stale.
    let tight = evaluate(Some(&entry("2026-07-30T14:25:00Z", 60)), true, at(NOW));
    assert_eq!(tight.state, StalenessState::Stale);
}

#[test]
fn an_absurd_declared_interval_does_not_overflow() {
    let verdict = evaluate(
        Some(&entry("2026-07-30T14:00:00Z", u64::MAX)),
        true,
        at(NOW),
    );
    assert_eq!(
        verdict.state,
        StalenessState::Fresh,
        "saturating, and fail-open: a producer suppressing its own alarm is self-harm"
    );
}

#[test]
fn a_report_stamped_in_the_future_is_fresh_with_a_zero_age() {
    // Clock skew between the pod and the control plane must not underflow into
    // "ancient".
    let verdict = evaluate(Some(&entry("2026-07-30T15:30:00Z", 600)), true, at(NOW));
    assert_eq!(verdict.state, StalenessState::Fresh);
    assert_eq!(verdict.age_secs, Some(0));
}

#[test]
fn an_unreadable_timestamp_never_alarms() {
    let verdict = evaluate(Some(&entry("not a timestamp", 600)), true, at(NOW));
    assert_eq!(
        verdict.state,
        StalenessState::Fresh,
        "a control-plane parse problem must not render as a stuck session"
    );
    assert_eq!(verdict.age_secs, None);
    assert_eq!(verdict.expected_interval_secs, Some(600));
}

#[test]
fn the_state_serializes_in_snake_case_for_the_wire() {
    for (state, wire) in [
        (StalenessState::NotRunning, "\"not_running\""),
        (StalenessState::NeverReported, "\"never_reported\""),
        (StalenessState::Fresh, "\"fresh\""),
        (StalenessState::Stale, "\"stale\""),
    ] {
        assert_eq!(serde_json::to_string(&state).expect("json"), wire);
    }
}
