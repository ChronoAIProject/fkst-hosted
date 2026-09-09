//! The per-repository schedule pass: the clock, expressed as a second enumeration
//! alongside the trigger-issue pass.
//!
//! It runs inside [`crate::reconcile::repo::reconcile_repo`], which only ever runs
//! on the Lease holder — so the clock inherits leader scoping from the reconciler
//! rather than needing its own election, and a lost Lease cancels it with every
//! other reconcile task. That is what keeps this feature inside the existing
//! control plane with no additional deployable.
//!
//! ## Cost
//!
//! One Search-free issue listing per repository, and — only for repositories that
//! actually have definitions — one or two comment pages per definition. A
//! repository with no scheduled workflows pays exactly one extra list call per
//! sweep and reads nothing else.
//!
//! ## Trust
//!
//! Run records are recovered from comments on an issue any repository collaborator
//! can comment on, so ONLY comments authored by the configured App identity are
//! trusted as records. Without that filter, anyone able to comment could forge a
//! terminal record to silence a schedule, or a dispatch record to strand it.

use std::collections::HashMap;

use k8s_openapi::chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::access_policy::AccessPolicy;
use crate::error::AppError;
use crate::github_app::comments::IssueCommentReader;
use crate::github_app::listing::{GithubListing, IssueSummary};
use crate::goals::scheduled_workflow_parse::{parse_scheduled_workflow, RunMode};
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::reserved_labels::{
    SCHEDULED_WORKFLOW_LABEL, WORKFLOW_RUN_LABEL, WORKFLOW_SCHEDULED_RUN_LABEL,
};
use crate::reconcile::schedule_authz::authorize_schedule_issue;
use crate::reconcile::schedule_plan::{
    check_min_interval, plan_invalid, plan_schedule, ScheduleEffect, ScheduleObservation,
};
use crate::reconcile::work_labels::apply_work_label_namespace;
use crate::reconcile_config::ReconcileConfig;
use crate::schedule::collect_records;

/// Plan every scheduled workflow on one repository.
///
/// Reads fail the WHOLE pass with an `Err` (the caller logs it and drops the
/// schedule effects for this sweep) rather than planning from partial data: an
/// empty run history read from a failed request would reset a cursor and re-fire a
/// slot that already ran. A 404 on a single issue's comments is not a failure — see
/// [`crate::github_app::comments`].
#[allow(clippy::too_many_arguments)]
pub async fn plan_repo_schedules(
    listing: &dyn GithubListing,
    comments: &dyn IssueCommentReader,
    token: &SecretString,
    repo: &RepoRef,
    regs: &[SessionRegistration],
    access: &AccessPolicy,
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Result<Vec<ScheduleEffect>, AppError> {
    // Without a configured App identity nothing can be trusted as a run record, so
    // every definition would look like it had never run and re-fire on every sweep.
    // Refusing to run the clock at all is the only safe reading of that state.
    let Some(bot_login) = cfg.github_bot_login.as_deref() else {
        return Ok(Vec::new());
    };

    let mut issues = listing
        .list_issues_by_label(token, &repo.owner, &repo.name, SCHEDULED_WORKFLOW_LABEL)
        .await?;
    if issues.is_empty() {
        return Ok(Vec::new());
    }
    // Issue-number order makes both the per-creator cap and the output deterministic.
    issues.sort_by_key(|issue| issue.number);

    let mut effects = Vec::new();
    let mut accepted_per_creator: HashMap<String, u32> = HashMap::new();
    for issue in &issues {
        match plan_one(
            issue,
            comments,
            token,
            repo,
            regs,
            access,
            bot_login,
            &mut accepted_per_creator,
            now,
            cfg,
        )
        .await
        {
            Ok(planned) => effects.extend(planned),
            Err(error) => return Err(error),
        }
    }
    Ok(effects)
}

/// Plan ONE definition: authorize from metadata, then parse, validate, read its
/// history, and decide.
#[allow(clippy::too_many_arguments)]
async fn plan_one(
    issue: &IssueSummary,
    comments: &dyn IssueCommentReader,
    token: &SecretString,
    repo: &RepoRef,
    regs: &[SessionRegistration],
    access: &AccessPolicy,
    bot_login: &str,
    accepted_per_creator: &mut HashMap<String, u32>,
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Result<Vec<ScheduleEffect>, AppError> {
    // Authorization decides on metadata alone, and holding the returned value is
    // the only way to reach the body.
    let (authorized, reg) = match authorize_schedule_issue(issue, regs, access, Some(bot_login)) {
        Ok(accepted) => accepted,
        Err(denial) => {
            return Ok(plan_invalid(
                issue.number,
                &issue.labels,
                denial.detail().to_string(),
            ))
        }
    };

    let spec = match parse_scheduled_workflow(authorized.body()) {
        Ok(spec) => spec,
        Err(error) => {
            return Ok(plan_invalid(
                issue.number,
                authorized.labels(),
                error.to_string(),
            ))
        }
    };

    if let Err(detail) = check_min_interval(&spec.run_mode, cfg) {
        return Ok(plan_invalid(issue.number, authorized.labels(), detail));
    }

    let work_label = match run_issue_work_label(&spec.run_mode, cfg) {
        Ok(label) => label,
        Err(detail) => return Ok(plan_invalid(issue.number, authorized.labels(), detail)),
    };

    // The cap counts ACCEPTED definitions in issue-number order, so the earliest
    // ones keep running and only the excess is rejected. Rejecting the whole set
    // would let one accidental burst take down a creator's working schedules.
    let creator_key = reg.creator_login.to_ascii_lowercase();
    let accepted = accepted_per_creator.entry(creator_key).or_insert(0);
    if *accepted >= cfg.cron_max_jobs_per_creator {
        return Ok(plan_invalid(
            issue.number,
            authorized.labels(),
            format!(
                "{} already has {} scheduled workflows on this repository, which is the \
                 deployment limit. Close one before opening another.",
                reg.creator_login, cfg.cron_max_jobs_per_creator
            ),
        ));
    }
    *accepted += 1;

    let history = comments
        .list_recent_issue_comments(
            token,
            &repo.owner,
            &repo.name,
            issue.number as u64,
            cfg.cron_history_pages,
        )
        .await?;
    let trusted: Vec<String> = history
        .into_iter()
        .filter(|comment| comment_is_from_bot(&comment.user_login, bot_login))
        .map(|comment| comment.body)
        .collect();
    let records = collect_records(&trusted);

    Ok(plan_schedule(
        &ScheduleObservation {
            schedule_issue: issue.number,
            labels: authorized.labels(),
            created_at: authorized.created_at(),
            spec: &spec,
            records: &records,
            work_label: &work_label,
            creator_login: &reg.creator_login,
        },
        now,
        cfg,
    ))
}

/// Whether a comment author is the configured App.
///
/// GitHub renders an App's author login as `<slug>[bot]` in most contexts but not
/// all, so the comparison tolerates the suffix in either direction — the same
/// leniency [`crate::session_access::policy`] applies to the App system principal.
pub fn comment_is_from_bot(author: &str, bot_login: &str) -> bool {
    let normalize = |login: &str| login.trim().trim_end_matches("[bot]").to_ascii_lowercase();
    !author.is_empty() && normalize(author) == normalize(bot_login)
}

/// The effective work label a run issue carries, derived from the RUN MODE.
///
/// A run issue is work for the workflow runner and for nothing else, so it carries
/// the runner's own label family rather than the session's work label. Deriving it
/// from the run mode instead of the session has three consequences that together
/// are the fix for #5890:
///
///  * A run never looks like ordinary development work. Carrying the session label
///    meant carrying `fkst-dev` in any deployment that mandates the devloop
///    adapters, so the dev intake — which has no knowledge of the run-issue marker
///    and gates on labels BEFORE it reads a body — admitted every run as something
///    to triage and implement.
///  * A session no longer has to resolve to exactly ONE work label to run a
///    schedule. That requirement rejected every session in a deployment whose
///    mandatory package set declares three, which was all of them.
///  * A repeating cadence is distinguishable from a one-shot at a glance.
///
/// Both labels are reserved, so a session may not adopt either name and neither
/// participates in collision detection.
pub fn run_issue_work_label(run_mode: &RunMode, cfg: &ReconcileConfig) -> Result<String, String> {
    let logical = match run_mode {
        RunMode::Once => WORKFLOW_RUN_LABEL,
        RunMode::Cron(_) => WORKFLOW_SCHEDULED_RUN_LABEL,
    };
    let effective = apply_work_label_namespace(
        std::slice::from_ref(&logical.to_string()),
        cfg.work_label_namespace.as_deref(),
    )
    .map_err(|error| format!("the run-issue work label is unusable: {error}"))?;
    effective
        .logical_to_effective
        .get(logical)
        .cloned()
        .ok_or_else(|| format!("the run-issue work label `{logical}` did not resolve"))
}

/// The effective work label a MANUAL run's issue must carry, after checking the
/// creator actually has a session to run it.
///
/// Shares [`run_issue_work_label`] with the clock rather than approximating it: a
/// manual run that routed somewhere a scheduled one would not is exactly the class
/// of surprise the fail-closed label rules exist to prevent.
///
/// The registration walk is now purely a PRECONDITION CHECK. It used to also drive
/// the label — expanding the manifest package set and discovering each package's
/// declared labels — but the run-issue label no longer depends on the session, so
/// that work is gone. What remains is worth keeping: dispatching a run for a
/// creator with no session would create an issue nothing ever wakes on, and
/// silence is a worse answer than a refusal that says why.
pub async fn resolve_manual_run_label(
    repo: &RepoRef,
    triggers: &[IssueSummary],
    creator_login: &str,
    run_mode: &RunMode,
    cfg: &ReconcileConfig,
) -> Result<String, String> {
    let mut regs: Vec<SessionRegistration> = triggers
        .iter()
        .filter_map(|issue| {
            let creator = match crate::reconcile::effective_creator(
                &issue.metadata(),
                cfg.github_bot_login.as_deref(),
            ) {
                crate::reconcile::CreatorResolution::Resolved(creator) => creator,
                crate::reconcile::CreatorResolution::Unattributable { .. } => return None,
            };
            // The installation id only seeds the deterministic session id, which
            // is used here purely as a lookup key within this one call — never
            // persisted, never compared with a real session's id.
            crate::reconcile::parse_registration(0, repo, issue, creator).ok()
        })
        .filter(|reg| reg.creator_login.eq_ignore_ascii_case(creator_login))
        .collect();
    // Same tie-break as the schedule pass: the lowest trigger issue owns it.
    regs.sort_by_key(|reg| reg.trigger_issue);
    if regs.is_empty() {
        return Err(format!(
            "no active session on this repository is owned by {creator_login}"
        ));
    }

    run_issue_work_label(run_mode, cfg)
}

#[cfg(test)]
#[path = "schedule_pass_tests.rs"]
mod tests;
