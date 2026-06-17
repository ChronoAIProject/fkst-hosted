//! The pure, replay-idempotent CAS merge applied to the remote progress
//! record, plus the identity projection committed to git and the display-only
//! timestamp helper.

use std::collections::HashMap;

use crate::model::{CompletedEntry, LifecycleEntry, ProgressRecord, WriterEntry, UNVERIFIED_SHA};

/// The writer's current run-head pointers, folded through [`merge_record`] so
/// they survive the CAS-merge inside the committed file (#139). A non-None
/// value from the writer wins over the base; otherwise the base's value is
/// preserved (a cold worker keeps the pointer it never learned).
#[derive(Debug, Clone, Copy, Default)]
pub struct HeadPointers {
    pub issue_number: Option<i64>,
    pub last_comment_id: Option<i64>,
}

/// Merge newly-observed completions/lifecycle/writer into the (possibly
/// absent) remote record. Pure and replay-idempotent:
/// - `completed[]` dedupes by `idem_key`, keeping the EARLIEST `at`;
/// - `lifecycle[]` appends only entries not already present (exact match);
/// - `writers[]` merges per `session_id` (min `first_at`, max `last_at`);
/// - the run-head pointers (`issue_number`/`last_comment_id`) survive from the
///   base unless this writer has a newer non-None value (writer wins);
/// - `completed_count` mirrors `completed.len()`; `last_commit_sha` is set to
///   [`UNVERIFIED_SHA`] in memory (the real sha is only known post-PUT and is
///   not needed inside the file for CAS).
#[allow(clippy::too_many_arguments)]
pub fn merge_record(
    base: Option<&ProgressRecord>,
    run_key: &str,
    package_name: &str,
    package_fingerprint: &str,
    new_completed: &[CompletedEntry],
    new_lifecycle: &[LifecycleEntry],
    writer: Option<&WriterEntry>,
    pointers: HeadPointers,
    max_fencing_token: i64,
    updated_at: String,
) -> ProgressRecord {
    let mut record = base.cloned().unwrap_or_else(|| {
        ProgressRecord::new(run_key, package_name, package_fingerprint, String::new())
    });

    let mut index_by_key: HashMap<String, usize> = record
        .completed
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.idem_key.clone(), index))
        .collect();
    for entry in new_completed {
        match index_by_key.get(&entry.idem_key) {
            Some(&index) => {
                // Same RFC3339 format: lexicographic order == time order.
                if entry.at < record.completed[index].at {
                    record.completed[index].at = entry.at.clone();
                }
            }
            None => {
                index_by_key.insert(entry.idem_key.clone(), record.completed.len());
                record.completed.push(entry.clone());
            }
        }
    }

    for entry in new_lifecycle {
        if !record.lifecycle.contains(entry) {
            record.lifecycle.push(entry.clone());
        }
    }

    if let Some(writer) = writer {
        match record
            .writers
            .iter_mut()
            .find(|existing| existing.session_id == writer.session_id)
        {
            Some(existing) => {
                if writer.first_at < existing.first_at {
                    existing.first_at = writer.first_at.clone();
                }
                if writer.last_at > existing.last_at {
                    existing.last_at = writer.last_at.clone();
                }
                existing.fencing_token = existing.fencing_token.max(writer.fencing_token);
                if existing.pod_id.is_none() {
                    existing.pod_id = writer.pod_id.clone();
                }
            }
            None => record.writers.push(writer.clone()),
        }
    }

    // Run-head pointers: the writer's non-None value wins; otherwise keep the
    // base's. A cold worker that never learned a pointer carries None and so
    // preserves whatever the committed file already knows.
    if pointers.issue_number.is_some() {
        record.issue_number = pointers.issue_number;
    }
    if pointers.last_comment_id.is_some() {
        record.last_comment_id = pointers.last_comment_id;
    }

    record.max_fencing_token = record.max_fencing_token.max(max_fencing_token);
    record.completed_count = record.completed.len() as i64;
    // The real blob sha is only known after the PUT; inside the file the
    // sentinel suffices (CAS uses the GitHub `sha` header, not this field).
    record.last_commit_sha = Some(UNVERIFIED_SHA.to_string());
    record.updated_at = updated_at;
    record
}

/// The minimal identity projection committed to git in `completed[].event`
/// (never the full payload). Pointer `/a/b` becomes key `"a/b"`; absent or
/// null pointers map to JSON `null`.
pub(crate) fn identity_projection(
    event_json: &serde_json::Value,
    pointers: &[String],
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for pointer in pointers {
        let key = pointer.trim_start_matches('/').to_string();
        let value = event_json
            .pointer(pointer)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        map.insert(key, value);
    }
    serde_json::Value::Object(map)
}

/// Current time as an RFC3339 string (display only — never load-bearing).
pub(crate) fn now_rfc3339() -> String {
    bson::DateTime::now()
        .try_to_rfc3339_string()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completed(idem: &str, at: &str) -> CompletedEntry {
        CompletedEntry {
            idem_key: idem.to_string(),
            event: serde_json::json!({"department": "d"}),
            at: at.to_string(),
        }
    }

    fn merged_with(
        base: Option<&ProgressRecord>,
        new_completed: &[CompletedEntry],
        token: i64,
    ) -> ProgressRecord {
        merge_record(
            base,
            "rk",
            "demo",
            "fp",
            new_completed,
            &[],
            None,
            HeadPointers::default(),
            token,
            "2026-06-11T00:00:00Z".to_string(),
        )
    }

    #[test]
    fn merge_preserves_existing_dedupes_and_keeps_the_earliest_at() {
        let mut base = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        base.completed = vec![completed("k1", "2026-06-10T00:00:00Z")];
        base.max_fencing_token = 2;

        let merged = merged_with(
            Some(&base),
            &[
                completed("k1", "2026-06-09T00:00:00Z"), // earlier observation wins
                completed("k2", "2026-06-11T00:00:00Z"),
            ],
            2,
        );
        assert_eq!(merged.completed.len(), 2);
        assert_eq!(merged.completed[0].idem_key, "k1");
        assert_eq!(merged.completed[0].at, "2026-06-09T00:00:00Z");
        assert_eq!(merged.completed[1].idem_key, "k2");
    }

    #[test]
    fn merge_is_idempotent_on_replay() {
        let new = vec![completed("k1", "t1"), completed("k2", "t2")];
        let once = merged_with(None, &new, 1);
        let twice = merged_with(Some(&once), &new, 1);
        assert_eq!(once.completed, twice.completed);
        assert_eq!(once.lifecycle, twice.lifecycle);
        assert_eq!(once.max_fencing_token, twice.max_fencing_token);
    }

    #[test]
    fn merge_appends_lifecycle_without_duplicates_and_merges_writers() {
        let entry = LifecycleEntry {
            transition: "running".to_string(),
            session_id: "s1".to_string(),
            pod_id: Some("pod-0".to_string()),
            fencing_token: 1,
            at: "t1".to_string(),
        };
        let writer = WriterEntry {
            session_id: "s1".to_string(),
            pod_id: Some("pod-0".to_string()),
            fencing_token: 1,
            first_at: "t1".to_string(),
            last_at: "t2".to_string(),
        };
        let first = merge_record(
            None,
            "rk",
            "demo",
            "fp",
            &[],
            std::slice::from_ref(&entry),
            Some(&writer),
            HeadPointers::default(),
            1,
            "t2".to_string(),
        );
        // Replay the same lifecycle + a widened writer window.
        let wider = WriterEntry {
            first_at: "t0".to_string(),
            last_at: "t3".to_string(),
            fencing_token: 2,
            ..writer.clone()
        };
        let second = merge_record(
            Some(&first),
            "rk",
            "demo",
            "fp",
            &[],
            std::slice::from_ref(&entry),
            Some(&wider),
            HeadPointers::default(),
            2,
            "t3".to_string(),
        );
        assert_eq!(second.lifecycle.len(), 1, "exact replay deduped");
        assert_eq!(second.writers.len(), 1, "same session merges");
        assert_eq!(second.writers[0].first_at, "t0");
        assert_eq!(second.writers[0].last_at, "t3");
        assert_eq!(second.writers[0].fencing_token, 2);
        assert_eq!(second.max_fencing_token, 2);
    }

    #[test]
    fn merge_takes_the_max_fencing_token() {
        let mut base = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        base.max_fencing_token = 7;
        assert_eq!(merged_with(Some(&base), &[], 3).max_fencing_token, 7);
        assert_eq!(merged_with(Some(&base), &[], 9).max_fencing_token, 9);
    }

    #[test]
    fn merge_record_preserves_issue_pointers() {
        // Base carries pointers; a writer with NONE must preserve them (a cold
        // worker that never learned the issue/comment keeps the file's value).
        let mut base = ProgressRecord::new("rk", "demo", "fp", "t0".to_string());
        base.issue_number = Some(11);
        base.last_comment_id = Some(99);
        let preserved = merge_record(
            Some(&base),
            "rk",
            "demo",
            "fp",
            &[],
            &[],
            None,
            HeadPointers::default(),
            0,
            "t1".to_string(),
        );
        assert_eq!(preserved.issue_number, Some(11), "base issue survives");
        assert_eq!(preserved.last_comment_id, Some(99), "base comment survives");

        // The writer's non-None pointers win over the base.
        let won = merge_record(
            Some(&base),
            "rk",
            "demo",
            "fp",
            &[],
            &[],
            None,
            HeadPointers {
                issue_number: Some(11),
                last_comment_id: Some(123),
            },
            0,
            "t2".to_string(),
        );
        assert_eq!(won.issue_number, Some(11));
        assert_eq!(won.last_comment_id, Some(123), "writer's comment id wins");

        // completed_count mirrors the merged completed length; the in-file sha
        // is the sentinel regardless of input.
        let mut with_completed = base.clone();
        with_completed.completed = vec![completed("k1", "t1")];
        let counted = merge_record(
            Some(&with_completed),
            "rk",
            "demo",
            "fp",
            &[completed("k2", "t2")],
            &[],
            None,
            HeadPointers::default(),
            0,
            "t3".to_string(),
        );
        assert_eq!(counted.completed_count, 2);
        assert_eq!(counted.last_commit_sha.as_deref(), Some(UNVERIFIED_SHA));
    }
}
