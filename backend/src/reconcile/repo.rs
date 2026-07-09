//! The per-repo reconcile driver (issue #359 §4.2, PR5b).
//!
//! Gathers the desired + observed state for ONE repository and drives it to
//! agreement: enumerate the open trigger issues → parse each into a registration
//! (or an invalid marker), observe the live substrate-session pods (through the
//! session backend, projected to the planner's
//! [`LivePod`](crate::reconcile::desired::LivePod) view), gate each registration on
//! its work label's open count, then run the pure planner and execute the actions.
//!
//! Error discipline: any GitHub/Kubernetes READ that fails aborts the WHOLE repo
//! with an `Err` (so no plan is ever executed on partial data — the loop logs it
//! and retries next sweep). Per-ACTION effects are best-effort inside [`execute`],
//! which never propagates, so one bad action never blocks the rest.

use std::collections::{HashMap, HashSet};

use k8s_openapi::chrono::Utc;

use crate::error::AppError;
use crate::log_access::LogSessionContext;
use crate::models::RepoRef;
use crate::reconcile::announce::parse_config_hash_marker;
use crate::reconcile::desired::{plan_repo, SessionRegistration};
use crate::reconcile::execute::{execute, ReconcileCtx};
use crate::reconcile::pending::{LabelCountPending, PendingWork};
use crate::reconcile::registry::parse_registration;

use super::{SUBSTRATE_ANNOUNCED_LABEL, SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_INVALID_LABEL};

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
    crate::reconcile::ensure_issue_templates(
        (installation_id, repo.clone()),
        &owner_repo,
        &ctx.github,
        &ctx.ensured_templates,
    )
    .await;

    // 1. One repo-scoped installation token drives every GitHub read below.
    let token = ctx.github.token_for_repo(&owner_repo, None).await?;

    // 2. Enumerate the open trigger issues, splitting valid registrations from
    //    invalid markers and recording which issues already carry the invalid flag.
    let issues = ctx
        .listing
        .list_issues_by_label(
            &token,
            &repo.owner,
            &repo.name,
            &cfg.substrate_trigger_label,
        )
        .await?;
    let mut regs = Vec::new();
    let mut invalid: Vec<(i64, String)> = Vec::new();
    let mut latched_invalid: HashSet<i64> = HashSet::new();
    let mut latched_announced: HashSet<i64> = HashSet::new();
    let mut latched_config_rejected: HashSet<i64> = HashSet::new();
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
        match parse_registration(installation_id, repo, issue) {
            Ok(reg) => regs.push(reg),
            Err(marker) => invalid.push(marker),
        }
    }

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

    // Record each valid session's log-access context so the identity-gated
    // log-download endpoint can reverse a `session_id` (a one-way hash) to the
    // author id + `### Log Access Allowlist` allow-list it authorizes against. Cheap in-memory
    // upsert; carries only public metadata (ids + the allow-list), never a token.
    record_log_contexts(ctx, &regs);

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

    // Best-effort, non-failing: give each open WORK issue (one carrying a session's
    // work label) a visible, fkst-owned "picked up" acknowledgment — a work issue is
    // otherwise often silent from GitHub's side, so the author has no signal it was
    // claimed. Mirrors the announce latch (comment + durable label), reuses the token
    // minted above, and gates on ≥1 registration; a failure here never aborts the
    // reconcile.
    crate::reconcile::work_ack::ack_open_work_issues(
        &ctx.github,
        ctx.listing.as_ref(),
        &token,
        repo,
        &regs,
    )
    .await;

    // 3. Observe the live pods for this repo (through the session backend).
    let live = ctx
        .backend
        .observe_repo(repo)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("list substrate-session pods: {e}")))?;

    // 4. Gate each registration on its work label's open-issue count.
    let gate = LabelCountPending::new(ctx.listing.as_ref(), &token);
    let mut pending: HashMap<String, bool> = HashMap::new();
    for reg in &regs {
        let is_pending = gate
            .has_pending(installation_id, repo, &reg.def.work_label)
            .await?;
        pending.insert(reg.session_id.clone(), is_pending);
    }

    // 5. Plan (pure), then execute each action best-effort.
    let actions = plan_repo(
        &regs,
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
    tracing::info!(
        owner_repo = %owner_repo,
        registrations = regs.len(),
        invalid = invalid.len(),
        live_pods = live.len(),
        actions = actions.len(),
        "reconcile repo: planned"
    );
    for action in actions {
        execute(action, repo, ctx).await;
    }
    Ok(())
}

/// Upsert every valid registration's [`LogSessionContext`] into the shared registry
/// the log-download endpoint authorizes against. Called every sweep so the map stays
/// current (a re-registration with an edited allow-list overwrites the old context);
/// carries only public metadata, never a token.
fn record_log_contexts(ctx: &ReconcileCtx, regs: &[SessionRegistration]) {
    for reg in regs {
        ctx.log_registry.upsert(
            reg.session_id.clone(),
            LogSessionContext {
                installation_id: reg.installation_id,
                repo: reg.repo.clone(),
                trigger_issue: reg.trigger_issue,
                author_id: reg.trigger_author_id,
                log_access: reg.log_access.clone(),
            },
        );
    }
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
