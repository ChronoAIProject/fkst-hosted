//! Tests for the I4 label-less reject backstop
//! ([`crate::reconcile::collision::detect_missing_work_labels`], epic #594) plus the
//! end-to-end fold-into-`invalid` + [`super::plan_repo`] flow the reconcile driver runs
//! (demote → flag once → auto-clear when a label appears). A session whose EFFECTIVE
//! work-label set is empty (no explicit `### Work Label` AND no package-declared labels)
//! can never be woken, so it is demoted through the SAME invalid path as a parse failure
//! rather than spawning a pod that would fail its own in-pod work-label guard. Split from
//! `desired_collision_tests.rs` to keep each file under the 500-line limit; fixtures live
//! in [`super::desired_test_fixtures`].

use std::collections::HashSet;

use super::desired_test_fixtures::*;
use super::{plan_repo, ReconcileAction};
use crate::reconcile::collision::{detect_missing_work_labels, MISSING_WORK_LABEL_DETAIL};

/// A registration whose `### Work Label` was omitted (the discovered-label-only case):
/// the effective set comes entirely from `work_labels_by_session`.
fn reg_no_explicit_label(session_id: &str, trigger_issue: i64) -> super::SessionRegistration {
    let mut r = reg(session_id, trigger_issue, "h");
    r.def.work_label = None;
    r
}

/// Fold the missing-label detector's output the way `reconcile::repo` does: drop the
/// label-less registrations from `regs` and append their markers to `invalid`.
fn demote_missing(
    mut regs: Vec<super::SessionRegistration>,
    work: &std::collections::HashMap<String, Vec<String>>,
) -> (Vec<super::SessionRegistration>, Vec<(i64, String)>) {
    let missing = detect_missing_work_labels(&regs, work);
    let losers: HashSet<i64> = missing.iter().map(|(i, _)| *i).collect();
    regs.retain(|r| !losers.contains(&r.trigger_issue));
    (regs, missing)
}

// ---- the pure detector -----------------------------------------------------

#[test]
fn a_label_less_session_is_demoted_with_the_missing_label_reason() {
    // No explicit `### Work Label` AND no discovered labels → the effective set is empty.
    let regs = vec![reg_no_explicit_label("s1", 1)];
    let work = work_labels(&[("s1", &[])]);
    assert_eq!(
        detect_missing_work_labels(&regs, &work),
        vec![(1, MISSING_WORK_LABEL_DETAIL.to_string())],
    );
}

#[test]
fn a_session_with_any_effective_label_is_not_demoted() {
    // An explicit label OR a package-discovered one keeps the session out of the reject.
    let regs = vec![reg("s1", 1, "h"), reg_no_explicit_label("s2", 2)];
    let work = work_labels(&[("s1", &["explicit"]), ("s2", &["discovered"])]);
    assert!(detect_missing_work_labels(&regs, &work).is_empty());
}

#[test]
fn a_missing_map_entry_is_treated_as_label_less() {
    // Defensive: a session absent from the map (should not happen) is label-less.
    let regs = vec![reg_no_explicit_label("s1", 1)];
    let work = work_labels(&[]); // s1 omitted entirely
    assert_eq!(
        detect_missing_work_labels(&regs, &work),
        vec![(1, MISSING_WORK_LABEL_DETAIL.to_string())],
    );
}

#[test]
fn missing_markers_are_sorted_ascending_by_issue() {
    let regs = vec![
        reg_no_explicit_label("s3", 3),
        reg_no_explicit_label("s1", 1),
    ];
    let work = work_labels(&[("s1", &[]), ("s3", &[])]);
    let markers = detect_missing_work_labels(&regs, &work);
    assert_eq!(
        markers.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![1, 3],
    );
}

// ---- end-to-end: fold into `invalid` + plan --------------------------------

#[test]
fn label_less_session_is_flagged_and_never_spawns() {
    // The label-less session (#1) reports pending, but after demotion it is out of `regs`
    // so the planner emits NO Spawn — only the flag (via the SAME path as a parse
    // failure). A spawned session therefore always carries ≥1 work label.
    let (regs, invalid) = demote_missing(
        vec![reg_no_explicit_label("s1", 1)],
        &work_labels(&[("s1", &[])]),
    );
    let actions = plan_repo(
        &regs,
        &work_labels(&[("s1", &[])]),
        &invalid,
        &[],
        &pending(&[("s1", true)]),
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
            trigger_issue: 1,
            detail: MISSING_WORK_LABEL_DETAIL.to_string(),
        }],
        "a label-less session is flagged invalid and never spawned",
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, ReconcileAction::Spawn { .. })),
        "no Spawn for a label-less session",
    );
}

#[test]
fn adding_a_label_auto_clears_the_missing_label_flag() {
    // The session gained a label (explicit or package-discovered) since the earlier
    // reject, so it is no longer demoted — it re-appears as a plain valid registration
    // and the standard reparse path CLEARS its still-latched invalid flag.
    let (regs, invalid) = demote_missing(
        vec![reg_no_explicit_label("s1", 1)],
        &work_labels(&[("s1", &["now-has-one"])]),
    );
    assert!(invalid.is_empty(), "a now-labeled session is not demoted");
    let actions = plan_repo(
        &regs,
        &work_labels(&[("s1", &["now-has-one"])]),
        &invalid,
        &[],
        &pending(&[("s1", false)]),
        &latched(&[1]), // still latched-invalid from the prior missing-label reject
        &latched(&[1]), // suppress the announce for a focused assertion
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::ClearInvalid { trigger_issue: 1 }],
        "adding a work label auto-clears the missing-label flag",
    );
}
