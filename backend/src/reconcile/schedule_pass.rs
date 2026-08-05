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
use crate::goals::scheduled_workflow_parse::parse_scheduled_workflow;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::reserved_labels::{is_reserved_label, SCHEDULED_WORKFLOW_LABEL};
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
    logical_work_labels: &HashMap<String, Vec<String>>,
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
            logical_work_labels,
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
    logical_work_labels: &HashMap<String, Vec<String>>,
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

    let work_label = match resolve_work_label(reg, logical_work_labels, cfg) {
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

/// Which single work label a definition's run issues carry.
///
/// Fail-closed when ambiguous rather than guessing: a run issue with the wrong
/// label wakes the wrong session, or no session at all, and the author would have
/// no way to tell which happened.
pub fn resolve_work_label(
    reg: &SessionRegistration,
    logical_work_labels: &HashMap<String, Vec<String>>,
    cfg: &ReconcileConfig,
) -> Result<String, String> {
    let logical = logical_work_labels
        .get(&reg.session_id)
        .cloned()
        .unwrap_or_default();
    let effective = apply_work_label_namespace(&logical, cfg.work_label_namespace.as_deref())
        .map_err(|error| format!("the session's work labels are unusable: {error}"))?;

    // An explicit `### Work Label` on the trigger is the author's own statement of
    // which queue this session serves, so it wins over discovery.
    if let Some(declared) = &reg.def.work_label {
        return effective
            .logical_to_effective
            .get(declared)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "session #{} declares work label `{declared}` but it did not resolve to an \
                     effective label",
                    reg.trigger_issue
                )
            });
    }

    let candidates: Vec<&String> = effective
        .logical
        .iter()
        .filter(|label| !is_reserved_label(label))
        .collect();
    match candidates.as_slice() {
        [single] => effective
            .logical_to_effective
            .get(*single)
            .cloned()
            .ok_or_else(|| "the discovered work label did not resolve".to_string()),
        [] => Err(format!(
            "session #{} has no work label, so a scheduled run has nothing to route to. Add a \
             `### Work Label` to that trigger issue.",
            reg.trigger_issue
        )),
        many => Err(format!(
            "session #{} has {} work labels ({}), so a scheduled run cannot pick one. Add an \
             explicit `### Work Label` to that trigger issue.",
            reg.trigger_issue,
            many.len(),
            many.iter()
                .map(|label| label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// The effective work label a MANUAL run's issue must carry.
///
/// Shares [`resolve_work_label`] with the clock rather than approximating it. A
/// manual run that routed somewhere a scheduled one would not is exactly the class
/// of surprise the fail-closed label rules exist to prevent, so this resolves from
/// the same inputs — the creator's registrations and their manifest-expanded
/// work-label sets — and fails closed for the same reasons.
pub async fn resolve_manual_run_label(
    http: &reqwest::Client,
    api_base: &str,
    token: &SecretString,
    repo: &RepoRef,
    triggers: &[IssueSummary],
    creator_login: &str,
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
    let Some(mut reg) = regs.into_iter().next() else {
        return Err(format!(
            "no active session on this repository is owned by {creator_login}"
        ));
    };

    // Discovery must walk the MANIFEST-EXPANDED package set, exactly as the
    // reconciler does, or a manifest-driven session's labels would be invisible.
    let expanded = crate::reconcile::effective_packages::resolve_effective_packages(
        http,
        api_base,
        token,
        std::slice::from_ref(&reg),
        &cfg.mandatory_packages,
    )
    .await;
    if let Some(packages) = expanded.by_session.get(&reg.session_id) {
        reg.effective_packages = packages.clone();
    }
    let logical = crate::reconcile::work_labels::resolve_work_label_sets(
        http,
        api_base,
        token,
        std::slice::from_ref(&reg),
    )
    .await;
    resolve_work_label(&reg, &logical, cfg)
}

#[cfg(test)]
#[path = "schedule_pass_tests.rs"]
mod tests;
