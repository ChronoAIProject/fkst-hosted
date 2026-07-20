//! Planner tests for the retire-notify path ([`super::plan_repo`]'s orphan-pod
//! branch): an orphan pod (its trigger issue closed) is killed, and when it recorded
//! its work label the planner ALSO emits [`ReconcileAction::RetireWorkIssues`] in the
//! same cycle so the still-open work issues can be notified + un-latched. Split from
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
            vec![
                ReconcileAction::Kill {
                    session_id: "orphan".to_string(),
                    reason: KillReason::TriggerClosed,
                },
                ReconcileAction::RetireWorkIssues {
                    work_label: Some("fkst-run".to_string()),
                },
            ],
            "orphan {liveness:?} pod with a work label must Kill + RetireWorkIssues"
        );
    }
}

#[test]
fn orphan_live_pod_without_work_label_only_kills() {
    // The existing kill-only path: an orphan pod with NO recorded work label emits
    // just the Kill — there is no label to list, so no RetireWorkIssues is planned.
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
        }],
        "an orphan pod without a work label must not emit RetireWorkIssues"
    );
}

#[test]
fn orphan_terminal_pod_with_work_label_only_cleans_up() {
    // Retire-notify rides ONLY the Starting/Live kill branch. A terminal orphan is
    // GC'd (CleanupTerminal) with no RetireWorkIssues, even if it carries a work label.
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
        }],
        "a terminal orphan is only cleaned up, never retire-notified"
    );
}
