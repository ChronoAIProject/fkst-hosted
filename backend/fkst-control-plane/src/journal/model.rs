//! Document shapes for the journaling layer: the GitHub progress-record file
//! (`fkst-hosted/progress-record@1`) — the SOLE machine-truth for a logical
//! run since the Mongo journaling collections were removed (#139).
//!
//! Load-bearing serde conventions:
//! - The progress record's run-head pointers (`issue_number`,
//!   `last_comment_id`, `last_commit_sha`, `completed_count`) are additive,
//!   `#[serde(default)]` fields. A pre-pointer file (written before #139)
//!   deserializes with defaults, so the schema tag stays `@1` (the optional
//!   fields are backward compatible — never bump to `@2`).
//! - `completed[].event` carries only the minimal identity projection, never
//!   the full payload (see `merge::identity_projection`).

use serde::{Deserialize, Serialize};

/// Schema tag of the GitHub progress-record file.
pub const PROGRESS_RECORD_SCHEMA: &str = "fkst-hosted/progress-record@1";

/// `last_commit_sha` sentinel: GitHub truth not yet verified by a successful
/// flush (set at start and when bootstrap cannot reach GitHub).
pub const UNVERIFIED_SHA: &str = "unverified";

/// Log-file reference journaled by a `log_watermark` lifecycle observation.
///
/// Still LIVE: it is the payload of `Transition::LogWatermark(LogRef)` and is
/// built by `sessions/service.rs::newest_child_log()`. Since the Mongo
/// lifecycle document was removed (#139) its data is no longer persisted
/// anywhere, but the type stays for the live in-process signal path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogRef {
    /// Path relative to the runtime root (e.g. `logs/framework-child/x.log`).
    pub path: String,
    /// Byte-size watermark at observation time.
    pub size: i64,
    /// Last-modified time of the file.
    pub modified: bson::DateTime,
}

// ---------------------------------------------------------------------------
// GitHub progress-record file (plain serde_json; committed to git).
// ---------------------------------------------------------------------------

/// One completed raised event in the GitHub record. `event` carries only the
/// minimal identity projection — never the full payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletedEntry {
    pub idem_key: String,
    pub event: serde_json::Value,
    /// RFC3339 earliest observation time (display only; never load-bearing).
    pub at: String,
}

/// One lifecycle transition in the GitHub record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleEntry {
    pub transition: String,
    pub session_id: String,
    pub pod_id: Option<String>,
    pub fencing_token: i64,
    pub at: String,
}

/// One writer (session) that contributed to the GitHub record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterEntry {
    pub session_id: String,
    pub pod_id: Option<String>,
    pub fencing_token: i64,
    pub first_at: String,
    pub last_at: String,
}

/// The per-logical-run progress record committed to
/// `.fkst-hosted/journal/<run_key>.json` — the SOLE machine-truth and the
/// redo source of truth (#139). The run-head pointers (`issue_number`,
/// `last_comment_id`, `last_commit_sha`, `completed_count`) used to live in
/// the removed Mongo journal-head collection; they now travel inside this file
/// so a cold worker recovers them with no datastore.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProgressRecord {
    pub schema: String,
    pub run_key: String,
    pub package_name: String,
    pub package_fingerprint: String,
    pub completed: Vec<CompletedEntry>,
    pub lifecycle: Vec<LifecycleEntry>,
    pub writers: Vec<WriterEntry>,
    pub max_fencing_token: i64,
    /// Issue this run mirrors its activity comment onto; `None` until known.
    #[serde(default)]
    pub issue_number: Option<i64>,
    /// Rolling activity comment id (`upsert_issue_comment`); `None` until the
    /// first comment is created.
    #[serde(default)]
    pub last_comment_id: Option<i64>,
    /// Last committed blob sha; [`UNVERIFIED_SHA`] inside the file (the real
    /// sha is only known post-PUT and is not needed for CAS).
    #[serde(default)]
    pub last_commit_sha: Option<String>,
    /// Size hint mirroring `completed.len()` (cheap to read without scanning).
    #[serde(default)]
    pub completed_count: i64,
    pub updated_at: String,
}

impl ProgressRecord {
    /// Fresh, empty record for a logical run.
    pub fn new(
        run_key: &str,
        package_name: &str,
        package_fingerprint: &str,
        updated_at: String,
    ) -> Self {
        Self {
            schema: PROGRESS_RECORD_SCHEMA.to_string(),
            run_key: run_key.to_string(),
            package_name: package_name.to_string(),
            package_fingerprint: package_fingerprint.to_string(),
            completed: Vec::new(),
            lifecycle: Vec::new(),
            writers: Vec::new(),
            max_fencing_token: 0,
            issue_number: None,
            last_comment_id: None,
            last_commit_sha: None,
            completed_count: 0,
            updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn completed(idem: &str) -> CompletedEntry {
        CompletedEntry {
            idem_key: idem.to_string(),
            event: json!({"department": "d"}),
            at: "2026-06-11T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn progress_record_serializes_with_the_schema_tag() {
        let record = ProgressRecord::new("rk", "demo", "fp", "2026-06-10T00:00:00Z".to_string());
        let value = serde_json::to_value(&record).expect("json");
        assert_eq!(value["schema"], PROGRESS_RECORD_SCHEMA);
        assert_eq!(value["completed"], json!([]));
        // The additive run-head pointers serialize at their defaults.
        assert_eq!(value["issue_number"], json!(null));
        assert_eq!(value["last_comment_id"], json!(null));
        assert_eq!(value["last_commit_sha"], json!(null));
        assert_eq!(value["completed_count"], json!(0));
        let back: ProgressRecord = serde_json::from_value(value).expect("round trip");
        assert_eq!(back, record);
    }

    #[test]
    fn progress_record_round_trips_with_pointers() {
        let mut record =
            ProgressRecord::new("rk", "demo", "fp", "2026-06-10T00:00:00Z".to_string());
        record.completed = vec![completed("k1"), completed("k2")];
        record.completed_count = 2;
        record.issue_number = Some(42);
        record.last_comment_id = Some(7);
        record.last_commit_sha = Some(UNVERIFIED_SHA.to_string());
        record.max_fencing_token = 5;

        let value = serde_json::to_value(&record).expect("json");
        let back: ProgressRecord = serde_json::from_value(value).expect("round trip");
        assert_eq!(back, record);
        assert_eq!(back.issue_number, Some(42));
        assert_eq!(back.last_comment_id, Some(7));
        assert_eq!(back.last_commit_sha.as_deref(), Some(UNVERIFIED_SHA));
        assert_eq!(back.completed_count, 2);
    }

    #[test]
    fn progress_record_pre_pointer_file_deserializes() {
        // A file written before #139 has NONE of the run-head pointer fields;
        // the additive `#[serde(default)]` fields must fill in defaults so the
        // schema tag can stay `@1` (backward compatible).
        let pre_pointer = json!({
            "schema": PROGRESS_RECORD_SCHEMA,
            "run_key": "rk",
            "package_name": "demo",
            "package_fingerprint": "fp",
            "completed": [],
            "lifecycle": [],
            "writers": [],
            "max_fencing_token": 3,
            "updated_at": "2026-06-10T00:00:00Z"
        });
        let record: ProgressRecord =
            serde_json::from_value(pre_pointer).expect("pre-pointer file deserializes");
        assert_eq!(record.issue_number, None);
        assert_eq!(record.last_comment_id, None);
        assert_eq!(record.last_commit_sha, None);
        assert_eq!(record.completed_count, 0);
        assert_eq!(record.max_fencing_token, 3);
    }
}
