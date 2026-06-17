//! The rolling activity-comment renderer (#139): a SHORT markdown summary of
//! a [`ProgressRecord`], posted on the flush cadence (never per event).
//!
//! Secret/privacy hygiene: it MUST NOT dump full `completed[]` event payloads
//! — only counts, the latest few lifecycle transitions, the current writer,
//! and `updated_at`. The full payloads never leave the machine-truth file.

use crate::model::ProgressRecord;

/// How many of the most-recent lifecycle transitions to surface.
const MAX_TRANSITIONS: usize = 5;

/// Render a compact markdown activity summary for `record`. The body carries
/// counts, the latest few lifecycle transitions (with their `at` timestamps),
/// the current writer, and `updated_at` — never raw completed-event payloads.
pub fn render_activity(record: &ProgressRecord) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "### fkst-hosted progress — `{}`",
        record.package_name
    ));
    lines.push(String::new());
    lines.push(format!(
        "- completed: **{}** | lifecycle: **{}**",
        record.completed.len(),
        record.lifecycle.len()
    ));
    lines.push(format!("- run: `{}`", record.run_key));

    // The current writer is the last one to have contributed.
    if let Some(writer) = record.writers.last() {
        let pod = writer.pod_id.as_deref().unwrap_or("-");
        lines.push(format!(
            "- writer: `{}` (pod `{}`, fencing {})",
            writer.session_id, pod, writer.fencing_token
        ));
    }

    lines.push(format!("- updated: `{}`", record.updated_at));

    if !record.lifecycle.is_empty() {
        lines.push(String::new());
        let total = record.lifecycle.len();
        let start = total.saturating_sub(MAX_TRANSITIONS);
        lines.push(format!(
            "Latest transitions (showing {} of {}):",
            total - start,
            total
        ));
        for entry in &record.lifecycle[start..] {
            lines.push(format!("- `{}` at `{}`", entry.transition, entry.at));
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::{CompletedEntry, LifecycleEntry, WriterEntry};

    fn record() -> ProgressRecord {
        let mut record =
            ProgressRecord::new("rk", "demo", "fp", "2026-06-12T00:00:00Z".to_string());
        // A completed entry whose event payload carries a value that MUST NOT
        // appear in the rendered summary.
        record.completed = vec![CompletedEntry {
            idem_key: "k1".to_string(),
            event: json!({"department": "SECRET_PAYLOAD_VALUE", "name": "leaky"}),
            at: "2026-06-12T00:00:00Z".to_string(),
        }];
        record.lifecycle = vec![
            LifecycleEntry {
                transition: "spawned".to_string(),
                session_id: "s1".to_string(),
                pod_id: Some("pod-0".to_string()),
                fencing_token: 1,
                at: "2026-06-12T00:00:01Z".to_string(),
            },
            LifecycleEntry {
                transition: "running".to_string(),
                session_id: "s1".to_string(),
                pod_id: Some("pod-0".to_string()),
                fencing_token: 1,
                at: "2026-06-12T00:00:02Z".to_string(),
            },
        ];
        record.writers = vec![WriterEntry {
            session_id: "s1".to_string(),
            pod_id: Some("pod-0".to_string()),
            fencing_token: 1,
            first_at: "2026-06-12T00:00:00Z".to_string(),
            last_at: "2026-06-12T00:00:02Z".to_string(),
        }];
        record
    }

    #[test]
    fn activity_body_omits_event_payloads() {
        let body = render_activity(&record());
        // Counts and transitions are present.
        assert!(body.contains("completed: **1**"), "count missing:\n{body}");
        assert!(body.contains("lifecycle: **2**"), "count missing:\n{body}");
        assert!(body.contains("`spawned`"), "transition missing:\n{body}");
        assert!(body.contains("`running`"), "transition missing:\n{body}");
        // The raw completed-event payload must NEVER appear.
        assert!(
            !body.contains("SECRET_PAYLOAD_VALUE"),
            "rendered activity leaked a completed-event payload:\n{body}"
        );
        assert!(
            !body.contains("\"name\":\"leaky\""),
            "rendered activity leaked a completed-event payload:\n{body}"
        );
    }

    #[test]
    fn activity_caps_the_transition_list() {
        let mut record = record();
        record.lifecycle = (0..8)
            .map(|i| LifecycleEntry {
                transition: format!("t{i}"),
                session_id: "s1".to_string(),
                pod_id: None,
                fencing_token: 1,
                at: format!("2026-06-12T00:00:0{i}Z"),
            })
            .collect();
        let body = render_activity(&record);
        // Only the latest MAX_TRANSITIONS appear; the oldest are dropped.
        assert!(
            !body.contains("`t0`"),
            "oldest transition should be dropped"
        );
        assert!(
            !body.contains("`t2`"),
            "oldest transition should be dropped"
        );
        assert!(body.contains("`t7`"), "newest transition must appear");
        assert!(body.contains("showing 5 of 8"));
    }
}
