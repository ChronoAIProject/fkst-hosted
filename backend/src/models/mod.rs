//! Shared wire/domain types.
//!
//! v1 is datastore-free and session state lives in Kubernetes (a session IS a
//! Pod), so the only surviving model is [`RepoRef`] — the `owner/name` GitHub
//! repository reference shared by the reconciler, the session-Pod launcher, and
//! the webhook nudge.

use serde::{Deserialize, Serialize};

/// GitHub repository reference: `owner/name`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, utoipa::ToSchema)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ref_round_trips_through_serde() {
        let repo = RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        };
        let json = serde_json::to_string(&repo).unwrap();
        assert_eq!(json, r#"{"owner":"acme","name":"site"}"#);
        let back: RepoRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, repo);
    }
}
