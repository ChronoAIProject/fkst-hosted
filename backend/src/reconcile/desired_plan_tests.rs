//! Exhaustive tests for the pure planner ([`super::plan_repo`]): one per
//! event→action matrix row (issue #359 §4.3), plus the precedence and clock-gating
//! cases. Fixtures live in [`super::desired_test_fixtures`]; the session-announcement
//! and determinism cases live in [`super::desired_announce_tests`].
//!
//! The pod-lifecycle rows pass a `latched_announced` set covering their valid
//! registrations so the one-time [`ReconcileAction::AnnounceSession`] is suppressed
//! and each assertion stays about the single lifecycle action under test.

use super::desired_test_fixtures::*;
use super::{plan_repo, KillReason, PodLiveness, ReconcileAction};

// ---- matrix rows -----------------------------------------------------------

#[test]
fn valid_absent_pending_spawns() {
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &[],
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::Spawn {
            reg: regs[0].clone(),
            detected_work_labels: vec![],
        }]
    );
}

#[test]
fn valid_absent_not_pending_does_nothing() {
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &[],
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty());
}

#[test]
fn absent_liveness_pod_is_treated_as_absent_and_spawns() {
    // A LivePod carrying liveness=Absent (not merely a missing entry) must be
    // handled identically to no pod at all.
    let regs = vec![reg("s1", 1, "h")];
    let live = vec![pod("s1", 1, PodLiveness::Absent, ago(10), None, None)];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::Spawn {
            reg: regs[0].clone(),
            detected_work_labels: vec![],
        }]
    );
}

#[test]
fn valid_live_pending_touches() {
    for liveness in [PodLiveness::Starting, PodLiveness::Live] {
        let regs = vec![reg("s1", 1, "h")];
        let live = vec![pod("s1", 1, liveness, ago(1000), Some(ago(1)), Some("h"))];
        let actions = plan_repo(
            &regs,
            &work_labels(&[]),
            &[],
            &live,
            &pending(&[("s1", true)]),
            &latched(&[]),
            &latched(&[1]),
            &config_hashes(&[]),
            &latched(&[]),
            now(),
            &cfg(300, 120),
        );
        assert_eq!(
            actions,
            vec![ReconcileAction::TouchPending {
                session_id: "s1".to_string()
            }],
            "liveness {liveness:?} + pending must TouchPending"
        );
    }
}

#[test]
fn valid_live_idle_past_both_clocks_kills_idle() {
    let regs = vec![reg("s1", 1, "h")];
    // Alive 1000s (>= 120 min lifetime), idle 500s (>= 300 grace).
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(1000),
        Some(ago(500)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::Kill {
            session_id: "s1".to_string(),
            reason: KillReason::Idle,
        }]
    );
}

#[test]
fn idle_not_killed_before_idle_grace() {
    let regs = vec![reg("s1", 1, "h")];
    // Alive 1000s (past min lifetime) but idle only 100s (< 300 grace).
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(1000),
        Some(ago(100)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty(), "not idle long enough → no kill");
}

#[test]
fn idle_not_killed_before_min_lifetime() {
    let regs = vec![reg("s1", 1, "h")];
    // idle_grace 60 < min_lifetime 600: idle for 100s (>= 60 grace) but alive only
    // 100s (< 600 min lifetime), so the min-lifetime shield must suppress the kill.
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(100),
        Some(ago(100)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(60, 600),
    );
    assert!(actions.is_empty(), "min-lifetime shield → no kill");
}

#[test]
fn config_mismatch_kills_config_changed_regardless_of_pending() {
    // "any" pending column: drift wins whether or not the session is pending.
    for is_pending in [true, false] {
        let regs = vec![reg("s1", 1, "want")];
        let live = vec![pod(
            "s1",
            1,
            PodLiveness::Live,
            ago(10),
            Some(ago(1)),
            Some("stale"),
        )];
        let actions = plan_repo(
            &regs,
            &work_labels(&[]),
            &[],
            &live,
            &pending(&[("s1", is_pending)]),
            &latched(&[]),
            &latched(&[1]),
            &config_hashes(&[]),
            &latched(&[]),
            now(),
            &cfg(300, 120),
        );
        assert_eq!(
            actions,
            vec![ReconcileAction::Kill {
                session_id: "s1".to_string(),
                reason: KillReason::ConfigChanged,
            }],
            "drift with pending={is_pending} must Kill(ConfigChanged)"
        );
    }
}

#[test]
fn config_drift_kill_beats_idle() {
    // Both drift AND idle-due hold; drift must take precedence.
    let regs = vec![reg("s1", 1, "want")];
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Live,
        ago(1000),
        Some(ago(500)),
        Some("stale"),
    )];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::Kill {
            session_id: "s1".to_string(),
            reason: KillReason::ConfigChanged,
        }]
    );
}

#[test]
fn unknown_pod_config_hash_is_not_drift() {
    // A pod with no recorded hash yields no drift decision (treated as no drift):
    // a pending session must still TouchPending, not be killed.
    let regs = vec![reg("s1", 1, "want")];
    let live = vec![pod("s1", 1, PodLiveness::Live, ago(10), Some(ago(1)), None)];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::TouchPending {
            session_id: "s1".to_string()
        }]
    );
}

#[test]
fn valid_terminal_cleans_up() {
    let regs = vec![reg("s1", 1, "h")];
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Terminal,
        ago(10),
        None,
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::CleanupTerminal {
            session_id: "s1".to_string()
        }]
    );
}

#[test]
fn valid_terminating_does_nothing() {
    let regs = vec![reg("s1", 1, "h")];
    let live = vec![pod(
        "s1",
        1,
        PodLiveness::Terminating,
        ago(10),
        Some(ago(1)),
        Some("h"),
    )];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[("s1", false)]),
        &latched(&[]),
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty(), "a Terminating pod is left alone");
}

#[test]
fn orphan_live_pod_is_killed_trigger_closed() {
    for liveness in [PodLiveness::Starting, PodLiveness::Live] {
        // No registration references this pod's session -> its trigger closed.
        let live = vec![pod("orphan", 9, liveness, ago(10), Some(ago(1)), Some("h"))];
        let actions = plan_repo(
            &[],
            &work_labels(&[]),
            &[],
            &live,
            &pending(&[]),
            &latched(&[]),
            &latched(&[]),
            &config_hashes(&[]),
            &latched(&[]),
            now(),
            &cfg(300, 120),
        );
        assert_eq!(
            actions,
            vec![ReconcileAction::Kill {
                session_id: "orphan".to_string(),
                reason: KillReason::TriggerClosed,
            }],
            "orphan {liveness:?} pod must Kill(TriggerClosed)"
        );
    }
}

#[test]
fn orphan_terminal_pod_is_cleaned_up() {
    let live = vec![pod("orphan", 9, PodLiveness::Terminal, ago(10), None, None)];
    let actions = plan_repo(
        &[],
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::CleanupTerminal {
            session_id: "orphan".to_string()
        }]
    );
}

#[test]
fn orphan_terminating_pod_does_nothing() {
    let live = vec![pod(
        "orphan",
        9,
        PodLiveness::Terminating,
        ago(10),
        None,
        None,
    )];
    let actions = plan_repo(
        &[],
        &work_labels(&[]),
        &[],
        &live,
        &pending(&[]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty());
}

#[test]
fn invalid_issue_not_latched_is_flagged() {
    let invalid = vec![(5, "missing `### Packages`".to_string())];
    let actions = plan_repo(
        &[],
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::FlagInvalid {
            trigger_issue: 5,
            detail: "missing `### Packages`".to_string(),
        }]
    );
}

#[test]
fn invalid_issue_already_latched_is_not_reflagged() {
    let invalid = vec![(5, "still bad".to_string())];
    let actions = plan_repo(
        &[],
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[5]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(
        actions.is_empty(),
        "an already-latched issue is not reflagged"
    );
}

#[test]
fn latched_issue_that_reparses_is_cleared() {
    // Issue 5 is latched-invalid but now appears as a valid registration. It is both
    // ClearInvalid'd AND (being valid + not yet announced) announced; the announce is
    // suppressed here via `latched_announced` to keep the assertion about the clear.
    let regs = vec![reg("s5", 5, "h")];
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &[],
        &[],
        &pending(&[("s5", false)]),
        &latched(&[5]),
        &latched(&[5]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::ClearInvalid { trigger_issue: 5 }]
    );
}

#[test]
fn latched_issue_still_invalid_is_not_cleared() {
    // Issue 5 is still invalid (in `invalid`, not in `regs`): no ClearInvalid, and
    // because it is latched, no re-FlagInvalid either → no action at all.
    let invalid = vec![(5, "still bad".to_string())];
    let actions = plan_repo(
        &[],
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[5]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty());
}

#[test]
fn empty_inputs_produce_no_actions() {
    let actions = plan_repo(
        &[],
        &work_labels(&[]),
        &[],
        &[],
        &pending(&[]),
        &latched(&[]),
        &latched(&[]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty());
}
