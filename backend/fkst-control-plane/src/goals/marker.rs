//! The hidden HTML-comment marker block the server writes into (and reads back
//! from) a goal's GitHub Issue (#137). Pure: no I/O.
//!
//! The marker is the SERVER-CONTROLLED source of truth for a goal's
//! owner/org/packages/repo when reading an issue back — surrounding issue-body
//! prose is ignored, and parsed `package_names`/`repo` are re-validated before
//! they are trusted (a body could have been hand-edited). It carries NO secret:
//! the engine prompt is NEVER written to GitHub (it lives in controller memory).
//!
//! Format (single block, machine-parseable, human-ignorable):
//! ```text
//! <!-- fkst-hosted:goal
//! {"v":1,"goal_id":"<uuid>","owner_user_id":"<id>","org_id":null,"package_names":["a"],"repo":null}
//! -->
//! ```

use serde::{Deserialize, Serialize};

use crate::goals::model::{validate_goal_fields, GoalDoc, RepoRef};

/// The token identifying the marker comment.
pub const MARKER_PREFIX: &str = "fkst-hosted:goal";

/// The parsed marker. `goal_id` is the string UUID; `repo`/`package_names` are
/// re-validated by [`parse_marker`] before they are returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalMarker {
    pub v: u8,
    pub goal_id: String,
    pub owner_user_id: String,
    pub org_id: Option<String>,
    pub package_names: Vec<String>,
    pub repo: Option<RepoRef>,
}

/// Errors parsing the marker out of an issue body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarkerError {
    #[error("no fkst-hosted:goal marker in the issue body")]
    Missing,
    #[error("malformed fkst-hosted:goal marker: {0}")]
    Malformed(String),
    #[error("unsupported fkst-hosted:goal marker version {0}")]
    UnsupportedVersion(u8),
}

/// Render the hidden marker comment for `doc` (compact JSON on one line).
pub fn render_marker(doc: &GoalDoc) -> String {
    let marker = GoalMarker {
        v: 1,
        goal_id: doc.id.to_string(),
        owner_user_id: doc.owner_user_id.clone(),
        org_id: doc.org_id.clone(),
        package_names: doc.package_names.clone(),
        repo: doc.repo.clone(),
    };
    let json = serde_json::to_string(&marker).expect("GoalMarker serializes");
    format!("<!-- {MARKER_PREFIX}\n{json}\n-->")
}

/// Locate the marker comment, extract + deserialize its JSON, reject `v != 1`,
/// and re-validate `package_names`/`repo` (a body could have been hand-edited).
pub fn parse_marker(issue_body: &str) -> Result<GoalMarker, MarkerError> {
    let start = match issue_body.find(MARKER_PREFIX) {
        Some(s) => s + MARKER_PREFIX.len(),
        None => return Err(MarkerError::Missing),
    };
    let after = &issue_body[start..];
    // The JSON sits between the prefix line and the comment terminator.
    let end = match after.find("-->") {
        Some(e) => e,
        // Prefix present but the comment is unterminated -> treat as absent.
        None => return Err(MarkerError::Missing),
    };
    let json = after[..end].trim();
    let marker: GoalMarker =
        serde_json::from_str(json).map_err(|e| MarkerError::Malformed(e.to_string()))?;
    if marker.v != 1 {
        return Err(MarkerError::UnsupportedVersion(marker.v));
    }
    // Re-validate the server-owned fields (title/description are dummy here — we
    // only need the package + repo checks; the prompt is never in the marker).
    validate_goal_fields("x", "x", &marker.package_names, marker.repo.as_ref())
        .map_err(MarkerError::Malformed)?;
    Ok(marker)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::model::GoalStatus;

    fn goal() -> GoalDoc {
        GoalDoc {
            id: bson::Uuid::new(),
            title: "Build it".to_string(),
            description: "SECRET-PROMPT".to_string(),
            package_names: vec!["pkg-a".to_string(), "pkg-b".to_string()],
            repo: Some(RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            }),
            status: GoalStatus::NotStarted,
            owner_user_id: "user-1".to_string(),
            org_id: Some("org-9".to_string()),
            active_session_id: None,
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
        }
    }

    #[test]
    fn marker_round_trips() {
        let doc = goal();
        let body = render_marker(&doc);
        let parsed = parse_marker(&body).unwrap();
        assert_eq!(parsed.v, 1);
        assert_eq!(parsed.goal_id, doc.id.to_string());
        assert_eq!(parsed.owner_user_id, "user-1");
        assert_eq!(parsed.org_id.as_deref(), Some("org-9"));
        assert_eq!(parsed.package_names, doc.package_names);
        assert_eq!(parsed.repo, doc.repo);
    }

    #[test]
    fn marker_never_contains_the_prompt() {
        assert!(!render_marker(&goal()).contains("SECRET-PROMPT"));
    }

    #[test]
    fn marker_missing_returns_missing() {
        assert_eq!(
            parse_marker("just an issue body, no marker"),
            Err(MarkerError::Missing)
        );
    }

    #[test]
    fn marker_corrupt_returns_malformed() {
        let body = format!("<!-- {MARKER_PREFIX}\n{{ not json\n-->");
        assert!(matches!(
            parse_marker(&body),
            Err(MarkerError::Malformed(_))
        ));
    }

    #[test]
    fn marker_v2_unsupported() {
        let body = format!(
            "<!-- {MARKER_PREFIX}\n{}\n-->",
            r#"{"v":2,"goal_id":"x","owner_user_id":"u","org_id":null,"package_names":["a"],"repo":null}"#
        );
        assert_eq!(parse_marker(&body), Err(MarkerError::UnsupportedVersion(2)));
    }

    #[test]
    fn marker_hand_edited_invalid_package_rejected() {
        let body = format!(
            "<!-- {MARKER_PREFIX}\n{}\n-->",
            r#"{"v":1,"goal_id":"x","owner_user_id":"u","org_id":null,"package_names":["bad name!"],"repo":null}"#
        );
        assert!(matches!(
            parse_marker(&body),
            Err(MarkerError::Malformed(_))
        ));
    }
}
