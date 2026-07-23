//! Effective session-creator resolution from trigger-issue metadata.
//!
//! A human-authored trigger belongs to its author. An App-authored trigger cannot
//! use the App as its human authority subject, so exactly one assignee attributes
//! the session to that person. This module accepts [`IssueMetadata`] rather than an
//! issue body by design: creator resolution always happens before trigger parsing.

use crate::github_app::listing::IssueMetadata;

/// The human identity that owns a session for authorization and routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCreator {
    pub login: String,
    /// Present for issue authors. GitHub's issue metadata does not expose an
    /// assignee id, so App-authored triggers carry only the assignee login.
    pub id: Option<i64>,
}

/// The result of attributing a trigger issue to a human creator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreatorResolution {
    Resolved(SessionCreator),
    /// A bot-authored trigger needs exactly one assignee to identify its creator.
    Unattributable {
        author_login: String,
        assignee_count: usize,
    },
}

/// Normalize GitHub's three App-login representations to a bare lowercase slug.
///
/// REST may return `slug[bot]`, GraphQL may return `slug`, and `gh` may render
/// `app/slug`. Strip one leading `app/` and one trailing `[bot]`, matching the
/// packages' `github-issue.scopes.normalize_login` behavior.
fn normalize_login(login: &str) -> String {
    let folded = login.to_ascii_lowercase();
    let without_app = folded.strip_prefix("app/").unwrap_or(&folded);
    without_app
        .strip_suffix("[bot]")
        .unwrap_or(without_app)
        .to_string()
}

/// Whether `login` is the configured FKST GitHub App identity.
///
/// Keep trigger attribution and system-authored work authorization on the same
/// normalization contract so REST, GraphQL, and `gh` actor forms cannot diverge.
pub(crate) fn is_expected_bot_login(login: &str, bot_login: Option<&str>) -> bool {
    bot_login
        .map(|bot| normalize_login(login) == normalize_login(bot))
        .unwrap_or(false)
}

/// Resolve the effective creator using issue metadata only.
pub fn effective_creator(meta: &IssueMetadata, bot_login: Option<&str>) -> CreatorResolution {
    let bot_authored = is_expected_bot_login(&meta.user_login, bot_login);

    if !bot_authored {
        return CreatorResolution::Resolved(SessionCreator {
            login: meta.user_login.clone(),
            id: Some(meta.user_id),
        });
    }

    match meta.assignees.as_slice() {
        [login] => CreatorResolution::Resolved(SessionCreator {
            login: login.clone(),
            id: None,
        }),
        assignees => CreatorResolution::Unattributable {
            author_login: meta.user_login.clone(),
            assignee_count: assignees.len(),
        },
    }
}

#[cfg(test)]
#[path = "creator_tests.rs"]
mod tests;
