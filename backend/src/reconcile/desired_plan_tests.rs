//! Exhaustive tests for the pure planner ([`super::plan_repo`]): one per
//! event→action matrix row (issue #359 §4.3), plus the precedence, clock-gating,
//! and determinism cases. Fixtures live in [`super::desired_test_fixtures`].

use std::collections::HashSet;

use super::desired_test_fixtures::*;
use super::{plan_repo, KillReason, PodLiveness, ReconcileAction};

// ---- matrix rows -----------------------------------------------------------

#[test]
fn valid_absent_pending_spawns() {
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", true)]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(actions, vec![ReconcileAction::Spawn(regs[0].clone())]);
}

#[test]
fn valid_absent_not_pending_does_nothing() {
    let regs = vec![reg("s1", 1, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s1", false)]),
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
        &[],
        &live,
        &pending(&[("s1", true)]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(actions, vec![ReconcileAction::Spawn(regs[0].clone())]);
}

#[test]
fn valid_live_pending_touches() {
    for liveness in [PodLiveness::Starting, PodLiveness::Live] {
        let regs = vec![reg("s1", 1, "h")];
        let live = vec![pod("s1", 1, liveness, ago(1000), Some(ago(1)), Some("h"))];
        let actions = plan_repo(
            &regs,
            &[],
            &live,
            &pending(&[("s1", true)]),
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
        &[],
        &live,
        &pending(&[("s1", false)]),
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
        &[],
        &live,
        &pending(&[("s1", false)]),
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
        &[],
        &live,
        &pending(&[("s1", false)]),
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
            &[],
            &live,
            &pending(&[("s1", is_pending)]),
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
        &[],
        &live,
        &pending(&[("s1", false)]),
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
        &[],
        &live,
        &pending(&[("s1", true)]),
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
        &[],
        &live,
        &pending(&[("s1", true)]),
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
        &[],
        &live,
        &pending(&[("s1", false)]),
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
            &[],
            &live,
            &pending(&[]),
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
        &[],
        &live,
        &pending(&[]),
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
        &[],
        &live,
        &pending(&[]),
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
        &invalid,
        &[],
        &pending(&[]),
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
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[5]),
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
    // Issue 5 is latched-invalid but now appears as a valid registration.
    let regs = vec![reg("s5", 5, "h")];
    let actions = plan_repo(
        &regs,
        &[],
        &[],
        &pending(&[("s5", false)]),
        &latched(&[5]),
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
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[5]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty());
}

#[test]
fn empty_inputs_produce_no_actions() {
    let actions = plan_repo(
        &[],
        &[],
        &[],
        &pending(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(actions.is_empty());
}

// ---- determinism / order-independence --------------------------------------

#[test]
fn clear_invalid_output_is_order_independent_of_the_set() {
    let regs = vec![reg("s3", 3, "h"), reg("s5", 5, "h"), reg("s8", 8, "h")];
    // Two logically-equal sets built by inserting the ids in different orders.
    let a: HashSet<i64> = [3, 5, 8].into_iter().collect();
    let b: HashSet<i64> = [8, 3, 5].into_iter().collect();
    let plan_a = plan_repo(&regs, &[], &[], &pending(&[]), &a, now(), &cfg(300, 120));
    let plan_b = plan_repo(&regs, &[], &[], &pending(&[]), &b, now(), &cfg(300, 120));
    assert_eq!(
        plan_a, plan_b,
        "set iteration order must not leak into output"
    );
    assert_eq!(
        plan_a,
        vec![
            ReconcileAction::ClearInvalid { trigger_issue: 3 },
            ReconcileAction::ClearInvalid { trigger_issue: 5 },
            ReconcileAction::ClearInvalid { trigger_issue: 8 },
        ],
        "cleared issues are emitted in ascending order"
    );
}

#[test]
fn plan_output_is_order_independent_of_the_pending_map() {
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h")];
    let live = vec![
        pod("s1", 1, PodLiveness::Live, ago(10), Some(ago(1)), Some("h")),
        pod("s2", 2, PodLiveness::Live, ago(10), Some(ago(1)), Some("h")),
    ];
    let m1 = pending(&[("s1", true), ("s2", true)]);
    let m2 = pending(&[("s2", true), ("s1", true)]);
    let p1 = plan_repo(&regs, &[], &live, &m1, &latched(&[]), now(), &cfg(300, 120));
    let p2 = plan_repo(&regs, &[], &live, &m2, &latched(&[]), now(), &cfg(300, 120));
    assert_eq!(p1, p2);
}
