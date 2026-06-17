//! Authoritative BSON document shapes for the `sessions` and `leases`
//! collections.
//!
//! Conventions (load-bearing for downstream queries):
//! - `Option<T>` fields serialize as explicit BSON `null` (no
//!   `skip_serializing_if`) so the document shape is stable.
//! - UUIDs are stored as `bson::Uuid` (BSON Binary subtype 4) — a raw
//!   `uuid::Uuid` would serialize as a *string* and silently never match
//!   `find_one({_id})` lookups. Convert to/from `uuid::Uuid` at the edges.
//! - Timestamps are `bson::DateTime` (millisecond UTC, driver-native) so
//!   round-trips are lossless.
//!
//! Re-exports: [`RepoRef`] is shared by both the sessions and goals domains.
//! The canonical definition lives here; `goals/model.rs` re-exports it.

use serde::{Deserialize, Serialize};

/// GitHub repository reference: `owner/name`. Shared by sessions (via
/// [`SessionDoc::repo`]) and goals; re-exported by `goals/model.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

/// Lifecycle state of a session. Serializes lowercase on the wire.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Pending,
    Validating,
    Running,
    Stopping,
    Stopped,
    Failed,
}

/// Why a session reached its terminal state (#180). Distinguishes the three
/// real terminal causes that `SessionStatus` alone collapses: a user-initiated
/// stop (`Terminated`), a graceful engine completion (`Completed`, an
/// uncommanded clean exit 0), and an error (`Failed`). Persisted ONLY on a
/// terminal write; `None` while the session is live. Maps 1:1 onto the goal
/// issue's three terminal labels (`fkst-terminated`/`fkst-completed`/
/// `fkst-failed`). Serializes snake_case on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCause {
    Terminated,
    Completed,
    Failed,
}

/// `sessions` collection: `_id` is a UUID stored as BSON Binary subtype 4.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDoc {
    #[serde(rename = "_id")]
    pub id: bson::Uuid,
    pub package_name: String,
    pub status: SessionStatus,
    pub pod_id: Option<String>,
    pub fencing_token: Option<i64>,
    pub pid: Option<i32>,
    pub runtime_dir: Option<String>,
    pub error: Option<String>,
    /// Logical-run identity stamped by the journaling layer (issue #25);
    /// lowercase sha256 hex. DELIBERATE exception to the explicit-null
    /// convention: the field is OMITTED when absent so documents written
    /// before journaling existed stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_key: Option<String>,
    /// User who owns this session. Omitted for legacy pre-auth docs
    /// (grandfathered open to any authenticated principal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<String>,
    /// Organization this session belongs to (inherited from the package).
    /// Omitted when the session is personal or the package has no org.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    /// Additional package names for multi-package sessions. New inserts
    /// always write >= 1 entry. Legacy docs (field absent) fall back to
    /// [`Self::effective_package_names`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_names: Vec<String>,
    /// Goal this session was spawned from, if any. Classic (non-goal)
    /// sessions leave this `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<bson::Uuid>,
    /// Target GitHub repo, inherited from the goal when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<RepoRef>,
    /// Non-secret pointer to the vault scope this session resolves its env
    /// from (issue #102): `global` for package sessions, the target repo for
    /// goal-triggered ones. Only this REFERENCE is persisted — the resolved
    /// secret values are NEVER written here; the driver re-resolves them from
    /// the vault on every (re)start, so a rotated secret is picked up on
    /// failover. Omitted on legacy docs (the driver then derives the scope
    /// from `repo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_scope: Option<crate::vault::EnvScopeRef>,
    /// Event that triggered this session (e.g. `"goal-trigger"`, `"manual"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triggered_by: Option<String>,
    /// NON-secret id of the per-session NyxID agent key minted for this run
    /// (issue #111), used to revoke the key at teardown. The full key
    /// (`nyxid_ag_…`) is NEVER persisted — it rides the engine `env_profile`
    /// as a `SecretString` only. Omitted when no NyxID token was provisioned
    /// (vault/nyxid disabled, or a pre-#111 document).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nyxid_key_id: Option<String>,
    /// NON-secret short prefix of the minted key for diagnostics only (e.g.
    /// `nyxid_ag_abc`). Never the full key. Omitted with `nyxid_key_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nyxid_key_prefix: Option<String>,
    /// Resolved Ornn skill/skillset pins to inject into the per-session codex
    /// (issue #114): name + concrete version + kind, NON-secret. Persisted so a
    /// failover rebuild re-resolves + re-injects the identical set. Omitted when
    /// the session pinned no skills (the common case) or on a pre-#114 document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ornn_skills: Option<Vec<crate::ornn::OrnnSkillPin>>,
    /// Why this session reached its terminal state (#180): user-stop
    /// (`Terminated`), graceful engine completion (`Completed`), or error
    /// (`Failed`). DELIBERATE exception to the explicit-null convention: the
    /// field is OMITTED while the session is live (and on pre-#180 documents)
    /// so non-terminal docs round-trip byte-identically; it is stamped only on
    /// the terminal CAS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_cause: Option<TerminalCause>,
    pub created_at: bson::DateTime,
    pub started_at: Option<bson::DateTime>,
    pub stopped_at: Option<bson::DateTime>,
}

impl SessionDoc {
    /// Returns the effective set of package names for this session.
    /// Falls back to `[package_name]` when the `package_names` vec is empty
    /// (legacy documents or single-package sessions).
    pub fn effective_package_names(&self) -> Vec<String> {
        if self.package_names.is_empty() {
            vec![self.package_name.clone()]
        } else {
            self.package_names.clone()
        }
    }

    /// Returns the lease key for this session. For goal sessions this is
    /// `"goal-<hyphenated-uuid>"`; for classic sessions it is the
    /// `package_name`. The result always satisfies `is_valid_name` because
    /// the UUID's hex chars and hyphens are in `[A-Za-z0-9_-]+`.
    pub fn lease_key(&self) -> String {
        match self.goal_id {
            Some(goal_id) => format!("goal-{}", goal_id),
            None => self.package_name.clone(),
        }
    }
}

/// `leases` collection: `_id` is the lease key — either a package name (for
/// classic sessions) or `"goal-<uuid>"` (for goal-triggered sessions). At most
/// one live holder per lease key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseDoc {
    #[serde(rename = "_id")]
    pub package_name: String,
    pub session_id: bson::Uuid,
    pub holder_pod: String,
    pub fencing_token: i64,
    pub expires_at: bson::DateTime,
    pub renewed_at: bson::DateTime,
}

#[cfg(test)]
mod tests {
    use bson::spec::BinarySubtype;
    use bson::Bson;

    use super::*;

    fn sample_session() -> SessionDoc {
        SessionDoc {
            id: bson::Uuid::new(),
            package_name: "demo-package".to_string(),
            status: SessionStatus::Running,
            pod_id: Some("pod-0".to_string()),
            fencing_token: Some(42),
            pid: Some(4242),
            runtime_dir: Some("/tmp/run".to_string()),
            error: None,
            run_key: None,
            owner_user_id: None,
            org_id: None,
            package_names: vec![],
            goal_id: None,
            repo: None,
            env_scope: None,
            triggered_by: None,
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            ornn_skills: None,
            terminal_cause: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            started_at: Some(bson::DateTime::from_millis(1_700_000_000_500)),
            stopped_at: None,
        }
    }

    fn sample_lease() -> LeaseDoc {
        LeaseDoc {
            package_name: "demo-package".to_string(),
            session_id: bson::Uuid::new(),
            holder_pod: "pod-0".to_string(),
            fencing_token: 7,
            expires_at: bson::DateTime::from_millis(1_700_000_060_000),
            renewed_at: bson::DateTime::from_millis(1_700_000_030_000),
        }
    }

    #[test]
    fn session_doc_round_trips_losslessly() {
        let doc = sample_session();
        let raw = bson::to_document(&doc).expect("serialize");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn lease_doc_round_trips_losslessly() {
        let doc = sample_lease();
        let raw = bson::to_document(&doc).expect("serialize");
        let back: LeaseDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn session_id_serializes_as_binary_subtype_uuid_not_string() {
        // Regression guard: a string `_id` would silently never match
        // `find_one({_id: <uuid>})` against driver-written Binary data.
        let raw = bson::to_document(&sample_session()).expect("serialize");
        match raw.get("_id").expect("_id present") {
            Bson::Binary(binary) => assert_eq!(binary.subtype, BinarySubtype::Uuid),
            other => panic!("expected Bson::Binary(subtype Uuid), got {other:?}"),
        }
    }

    #[test]
    fn session_status_serializes_lowercase() {
        let cases = [
            (SessionStatus::Pending, "pending"),
            (SessionStatus::Validating, "validating"),
            (SessionStatus::Running, "running"),
            (SessionStatus::Stopping, "stopping"),
            (SessionStatus::Stopped, "stopped"),
            (SessionStatus::Failed, "failed"),
        ];
        for (status, expected) in cases {
            let bson = bson::to_bson(&status).expect("serialize");
            assert_eq!(bson, Bson::String(expected.to_string()));
        }
    }

    #[test]
    fn lease_session_id_serializes_as_binary_subtype_uuid_not_string() {
        // Regression guard for the lease coordination layer: `session_id`
        // must stay Binary subtype 4 on BOTH sides of the `sessions._id`
        // join — a string here would silently never match the driver-written
        // Binary `_id` of `sessions` (and vice versa).
        let raw = bson::to_document(&sample_lease()).expect("serialize");
        match raw.get("session_id").expect("session_id present") {
            Bson::Binary(binary) => assert_eq!(binary.subtype, BinarySubtype::Uuid),
            other => panic!("expected Bson::Binary(subtype Uuid), got {other:?}"),
        }
    }

    #[test]
    fn lease_doc_id_carries_the_package_name() {
        let raw = bson::to_document(&sample_lease()).expect("serialize");
        assert_eq!(
            raw.get("_id").expect("_id present"),
            &Bson::String("demo-package".to_string())
        );
        assert!(
            !raw.contains_key("package_name"),
            "package_name must map onto _id only"
        );
    }

    #[test]
    fn run_key_is_omitted_when_absent_and_round_trips_when_set() {
        // Omitted (not null) when absent: pre-journaling documents and new
        // ones stay byte-identical until the journaler stamps the key.
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(!raw.contains_key("run_key"), "run_key must be omitted");
        // Old documents (no field at all) still deserialize.
        let mut without = raw.clone();
        without.remove("run_key");
        let back: SessionDoc = bson::from_document(without).expect("deserialize");
        assert_eq!(back.run_key, None);

        let mut doc = sample_session();
        doc.run_key = Some("ab".repeat(32));
        let raw = bson::to_document(&doc).expect("serialize");
        assert_eq!(raw.get_str("run_key").expect("run_key"), "ab".repeat(32));
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn terminal_cause_is_omitted_when_absent_and_round_trips_when_set() {
        // Omitted (not null) while live: pre-#180 documents and live ones stay
        // byte-identical until the terminal CAS stamps the cause.
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(
            !raw.contains_key("terminal_cause"),
            "terminal_cause must be omitted while live"
        );
        // Old documents (no field at all) still deserialize.
        let mut without = raw.clone();
        without.remove("terminal_cause");
        let back: SessionDoc = bson::from_document(without).expect("deserialize");
        assert_eq!(back.terminal_cause, None);

        // Each terminal cause round-trips and serializes snake_case.
        for (cause, wire) in [
            (TerminalCause::Terminated, "terminated"),
            (TerminalCause::Completed, "completed"),
            (TerminalCause::Failed, "failed"),
        ] {
            let mut doc = sample_session();
            doc.terminal_cause = Some(cause);
            let raw = bson::to_document(&doc).expect("serialize");
            assert_eq!(raw.get_str("terminal_cause").expect("terminal_cause"), wire);
            let back: SessionDoc = bson::from_document(raw).expect("deserialize");
            assert_eq!(back, doc);
        }
    }

    #[test]
    fn none_fields_serialize_as_explicit_null() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert_eq!(raw.get("error").expect("error present"), &Bson::Null);
        assert_eq!(
            raw.get("stopped_at").expect("stopped_at present"),
            &Bson::Null
        );
    }

    // ---- ownership field serde tests ----

    #[test]
    fn ownership_fields_are_omitted_when_absent() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(
            !raw.contains_key("owner_user_id"),
            "owner_user_id must be omitted when absent"
        );
        assert!(
            !raw.contains_key("org_id"),
            "org_id must be omitted when absent"
        );
    }

    #[test]
    fn ownership_fields_round_trip_when_set() {
        let mut doc = sample_session();
        doc.owner_user_id = Some("user-42".to_string());
        doc.org_id = Some("org-1".to_string());
        let raw = bson::to_document(&doc).expect("serialize");
        assert_eq!(
            raw.get_str("owner_user_id").expect("owner_user_id"),
            "user-42"
        );
        assert_eq!(raw.get_str("org_id").expect("org_id"), "org-1");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn legacy_docs_without_ownership_fields_still_deserialize() {
        let mut raw = bson::to_document(&sample_session()).expect("serialize");
        raw.remove("owner_user_id");
        raw.remove("org_id");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back.owner_user_id, None);
        assert_eq!(back.org_id, None);
    }

    // ---- new session fields (goal integration) serde tests ----

    #[test]
    fn new_fields_are_omitted_when_absent() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(
            !raw.contains_key("package_names"),
            "package_names must be omitted when empty"
        );
        assert!(
            !raw.contains_key("goal_id"),
            "goal_id must be omitted when None"
        );
        assert!(!raw.contains_key("repo"), "repo must be omitted when None");
        assert!(
            !raw.contains_key("triggered_by"),
            "triggered_by must be omitted when None"
        );
    }

    #[test]
    fn new_fields_round_trip_when_set() {
        let mut doc = sample_session();
        doc.package_names = vec!["pkg-a".to_string(), "pkg-b".to_string()];
        doc.goal_id = Some(bson::Uuid::new());
        doc.repo = Some(RepoRef {
            owner: "acme".to_string(),
            name: "my-repo".to_string(),
        });
        doc.triggered_by = Some("goal-trigger".to_string());
        let raw = bson::to_document(&doc).expect("serialize");
        assert_eq!(
            raw.get_array("package_names").expect("package_names").len(),
            2
        );
        match raw.get("goal_id").expect("goal_id present") {
            Bson::Binary(_) => {}
            other => panic!("expected Bson::Binary for goal_id, got {other:?}"),
        }
        match raw.get("repo").expect("repo present") {
            Bson::Document(_) => {}
            other => panic!("expected Bson::Document for repo, got {other:?}"),
        }
        assert_eq!(
            raw.get_str("triggered_by").expect("triggered_by"),
            "goal-trigger"
        );
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn pre_existing_docs_without_new_fields_still_deserialize() {
        let mut raw = bson::to_document(&sample_session()).expect("serialize");
        // Simulate a document written before the new fields existed.
        raw.remove("package_names");
        raw.remove("goal_id");
        raw.remove("repo");
        raw.remove("triggered_by");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back.package_names, Vec::<String>::new());
        assert_eq!(back.goal_id, None);
        assert_eq!(back.repo, None);
        assert_eq!(back.triggered_by, None);
    }

    // ---- env_scope (vault injection pointer, issue #102) serde tests ----

    #[test]
    fn env_scope_is_omitted_when_absent() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(
            !raw.contains_key("env_scope"),
            "env_scope must be omitted when None"
        );
    }

    #[test]
    fn env_scope_round_trips_when_set() {
        let mut doc = sample_session();
        doc.env_scope = Some(crate::vault::EnvScopeRef::repo("acme", "site"));
        let raw = bson::to_document(&doc).expect("serialize");
        match raw.get("env_scope").expect("env_scope present") {
            Bson::Document(_) => {}
            other => panic!("expected Bson::Document for env_scope, got {other:?}"),
        }
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn env_scope_holds_only_a_non_secret_scope_reference() {
        // The persisted pointer must never carry secret material — it is the
        // scope (global / repo) only; the driver re-resolves values per start.
        let mut doc = sample_session();
        doc.env_scope = Some(crate::vault::EnvScopeRef::global());
        let raw = bson::to_document(&doc).expect("serialize");
        let scope = raw.get_document("env_scope").expect("env_scope document");
        assert!(scope.get_bool("global").expect("global flag"));
        assert!(
            !scope.contains_key("value")
                && !scope.contains_key("value_plain")
                && !scope.contains_key("value_enc"),
            "env_scope must not carry any value-bearing field"
        );
    }

    // ---- nyxid session token refs (issue #111) serde tests ----

    #[test]
    fn nyxid_key_refs_are_omitted_when_absent() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(
            !raw.contains_key("nyxid_key_id"),
            "nyxid_key_id must be omitted when None"
        );
        assert!(
            !raw.contains_key("nyxid_key_prefix"),
            "nyxid_key_prefix must be omitted when None"
        );
    }

    #[test]
    fn nyxid_key_refs_round_trip_and_hold_no_full_key() {
        let mut doc = sample_session();
        doc.nyxid_key_id = Some("key-123".to_string());
        // Only a short, non-secret PREFIX is persisted — never the full key.
        doc.nyxid_key_prefix = Some("nyxid_ag_abc".to_string());
        let raw = bson::to_document(&doc).expect("serialize");
        assert_eq!(raw.get_str("nyxid_key_id").expect("id"), "key-123");
        assert_eq!(
            raw.get_str("nyxid_key_prefix").expect("prefix"),
            "nyxid_ag_abc"
        );
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn pre_111_docs_without_nyxid_refs_still_deserialize() {
        let mut raw = bson::to_document(&sample_session()).expect("serialize");
        raw.remove("nyxid_key_id");
        raw.remove("nyxid_key_prefix");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back.nyxid_key_id, None);
        assert_eq!(back.nyxid_key_prefix, None);
    }

    // ---- ornn_skills (pinned skill set, issue #114) serde tests ----

    #[test]
    fn ornn_skills_is_omitted_when_absent() {
        let raw = bson::to_document(&sample_session()).expect("serialize");
        assert!(
            !raw.contains_key("ornn_skills"),
            "ornn_skills must be omitted when None"
        );
    }

    #[test]
    fn ornn_skills_round_trips_and_holds_only_non_secret_pins() {
        let mut doc = sample_session();
        doc.ornn_skills = Some(vec![crate::ornn::OrnnSkillPin {
            kind: crate::ornn::OrnnPinKind::Skillset,
            name: "web-research".to_string(),
            version: "2.0".to_string(),
        }]);
        let raw = bson::to_document(&doc).expect("serialize");
        let pins = raw.get_array("ornn_skills").expect("ornn_skills array");
        assert_eq!(pins.len(), 1);
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn pre_114_docs_without_ornn_skills_still_deserialize() {
        let mut raw = bson::to_document(&sample_session()).expect("serialize");
        raw.remove("ornn_skills");
        let back: SessionDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back.ornn_skills, None);
    }

    #[test]
    fn lease_key_returns_package_name_for_classic_sessions() {
        let doc = sample_session();
        assert_eq!(doc.lease_key(), "demo-package");
    }

    #[test]
    fn lease_key_returns_goal_prefix_for_goal_sessions() {
        let mut doc = sample_session();
        let goal_uuid = bson::Uuid::parse_str("a1b2c3d4-e5f6-7890-abcd-ef1234567890").unwrap();
        doc.goal_id = Some(goal_uuid);
        let key = doc.lease_key();
        assert_eq!(key, "goal-a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    }

    #[test]
    fn lease_key_for_goal_sessions_passes_is_valid_name() {
        let mut doc = sample_session();
        let goal_uuid = bson::Uuid::parse_str("a1b2c3d4-e5f6-7890-abcd-ef1234567890").unwrap();
        doc.goal_id = Some(goal_uuid);
        let key = doc.lease_key();
        // The lease key must satisfy the package-name regex [A-Za-z0-9_-]+
        let re = regex::Regex::new("^[A-Za-z0-9_-]+$").unwrap();
        assert!(
            re.is_match(&key),
            "lease key {:?} must match [A-Za-z0-9_-]+",
            key
        );
    }

    #[test]
    fn effective_package_names_falls_back_to_single_package_name() {
        let doc = sample_session();
        assert_eq!(doc.effective_package_names(), vec!["demo-package"]);
    }

    #[test]
    fn effective_package_names_returns_vec_when_non_empty() {
        let mut doc = sample_session();
        doc.package_names = vec!["pkg-a".to_string(), "pkg-b".to_string()];
        assert_eq!(doc.effective_package_names(), vec!["pkg-a", "pkg-b"]);
    }

    #[test]
    fn repo_ref_round_trips_losslessly() {
        let r = RepoRef {
            owner: "acme".to_string(),
            name: "billing".to_string(),
        };
        let raw = bson::to_document(&r).expect("serialize");
        let back: RepoRef = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, r);
    }
}
