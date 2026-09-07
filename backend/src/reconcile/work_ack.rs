//! Best-effort, exactly-once work-issue feedback.
//!
//! One repo-level snapshot across the union of active work labels drives both the
//! global unrouted surface and each session's routed ack/reject decision. Issues
//! are projected to metadata immediately and deduped by number across labels.

use std::collections::{HashMap, HashSet};

use secrecy::SecretString;

use crate::access_policy::AccessPolicy;
use crate::github_app::listing::{GithubListing, IssueMetadata};
use crate::github_app::GithubAppTokens;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::routing::{route_work_issue, WorkRouting};
use crate::reconcile::work_authz::is_work_author_allowed_with_bot;

use super::{WORK_PICKED_UP_LABEL, WORK_UNAUTHORIZED_LABEL, WORK_UNROUTED_LABEL};

pub fn work_ack_comment(session_name: &str, work_label: &str) -> String {
    format!(
        "👀 **Picked up by fkst session `{session_name}`.**\n\n\
         A fkst pod is working this repo's `{work_label}` issues, including this one. \
         The session posts its progress on this issue as it works, and the outcome \
         will be a pull request (or, for issue-producing sessions, linked issues)."
    )
}

pub fn work_unauthorized_comment(
    author_login: &str,
    session_name: &str,
    creator_login: &str,
    trigger_issue: i64,
) -> String {
    format!(
        "🚫 **@{author_login} is not authorized to raise work for fkst session \
         `{session_name}`.**\n\n\
         Only the session's **creator** (@{creator_login}), the logins listed under \
         **Session Collaborators**, and this deployment's **fkst administrators** may \
         open work issues for it — so this issue will NOT be picked up. See the \
         session's trigger issue (#{trigger_issue})."
    )
}

pub fn work_unrouted_comment() -> &'static str {
    "⚠️ **This issue carries an fkst work label but is not routed to any session.**\n\n\
     Work issues are picked up only when **exactly one assignee** is set and that \
     assignee is the creator of an active fkst session watching the label. Assign \
     the matching session creator to route it; it will not be worked until then."
}

/// Process all open work issues for the active registrations in one repository.
/// Every read/write is best-effort; a failed label listing skips only that label,
/// and every durable feedback path is retried on a later reconcile as appropriate.
pub async fn ack_open_work_issues(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    regs: &[SessionRegistration],
    work_labels_by_session: &HashMap<String, Vec<String>>,
    global_admins: &AccessPolicy,
) {
    ack_open_work_issues_with_bot(
        github,
        listing,
        token,
        repo,
        regs,
        work_labels_by_session,
        global_admins,
        None,
    )
    .await;
}

/// Production work feedback with the configured App identity admitted as a
/// system-authored work principal. The public wrapper above retains the strict
/// human-only behavior for callers that do not provide an App identity.
#[allow(clippy::too_many_arguments)]
pub async fn ack_open_work_issues_with_bot(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    regs: &[SessionRegistration],
    work_labels_by_session: &HashMap<String, Vec<String>>,
    global_admins: &AccessPolicy,
    github_bot_login: Option<&str>,
) {
    if regs.is_empty() {
        return;
    }

    let mut unique_labels = Vec::new();
    let mut seen_labels = HashSet::new();
    for reg in regs {
        for label in labels_for(reg, work_labels_by_session) {
            if seen_labels.insert(label.clone()) {
                unique_labels.push(label.clone());
            }
        }
    }

    let mut issues = Vec::new();
    let mut seen_issues = HashSet::new();
    for label in &unique_labels {
        match listing
            .list_issues_by_label(token, &repo.owner, &repo.name, label)
            .await
        {
            Ok(list) => {
                for issue in list {
                    if seen_issues.insert(issue.number) {
                        issues.push(issue.metadata());
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    owner = %repo.owner,
                    name = %repo.name,
                    work_label = %label,
                    error = %error,
                    "work-ack: listing open work issues failed; will retry next reconcile"
                );
            }
        }
    }

    // Unrouted is a repo-level decision: a sole assignee must match at least one
    // active creator watching at least one work label carried by the issue.
    for issue in &issues {
        let routed = regs.iter().any(|reg| {
            issue_matches_labels(issue, labels_for(reg, work_labels_by_session))
                && route_work_issue(issue, &reg.creator_login) == WorkRouting::Routed
        });
        let carries_unrouted = carries_label(issue, WORK_UNROUTED_LABEL);
        if routed {
            if carries_unrouted {
                clear_unrouted(github, repo, issue.number).await;
            }
        } else if !carries_unrouted {
            flag_unrouted(github, repo, issue.number).await;
        }
    }

    for reg in regs {
        let labels = labels_for(reg, work_labels_by_session);
        for issue in &issues {
            if !issue_matches_labels(issue, labels)
                || route_work_issue(issue, &reg.creator_login) != WorkRouting::Routed
            {
                continue;
            }

            let carries_unauthorized = carries_label(issue, WORK_UNAUTHORIZED_LABEL);
            if !is_work_author_allowed_with_bot(
                reg,
                global_admins,
                issue.user_id,
                &issue.user_login,
                github_bot_login,
            ) {
                if !carries_unauthorized {
                    reject_issue(github, repo, issue, reg).await;
                }
                continue;
            }

            if carries_unauthorized {
                clear_unauthorized(github, repo, issue.number).await;
            }
            if carries_label(issue, WORK_PICKED_UP_LABEL) {
                continue;
            }
            ack_issue(
                github,
                repo,
                issue.number,
                &reg.def.name,
                first_matching_label(issue, labels),
            )
            .await;
        }
    }
}

fn labels_for<'a>(
    reg: &SessionRegistration,
    work_labels_by_session: &'a HashMap<String, Vec<String>>,
) -> &'a [String] {
    work_labels_by_session
        .get(&reg.session_id)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn issue_matches_labels(issue: &IssueMetadata, labels: &[String]) -> bool {
    labels
        .iter()
        .any(|label| issue.labels.iter().any(|candidate| candidate == label))
}

fn carries_label(issue: &IssueMetadata, label: &str) -> bool {
    issue.labels.iter().any(|candidate| candidate == label)
}

fn first_matching_label<'a>(issue: &IssueMetadata, labels: &'a [String]) -> &'a str {
    labels
        .iter()
        .find(|label| issue.labels.iter().any(|candidate| candidate == *label))
        .or_else(|| labels.first())
        .map(String::as_str)
        .unwrap_or("")
}

async fn reject_issue(
    github: &GithubAppTokens,
    repo: &RepoRef,
    issue: &IssueMetadata,
    reg: &SessionRegistration,
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    if let Err(error) = github
        .add_issue_labels(
            &owner_repo,
            issue.number as u64,
            &[WORK_UNAUTHORIZED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = issue.number, error = %error, "work-authz: latch unauthorized label failed; skipping comment, will retry next pass");
        return;
    }
    let comment = work_unauthorized_comment(
        &issue.user_login,
        &reg.def.name,
        &reg.creator_login,
        reg.trigger_issue,
    );
    if let Err(error) = github
        .post_issue_comment(&owner_repo, issue.number as u64, &comment)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = issue.number, error = %error, "work-authz: reject comment failed (label already latched; not retried)");
    }
}

async fn clear_unauthorized(github: &GithubAppTokens, repo: &RepoRef, number: i64) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    if let Err(error) = github
        .remove_issue_label(&owner_repo, number as u64, WORK_UNAUTHORIZED_LABEL)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-authz: clearing stale unauthorized label failed; will retry next reconcile");
    }
}

async fn flag_unrouted(github: &GithubAppTokens, repo: &RepoRef, number: i64) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    if let Err(error) = github
        .add_issue_labels(
            &owner_repo,
            number as u64,
            &[WORK_UNROUTED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-routing: latch unrouted label failed; skipping comment, will retry next pass");
        return;
    }
    if let Err(error) = github
        .post_issue_comment(&owner_repo, number as u64, work_unrouted_comment())
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-routing: unrouted comment failed (label already latched; not retried)");
    }
}

async fn clear_unrouted(github: &GithubAppTokens, repo: &RepoRef, number: i64) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    if let Err(error) = github
        .remove_issue_label(&owner_repo, number as u64, WORK_UNROUTED_LABEL)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-routing: clearing stale unrouted label failed; will retry next reconcile");
    }
}

async fn ack_issue(
    github: &GithubAppTokens,
    repo: &RepoRef,
    number: i64,
    session_name: &str,
    work_label: &str,
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let comment = work_ack_comment(session_name, work_label);
    if let Err(error) = github
        .post_issue_comment(&owner_repo, number as u64, &comment)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-ack: issue comment failed");
    }
    if let Err(error) = github
        .add_issue_labels(
            &owner_repo,
            number as u64,
            &[WORK_PICKED_UP_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-ack: latch picked-up label failed");
    }
}

#[cfg(test)]
#[path = "work_ack_authz_tests.rs"]
mod authz_tests;
#[cfg(test)]
#[path = "work_ack_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "work_ack_test_support.rs"]
mod work_ack_test_support;
