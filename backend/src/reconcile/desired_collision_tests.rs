//! Tests for the R4a work-label collision backstop
//! ([`crate::reconcile::collision::detect_work_label_collisions`]) plus the end-to-end
//! fold-into-`invalid` + [`super::plan_repo`] flow the reconcile driver runs (demote →
//! flag once → auto-clear on resolution). Split out of the planner test files to keep
//! each under the 500-line limit; fixtures live in [`super::desired_test_fixtures`].
//!
//! The end-to-end tests mirror `reconcile::repo` exactly: detect the losers, remove
//! them from `regs`, extend `invalid` with the markers, then plan — so they prove the
//! reuse of the existing invalid path, not just the pure detector.

use std::collections::HashSet;

use super::desired_test_fixtures::*;
use super::{plan_repo, ReconcileAction};
use crate::reconcile::collision::detect_work_label_collisions;

/// A registration whose `### Work Label` was omitted (the discovered-label-only case):
/// the effective set comes entirely from `work_labels_by_session`.
fn reg_no_explicit_label(session_id: &str, trigger_issue: i64) -> super::SessionRegistration {
    let mut r = reg(session_id, trigger_issue, "h");
    r.def.work_label = None;
    r
}

/// Fold the detector's output the way `reconcile::repo` does: drop the losers from
/// `regs` and append their markers to `invalid`.
fn demote(
    mut regs: Vec<super::SessionRegistration>,
    work: &std::collections::HashMap<String, Vec<String>>,
) -> (Vec<super::SessionRegistration>, Vec<(i64, String)>) {
    let collisions = detect_work_label_collisions(&regs, work);
    let losers: HashSet<i64> = collisions.iter().map(|(i, _)| *i).collect();
    regs.retain(|r| !losers.contains(&r.trigger_issue));
    (regs, collisions)
}

// ---- the pure detector -----------------------------------------------------

#[test]
fn two_sessions_sharing_a_label_demote_the_higher_issue() {
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h")];
    let work = work_labels(&[("s1", &["wl"]), ("s2", &["wl"])]);
    assert_eq!(
        detect_work_label_collisions(&regs, &work),
        vec![(
            2,
            "work label 'wl' collides with active session #1".to_string()
        )],
        "the lower trigger issue wins; only the higher is demoted",
    );
}

#[test]
fn disjoint_label_sets_never_collide() {
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h")];
    let work = work_labels(&[("s1", &["alpha"]), ("s2", &["beta"])]);
    assert!(
        detect_work_label_collisions(&regs, &work).is_empty(),
        "no shared label → no collision",
    );
}

#[test]
fn discovered_label_only_overlap_still_collides() {
    // Neither trigger carries an explicit `### Work Label`; the shared label is
    // package-discovered. The detector treats the effective set uniformly, so they
    // still collide and the higher issue is demoted.
    let regs = vec![
        reg_no_explicit_label("s1", 1),
        reg_no_explicit_label("s2", 2),
    ];
    let work = work_labels(&[("s1", &["discovered"]), ("s2", &["discovered"])]);
    assert_eq!(
        detect_work_label_collisions(&regs, &work),
        vec![(
            2,
            "work label 'discovered' collides with active session #1".to_string()
        )],
    );
}

#[test]
fn empty_effective_label_set_never_collides() {
    // s1 has an empty set (shares no queue); s2 is the sole holder of "wl". Neither is
    // a loser — an empty-set session cannot collide, and a sole holder owns its queue.
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h")];
    let work = work_labels(&[("s1", &[]), ("s2", &["wl"])]);
    assert!(detect_work_label_collisions(&regs, &work).is_empty());
}

#[test]
fn a_missing_map_entry_is_treated_as_an_empty_set() {
    // Defensive: a session absent from the map (should not happen — the driver fills
    // every session) is treated as label-less and never collides.
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h")];
    let work = work_labels(&[("s2", &["wl"])]); // s1 omitted entirely
    assert!(detect_work_label_collisions(&regs, &work).is_empty());
}

#[test]
fn winner_is_the_lowest_issue_regardless_of_registration_order() {
    // The higher issue appears FIRST in `regs`; the winner is still the lower one.
    let regs = vec![reg("s2", 2, "h"), reg("s1", 1, "h")];
    let work = work_labels(&[("s2", &["wl"]), ("s1", &["wl"])]);
    assert_eq!(
        detect_work_label_collisions(&regs, &work),
        vec![(
            2,
            "work label 'wl' collides with active session #1".to_string()
        )],
    );
}

#[test]
fn multiple_disjoint_groups_resolve_independently() {
    // Group A over "a": #1 wins, #2 loses. Group B over "b": #3 wins, #4 loses.
    let regs = vec![
        reg("s1", 1, "h"),
        reg("s2", 2, "h"),
        reg("s3", 3, "h"),
        reg("s4", 4, "h"),
    ];
    let work = work_labels(&[
        ("s1", &["a"]),
        ("s2", &["a"]),
        ("s3", &["b"]),
        ("s4", &["b"]),
    ]);
    assert_eq!(
        detect_work_label_collisions(&regs, &work),
        vec![
            (
                2,
                "work label 'a' collides with active session #1".to_string()
            ),
            (
                4,
                "work label 'b' collides with active session #3".to_string()
            ),
        ],
    );
}

#[test]
fn pairwise_overlap_chain_demotes_every_non_owner() {
    // A{x}#1, B{x,y}#2, C{y}#3 — a chain where A∩C is empty. Per the documented rule
    // (per-label lowest-issue-wins, lose-on-any → demoted): A owns x, B owns y, so B
    // loses x and C loses y; both B and C are demoted and only A survives.
    let regs = vec![reg("s1", 1, "h"), reg("s2", 2, "h"), reg("s3", 3, "h")];
    let work = work_labels(&[("s1", &["x"]), ("s2", &["x", "y"]), ("s3", &["y"])]);
    assert_eq!(
        detect_work_label_collisions(&regs, &work),
        vec![
            (
                2,
                "work label 'x' collides with active session #1".to_string()
            ),
            (
                3,
                "work label 'y' collides with active session #2".to_string()
            ),
        ],
    );
}

#[test]
fn detection_is_order_independent() {
    // The same logical input built with different registration + label-slice orders
    // must yield byte-identical output (no map/set iteration order leaks in).
    let regs_a = vec![reg("s1", 1, "h"), reg("s2", 2, "h"), reg("s3", 3, "h")];
    let regs_b = vec![reg("s3", 3, "h"), reg("s1", 1, "h"), reg("s2", 2, "h")];
    let work_a = work_labels(&[("s1", &["x"]), ("s2", &["x", "y"]), ("s3", &["y"])]);
    let work_b = work_labels(&[("s2", &["y", "x"]), ("s3", &["y"]), ("s1", &["x"])]);
    let a = detect_work_label_collisions(&regs_a, &work_a);
    let b = detect_work_label_collisions(&regs_b, &work_b);
    assert_eq!(a, b, "detection must not depend on iteration order");
    assert_eq!(
        a,
        vec![
            (
                2,
                "work label 'x' collides with active session #1".to_string()
            ),
            (
                3,
                "work label 'y' collides with active session #2".to_string()
            ),
        ],
    );
}

#[test]
fn a_single_registration_never_collides() {
    let regs = vec![reg("s1", 1, "h")];
    let work = work_labels(&[("s1", &["wl"])]);
    assert!(detect_work_label_collisions(&regs, &work).is_empty());
}

// ---- end-to-end: fold into `invalid` + plan --------------------------------

#[test]
fn loser_folded_into_invalid_is_flagged_and_winner_stays_valid() {
    // Both sessions share "wl". After demotion the winner (#1) stays in `regs` and the
    // loser (#2) is flagged via the SAME FlagInvalid path as a parse failure. #1's
    // announce is suppressed (latched-announced) and it is not pending, so the ONLY
    // action is the loser's flag — proving the winner is untouched.
    let (regs, invalid) = demote(
        vec![reg("s1", 1, "h"), reg("s2", 2, "h")],
        &work_labels(&[("s1", &["wl"]), ("s2", &["wl"])]),
    );
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &invalid,
        &[],
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
        vec![ReconcileAction::FlagInvalid {
            trigger_issue: 2,
            detail: "work label 'wl' collides with active session #1".to_string(),
        }]
    );
}

#[test]
fn demoted_loser_never_spawns_a_competing_pod() {
    // Even though the loser (#2) reports pending, it is out of `regs` after demotion,
    // so the planner emits NO Spawn for it — the backstop's core guarantee. The winner
    // (#1), also pending, spawns as normal.
    let (regs, invalid) = demote(
        vec![reg("s1", 1, "h"), reg("s2", 2, "h")],
        &work_labels(&[("s1", &["wl"]), ("s2", &["wl"])]),
    );
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[("s1", true), ("s2", true)]),
        &latched(&[]),
        &latched(&[1]), // suppress #1 announce for a focused assertion
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(
        actions.contains(&ReconcileAction::Spawn {
            reg: regs[0].clone(),
            detected_work_labels: vec![],
        }),
        "the winner spawns",
    );
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, ReconcileAction::Spawn { reg, .. } if reg.trigger_issue == 2)),
        "the demoted loser must never spawn a competing pod",
    );
    assert!(
        actions.contains(&ReconcileAction::FlagInvalid {
            trigger_issue: 2,
            detail: "work label 'wl' collides with active session #1".to_string(),
        }),
        "and it is flagged invalid instead",
    );
}

#[test]
fn already_flagged_loser_is_not_reflagged() {
    // The loser (#2) already carries the invalid label (latched) from a prior sweep:
    // the flag is deduped, so no re-comment/re-flag — no action at all this pass.
    let (regs, invalid) = demote(
        vec![reg("s1", 1, "h"), reg("s2", 2, "h")],
        &work_labels(&[("s1", &["wl"]), ("s2", &["wl"])]),
    );
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[("s1", false)]),
        &latched(&[2]), // already flagged invalid
        &latched(&[1]),
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert!(
        actions.is_empty(),
        "an already-flagged loser is neither re-flagged nor cleared",
    );
}

#[test]
fn collision_resolved_auto_clears_the_invalid_flag() {
    // The winner (#1) has closed, so only #2 remains — now the sole holder of "wl" and
    // therefore no longer a loser. It still carries the invalid label from the earlier
    // collision, so the standard reparse path CLEARS it (and, no longer suppressed, the
    // session announces). This is the auto-clear-on-resolution guarantee.
    let (regs, invalid) = demote(vec![reg("s2", 2, "h")], &work_labels(&[("s2", &["wl"])]));
    assert!(invalid.is_empty(), "the sole holder is not a loser");
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[("s2", false)]),
        &latched(&[2]), // still latched-invalid from the prior collision
        &latched(&[2]), // suppress the (now un-suppressed) announce for focus
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![ReconcileAction::ClearInvalid { trigger_issue: 2 }],
        "resolving the collision auto-clears the loser's invalid flag",
    );
}

#[test]
fn folded_losers_flag_in_deterministic_issue_order() {
    // Two independent collisions; regs deliberately given highest-issue-first. The
    // FlagInvalid actions must still come out in ascending issue order (the detector
    // sorts its markers, and plan_repo emits `invalid` in order).
    let (regs, invalid) = demote(
        vec![
            reg("s4", 4, "h"),
            reg("s3", 3, "h"),
            reg("s2", 2, "h"),
            reg("s1", 1, "h"),
        ],
        &work_labels(&[
            ("s1", &["a"]),
            ("s2", &["a"]),
            ("s3", &["b"]),
            ("s4", &["b"]),
        ]),
    );
    let announced = latched(&[1, 3]); // suppress the two winners' announces
    let actions = plan_repo(
        &regs,
        &work_labels(&[]),
        &invalid,
        &[],
        &pending(&[]),
        &latched(&[]),
        &announced,
        &config_hashes(&[]),
        &latched(&[]),
        now(),
        &cfg(300, 120),
    );
    assert_eq!(
        actions,
        vec![
            ReconcileAction::FlagInvalid {
                trigger_issue: 2,
                detail: "work label 'a' collides with active session #1".to_string(),
            },
            ReconcileAction::FlagInvalid {
                trigger_issue: 4,
                detail: "work label 'b' collides with active session #3".to_string(),
            },
        ]
    );
}
