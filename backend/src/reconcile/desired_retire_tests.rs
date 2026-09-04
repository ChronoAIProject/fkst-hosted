//! Planner tests for the retire-notify path ([`super::plan_repo`]'s orphan-pod
//! branch): an orphan pod with recorded work labels emits one transactional
//! [`ReconcileAction::RetireSession`] before replacement actions, so failed durable
//! retirement keeps the runtime available for the next retry. Split from
//! `desired_plan_tests` to keep each test file under the 500-line limit; fixtures live
//! in [`super::desired_test_fixtures`].

use super::desired_test_fixtures::*;
use super::{plan_repo, KillReason, PodLiveness, ReconcileAction};

#[test]
fn orphan_live_pod_with_work_label_also_retires_its_work_issues() {
    for liveness in [PodLiveness::Starting, PodLiveness::Live] {
        // An orphan pod that recorded its work label: the trigger closed, so the pod
        // is killed AND its still-open work issues are retire-notified (same cycle).
        let live = vec![pod_with_work_label(
            "orphan",
            9,
            liveness,
            ago(10),
            Some(ago(1)),
            Some("h"),
            "fkst-run",
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
        assert_eq!(
            actions,
            vec![ReconcileAction::RetireSession {
                session_id: "orphan".to_string(),
                work_labels: vec!["fkst-run".to_string()],
                audit: orphan_audit(&live[0]),
            }],
            "orphan {liveness:?} pod with a work label must retire before stopping"
        );
    }
}

#[test]
fn orphan_live_pod_with_multiple_work_labels_retires_across_all_of_them() {
    // A multi-label session (epic #594 I4): the orphan carries its FULL effective set, so
    // the retire action lists EVERY label — not just one — when the trigger closes.
    let live = vec![pod_with_work_labels(
        "orphan",
        9,
        PodLiveness::Live,
        ago(10),
        Some(ago(1)),
        Some("h"),
        &["fkst-run", "pkg-discovered"],
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
    assert_eq!(
        actions,
        vec![ReconcileAction::RetireSession {
            session_id: "orphan".to_string(),
            work_labels: vec!["fkst-run".to_string(), "pkg-discovered".to_string()],
            audit: orphan_audit(&live[0]),
        }],
        "an orphan carrying a multi-label set must retire across every label"
    );
}

#[test]
fn orphan_retirement_precedes_active_replacement_actions() {
    let replacement = reg("replacement", 10, "new-hash");
    let live = vec![pod_with_work_label(
        "orphan",
        9,
        PodLiveness::Live,
        ago(10),
        Some(ago(1)),
        Some("old-hash"),
        "fkst-run",
    )];
    let actions = plan_repo(
        std::slice::from_ref(&replacement),
        &work_labels(&[("replacement", &["fkst-run"])]),
        &[],
        &live,
        &pending(&[("replacement", true)]),
        &latched(&[]),
        &latched(&[10]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );

    assert!(matches!(
        actions.first(),
        Some(ReconcileAction::RetireSession { session_id, .. }) if session_id == "orphan"
    ));
    assert!(actions.iter().skip(1).any(
        |action| matches!(action, ReconcileAction::Spawn { reg, .. } if reg.session_id == "replacement")
    ));
}

#[test]
fn orphan_live_pod_without_work_label_only_kills() {
    // The existing kill-only path: an orphan pod with NO recorded work label emits
    // just the Kill — there is no label to list, so no RetireSession is planned.
    let live = vec![pod(
        "orphan",
        9,
        PodLiveness::Live,
        ago(10),
        Some(ago(1)),
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
    assert_eq!(
        actions,
        vec![ReconcileAction::Kill {
            session_id: "orphan".to_string(),
            reason: KillReason::TriggerClosed,
            audit: orphan_audit(&live[0]),
        }],
        "an orphan pod without a work label must not emit RetireSession"
    );
}

#[test]
fn orphan_terminal_pod_with_work_label_only_cleans_up() {
    // Retire-notify rides ONLY the Starting/Live kill branch. A terminal orphan is
    // GC'd (CleanupTerminal) with no RetireSession, even if it carries a work label.
    let live = vec![pod_with_work_label(
        "orphan",
        9,
        PodLiveness::Terminal,
        ago(10),
        None,
        None,
        "fkst-run",
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
    assert_eq!(
        actions,
        vec![ReconcileAction::CleanupTerminal {
            session_id: "orphan".to_string(),
            audit: orphan_audit(&live[0]),
        }],
        "a terminal orphan is only cleaned up, never retire-notified"
    );
}
