//! The per-repo reconcile driver (issue #359 §4.2, PR5b).
//!
//! Gathers the desired + observed state for ONE repository and drives it to
//! agreement: enumerate the open trigger issues → authorize each effective creator
//! from metadata → parse accepted issues into registrations (or invalid markers),
//! observe the live substrate-session pods (through the
//! session backend, projected to the planner's
//! [`LivePod`](crate::reconcile::desired::LivePod) view), gate each registration on
//! its work label's open count, then run the pure planner and execute the actions.
//!
//! Error discipline: any GitHub/Kubernetes READ that fails aborts the WHOLE repo
//! with an `Err` (so no plan is ever executed on partial data — the loop logs it
//! and retries next sweep). Ordinary action effects remain best-effort; incomplete
//! orphan retirement deliberately stops the pass while keeping its runtime as the
//! next sweep's retry owner.

use std::collections::{HashMap, HashSet};

use k8s_openapi::chrono::Utc;
use secrecy::SecretString;

use crate::access_policy::AccessPolicy;
use crate::error::AppError;
use crate::github_app::listing::{GithubListing, IssueSummary};
use crate::models::RepoRef;
use crate::reconcile::announce::parse_config_hash_marker;
use crate::reconcile::collision::{detect_missing_work_labels, detect_work_label_collisions};
use crate::reconcile::creator::{effective_creator, CreatorResolution, SessionCreator};
use crate::reconcile::desired::{plan_repo, plan_trigger_authorization, SessionRegistration};
use crate::reconcile::effective_packages::EffectivePackages;
use crate::reconcile::execute::{execute, ReconcileCtx};
use crate::reconcile::pending::{LabelCountPending, PendingWork};
use crate::reconcile::registry::parse_registration;
use crate::reconcile::session_contexts::record_session_contexts;
use crate::reconcile::trigger_authz::{
    check_trigger_creator, TriggerAuthzCache, TriggerGateDecision,
};
use crate::reconcile::work_labels::{apply_work_label_namespace, resolve_work_label_sets};

use super::{
    SUBSTRATE_ANNOUNCED_LABEL, SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_INVALID_LABEL,
    TRIGGER_UNAUTHORIZED_LABEL,
};

#[derive(Default)]
struct ClassifiedTriggers {
    registrations: Vec<SessionRegistration>,
    invalid: Vec<(i64, String)>,
    unauthorized: Vec<(i64, String)>,
    /// Issues that definitively passed the creator gate this pass. Kept separate
    /// from registrations because a passed trigger may later be demoted by parsing,
    /// manifest, label, or collision validation and must still clear an old auth latch.
    authorized_issues: HashSet<i64>,
}

/// Apply the deployment access policy (to the issue AUTHOR and to the resolved
/// EFFECTIVE creator) and the metadata-only creator-authority gate before
/// parsing. A transport-deferred, previously announced trigger is parsed to
/// preserve its live desired state; an unlatched deferred trigger is skipped
/// without feedback.
#[allow(clippy::too_many_arguments)]
async fn classify_triggers(
    installation_id: i64,
    repo: &RepoRef,
    issues: &[IssueSummary],
    listing: &dyn GithubListing,
    token: &SecretString,
    access: &AccessPolicy,
    bot_login: Option<&str>,
    latched_announced: &HashSet<i64>,
) -> ClassifiedTriggers {
    let mut classified = ClassifiedTriggers::default();
    let mut authz_cache = TriggerAuthzCache::default();

    for issue in issues {
        // The deployment-wide access policy stays first and deliberately silent.
        if !access.allows(issue.user_id, &issue.user_login) {
            tracing::info!(
                repo = %format!("{}/{}", repo.owner, repo.name),
                issue = issue.number,
                author_id = issue.user_id,
                "access policy: trigger issue author not admitted; ignoring"
            );
            continue;
        }

        let creator = match effective_creator(&issue.metadata(), bot_login) {
            CreatorResolution::Resolved(creator) => creator,
            CreatorResolution::Unattributable { assignee_count, .. } => {
                classified.unauthorized.push((
                    issue.number,
                    format!(
                        "a bot-authored trigger must have exactly one assignee (found {assignee_count}) to attribute a session creator"
                    ),
                ));
                continue;
            }
        };

        // The access policy applies to the EFFECTIVE creator too: an App-authored
        // trigger's author is the bot, which a denylist always admits — without
        // this gate a blocked user could own sessions via seeded triggers, and
        // blocking a seeded session's creator would never revoke it. Dropping the
        // registration here makes the planner orphan its pod on the next
        // reconcile, exactly like the author gate above (silent by design).
        // Issue metadata carries no assignee id, so an assignee-derived creator
        // matches by LOGIN only (`-1` can never match an all-digit id entry) —
        // list blocked users by login to cover seeded creators.
        if !access.allows(creator.id.unwrap_or(-1), &creator.login) {
            tracing::info!(
                repo = %format!("{}/{}", repo.owner, repo.name),
                issue = issue.number,
                "access policy: trigger's effective creator not admitted; ignoring"
            );
            continue;
        }

        match check_trigger_creator(listing, token, repo, access, &creator, &mut authz_cache).await
        {
            TriggerGateDecision::Authorized => {
                classified.authorized_issues.insert(issue.number);
                parse_classified_registration(
                    installation_id,
                    repo,
                    issue,
                    creator,
                    &mut classified,
                );
            }
            TriggerGateDecision::Unauthorized { reason } => {
                classified.unauthorized.push((issue.number, reason));
            }
            TriggerGateDecision::Deferred if latched_announced.contains(&issue.number) => {
                tracing::warn!(
                    repo = %format!("{}/{}", repo.owner, repo.name),
                    issue = issue.number,
                    "trigger creator authorization deferred; preserving announced registration this pass"
                );
                parse_classified_registration(
                    installation_id,
                    repo,
                    issue,
                    creator,
                    &mut classified,
                );
            }
            TriggerGateDecision::Deferred => {}
        }
    }
    classified
}

fn parse_classified_registration(
    installation_id: i64,
    repo: &RepoRef,
    issue: &IssueSummary,
    creator: SessionCreator,
    classified: &mut ClassifiedTriggers,
) {
    match parse_registration(installation_id, repo, issue, creator) {
        Ok(registration) => classified.registrations.push(registration),
        Err(marker) => classified.invalid.push(marker),
    }
}

/// Reconcile ONE repository against its open trigger issues + live pods.
pub async fn reconcile_repo(
    installation_id: i64,
    repo: &RepoRef,
    ctx: &ReconcileCtx,
) -> Result<(), AppError> {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let cfg = &ctx.config.reconcile;

    // Best-effort, non-failing: keep this repo's issue templates at the bundled
    // version. Gated (one round-trip per repo per version/TTL) so it is a cheap
    // no-op on the vast majority of reconciles. Placed BEFORE the fallible reads
    // below so a later GitHub/K8s read failure (which `?`-returns) never skips the
    // template ensure; a failure inside the ensure never aborts the reconcile.
    let templates_installed = crate::reconcile::ensure_issue_templates(
        (installation_id, repo.clone()),
        &owner_repo,
        &ctx.github,
        &ctx.ensured_templates,
    )
    .await;
    // Ride along with the (rare) template install rather than paying six label
    // round-trips per repo per sweep. A repository that already has the current
    // templates already had this run; the version bump that ships a new template
    // is what carries the bootstrap out to the existing fleet.
    if templates_installed {
        crate::github_app::labels::ensure_platform_labels(&ctx.github, &owner_repo).await;
    }

    // 1. One repo-scoped installation token drives every GitHub read below.
    let token = ctx.github.token_for_repo(&owner_repo, None).await?;

    // 2. Enumerate open triggers and read their durable latch state before the
    //    metadata-only creator gate. Trigger bodies are parsed only after that gate.
    let issues = ctx
        .listing
        .list_issues_by_label(
            &token,
            &repo.owner,
            &repo.name,
            &cfg.substrate_trigger_label,
        )
        .await?;
    let mut latched_invalid: HashSet<i64> = HashSet::new();
    let mut latched_announced: HashSet<i64> = HashSet::new();
    let mut latched_config_rejected: HashSet<i64> = HashSet::new();
    let mut latched_trigger_unauthorized: HashSet<i64> = HashSet::new();
    for issue in &issues {
        if issue.labels.iter().any(|l| l == SUBSTRATE_INVALID_LABEL) {
            latched_invalid.insert(issue.number);
        }
        if issue.labels.iter().any(|l| l == SUBSTRATE_ANNOUNCED_LABEL) {
            latched_announced.insert(issue.number);
        }
        if issue
            .labels
            .iter()
            .any(|l| l == SUBSTRATE_CONFIG_REJECTED_LABEL)
        {
            latched_config_rejected.insert(issue.number);
        }
        if issue
            .labels
            .iter()
            .any(|label| label == TRIGGER_UNAUTHORIZED_LABEL)
        {
            latched_trigger_unauthorized.insert(issue.number);
        }
    }
    let ClassifiedTriggers {
        registrations: mut regs,
        mut invalid,
        unauthorized: trigger_unauthorized,
        authorized_issues,
    } = classify_triggers(
        installation_id,
        repo,
        &issues,
        ctx.listing.as_ref(),
        &token,
        &ctx.config.access,
        cfg.github_bot_login.as_deref(),
        &latched_announced,
    )
    .await;

    // Config immutability: for each ANNOUNCED trigger, recover the ORIGINAL
    // full_config_hash latched (as a hidden marker) in its announcement comment so the
    // planner can reject a later edit. Bounded to announced triggers (a pre-announce
    // trigger has no marker yet — this also caps the added API cost), and best-effort:
    // a comment-list failure just skips that trigger this cycle (retried next
    // reconcile) rather than aborting the whole repo — this additive check must never
    // wedge the core pod reconcile.
    let mut latched_config_hash: HashMap<i64, String> = HashMap::new();
    for reg in &regs {
        if !latched_announced.contains(&reg.trigger_issue) {
            continue;
        }
        match ctx
            .github
            .list_issue_comments(&owner_repo, reg.trigger_issue as u64)
            .await
        {
            Ok(comments) => {
                if let Some(original) = parse_config_hash_marker(&comments) {
                    latched_config_hash.insert(reg.trigger_issue, original);
                }
            }
            Err(error) => {
                tracing::warn!(
                    owner_repo = %owner_repo,
                    issue = reg.trigger_issue,
                    error = %error,
                    "reconcile: config-hash marker fetch failed; skipping immutability check this cycle"
                );
            }
        }
    }

    // Track this repo in the shared active-repos set so the sweep keeps reconciling
    // it while it has ≥1 registration (closes the first-spawn/search-lag gap); drop
    // it when the last trigger issue is gone so idle repos don't churn the sweep.
    set_active(ctx, installation_id, repo, !regs.is_empty());

    // Publish this repository's COMPLETE session-access set so every
    // session-scoped route can reverse a `session_id` (a one-way hash) into the
    // creator + collaborator + log-access facts it authorizes against. Cheap
    // in-memory replacement; carries only public metadata, never a token.
    record_session_contexts(ctx, installation_id, repo, &regs);

    // I7 manifest expand pass (epic #594). Resolve each session's EFFECTIVE package set —
    // its explicit `### Packages` followed by every `### Manifest` reference expanded into
    // packages (deduped, explicit-first) — using the repo installation token minted above.
    // FAIL-CLOSED: a session whose manifest cannot be fetched/parsed/validated, or whose
    // effective set comes out empty, is DEMOTED into `invalid` here (the SAME
    // flag/comment/auto-clear path as a collision or a missing label) and removed from
    // `regs` before it can spawn. The resolved set is stamped onto each SURVIVING reg so
    // every downstream consumer (reachability, package roots, work-label discovery) reads
    // it. Done BEFORE resolve_work_label_sets so label discovery walks the effective set,
    // surfacing a manifest package's `[github].work_labels`.
    let EffectivePackages {
        by_session: effective_by_session,
        demotions: manifest_demotions,
        package_env_by_session,
    } = crate::reconcile::effective_packages::resolve_effective_packages(
        &ctx.http,
        &ctx.config.github_api_base_url,
        &token,
        &regs,
        &cfg.mandatory_packages,
    )
    .await;
    if !manifest_demotions.is_empty() {
        let losers: HashSet<i64> = manifest_demotions.iter().map(|(issue, _)| *issue).collect();
        tracing::info!(
            owner_repo = %owner_repo,
            demoted = manifest_demotions.len(),
            "reconcile: manifest expansion failed for trigger(s); demoting to invalid"
        );
        regs.retain(|reg| !losers.contains(&reg.trigger_issue));
        invalid.extend(manifest_demotions);
    }
    for reg in &mut regs {
        if let Some(packages) = effective_by_session.get(&reg.session_id) {
            reg.effective_packages = packages.clone();
        }
        // The effective (manifest-merged) configuration replaces the trigger-only
        // map parsed at registration, so every downstream consumer reads one value
        // and never has to re-apply precedence.
        if let Some(package_env) = package_env_by_session.get(&reg.session_id) {
            reg.effective_package_env = package_env.clone();
        }
    }

    // Resolve each session's FULL work-label set (explicit ∪ package-discovered) ONCE
    // for this pass, keyed by session id. Shared by the reject surface (so it rejects
    // over the same set the pending gate authorizes over — no asymmetry) and the
    // pending gate below. Discovered labels are cached per config-hash (packages are
    // immutable per session config), bounding the manifest fetches to one resolve per
    // distinct session config. Walks each session's EFFECTIVE package set (I7), so a
    // manifest's packages' `[github].work_labels` are auto-discovered too.
    let logical_work_labels_by_session =
        resolve_work_label_sets(&ctx.http, &ctx.config.github_api_base_url, &token, &regs).await;

    // Apply the deployment/provider namespace after package discovery. Trigger bodies
    // and package manifests stay provider-neutral; every GitHub-facing operation below
    // uses only this effective set. Invalid/overlong labels and case-insensitive output
    // collisions fail closed through the ordinary invalid-trigger latch.
    let mut effective_work_labels_by_session: HashMap<String, Vec<String>> = HashMap::new();
    let mut work_label_demotions = Vec::new();
    for reg in &regs {
        let logical = logical_work_labels_by_session
            .get(&reg.session_id)
            .cloned()
            .unwrap_or_default();
        match apply_work_label_namespace(&logical, cfg.work_label_namespace.as_deref()) {
            Ok(labels) => {
                effective_work_labels_by_session.insert(reg.session_id.clone(), labels.effective);
            }
            Err(error) => work_label_demotions.push((
                reg.trigger_issue,
                format!("invalid effective work labels: {error}"),
            )),
        }
    }
    if !work_label_demotions.is_empty() {
        let losers: HashSet<i64> = work_label_demotions
            .iter()
            .map(|(issue, _)| *issue)
            .collect();
        tracing::info!(
            owner_repo = %owner_repo,
            demoted = work_label_demotions.len(),
            "reconcile: effective work-label validation failed; demoting trigger(s) to invalid"
        );
        regs.retain(|reg| !losers.contains(&reg.trigger_issue));
        invalid.extend(work_label_demotions);
    }

    // I4 label-less reject (epic #594). A session whose EFFECTIVE work-label set is empty
    // (no explicit `### Work Label` AND no package-declared `[github].work_labels`) can
    // never be woken, so it is DEMOTED into the `invalid` set here — flowing through the
    // SAME flag/comment/auto-clear path as a parse failure or collision, and auto-clearing
    // the moment a work label appears. Done BEFORE the collision + pending gate so a
    // label-less trigger is removed from `regs` up front (a label-less session shares no
    // queue, so it never collides anyway; ordering it first keeps its reason precise). A
    // spawned session therefore always carries ≥1 work label, keeping the in-pod guard
    // satisfied.
    let missing = detect_missing_work_labels(&regs, &logical_work_labels_by_session);
    if !missing.is_empty() {
        let losers: HashSet<i64> = missing.iter().map(|(issue, _)| *issue).collect();
        tracing::info!(
            owner_repo = %owner_repo,
            demoted = missing.len(),
            "reconcile: label-less trigger(s) detected; demoting to invalid"
        );
        regs.retain(|reg| !losers.contains(&reg.trigger_issue));
        invalid.extend(missing);
    }

    // R4a work-label collision backstop (epic #572). A trigger issue can be created
    // directly on GitHub, so this server-side guard — not the authoring convention — is
    // the real guarantee that two active sessions never compete over the same
    // work-label queue on one repo. Among the OPEN, otherwise-valid registrations the
    // lowest-trigger-issue holder OWNS each shared label; every loser is DEMOTED into
    // the `invalid` set here, so it flows through the SAME flag/comment/auto-clear path
    // as a parse failure (it un-flags itself the moment the collision resolves and it
    // becomes a plain valid registration again). Removing losers from `regs` before the
    // pending gate + planner is what actually blocks the competing pod from spawning.
    let collisions = detect_work_label_collisions(&regs, &effective_work_labels_by_session);
    if !collisions.is_empty() {
        let losers: HashSet<i64> = collisions.iter().map(|(issue, _)| *issue).collect();
        tracing::info!(
            owner_repo = %owner_repo,
            demoted = collisions.len(),
            "reconcile: work-label collision(s) detected; demoting the losing trigger(s) to invalid"
        );
        regs.retain(|reg| !losers.contains(&reg.trigger_issue));
        invalid.extend(collisions);
    }

    // Validate each source branch in the snapshot phase so a missing ref uses
    // the durable invalid latch (one comment, then automatic recovery). Any
    // transport error aborts the whole repo pass; planning from a partial branch
    // snapshot could otherwise spawn a session with unverified inputs.
    let mut default_branch: Option<String> = None;
    let mut missing_sources: Vec<(i64, String)> = Vec::new();
    for reg in &regs {
        let source = match &reg.def.source_branch {
            Some(source) => source.clone(),
            None => match &default_branch {
                Some(source) => source.clone(),
                None => {
                    let source = ctx.github.repo_default_branch(&owner_repo).await?;
                    default_branch = Some(source.clone());
                    source
                }
            },
        };
        if ctx
            .github
            .branch_head_sha(&owner_repo, &source)
            .await?
            .is_none()
        {
            missing_sources.push((
                reg.trigger_issue,
                format!(
                    "source branch '{source}' was not found on {}/{}",
                    repo.owner, repo.name
                ),
            ));
        }
    }
    if !missing_sources.is_empty() {
        let losers: HashSet<i64> = missing_sources.iter().map(|(issue, _)| *issue).collect();
        tracing::info!(
            owner_repo = %owner_repo,
            demoted = missing_sources.len(),
            "reconcile: missing source branch(es); demoting trigger(s) to invalid"
        );
        regs.retain(|reg| !losers.contains(&reg.trigger_issue));
        invalid.extend(missing_sources);
    }

    // Best-effort, non-failing: if ANY registered session on this repo opted into
    // auto-merge (`### Auto-merge`), merge the App bot's mergeable open PRs. Mirrors
    // the ensure_issue_templates hook — a failure here never aborts the reconcile.
    crate::reconcile::automerge::auto_merge_bot_pull_requests(
        &ctx.github,
        &owner_repo,
        cfg.github_bot_login.as_deref(),
        regs.iter().any(|r| r.auto_merge),
    )
    .await;

    // 3. Observe the live pods for this repo (through the session backend).
    let live = ctx
        .backend
        .observe_repo(repo)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("list substrate-session pods: {e}")))?;

    // 4. Gate each registration on the open-issue count of its work-label SET
    //    (`work_labels_by_session`, resolved above): the trigger's explicit label plus
    //    every label its packages auto-declare (`[github].work_labels`, resolved
    //    transitively over `[event_deps]`). So a session wakes on any of its packages'
    //    own labels without the operator restating them in the trigger issue.
    let gate = LabelCountPending::new_with_bot_login(
        ctx.listing.as_ref(),
        &token,
        cfg.github_bot_login.as_deref(),
    );
    let mut pending: HashMap<String, bool> = HashMap::new();
    for reg in &regs {
        let labels = effective_work_labels_by_session
            .get(&reg.session_id)
            .cloned()
            .unwrap_or_default();
        let is_pending = gate
            .has_pending(installation_id, repo, &labels, reg, &ctx.config.access)
            .await?;
        pending.insert(reg.session_id.clone(), is_pending);
    }

    // 5. Plan (pure), then execute each action best-effort.
    let mut actions = plan_repo(
        &regs,
        &logical_work_labels_by_session,
        &invalid,
        &live,
        &pending,
        &latched_invalid,
        &latched_announced,
        &latched_config_hash,
        &latched_config_rejected,
        Utc::now(),
        cfg,
    );
    actions.extend(plan_trigger_authorization(
        &trigger_unauthorized,
        &authorized_issues,
        &latched_trigger_unauthorized,
    ));
    tracing::info!(
        owner_repo = %owner_repo,
        registrations = regs.len(),
        invalid = invalid.len(),
        trigger_unauthorized = trigger_unauthorized.len(),
        live_pods = live.len(),
        actions = actions.len(),
        "reconcile repo: planned"
    );
    let retirement_planned = actions.iter().any(|action| {
        matches!(
            action,
            crate::reconcile::desired::ReconcileAction::RetireSession { .. }
        )
    });
    let mut retirement_incomplete = false;
    for action in actions {
        if retirement_incomplete
            && !matches!(
                &action,
                crate::reconcile::desired::ReconcileAction::RetireSession { .. }
                    | crate::reconcile::desired::ReconcileAction::CleanupTerminal { .. }
            )
        {
            continue;
        }
        if !execute(action, repo, ctx).await {
            retirement_incomplete = true;
            tracing::warn!(
                owner_repo = %owner_repo,
                "reconcile repo: retirement incomplete; keeping runtime as retry owner"
            );
        }
    }
    if retirement_incomplete {
        return Ok(());
    }

    // A pass that retires an orphan never re-admits replacement work immediately.
    // The next observation must first confirm the old runtime is absent/terminating;
    // otherwise a failed stop could let that orphan re-retire the work next sweep.
    if !retirement_planned {
        crate::reconcile::work_ack::ack_open_work_issues_with_bot(
            &ctx.github,
            ctx.listing.as_ref(),
            ctx.comments.as_ref(),
            &token,
            repo,
            &regs,
            &effective_work_labels_by_session,
            &ctx.config.access,
            cfg.github_bot_login.as_deref(),
        )
        .await;
    }

    // 5b. The schedule pass: a SECOND enumeration, over this repository's open
    //     `fkst-scheduled-workflow` definitions. Deliberately not folded into
    //     `plan_repo`: it diffs a different desired state (a clock against a run
    //     history) against a different observed state (issue labels and markers),
    //     and it must not be able to change which pods the lifecycle planner
    //     spawns. It reaches sessions only the way a human does — by creating an
    //     ordinary routed work issue.
    //
    //     It runs here, and therefore only on the Lease holder, inheriting leader
    //     scoping from the reconciler rather than needing its own election.
    //
    //     Fail-soft as a WHOLE: a read failure drops this sweep's schedule effects
    //     with a warning instead of aborting the repo, because the session
    //     lifecycle above must never depend on the clock. Planning from partial
    //     reads is separately impossible — the pass itself returns `Err` rather
    //     than treating a failed history read as an empty one.
    match crate::reconcile::schedule_pass::plan_repo_schedules(
        ctx.listing.as_ref(),
        ctx.comments.as_ref(),
        &token,
        repo,
        &regs,
        &ctx.config.access,
        Utc::now(),
        cfg,
    )
    .await
    {
        Ok(effects) => {
            if !effects.is_empty() {
                tracing::info!(
                    owner_repo = %owner_repo,
                    effects = effects.len(),
                    "reconcile: schedule pass planned"
                );
            }
            for effect in effects {
                crate::reconcile::schedule_execute::execute_schedule_effect(
                    effect,
                    repo,
                    &ctx.github,
                )
                .await;
            }
        }
        Err(error) => tracing::warn!(
            owner_repo = %owner_repo,
            error = %error,
            "reconcile: schedule pass failed; retrying next sweep"
        ),
    }

    // 6. Backfill durable creator/trigger attribution onto any live runtime this
    //    pass matched to a registration but that predates the launch stamp. Uses
    //    the stamp already read in step 3, so a settled runtime costs no API call;
    //    a conflict or a permanent failure is parked by the bounded gate rather
    //    than re-decided every sweep.
    //
    //    It runs after the actions, but works from the PRE-action `live`
    //    snapshot, so a runtime an action just killed still reads as `Live` here
    //    and is patched on its way out. That is harmless by construction: the
    //    patch re-reads the runtime and a deleted one answers 404 →
    //    `RuntimeIdentityOutcome::NotFound`, which writes nothing and emits
    //    nothing. Re-observing after the actions purely to avoid that no-op would
    //    cost one extra backend LIST per repo per sweep.
    crate::reconcile::runtime_identity::backfill_runtime_identities(ctx, &regs, &live).await;
    Ok(())
}

/// Insert or remove `(installation, repo)` in the shared active-repos set (present
/// while the repo has ≥1 open trigger registration). Poison-safe: a panic elsewhere
/// while the lock is held never wedges the reconciler.
fn set_active(ctx: &ReconcileCtx, installation_id: i64, repo: &RepoRef, active: bool) {
    let key = (installation_id, repo.clone());
    let mut set = ctx.active_repos.lock().unwrap_or_else(|e| e.into_inner());
    if active {
        set.insert(key);
    } else {
        set.remove(&key);
    }
}

#[cfg(test)]
#[path = "repo_tests.rs"]
mod tests;
