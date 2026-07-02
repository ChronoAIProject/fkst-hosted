//! Planner tests for the config-immutability check ([`super::plan_repo`]): once a
//! trigger is announced, a config edit (its CURRENT `full_config_hash` differs from
//! the ORIGINAL latched in `latched_config_hash`) is REJECTED — the edit never drives
//! a Spawn/`Kill { ConfigChanged }`, and a one-time [`ReconcileAction::RejectConfigChange`]
//! is emitted (deduped by `latched_config_rejected`). Fixtures live in
//! [`super::desired_test_fixtures`].

use super::desired_test_fixtures::*;
use super::{full_config_hash, plan_repo, KillReason, PodLiveness, ReconcileAction};

#[test]
fn latched_original_equal_to_current_is_not_rejected() {
    // The latched original == the registration's current full hash -> no edit, so no
    // rejection: a live pending pod just refreshes its clock as normal.
    let regs = vec![reg("s1", 1, "h")];
    let original = full_config_hash(&regs[0]);
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(1000),
        Some(ago(1)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[(1, &original)]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::TouchPending {
            session_id: "s1".to_string()
        }],
        "an unchanged config plans normally, no RejectConfigChange"
    );
}

#[test]
fn changed_config_is_rejected_once_and_does_not_respawn() {
    // The registration was edited: its pod-subset hash drifted from the live pod's
    // ("h" -> "h2") AND its full hash differs from the latched original -> rejected.
    // The drift kill is SUPPRESSED (the pod keeps serving its original config) and the
    // rejection feedback is emitted exactly once.
    let regs = vec![reg("s1", 1, "h2")];
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(1000),
        Some(ago(1)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[(1, "original-differs")]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![
            ReconcileAction::TouchPending {
                session_id: "s1".to_string()
            },
            ReconcileAction::RejectConfigChange { trigger_issue: 1 },
        ],
        "the drift kill is suppressed; the rejection is emitted once"
    );
    // No pod respawn is driven by the edit (belt-and-braces over the assert_eq above).
    assert!(
        !actions.iter().any(|a| matches!(
            a,
            ReconcileAction::Kill {
                reason: KillReason::ConfigChanged,
                ..
            } | ReconcileAction::Spawn(_)
        )),
        "a rejected edit never respawns the pod"
    );
}

#[test]
fn changed_config_suppresses_spawn_when_pod_absent() {
    // Absent + pending would normally Spawn; a rejected edit suppresses that too — we
    // cannot spawn the original config (only its hash is latched) and must not spawn
    // the edited one. Only the one-time rejection is emitted.
    let regs = vec![reg("s1", 1, "h2")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[(1, "original-differs")]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::RejectConfigChange { trigger_issue: 1 }],
        "a rejected edit suppresses the spawn"
    );
}

#[test]
fn changed_config_already_rejected_is_not_re_emitted() {
    // The trigger already carries the rejected latch -> the rejection is deduped (still
    // no respawn), leaving only the normal live-pod action.
    let regs = vec![reg("s1", 1, "h2")];
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(1000),
        Some(ago(1)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[(1, "original-differs")]),
        &latched(&[1]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::TouchPending {
            session_id: "s1".to_string()
        }],
        "an already-rejected edit is not re-commented"
    );
}

#[test]
fn pre_announce_no_latched_hash_plans_normally() {
    // No latched original (the issue is not yet announced) -> the immutability check is
    // a no-op: a pending absent pod spawns AND announces, with no rejection.
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, ReconcileAction::Spawn(_))),
        "a pre-announce registration spawns normally"
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, ReconcileAction::RejectConfigChange { .. })),
        "no rejection without a latched original"
    );
}
