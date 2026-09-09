use super::*;

#[test]
fn every_reserved_label_is_recognised() {
    for label in RESERVED_LABELS {
        assert!(is_reserved_label(label), "{label} must be reserved");
    }
}

#[test]
fn recognition_is_case_insensitive_and_whitespace_tolerant() {
    // GitHub label identity is case-insensitive, so a case-shifted spelling would
    // collide with the reserved label on GitHub even though it looks different here.
    assert!(is_reserved_label("FKST-Scheduled-Workflow"));
    assert!(is_reserved_label("  fkst-cron-running  "));
}

#[test]
fn ordinary_work_labels_are_not_reserved() {
    for label in [
        "fkst-dev",
        "fkst-scheduled",
        "fkst-scheduled-workflows",
        "scheduled-workflow",
        "fkst-cron",
        "",
    ] {
        assert!(!is_reserved_label(label), "{label} must stay available");
    }
}

#[test]
fn the_rejection_names_the_label_and_the_reserved_set() {
    let message = reserved_work_label_rejection(SCHEDULED_WORKFLOW_LABEL)
        .expect("a reserved label is rejected");
    assert!(message.contains(SCHEDULED_WORKFLOW_LABEL), "{message}");
    assert!(message.contains("reserved"), "{message}");
    assert!(
        message.contains(CRON_PAUSED_LABEL),
        "lists the whole reserved set so the author can avoid all of them: {message}"
    );
}

#[test]
fn an_available_label_produces_no_rejection() {
    assert!(reserved_work_label_rejection("fkst-dev").is_none());
}
