//! Session-lifecycle label vocabulary for a goal's GitHub Issue (#180).
//!
//! Replaces the old goal-status-centric `status:*` scheme. A goal issue now
//! carries `fkst-goal` (it IS an fkst goal) plus, once a session spawns,
//! `fkst-session-<id>` (the linkage) and exactly one lifecycle word
//! (`fkst-running` while live; one of `fkst-terminated`/`fkst-completed`/
//! `fkst-failed` at the terminal). The three terminal labels map 1:1 onto the
//! three real terminal causes (user-stop / graceful exit / error).
//!
//! GitHub's issue PATCH REPLACES the whole label set, so the updater is always
//! read-then-replace ([`merge_labels`]) — it must never drop unrelated labels,
//! and in particular never `fkst-goal` or the `fkst-session-<id>` link.
//!
//! NOTE: the hidden `marker.rs` server marker (`fkst-hosted:goal`, an HTML
//! comment in the body) is DISTINCT from the visible `fkst-goal` label here and
//! is unchanged by this module.

/// The label every fkst-hosted goal issue carries.
pub const GOAL_LABEL: &str = "fkst-goal";
/// The linked session is Running (added at Validating→Running; removed at the
/// terminal in favour of the matching terminal label).
pub const LABEL_RUNNING: &str = "fkst-running";
/// Terminal: the triggerer stopped the session (a deliberate user action).
pub const LABEL_TERMINATED: &str = "fkst-terminated";
/// Terminal: the engine finished gracefully on its own (uncommanded exit 0).
pub const LABEL_COMPLETED: &str = "fkst-completed";
/// Terminal: the session ended with an error (uncommanded non-zero / signal).
pub const LABEL_FAILED: &str = "fkst-failed";

/// Per-session linkage label, e.g. `fkst-session-<uuid>`. Stamped when a
/// session spawns for the goal so the issue points at the concrete run.
pub fn session_label(session_id: bson::Uuid) -> String {
    format!("fkst-session-{session_id}")
}

/// The full label set for a freshly-created/adopted goal issue: just the goal
/// label (no lifecycle word — that is driven by the session, not goal status).
pub fn initial_labels() -> Vec<String> {
    vec![GOAL_LABEL.to_string()]
}

/// Compute the replacement label set for a read-then-replace PATCH:
/// `(current ∪ add) \ remove`, order-stable and de-duplicated. Unrelated
/// labels in `current` are preserved; a label that appears in BOTH `add` and
/// `remove` is removed (remove wins) — callers never overlap the two, but the
/// rule keeps the function total. Used by the [`crate::goals::GoalIssueStore`]
/// label updater so the set math is unit-testable in isolation.
pub fn merge_labels(current: &[String], add: &[&str], remove: &[&str]) -> Vec<String> {
    let mut next: Vec<String> = Vec::with_capacity(current.len() + add.len());
    // Preserve current order; drop anything being removed.
    for label in current {
        if !remove.iter().any(|r| r == label) && !next.iter().any(|n| n == label) {
            next.push(label.clone());
        }
    }
    // Append additions not already present and not being removed.
    for label in add {
        if !remove.contains(label) && !next.iter().any(|n| n == *label) {
            next.push((*label).to_string());
        }
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_label_formats_with_prefix() {
        let id = bson::Uuid::new();
        assert_eq!(session_label(id), format!("fkst-session-{id}"));
    }

    #[test]
    fn initial_labels_is_just_the_goal_label() {
        assert_eq!(initial_labels(), vec![GOAL_LABEL.to_string()]);
    }

    #[test]
    fn merge_running_to_completed_keeps_goal_and_session_links() {
        let sid = bson::Uuid::new();
        let session = session_label(sid);
        let current = vec![
            GOAL_LABEL.to_string(),
            session.clone(),
            LABEL_RUNNING.to_string(),
        ];
        let next = merge_labels(
            &current,
            &[LABEL_COMPLETED],
            &[LABEL_RUNNING, LABEL_TERMINATED, LABEL_FAILED],
        );
        assert!(next.contains(&GOAL_LABEL.to_string()), "{next:?}");
        assert!(next.contains(&session), "session link preserved: {next:?}");
        assert!(next.contains(&LABEL_COMPLETED.to_string()), "{next:?}");
        assert!(!next.contains(&LABEL_RUNNING.to_string()), "{next:?}");
    }

    #[test]
    fn merge_preserves_unrelated_labels_and_dedups() {
        let current = vec![
            GOAL_LABEL.to_string(),
            "bug".to_string(),
            LABEL_RUNNING.to_string(),
        ];
        let next = merge_labels(&current, &[LABEL_TERMINATED, "bug"], &[LABEL_RUNNING]);
        // Unrelated `bug` is preserved exactly once (no duplicate from `add`).
        assert_eq!(next.iter().filter(|l| *l == "bug").count(), 1, "{next:?}");
        assert!(next.contains(&LABEL_TERMINATED.to_string()), "{next:?}");
        assert!(!next.contains(&LABEL_RUNNING.to_string()), "{next:?}");
    }

    #[test]
    fn merge_is_noop_when_nothing_changes() {
        let current = vec![GOAL_LABEL.to_string(), LABEL_RUNNING.to_string()];
        let next = merge_labels(&current, &[LABEL_RUNNING], &[]);
        assert_eq!(next, current, "re-adding an existing label is a no-op");
    }

    #[test]
    fn merge_remove_wins_over_add_for_the_same_label() {
        let current: Vec<String> = vec![];
        let next = merge_labels(&current, &[LABEL_RUNNING], &[LABEL_RUNNING]);
        assert!(next.is_empty(), "remove wins over add: {next:?}");
    }
}
