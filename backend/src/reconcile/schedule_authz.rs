//! The authorized body-read carve-out for scheduled-workflow issues.
//!
//! The control plane deliberately never reads a work issue's BODY. Issues are
//! projected to a content-free [`IssueMetadata`] at the listing boundary
//! ([`crate::github_app::listing`]) so that no authorization predicate can be
//! influenced by user content — a predicate that reads a body is a predicate an
//! author can argue with.
//!
//! The scheduled-workflow kind needs a body: the definition IS the body. This
//! module is the narrowest possible carve-out, and the ordering it guarantees is
//! encoded in the type system rather than in a comment:
//!
//! - [`AuthorizedScheduleIssue`]'s fields are private to this module and it has no
//!   public constructor, so the ONLY way to obtain one — and therefore the only way
//!   to reach a scheduled-workflow body — is [`authorize_schedule_issue`];
//! - that function decides using `issue.metadata()` ALONE, exactly like the
//!   pending gate, and moves the body across only after every predicate has passed.
//!
//! What the invariant still protects is unchanged: routing and authority are
//! decided on assignee/author/label metadata, so a body cannot talk its way into
//! being worked.

use k8s_openapi::chrono::{DateTime, Utc};

use crate::access_policy::AccessPolicy;
use crate::github_app::listing::IssueSummary;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::routing::{route_work_issue, WorkRouting};
use crate::reconcile::work_authz::is_work_author_allowed_with_bot;

/// A scheduled-workflow issue that has passed routing AND author authority, and
/// therefore may have its body parsed.
///
/// Constructible only by [`authorize_schedule_issue`] (private fields, no public
/// constructor). Holding one is proof the predicates ran.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedScheduleIssue {
    number: i64,
    body: String,
    title: String,
    labels: Vec<String>,
    created_at: DateTime<Utc>,
}

impl AuthorizedScheduleIssue {
    pub fn number(&self) -> i64 {
        self.number
    }

    /// The definition text. Reachable only through this type, which is the whole
    /// point of the carve-out.
    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The issue's current labels, so the caller can read its own durable latches
    /// (paused, running, invalid) without a second GitHub round-trip.
    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    /// The recurrence ANCHOR: a definition never fires for a slot that predates its
    /// own creation, however long the control plane was away.
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }
}

/// Why a scheduled-workflow issue was not accepted.
///
/// Every variant carries the author-facing detail verbatim, because all of them
/// land on the same clearable
/// [`crate::reconcile::reserved_labels::SCHEDULE_INVALID_LABEL`] latch: the author
/// needs to know which of the three things to fix, not which enum variant fired.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleDenial {
    /// Zero or several assignees: there is no unambiguous session to run it.
    Unrouted(String),
    /// Exactly one assignee, but nobody on this repository runs sessions for them.
    NoSession(String),
    /// Routed, but the issue's author may not raise work for that session.
    Unauthorized(String),
}

impl ScheduleDenial {
    /// The detail rendered into the invalid-latch comment.
    pub fn detail(&self) -> &str {
        match self {
            ScheduleDenial::Unrouted(detail)
            | ScheduleDenial::NoSession(detail)
            | ScheduleDenial::Unauthorized(detail) => detail,
        }
    }
}

/// Decide whether `issue` may be read as a scheduled-workflow definition, and which
/// registration owns it.
///
/// Ordering is the contract: routing, then session lookup, then author authority —
/// all from metadata — and only then the body. `regs` is the repository's accepted
/// registrations for this pass.
///
/// When one creator runs several sessions on a repository, the LOWEST trigger issue
/// owns the schedule. It is arbitrary but deterministic, and it matches the
/// tie-break the work-label collision backstop already uses, so an operator only
/// has to learn "lowest issue number wins" once.
pub fn authorize_schedule_issue<'a>(
    issue: &IssueSummary,
    regs: &'a [SessionRegistration],
    access: &AccessPolicy,
    bot_login: Option<&str>,
) -> Result<(AuthorizedScheduleIssue, &'a SessionRegistration), ScheduleDenial> {
    let meta = issue.metadata();

    let assignee = match meta.assignees.as_slice() {
        [assignee] => assignee,
        assignees => {
            return Err(ScheduleDenial::Unrouted(format!(
                "a scheduled workflow needs exactly one assignee — the creator of the session \
                 that will run it — but this issue has {}. Assign exactly that person.",
                assignees.len()
            )))
        }
    };

    let owner = regs
        .iter()
        .filter(|reg| {
            // Reuse the sole-assignee routing predicate rather than re-deriving it,
            // so a schedule routes by exactly the rule work issues route by.
            route_work_issue(&meta, &reg.creator_login) == WorkRouting::Routed
        })
        .min_by_key(|reg| reg.trigger_issue)
        .ok_or_else(|| {
            ScheduleDenial::NoSession(format!(
                "no active session on this repository is owned by {assignee}. Open a \
                 `fkst-substrate-trigger` issue for that creator first, then reassign this one."
            ))
        })?;

    if !is_work_author_allowed_with_bot(owner, access, meta.user_id, &meta.user_login, bot_login) {
        return Err(ScheduleDenial::Unauthorized(format!(
            "the author of this issue may not schedule work for session #{}. Only its creator, \
             a listed `### Session Collaborators` login, or a deployment administrator may.",
            owner.trigger_issue
        )));
    }

    Ok((
        AuthorizedScheduleIssue {
            number: issue.number,
            body: issue.body.clone(),
            title: issue.title.clone(),
            labels: issue.labels.clone(),
            created_at: issue.created_at,
        },
        owner,
    ))
}

#[cfg(test)]
#[path = "schedule_authz_tests.rs"]
mod tests;
