use std::collections::HashSet;

use super::{plan_trigger_authorization, ReconcileAction};

fn set(values: &[i64]) -> HashSet<i64> {
    values.iter().copied().collect()
}

#[test]
fn unauthorized_trigger_is_flagged_only_before_the_latch_exists() {
    let marker = [(7, "creator lacks authority".to_string())];
    assert_eq!(
        plan_trigger_authorization(&marker, &HashSet::new(), &HashSet::new()),
        vec![ReconcileAction::FlagTriggerUnauthorized {
            trigger_issue: 7,
            detail: "creator lacks authority".to_string(),
        }]
    );
    assert!(plan_trigger_authorization(&marker, &HashSet::new(), &set(&[7])).is_empty());
}

#[test]
fn authorized_latched_trigger_clears_in_deterministic_order() {
    assert_eq!(
        plan_trigger_authorization(&[], &set(&[2, 4]), &set(&[9, 4, 2])),
        vec![
            ReconcileAction::ClearTriggerUnauthorized { trigger_issue: 2 },
            ReconcileAction::ClearTriggerUnauthorized { trigger_issue: 4 },
        ]
    );
}

#[test]
fn deferred_or_silently_skipped_trigger_does_not_clear_a_latch() {
    assert!(plan_trigger_authorization(&[], &HashSet::new(), &set(&[7])).is_empty());
}
