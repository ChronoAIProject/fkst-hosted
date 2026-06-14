//! Goal domain models, size-limit constants, and validation.
//!
//! A *goal* captures the user's intent (a prompt the engine will eventually
//! work on), the set of fkst packages to run it with, and an optional target
//! GitHub repo. The full [`GoalStatus`] lifecycle is defined now; only
//! `NotStarted` is written in this issue — the trigger issue adds writers.
//!
//! Conventions (load-bearing for downstream queries):
//! - `_id` is a `bson::Uuid` (Binary subtype 4) — matches the sessions
//!   collection convention and is query-safe for `find_one({_id})`.
//! - `Option<T>` fields serialize as explicit BSON `null` (no
//!   `skip_serializing_if`) — the explicit-null convention for new
//!   collections.
//! - Timestamps are `bson::DateTime` (millisecond UTC).

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// MongoDB collection holding goal documents.
pub const GOALS_COLLECTION: &str = "goals";

/// Maximum number of characters in a goal title (inclusive).
pub const MAX_GOAL_TITLE_CHARS: usize = 200;
/// Maximum byte size of the description field (inclusive; 16 KiB). The
/// description becomes the session's `goal.json` content. **Content is NEVER
/// logged** — only byte length.
pub const MAX_GOAL_DESCRIPTION_BYTES: usize = 16_384;
/// Maximum number of package names per goal (inclusive).
pub const MAX_GOAL_PACKAGES: usize = 16;
/// Maximum byte length of a single package name (matches
/// `MAX_PACKAGE_NAME_BYTES` in sessions).
pub const MAX_PACKAGE_NAME_BYTES: usize = 128;

/// Full lifecycle of a goal. All five variants are defined NOW; only
/// `NotStarted` is written in this issue. The trigger issue adds the
/// transitions without schema changes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    NotStarted,
    Triggered,
    Running,
    Stopped,
    Failed,
}

/// GitHub repository reference: `owner/name`.
/// Canonical definition is in [`crate::models::RepoRef`]; re-exported here
/// for backward compatibility.
pub use crate::models::RepoRef;

/// `goals` collection document: `_id` is a UUID stored as BSON Binary subtype 4.
///
/// `active_session_id` is reserved for the trigger issue (one active session
/// per goal); always `null` in this issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalDoc {
    #[serde(rename = "_id")]
    pub id: bson::Uuid,
    pub title: String,
    /// The engine-facing goal prompt; **content NEVER logged**.
    pub description: String,
    /// 1..=16 validated package names.
    pub package_names: Vec<String>,
    /// Optional target GitHub repo (explicit null).
    pub repo: Option<RepoRef>,
    pub status: GoalStatus,
    pub owner_user_id: String,
    /// Organization this goal belongs to (explicit null for personal goals).
    pub org_id: Option<String>,
    /// Reserved: written by the trigger issue (one active session per goal).
    pub active_session_id: Option<bson::Uuid>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
}

/// Anchored package-name pattern (same as `packages::is_valid_name`).
fn package_name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_-]+$").expect("static package name regex"))
}

/// Anchored GitHub owner pattern: `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$`.
fn repo_owner_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})$").expect("static repo owner regex")
    })
}

/// Anchored GitHub repo name pattern: `^[A-Za-z0-9._-]{1,100}$`.
fn repo_name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z0-9._-]{1,100}$").expect("static repo name regex"))
}

/// Pure validation of goal fields (no I/O). Checks title, description,
/// package_names format/count, and repo format. Package existence and
/// share-level checks are I/O and done at the handler level.
///
/// Returns `Err(String)` with a stable, ordered prefix for the first
/// violation, matching the `NewPackage::validate` precedent.
pub fn validate_goal_fields(
    title: &str,
    description: &str,
    package_names: &[String],
    repo: Option<&RepoRef>,
) -> Result<(), String> {
    // 1. Title: trimmed non-empty, <= 200 chars.
    let trimmed_title = title.trim();
    if trimmed_title.is_empty() {
        return Err("empty title".to_string());
    }
    if trimmed_title.len() > MAX_GOAL_TITLE_CHARS {
        return Err(format!(
            "title too long: {} chars exceeds {MAX_GOAL_TITLE_CHARS}",
            trimmed_title.len()
        ));
    }

    // 2. Description: non-empty, <= 16384 bytes.
    if description.is_empty() {
        return Err("empty description".to_string());
    }
    if description.len() > MAX_GOAL_DESCRIPTION_BYTES {
        return Err(format!(
            "description too large: {} bytes exceeds {MAX_GOAL_DESCRIPTION_BYTES}",
            description.len()
        ));
    }

    // 3. Package names: 1..=16; each matches ^[A-Za-z0-9_-]+$ and <=128 bytes;
    //    no duplicates.
    if package_names.is_empty() {
        return Err("at least one package is required".to_string());
    }
    if package_names.len() > MAX_GOAL_PACKAGES {
        return Err(format!(
            "too many packages: {} exceeds {MAX_GOAL_PACKAGES}",
            package_names.len()
        ));
    }
    let mut seen = HashSet::new();
    for name in package_names {
        if name.is_empty() {
            return Err("empty package name".to_string());
        }
        if name.len() > MAX_PACKAGE_NAME_BYTES {
            return Err(format!(
                "package name too long: {:?} exceeds {MAX_PACKAGE_NAME_BYTES} bytes",
                name
            ));
        }
        if !package_name_regex().is_match(name) {
            return Err(format!(
                "invalid package name: {:?} must fully match [A-Za-z0-9_-]+",
                name
            ));
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(format!("duplicate package name: {:?}", name));
        }
    }

    // 4. Repo validation (when present).
    if let Some(repo) = repo {
        if !repo_owner_regex().is_match(&repo.owner) {
            return Err(format!(
                "invalid repo owner: {:?} must match [A-Za-z0-9](?:[A-Za-z0-9-]{{0,38}})",
                repo.owner
            ));
        }
        if !repo_name_regex().is_match(&repo.name) {
            return Err(format!(
                "invalid repo name: {:?} must match [A-Za-z0-9._-]{{1,100}}",
                repo.name
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use bson::Bson;

    use super::*;

    fn sample_goal_doc() -> GoalDoc {
        GoalDoc {
            id: bson::Uuid::new(),
            title: "Build a billing pipeline".to_string(),
            description: "Create a billing pipeline that processes invoices.".to_string(),
            package_names: vec!["billing-core".to_string()],
            repo: None,
            status: GoalStatus::NotStarted,
            owner_user_id: "user-42".to_string(),
            org_id: None,
            active_session_id: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        }
    }

    // ---- serde tests ----

    #[test]
    fn goal_doc_round_trips_losslessly() {
        let doc = sample_goal_doc();
        let raw = bson::to_document(&doc).expect("serialize");
        let back: GoalDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    #[test]
    fn goal_status_serializes_snake_case() {
        let cases = [
            (GoalStatus::NotStarted, "not_started"),
            (GoalStatus::Triggered, "triggered"),
            (GoalStatus::Running, "running"),
            (GoalStatus::Stopped, "stopped"),
            (GoalStatus::Failed, "failed"),
        ];
        for (status, expected) in cases {
            let bson = bson::to_bson(&status).expect("serialize");
            assert_eq!(bson, Bson::String(expected.to_string()));
        }
    }

    #[test]
    fn explicit_null_fields_serialize_as_null() {
        let raw = bson::to_document(&sample_goal_doc()).expect("serialize");
        assert_eq!(raw.get("repo").expect("repo present"), &Bson::Null);
        assert_eq!(raw.get("org_id").expect("org_id present"), &Bson::Null);
        assert_eq!(
            raw.get("active_session_id")
                .expect("active_session_id present"),
            &Bson::Null
        );
    }

    #[test]
    fn set_fields_round_trip() {
        let mut doc = sample_goal_doc();
        doc.repo = Some(RepoRef {
            owner: "acme".to_string(),
            name: "billing".to_string(),
        });
        doc.org_id = Some("org-1".to_string());
        doc.active_session_id = Some(bson::Uuid::new());
        doc.status = GoalStatus::Running;
        let raw = bson::to_document(&doc).expect("serialize");
        let back: GoalDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, doc);
    }

    // ---- validation tests ----

    #[test]
    fn validate_accepts_minimal_goal() {
        assert_eq!(
            validate_goal_fields("My Goal", "Do the thing", &["pkg".to_string()], None,),
            Ok(())
        );
    }

    #[test]
    fn validate_accepts_goal_with_repo() {
        assert_eq!(
            validate_goal_fields(
                "My Goal",
                "Do the thing",
                &["pkg".to_string()],
                Some(&RepoRef {
                    owner: "acme".to_string(),
                    name: "my-repo".to_string(),
                }),
            ),
            Ok(())
        );
    }

    #[test]
    #[allow(clippy::type_complexity)]
    fn validate_rejects_each_violation_with_stable_prefix() {
        let long_title = "x".repeat(MAX_GOAL_TITLE_CHARS + 1);
        let long_desc = "x".repeat(MAX_GOAL_DESCRIPTION_BYTES + 1);
        let cases: Vec<(&str, &str, Vec<String>, Option<RepoRef>, &str)> = vec![
            // 1. Title
            ("", "desc", vec!["p".to_string()], None, "empty title"),
            (" ", "desc", vec!["p".to_string()], None, "empty title"),
            (
                &long_title,
                "desc",
                vec!["p".to_string()],
                None,
                "title too long",
            ),
            // 2. Description
            (
                "title",
                "",
                vec!["p".to_string()],
                None,
                "empty description",
            ),
            (
                "title",
                &long_desc,
                vec!["p".to_string()],
                None,
                "description too large",
            ),
            // 3. Package names
            (
                "title",
                "desc",
                vec![],
                None,
                "at least one package is required",
            ),
            (
                "title",
                "desc",
                (0..=MAX_GOAL_PACKAGES).map(|i| format!("p{i}")).collect(),
                None,
                "too many packages",
            ),
            (
                "title",
                "desc",
                vec!["p".to_string(), "p".to_string()],
                None,
                "duplicate package name",
            ),
            (
                "title",
                "desc",
                vec!["bad name!".to_string()],
                None,
                "invalid package name",
            ),
            // 4. Repo
            (
                "title",
                "desc",
                vec!["p".to_string()],
                Some(RepoRef {
                    owner: "-bad".to_string(),
                    name: "ok".to_string(),
                }),
                "invalid repo owner",
            ),
            (
                "title",
                "desc",
                vec!["p".to_string()],
                Some(RepoRef {
                    owner: "acme".to_string(),
                    name: "".to_string(),
                }),
                "invalid repo name",
            ),
        ];

        for (i, (title, description, package_names, repo, expected_prefix)) in
            cases.into_iter().enumerate()
        {
            let err = validate_goal_fields(title, description, &package_names, repo.as_ref())
                .expect_err(&format!("case {i}: must be rejected"));
            assert!(
                err.starts_with(expected_prefix),
                "case {i}: expected prefix {expected_prefix:?}, got {err:?}"
            );
        }
    }
}
