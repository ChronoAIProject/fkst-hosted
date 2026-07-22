//! Pure assignee routing for work issues.
//!
//! Label membership is deliberately the caller's concern. This predicate accepts
//! metadata only, so neither routing nor authorization can inspect issue content.

use crate::github_app::listing::IssueMetadata;

/// How one labeled work issue relates to one active session creator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkRouting {
    /// Exactly one assignee, matching this session's creator login.
    Routed,
    /// No unambiguous routing key exists.
    NotRouted { assignee_count: usize },
    /// Exactly one assignee exists, but it names another creator.
    WrongAssignee,
}

/// Route `meta` to `creator_login` using the sole-assignee contract.
pub fn route_work_issue(meta: &IssueMetadata, creator_login: &str) -> WorkRouting {
    match meta.assignees.as_slice() {
        [assignee] if assignee.eq_ignore_ascii_case(creator_login) => WorkRouting::Routed,
        [_] => WorkRouting::WrongAssignee,
        assignees => WorkRouting::NotRouted {
            assignee_count: assignees.len(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(assignees: &[&str]) -> IssueMetadata {
        IssueMetadata {
            number: 7,
            labels: vec!["fkst-work".to_string()],
            state: "open".to_string(),
            assignees: assignees.iter().map(|value| value.to_string()).collect(),
            user_login: "author".to_string(),
            user_id: 42,
        }
    }

    #[test]
    fn exactly_one_matching_assignee_routes_case_insensitively() {
        assert_eq!(
            route_work_issue(&metadata(&["ALICE"]), "alice"),
            WorkRouting::Routed
        );
    }

    #[test]
    fn zero_or_multiple_assignees_are_not_routed() {
        assert_eq!(
            route_work_issue(&metadata(&[]), "alice"),
            WorkRouting::NotRouted { assignee_count: 0 }
        );
        assert_eq!(
            route_work_issue(&metadata(&["alice", "bob"]), "alice"),
            WorkRouting::NotRouted { assignee_count: 2 }
        );
    }

    #[test]
    fn one_non_matching_assignee_is_someone_elses_route() {
        assert_eq!(
            route_work_issue(&metadata(&["bob"]), "alice"),
            WorkRouting::WrongAssignee
        );
    }
}
