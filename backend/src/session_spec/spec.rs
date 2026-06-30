//! The non-secret per-session descriptor written into a Job pod.

use serde::{Deserialize, Serialize};

use crate::models::RepoRef;

/// Fixed namespace for the deterministic per-session UUIDv5. Constant so the
/// same `(installation_id, owner, name, issue_number)` always derives the same
/// session id — a webhook redelivery therefore maps to the SAME session, the
/// SAME `fkst-sess-<id>` Job name, and a `create` that no-ops on AlreadyExists
/// (the at-most-one-Job-per-session guarantee). The bytes are an arbitrary,
/// stable random UUID dedicated to fkst sessions.
const SESSION_NAMESPACE: uuid::Uuid = uuid::Uuid::from_bytes([
    0x9f, 0x2a, 0x4c, 0x6e, 0x1b, 0x83, 0x4d, 0x7a, 0xa5, 0xe0, 0x7c, 0x11, 0x3d, 0x52, 0x88, 0x64,
]);

/// Derive the deterministic session id for an issue-triggered session.
///
/// Returns the canonical lowercase, hyphenated UUID string (36 chars), which is
/// a valid DNS-1123 label component, so `fkst-sess-<id>` is a legal Kubernetes
/// object name (46 chars, within the 63-char limit). Same inputs → same id.
pub fn derive_session_id(
    installation_id: i64,
    owner: &str,
    name: &str,
    issue_number: i64,
) -> String {
    // A stable, unambiguous canonical name. `#` separates the issue number so
    // `(owner="a", name="b#1", issue=2)` cannot collide with
    // `(owner="a", name="b", issue="1#2")`-style ambiguity in practice.
    let canonical = format!("{installation_id}/{owner}/{name}#{issue_number}");
    uuid::Uuid::new_v5(&SESSION_NAMESPACE, canonical.as_bytes()).to_string()
}

/// The goal a session runs: the human title + the engine prompt (the user's
/// GitHub issue body). This is issue text, not a credential — but it is mounted
/// into the pod privately (the SessionSpec rides the per-session Secret volume,
/// not a `kubectl describe`-visible ConfigMap or env var).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionGoal {
    pub title: String,
    pub prompt: String,
}

/// The non-secret descriptor of one substrate session.
///
/// Carries NO credentials — the GitHub App token and the static LLM API key live
/// in the mounted Secret volume described by [`crate::session_spec::CredsLayout`].
/// This separation is what lets the control plane write the SessionSpec freely
/// while keeping every token off the descriptor (and out of any `{:?}`
/// rendering).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionSpec {
    /// Deterministic id from [`derive_session_id`]; the `fkst-sess-<id>` Job
    /// name keys the at-most-one-Job-per-session idempotency guarantee.
    pub session_id: String,
    /// The logical run key (the session's progress-record identity).
    pub run_key: String,
    /// The GitHub App installation the token was minted from.
    pub installation_id: i64,
    /// The target repository the session clones and works in.
    pub repo: RepoRef,
    /// The GitHub login of the **issue author** — the authorization subject for
    /// the issue-comment control path (`/stop`, `/status`): only this login may
    /// drive the session. This is deliberately DISTINCT from [`Self::repo`]'s
    /// `owner` (the repo owner): the App token is scoped to the repo, and the
    /// deterministic session id is derived from the repo owner, but who is
    /// *allowed to control* the session is its issue author.
    pub owner_login: String,
    /// The triggering issue number (progress is reported back to it).
    pub issue_number: i64,
    /// What to run.
    pub goal: SessionGoal,
    /// The `.fkst/packages/<name>` packages named in the issue.
    pub package_names: Vec<String>,
    /// The dedicated branch the pod commits its `.fkst/log/<run_key>.log`
    /// checkpoints to (e.g. `fkst/session-<id>`), keeping the code PR clean.
    pub log_branch: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionSpec {
        SessionSpec {
            session_id: derive_session_id(42, "acme", "site", 7),
            run_key: "a1b2c3".into(),
            installation_id: 42,
            repo: RepoRef {
                owner: "acme".into(),
                name: "site".into(),
            },
            owner_login: "acme".into(),
            issue_number: 7,
            goal: SessionGoal {
                title: "Add dark mode".into(),
                prompt: "Implement a dark-mode toggle in settings.".into(),
            },
            package_names: vec!["web".into()],
            log_branch: "fkst/session-x".into(),
        }
    }

    #[test]
    fn session_id_is_deterministic_for_the_same_inputs() {
        let a = derive_session_id(42, "acme", "site", 7);
        let b = derive_session_id(42, "acme", "site", 7);
        assert_eq!(
            a, b,
            "same inputs must derive the same id (redelivery dedup)"
        );
        // Canonical hyphenated lowercase UUID: 36 chars, fits fkst-sess-<id>.
        assert_eq!(a.len(), 36);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
    }

    #[test]
    fn session_id_differs_when_any_input_differs() {
        let base = derive_session_id(42, "acme", "site", 7);
        assert_ne!(base, derive_session_id(43, "acme", "site", 7));
        assert_ne!(base, derive_session_id(42, "other", "site", 7));
        assert_ne!(base, derive_session_id(42, "acme", "other", 7));
        assert_ne!(base, derive_session_id(42, "acme", "site", 8));
    }

    #[test]
    fn session_spec_round_trips_through_serde() {
        let spec = sample();
        let json = serde_json::to_string(&spec).unwrap();
        let back: SessionSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(spec, back);
    }

    #[test]
    fn session_spec_rejects_unknown_fields() {
        let mut value = serde_json::to_value(sample()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("rogue".into(), serde_json::json!(true));
        let json = serde_json::to_string(&value).unwrap();
        assert!(serde_json::from_str::<SessionSpec>(&json).is_err());
    }

    #[test]
    fn debug_carries_no_credentials() {
        // SessionSpec holds no token/secret fields by construction; this guards
        // against a future field regressing that invariant. The prompt IS in the
        // descriptor (it is issue text, not a credential), so we assert the
        // structural property: no `SecretString` and no token-shaped value.
        let rendered = format!("{:?}", sample());
        assert!(!rendered.contains("ghs_"));
        assert!(!rendered.to_lowercase().contains("secretstring"));
    }
}
