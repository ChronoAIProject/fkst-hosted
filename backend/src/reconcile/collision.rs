//! Work-label collision backstop for the reconcile planner (epic #572, R4a).
//!
//! A `fkst-substrate-trigger` issue can be created directly on GitHub, so the
//! one-work-label-per-trigger authoring rule cannot be enforced at authoring time
//! alone. This module is the AUTHORITATIVE server-side guard that stops two active
//! sessions on the same repo from competing over one work-label queue: among the
//! repo's OPEN, otherwise-valid registrations it finds every group whose EFFECTIVE
//! work-label sets (explicit `### Work Label` ∪ package-discovered, precomputed per
//! session by the reconcile driver) intersect, and demotes the losers so only one
//! session ever claims a given queue.
//!
//! The result is folded into the reconcile planner's `invalid` input
//! ([`crate::reconcile::desired::plan_repo`]), so a demoted loser flows through the
//! EXISTING invalid path — it is flagged with `fkst-substrate-invalid` + a comment and
//! AUTO-CLEARS the moment the collision is resolved (the winner is closed, or the
//! loser's work label is changed), when it re-appears as a plain valid registration.
//!
//! Collision tests live alongside the planner tests in `desired_collision_tests.rs`
//! (registered under the `desired` module) so they can reuse the shared fixtures.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::reconcile::desired::SessionRegistration;

/// Detect work-label collisions among OPEN, otherwise-valid registrations and return
/// the loser markers `(trigger_issue, reason)` for the reconcile driver to fold into
/// the planner's `invalid` Vec.
///
/// ## Collision rule (per-label, lowest-issue-wins)
///
/// Two sessions collide when their effective label sets share at least one label —
/// they would otherwise spawn competing pods over the same work queue. Grouping is per
/// INDIVIDUAL label: for each label the registration with the LOWEST trigger issue
/// number OWNS the queue; every other holder LOSES on that label. A registration that
/// loses on ANY of its labels is demoted.
///
/// This stays simple and order-free even for a pairwise-overlap chain: with `A{x}#1`,
/// `B{x,y}#2`, `C{y}#3`, label `x` is owned by A and label `y` by B, so B loses `x`
/// and C loses `y` — both B and C are demoted and only A survives. (A demoted owner
/// still owns its other labels for citation purposes, so C's reason cites #2 even
/// though #2 is itself demoted.)
///
/// A session with an EMPTY effective label set shares no queue and never collides.
///
/// ## Determinism
///
/// The output depends only on the `(session, label-set, issue-number)` facts, never on
/// the iteration order of `work_labels_by_session` or `regs`: label ownership is a
/// `min()` reduction, each loser cites its lexicographically-lowest losing label, and
/// the markers are returned sorted ascending by trigger issue.
pub fn detect_work_label_collisions(
    regs: &[SessionRegistration],
    work_labels_by_session: &HashMap<String, Vec<String>>,
) -> Vec<(i64, String)> {
    // 1. Owner of each label = the lowest trigger-issue number that holds it. Both the
    //    BTreeMap and the min() reduction are order-independent, so ownership never
    //    depends on the iteration order of `regs` or `work_labels_by_session`.
    let mut owner_by_label: BTreeMap<&str, i64> = BTreeMap::new();
    for reg in regs {
        let Some(labels) = work_labels_by_session.get(&reg.session_id) else {
            continue;
        };
        for label in labels {
            owner_by_label
                .entry(label.as_str())
                .and_modify(|owner| *owner = (*owner).min(reg.trigger_issue))
                .or_insert(reg.trigger_issue);
        }
    }

    // 2. A registration loses if any of its labels is owned by a lower trigger issue.
    //    Cite the lexicographically-lowest losing label (via the ordered BTreeSet) and
    //    that label's owner so the reason string is deterministic; a session losing on
    //    ANY label is demoted, so the first losing label is enough.
    let mut losers: Vec<(i64, String)> = Vec::new();
    for reg in regs {
        let Some(labels) = work_labels_by_session.get(&reg.session_id) else {
            continue;
        };
        let sorted: BTreeSet<&str> = labels.iter().map(String::as_str).collect();
        for label in sorted {
            match owner_by_label.get(label) {
                Some(&owner) if owner != reg.trigger_issue => {
                    losers.push((
                        reg.trigger_issue,
                        format!("work label '{label}' collides with active session #{owner}"),
                    ));
                    break;
                }
                _ => {}
            }
        }
    }
    losers.sort_by_key(|(issue, _)| *issue);
    losers
}
