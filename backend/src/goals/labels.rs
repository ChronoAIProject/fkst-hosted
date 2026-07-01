//! Read-then-replace label set-math for a GitHub issue's label set.
//!
//! GitHub's issue PATCH REPLACES the whole label set, so any label update the
//! reconciler makes must be read-then-replace ([`merge_labels`]) — it must never
//! drop unrelated labels a maintainer added. This module is the single, testable
//! home of that set math; the Model-A lifecycle-label vocabulary
//! (`fkst-goal`/`fkst-running`/…) was removed with the Job watcher.

/// Compute the replacement label set for a read-then-replace PATCH:
/// `(current ∪ add) \ remove`, order-stable and de-duplicated. Unrelated labels
/// in `current` are preserved; a label that appears in BOTH `add` and `remove` is
/// removed (remove wins) — callers never overlap the two, but the rule keeps the
/// function total.
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
    fn merge_adds_and_removes_while_keeping_unrelated_links() {
        let current = vec![
            "fkst-substrate-trigger".to_string(),
            "fkst-running".to_string(),
        ];
        let next = merge_labels(
            &current,
            &["fkst-completed"],
            &["fkst-running", "fkst-terminated", "fkst-failed"],
        );
        assert!(
            next.contains(&"fkst-substrate-trigger".to_string()),
            "unrelated trigger label preserved: {next:?}"
        );
        assert!(next.contains(&"fkst-completed".to_string()), "{next:?}");
        assert!(!next.contains(&"fkst-running".to_string()), "{next:?}");
    }

    #[test]
    fn merge_preserves_unrelated_labels_and_dedups() {
        let current = vec![
            "fkst-substrate-trigger".to_string(),
            "bug".to_string(),
            "fkst-running".to_string(),
        ];
        let next = merge_labels(&current, &["fkst-terminated", "bug"], &["fkst-running"]);
        // Unrelated `bug` is preserved exactly once (no duplicate from `add`).
        assert_eq!(next.iter().filter(|l| *l == "bug").count(), 1, "{next:?}");
        assert!(next.contains(&"fkst-terminated".to_string()), "{next:?}");
        assert!(!next.contains(&"fkst-running".to_string()), "{next:?}");
    }

    #[test]
    fn merge_is_noop_when_nothing_changes() {
        let current = vec![
            "fkst-substrate-trigger".to_string(),
            "fkst-running".to_string(),
        ];
        let next = merge_labels(&current, &["fkst-running"], &[]);
        assert_eq!(next, current, "re-adding an existing label is a no-op");
    }

    #[test]
    fn merge_remove_wins_over_add_for_the_same_label() {
        let current: Vec<String> = vec![];
        let next = merge_labels(&current, &["fkst-running"], &["fkst-running"]);
        assert!(next.is_empty(), "remove wins over add: {next:?}");
    }
}
