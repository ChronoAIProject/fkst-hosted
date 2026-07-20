//! One-time acknowledgment (or R3 authority REJECTION) of every open WORK issue.
//!
//! A user files a WORK issue (one carrying a session's work label) and the pod picks
//! it up and works it — but from GitHub the issue often shows NO labels or comments
//! (a session's output, e.g. the codex-triage package, lands elsewhere), so the
//! author has no signal it was even claimed. This step gives the WORK issue a
//! visible, fkst-hosted-owned acknowledgment: on each reconcile, for every VALID
//! registration in the repo it LISTs the open issues carrying that session's work
//! label(s) and, for each one NOT yet acknowledged, posts a friendly "picked up"
//! comment and latches [`crate::reconcile::WORK_PICKED_UP_LABEL`].
//!
//! R3 authority gate (epic #572): when the operator opts into enforcement (the
//! [`WorkAuthz`] the driver passes has `enforce == true`), the step processes the
//! session's FULL work-label set (explicit ∪ package-discovered — the same set the
//! pending gate authorizes over, so there is no reject/pending asymmetry) and, for an
//! issue whose author may NOT raise work for the session, REJECTS it instead of
//! picking it up: it never acks such an issue; instead, once only, it latches
//! [`crate::reconcile::WORK_UNAUTHORIZED_LABEL`] then posts the reject comment. When
//! enforcement is off, only the explicit `### Work Label` is acked — byte-identical
//! to pre-R3.
//!
//! It exactly mirrors the session-announce pattern (comment + durable latch read
//! back each reconcile) so a control-plane restart never re-posts, and mirrors the
//! `ensure_issue_templates`/`auto_merge_bot_pull_requests` hooks: called
//! best-effort from the per-repo driver, it NEVER fails the reconcile (a list/post
//! failure is logged and skipped), and it reuses the repo-scoped installation token
//! the driver already minted. The comment + label carry only PUBLIC metadata (the
//! session name, work label, author login) — never the minted token or any secret.

use std::collections::{HashMap, HashSet};

use secrecy::SecretString;

use crate::github_app::listing::{GithubListing, IssueSummary};
use crate::github_app::GithubAppTokens;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::work_authz::WorkAuthz;

use super::{WORK_PICKED_UP_LABEL, WORK_UNAUTHORIZED_LABEL};

/// Render the "picked up" acknowledgment comment for a work issue (pure;
/// unit-tested). Both `session_name` and `work_label` are public metadata safe to
/// display verbatim in backticks.
pub fn work_ack_comment(session_name: &str, work_label: &str) -> String {
    format!(
        "👀 **Picked up by fkst session `{session_name}`.**\n\n\
         A fkst pod is working this repo's `{work_label}` issues, including this one. \
         The session posts its progress on this issue as it works, and the outcome \
         will be a pull request (or, for issue-producing sessions, linked issues)."
    )
}

/// Render the "not authorized to raise work" rejection comment (pure; unit-tested).
/// `author_login` is the (public) login already surfaced elsewhere, `session_name`
/// is public metadata, and `trigger_issue` points the author at where authority is
/// declared. Naming who MAY raise work is deliberate — the reject must be visible
/// and self-explaining, never a silent drop.
pub fn work_unauthorized_comment(
    author_login: &str,
    session_name: &str,
    trigger_issue: i64,
) -> String {
    format!(
        "🚫 **@{author_login} is not authorized to raise work for fkst session \
         `{session_name}`.**\n\n\
         Only the session's **author**, the logins listed under **Session \
         Collaborators**, and this repository's **admins / organization owners** may \
         open work issues for it — so this issue will NOT be picked up. See the \
         session's trigger issue (#{trigger_issue}) for who may raise work and how to \
         be added."
    )
}

/// Best-effort, NON-failing: process every open work issue exactly once.
///
/// For each registration in `regs`, LIST the open issues carrying the session's work
/// label(s) and take a single once-only action per issue:
///
/// - When `authz.enforce` AND the issue's author is NOT authorized to raise work for
///   the session, the issue is REJECTED — never picked up; unless it already carries
///   [`WORK_UNAUTHORIZED_LABEL`], the label is latched (the once-only gate) and then
///   the reject comment is posted.
/// - Otherwise (authorized, or enforcement off) the issue is ACKNOWLEDGED — unless it
///   already carries [`WORK_PICKED_UP_LABEL`], the "picked up" comment is posted and
///   that label latched. A now-authorized issue still carrying a stale unauthorized
///   latch is self-healed (the label cleared) first.
///
/// The label set processed depends on the pass's [`WorkAuthz`]: when enforcing, the
/// session's FULL set (explicit ∪ package-discovered, from `work_labels_by_session` —
/// the same set the pending gate authorizes over); when not, the explicit
/// `### Work Label` only (a label-less session is then a no-op), byte-identical to
/// pre-R3. No-op when there are no registrations (mirrors the other driver hooks'
/// ≥1-registration gate). Reuses the repo-scoped installation `token` the driver
/// minted. Every GitHub call is best-effort: a failure is logged and skipped, never
/// propagated, so one bad issue never stalls the rest of the reconcile.
pub async fn ack_open_work_issues(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    regs: &[SessionRegistration],
    work_labels_by_session: &HashMap<String, Vec<String>>,
    authz: &WorkAuthz,
) {
    if regs.is_empty() {
        return;
    }
    for reg in regs {
        // The labels to process for THIS session. Enforcing → the full work-label set
        // (so an unauthorized issue on ANY of the session's labels is rejected, with
        // no reject/pending asymmetry). Not enforcing → the explicit label only,
        // exactly pre-R3 (a label-less session's discovered work needs no ack).
        let labels: Vec<String> = if authz.enforce {
            work_labels_by_session
                .get(&reg.session_id)
                .cloned()
                .unwrap_or_default()
        } else {
            reg.def.work_label.iter().cloned().collect()
        };
        if labels.is_empty() {
            continue;
        }
        process_reg(github, listing, token, repo, reg, &labels, authz).await;
    }
}

/// Ack-or-reject every open issue across the session's `labels`, exactly once each.
/// Issues are collected + DEDUPED by number (an issue can carry several of the
/// session's labels) so each is processed once regardless of how many it bears. A
/// per-label listing failure is logged and skipped (the other labels still run; the
/// next reconcile retries the failed one).
async fn process_reg(
    github: &GithubAppTokens,
    listing: &dyn GithubListing,
    token: &SecretString,
    repo: &RepoRef,
    reg: &SessionRegistration,
    labels: &[String],
    authz: &WorkAuthz,
) {
    let session_name = reg.def.name.as_str();

    let mut seen: HashSet<i64> = HashSet::new();
    let mut issues: Vec<IssueSummary> = Vec::new();
    for label in labels {
        match listing
            .list_issues_by_label(token, &repo.owner, &repo.name, label)
            .await
        {
            Ok(list) => {
                for issue in list {
                    if seen.insert(issue.number) {
                        issues.push(issue);
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

    for issue in issues {
        let carries_unauthorized = issue.labels.iter().any(|l| l == WORK_UNAUTHORIZED_LABEL);

        // R3 authority gate: an author who may NOT raise work for the session is
        // rejected visibly (never picked up), once only — the durable unauthorized
        // latch (read back each reconcile) is the dedupe gate.
        if !authz.allows(reg, issue.user_id, &issue.user_login) {
            if !carries_unauthorized {
                reject_issue(
                    github,
                    repo,
                    issue.number,
                    &issue.user_login,
                    session_name,
                    reg.trigger_issue,
                )
                .await;
            }
            continue;
        }

        // Authorized. Self-heal: an issue that still carries a now-stale unauthorized
        // latch (the admin tier recovered after a blip, or the author became a repo
        // admin) has the label cleared before acking, so its label + comment history
        // stop contradicting the current decision. Only under enforcement — a
        // flag-off deploy never applies the label, so this is a no-op there.
        if authz.enforce && carries_unauthorized {
            clear_unauthorized(github, repo, issue.number).await;
        }

        // Acknowledge once. The durable picked-up latch makes this idempotent.
        if issue.labels.iter().any(|l| l == WORK_PICKED_UP_LABEL) {
            continue;
        }
        ack_issue(
            github,
            repo,
            issue.number,
            session_name,
            first_matching_label(&issue, labels),
        )
        .await;
    }
}

/// The first of the session's `labels` that `issue` actually carries — the label the
/// ack comment names. The issue came from one of these labels' listings, so it
/// carries at least one; the fallback is defensive.
fn first_matching_label<'a>(issue: &IssueSummary, labels: &'a [String]) -> &'a str {
    labels
        .iter()
        .find(|l| issue.labels.iter().any(|il| il == *l))
        .or_else(|| labels.first())
        .map(String::as_str)
        .unwrap_or("")
}

/// Reject ONE unauthorized work issue: latch the durable [`WORK_UNAUTHORIZED_LABEL`]
/// FIRST, then post the reject comment. The label is the once-only gate, so it MUST
/// land before the comment: if the latch write fails we skip the comment and retry
/// BOTH next pass (the missing gate would otherwise let a later pass re-post a
/// comment that already succeeded — a duplicate). If the latch succeeds but the
/// comment fails, the gate is set, so the comment is simply not retried — never
/// double-posted.
async fn reject_issue(
    github: &GithubAppTokens,
    repo: &RepoRef,
    number: i64,
    author_login: &str,
    session_name: &str,
    trigger_issue: i64,
) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);

    if let Err(error) = github
        .add_issue_labels(
            &owner_repo,
            number as u64,
            &[WORK_UNAUTHORIZED_LABEL.to_string()],
        )
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-authz: latch unauthorized label failed; skipping comment, will retry next pass");
        return;
    }
    let comment = work_unauthorized_comment(author_login, session_name, trigger_issue);
    if let Err(error) = github
        .post_issue_comment(&owner_repo, number as u64, &comment)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-authz: reject comment failed (label already latched; not retried)");
    }
}

/// Self-heal: clear a now-stale [`WORK_UNAUTHORIZED_LABEL`] from an issue whose author
/// is authorized again. Best-effort — a failure is logged and retried next reconcile.
async fn clear_unauthorized(github: &GithubAppTokens, repo: &RepoRef, number: i64) {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    if let Err(error) = github
        .remove_issue_label(&owner_repo, number as u64, WORK_UNAUTHORIZED_LABEL)
        .await
    {
        tracing::warn!(owner_repo = %owner_repo, issue = number, error = %error, "work-authz: clearing stale unauthorized label failed; will retry next reconcile");
    }
}

/// Acknowledge ONE work issue: post the comment, then latch the durable label.
/// Mirrors the executor's announce arm — the comment is best-effort (a failure is
/// logged, never propagated) and the label add is additive/idempotent.
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

// Tests are split so each file stays under the 500-line limit: shared harness +
// fixtures, the ack tests, and the R3 authority reject tests.
#[cfg(test)]
#[path = "work_ack_authz_tests.rs"]
mod authz_tests;
#[cfg(test)]
#[path = "work_ack_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "work_ack_test_support.rs"]
mod work_ack_test_support;
