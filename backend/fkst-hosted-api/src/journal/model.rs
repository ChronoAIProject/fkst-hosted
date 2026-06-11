//! Document shapes for the journaling layer: the Mongo `session_progress`
//! and `run_journals` collections, and the GitHub progress-record file
//! (`fkst-hosted/progress-record@1`).
//!
//! Load-bearing serde conventions:
//! - `SessionProgressDoc.idem_key` / `event_json` are OMITTED (not `null`)
//!   when absent (`skip_serializing_if`): the partial unique index
//!   `sp_run_idem_uniq` filters on `idem_key $exists`, and a stored `null`
//!   would still satisfy `$exists` and wrongly collide all lifecycle docs.
//! - `seq` is stored as `i64` (BSON has no u64); it is a per-session counter
//!   and can never realistically overflow.
//! - `event_json` goes through a dotted-key sanitization pass first: BSON
//!   keys cannot contain `.` or start with `$`, so an envelope violating that
//!   is stored verbatim as the string `event_json_raw` instead of failing
//!   the write. Identity keys are derived BEFORE sanitization.

use serde::{Deserialize, Serialize};

use crate::journal::parse::canonical_json;

/// Mongo collection of per-signal progress documents.
pub const SESSION_PROGRESS_COLLECTION: &str = "session_progress";

/// Mongo collection of per-logical-run journal heads.
pub const RUN_JOURNALS_COLLECTION: &str = "run_journals";

/// Schema tag of the GitHub progress-record file.
pub const PROGRESS_RECORD_SCHEMA: &str = "fkst-hosted/progress-record@1";

/// `last_commit_sha` sentinel: GitHub truth not yet verified by a successful
/// flush (set at start and when bootstrap cannot reach GitHub).
pub const UNVERIFIED_SHA: &str = "unverified";

/// Kind discriminator of a progress document.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProgressKind {
    Raised,
    Lifecycle,
}

/// Log-file reference journaled by a `log_watermark` lifecycle record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogRef {
    /// Path relative to the runtime root (e.g. `logs/framework-child/x.log`).
    pub path: String,
    /// Byte-size watermark at observation time.
    pub size: i64,
    /// Last-modified time of the file.
    pub modified: bson::DateTime,
}

/// `lifecycle` subdocument of a `kind=lifecycle` progress doc. Optional
/// fields are omitted entirely when absent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LifecycleDoc {
    /// `spawned|validating|running|stopping|stopped|failed|malformed_raised|log_watermark`.
    pub transition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_ref: Option<LogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// One observed progress signal (`session_progress` collection).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionProgressDoc {
    /// Progress record id (uuid v4, string).
    #[serde(rename = "_id")]
    pub id: String,
    /// FK -> `sessions._id` (uuid string form).
    pub session_id: String,
    pub package_name: String,
    /// Logical-run identity (lowercase hex, 64 chars).
    pub run_key: String,
    pub kind: ProgressKind,
    /// Per-session monotonic counter (order observed).
    pub seq: i64,
    /// PRESENT ONLY for `kind=raised`; OMITTED for lifecycle (load-bearing
    /// for the partial unique index — see module docs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idem_key: Option<String>,
    /// Decoded RAISED payload, verbatim, as BSON. OMITTED for lifecycle and
    /// when the payload is BSON-unstorable (see `event_json_raw`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_json: Option<bson::Bson>,
    /// Verbatim canonical JSON string fallback when the payload contains
    /// BSON-illegal keys (`.` / leading `$`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_json_raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_json_unstorable: Option<bool>,
    /// Present iff `kind=lifecycle`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleDoc>,
    /// Writer pod (lease `holder_pod`); `null` when unknown (v1 local runs).
    pub pod_id: Option<String>,
    /// Writer's lease fencing token (stale-writer guard); 0 when no lease
    /// exists yet (v1 single-pod posture).
    pub fencing_token: i64,
    pub recorded_at: bson::DateTime,
}

/// GitHub sync head subdocument of a `run_journals` doc.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RunJournalGithub {
    /// Resolved `owner/name`; `null` when GitHub journaling is disabled or
    /// coordinates are unresolved.
    pub repo: Option<String>,
    pub branch: String,
    pub journal_path: String,
    /// Our last committed blob sha; [`UNVERIFIED_SHA`] until the first
    /// successful flush; `null` never (always a string once set).
    pub last_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_comment_id: Option<i64>,
}

/// One logical run's journal head (`run_journals` collection; `_id` is the
/// `run_key`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunJournalDoc {
    #[serde(rename = "_id")]
    pub run_key: String,
    pub package_name: String,
    /// Size hint; the authoritative set lives in `session_progress`.
    pub completed_idem_keys_count: i64,
    pub github: RunJournalGithub,
    /// Highest fencing token that has successfully written GitHub for this
    /// run ("highest seen" semantics — not the per-record writer token).
    pub max_fencing_token: i64,
    pub updated_at: bson::DateTime,
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
/// `.fkst-hosted/journal/<run_key>.json` (the redo source of truth).
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
            updated_at,
        }
    }
}

/// Outcome of the BSON dotted-key sanitization pass over a decoded payload.
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedEvent {
    /// The storable BSON payload, or `None` when unstorable.
    pub event_json: Option<bson::Bson>,
    /// Canonical-JSON string fallback, set iff unstorable.
    pub event_json_raw: Option<String>,
    /// True when the fallback was taken.
    pub unstorable: bool,
}

/// True when any key in `value` (recursively) contains `.` or starts with
/// `$` — illegal as BSON document keys.
fn has_bson_illegal_keys(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(map) => map.iter().any(|(key, child)| {
            key.contains('.') || key.starts_with('$') || has_bson_illegal_keys(child)
        }),
        serde_json::Value::Array(items) => items.iter().any(has_bson_illegal_keys),
        _ => false,
    }
}

/// Sanitize a decoded RAISED payload for BSON storage. A payload with
/// BSON-illegal keys (or one that fails BSON conversion for any other
/// reason) is stored verbatim as canonical JSON in `event_json_raw` instead
/// of failing the write. Identity (`idem_key`) is derived by the caller from
/// the ORIGINAL value before this pass, so identity is unaffected.
pub fn sanitize_event_json(value: &serde_json::Value) -> SanitizedEvent {
    if !has_bson_illegal_keys(value) {
        if let Ok(bson_value) = bson::to_bson(value) {
            return SanitizedEvent {
                event_json: Some(bson_value),
                event_json_raw: None,
                unstorable: false,
            };
        }
    }
    SanitizedEvent {
        event_json: None,
        event_json_raw: Some(canonical_json(value)),
        unstorable: true,
    }
}

#[cfg(test)]
mod tests {
    use bson::Bson;
    use serde_json::json;

    use super::*;

    fn raised_doc() -> SessionProgressDoc {
        SessionProgressDoc {
            id: "11111111-1111-4111-8111-111111111111".to_string(),
            session_id: "22222222-2222-4222-8222-222222222222".to_string(),
            package_name: "demo".to_string(),
            run_key: "ab".repeat(32),
            kind: ProgressKind::Raised,
            seq: 0,
            idem_key: Some("cd".repeat(32)),
            event_json: Some(bson::to_bson(&json!({"department":"d"})).expect("bson")),
            event_json_raw: None,
            event_json_unstorable: None,
            lifecycle: None,
            pod_id: Some("pod-0".to_string()),
            fencing_token: 7,
            recorded_at: bson::DateTime::from_millis(1_700_000_000_000),
        }
    }

    fn lifecycle_doc() -> SessionProgressDoc {
        SessionProgressDoc {
            kind: ProgressKind::Lifecycle,
            idem_key: None,
            event_json: None,
            lifecycle: Some(LifecycleDoc {
                transition: "running".to_string(),
                ..LifecycleDoc::default()
            }),
            ..raised_doc()
        }
    }

    // ---- session_progress shape -------------------------------------------

    #[test]
    fn raised_doc_round_trips_losslessly() {
        let doc = raised_doc();
        let raw = bson::to_document(&doc).expect("serialize");
        let back: SessionProgressDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn lifecycle_doc_omits_idem_key_and_event_json_entirely() {
        // Load-bearing: a stored `idem_key: null` would still satisfy the
        // partial index's `$exists` filter and collide all lifecycle docs.
        let raw = bson::to_document(&lifecycle_doc()).expect("serialize");
        assert!(!raw.contains_key("idem_key"), "idem_key must be omitted");
        assert!(
            !raw.contains_key("event_json"),
            "event_json must be omitted"
        );
        assert!(raw.contains_key("lifecycle"));
    }

    #[test]
    fn raised_doc_omits_the_lifecycle_field() {
        let raw = bson::to_document(&raised_doc()).expect("serialize");
        assert!(!raw.contains_key("lifecycle"));
        assert_eq!(raw.get_str("kind").expect("kind"), "raised");
        assert!(raw.contains_key("idem_key"));
    }

    #[test]
    fn lifecycle_optional_fields_are_omitted_when_absent() {
        let mut doc = lifecycle_doc();
        doc.lifecycle = Some(LifecycleDoc {
            transition: "spawned".to_string(),
            pid: Some(4242),
            ..LifecycleDoc::default()
        });
        let raw = bson::to_document(&doc).expect("serialize");
        let lifecycle = raw.get_document("lifecycle").expect("lifecycle");
        assert_eq!(lifecycle.get_i32("pid").expect("pid"), 4242);
        for absent in ["exit_code", "error", "log_ref", "detail"] {
            assert!(!lifecycle.contains_key(absent), "{absent} must be omitted");
        }
    }

    #[test]
    fn progress_kind_serializes_lowercase() {
        assert_eq!(
            bson::to_bson(&ProgressKind::Raised).expect("bson"),
            Bson::String("raised".to_string())
        );
        assert_eq!(
            bson::to_bson(&ProgressKind::Lifecycle).expect("bson"),
            Bson::String("lifecycle".to_string())
        );
    }

    // ---- run_journals shape --------------------------------------------------

    #[test]
    fn run_journal_doc_round_trips_with_id_as_run_key() {
        let doc = RunJournalDoc {
            run_key: "ef".repeat(32),
            package_name: "demo".to_string(),
            completed_idem_keys_count: 42,
            github: RunJournalGithub {
                repo: Some("owner/name".to_string()),
                branch: "main".to_string(),
                journal_path: format!(".fkst-hosted/journal/{}.json", "ef".repeat(32)),
                last_commit_sha: Some(UNVERIFIED_SHA.to_string()),
                issue_number: Some(123),
                last_comment_id: None,
            },
            max_fencing_token: 7,
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        };
        let raw = bson::to_document(&doc).expect("serialize");
        assert_eq!(
            raw.get_str("_id").expect("_id"),
            doc.run_key.as_str(),
            "_id must carry the run_key"
        );
        let back: RunJournalDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    // ---- progress record (GitHub file) -----------------------------------------

    #[test]
    fn progress_record_serializes_with_the_schema_tag() {
        let record = ProgressRecord::new("rk", "demo", "fp", "2026-06-10T00:00:00Z".to_string());
        let value = serde_json::to_value(&record).expect("json");
        assert_eq!(value["schema"], PROGRESS_RECORD_SCHEMA);
        assert_eq!(value["completed"], json!([]));
        let back: ProgressRecord = serde_json::from_value(value).expect("round trip");
        assert_eq!(back, record);
    }

    // ---- sanitize_event_json ------------------------------------------------------

    #[test]
    fn clean_payload_is_stored_as_bson() {
        let value = json!({"department":"d","nested":{"k":[1,2]}});
        let out = sanitize_event_json(&value);
        assert!(!out.unstorable);
        assert!(out.event_json.is_some());
        assert!(out.event_json_raw.is_none());
    }

    #[test]
    fn dotted_key_payload_falls_back_to_raw_string() {
        let value = json!({"bad.key": 1, "ok": 2});
        let out = sanitize_event_json(&value);
        assert!(out.unstorable);
        assert!(out.event_json.is_none());
        assert_eq!(
            out.event_json_raw.as_deref(),
            Some(r#"{"bad.key":1,"ok":2}"#)
        );
    }

    #[test]
    fn dollar_prefixed_key_falls_back_even_when_nested() {
        let value = json!({"outer": {"inner": [{"$op": true}]}});
        let out = sanitize_event_json(&value);
        assert!(out.unstorable);
        assert!(out.event_json_raw.is_some());
    }
}
