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

use std::collections::{BTreeSet, HashMap, HashSet};

use k8s_openapi::chrono::Utc;
use secrecy::SecretString;

use crate::error::AppError;
use crate::log_access::LogSessionContext;
use crate::models::RepoRef;
use crate::reconcile::announce::parse_config_hash_marker;
use crate::reconcile::collision::detect_work_label_collisions;
use crate::reconcile::desired::{plan_repo, SessionRegistration};
use crate::reconcile::execute::{execute, ReconcileCtx};
use crate::reconcile::pending::{LabelCountPending, PendingWork};
use crate::reconcile::registry::parse_registration;
use crate::reconcile::work_authz::WorkAuthz;

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
        // Deployment-wide access policy: a trigger issue authored by a user who
        // is not allowlisted is IGNORED before parsing — no registration (so no
        // spawn, on ANY path: webhook nudge or full resync), and no invalid-config
        // marker either (an unauthorized author gets zero service interaction).
        // Removing an author from the list de-desires their sessions: the live pod
        // becomes an orphan the planner tears down on the next reconcile.
        if !ctx.config.access.allows(issue.user_id, &issue.user_login) {
            tracing::info!(
                repo = %format!("{}/{}", repo.owner, repo.name),
                issue = issue.number,
                author_id = issue.user_id,
                "access policy: trigger issue author not allowlisted; ignoring"
            );
            continue;
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

    // R3 work-issue AUTHORITY gate (epic #572). THREE states, resolved once per pass
    // and shared by the reject surface + the pending gate:
    //   - flag OFF                        -> WorkAuthz::off()      (byte-identical pre-R3)
    //   - flag ON + admin lookup OK       -> enforce w/ the admin set
    //   - flag ON + admin lookup FAILED   -> enforce w/ an EMPTY admin set: author ∪
    //     collaborators (which need no API) are STILL enforced, so strangers are
    //     still rejected during a transient blip; only the admin tier is unavailable
    //     that pass (it recovers next sweep). Never a full fail-open.
    // The admin fetch is skipped entirely (and stays off) when the flag is off or the
    // repo has no registrations — one fetch per repo per pass, mirroring the
    // per-config-hash work-label cache.
    let authz = if cfg.enforce_work_issue_authz && !regs.is_empty() {
        match ctx
            .listing
            .list_repo_admins(&token, &repo.owner, &repo.name)
            .await
        {
            Ok(admins) => WorkAuthz::enforcing(admins),
            Err(error) => {
                tracing::warn!(
                    owner_repo = %owner_repo,
                    error = %error,
                    "reconcile: repo-admin lookup failed; enforcing author+collaborators only this pass (admin tier unavailable)"
                );
                WorkAuthz::enforcing(Vec::new())
            }
        }
    } else {
        WorkAuthz::off()
    };

    // Resolve each session's FULL work-label set (explicit ∪ package-discovered) ONCE
    // for this pass, keyed by session id. Shared by the reject surface (so it rejects
    // over the same set the pending gate authorizes over — no asymmetry) and the
    // pending gate below. Discovered labels are cached per config-hash (packages are
    // immutable per session config), bounding the manifest fetches to one resolve per
    // distinct session config.
    let work_labels_by_session = resolve_work_label_sets(ctx, &token, &regs).await;

    // R4a work-label collision backstop (epic #572). A trigger issue can be created
    // directly on GitHub, so this server-side guard — not the authoring convention — is
    // the real guarantee that two active sessions never compete over the same
    // work-label queue on one repo. Among the OPEN, otherwise-valid registrations the
    // lowest-trigger-issue holder OWNS each shared label; every loser is DEMOTED into
    // the `invalid` set here, so it flows through the SAME flag/comment/auto-clear path
    // as a parse failure (it un-flags itself the moment the collision resolves and it
    // becomes a plain valid registration again). Removing losers from `regs` before the
    // pending gate + planner is what actually blocks the competing pod from spawning.
    let collisions = detect_work_label_collisions(&regs, &work_labels_by_session);
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
        &work_labels_by_session,
        &authz,
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
    let gate = LabelCountPending::new(ctx.listing.as_ref(), &token);
    let mut pending: HashMap<String, bool> = HashMap::new();
    for reg in &regs {
        let labels = work_labels_by_session
            .get(&reg.session_id)
            .cloned()
            .unwrap_or_default();
        // When enforcing, a session is pending only while it has an OPEN work-label
        // issue raised by an AUTHORIZED author; otherwise the cheap author-blind
        // Search count (byte-identical pre-R3 behavior).
        let is_pending = if authz.enforce {
            gate.has_pending_authorized(installation_id, repo, &labels, reg, &authz.admins)
                .await?
        } else {
            gate.has_pending(installation_id, repo, &labels).await?
        };
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

/// Resolve every registration's FULL work-label set (explicit `### Work Label` ∪ the
/// labels its packages auto-declare) into a `session_id -> labels` map, once per pass.
/// Discovered labels are cached per `config_hash` (packages are immutable per session
/// config), so the manifest fetches are bounded to one resolve per distinct config.
/// Shared by the ack/reject surface and the pending gate so both act on the same set.
async fn resolve_work_label_sets(
    ctx: &ReconcileCtx,
    token: &SecretString,
    regs: &[SessionRegistration],
) -> HashMap<String, Vec<String>> {
    let mut discovered_cache: HashMap<String, BTreeSet<String>> = HashMap::new();
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for reg in regs {
        let discovered = match discovered_cache.get(&reg.config_hash) {
            Some(set) => set.clone(),
            None => {
                let set = crate::reconcile::work_labels::resolve_work_labels(
                    &ctx.http,
                    &ctx.config.github_api_base_url,
                    token,
                    &reg.def.packages,
                )
                .await;
                discovered_cache.insert(reg.config_hash.clone(), set.clone());
                set
            }
        };
        let mut labels: BTreeSet<String> = discovered;
        if let Some(wl) = &reg.def.work_label {
            labels.insert(wl.clone());
        }
        out.insert(reg.session_id.clone(), labels.into_iter().collect());
    }
    out
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
