//! The per-repo reconcile driver (issue #359 §4.2, PR5b).
//!
//! Gathers the desired + observed state for ONE repository and drives it to
//! agreement: enumerate the open trigger issues → parse each into a registration
//! (or an invalid marker), LIST the live substrate-session pods and project them to
//! the planner's [`LivePod`] view, gate each registration on its work label's open
//! count, then run the pure planner and execute the resulting actions.
//!
//! Error discipline: any GitHub/Kubernetes READ that fails aborts the WHOLE repo
//! with an `Err` (so no plan is ever executed on partial data — the loop logs it
//! and retries next sweep). Per-ACTION effects are best-effort inside [`execute`],
//! which never propagates, so one bad action never blocks the rest.

use std::collections::{HashMap, HashSet};

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::{DateTime, Utc};
use kube::api::{Api, ListParams};

use crate::error::AppError;
use crate::k8s::session_launcher::{
    ANNOTATION_CONFIG_HASH, ANNOTATION_INSTALLATION, ANNOTATION_LAST_PENDING_AT, ANNOTATION_OWNER,
    ANNOTATION_REPO, ANNOTATION_TRIGGER_ISSUE, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE,
    SESSION_ID_LABEL,
};
use crate::models::RepoRef;
use crate::reconcile::desired::{plan_repo, LivePod, PodLiveness};
use crate::reconcile::execute::{execute, ReconcileCtx};
use crate::reconcile::pending::{LabelCountPending, PendingWork};
use crate::reconcile::registry::parse_registration;

use super::SUBSTRATE_INVALID_LABEL;

/// Reconcile ONE repository against its open trigger issues + live pods.
pub async fn reconcile_repo(
    installation_id: i64,
    repo: &RepoRef,
    ctx: &ReconcileCtx,
) -> Result<(), AppError> {
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let cfg = &ctx.config.reconcile;

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
    for issue in &issues {
        if issue.labels.iter().any(|l| l == SUBSTRATE_INVALID_LABEL) {
            latched_invalid.insert(issue.number);
        }
        match parse_registration(installation_id, repo, issue) {
            Ok(reg) => regs.push(reg),
            Err(marker) => invalid.push(marker),
        }
    }

    // 3. Observe the live pods for this repo.
    let live = list_live_pods(ctx, repo).await?;

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

/// LIST the substrate-session pods and project the ones belonging to `repo` into
/// the planner's [`LivePod`] view.
async fn list_live_pods(ctx: &ReconcileCtx, repo: &RepoRef) -> Result<Vec<LivePod>, AppError> {
    let pods: Api<Pod> = Api::namespaced(ctx.kube.client().clone(), ctx.kube.namespace());
    let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("list substrate-session pods: {e}")))?;
    Ok(list
        .items
        .iter()
        .filter(|pod| pod_matches_repo(pod, repo))
        .filter_map(pod_to_live)
        .collect())
}

/// Read a pod annotation as `&str`, if present.
fn annotation<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(key))
        .map(String::as_str)
}

/// Recover the `(installation, repo)` reconcile key a live pod belongs to from its
/// stamped annotations. `None` when any of the three annotations is missing /
/// unparseable (the pod is not one of ours, or is malformed). Used by the sweep to
/// enqueue every repo that currently has a live pod.
pub fn repo_key_from_pod(pod: &Pod) -> Option<(i64, RepoRef)> {
    let owner = annotation(pod, ANNOTATION_OWNER)?;
    let name = annotation(pod, ANNOTATION_REPO)?;
    let installation = annotation(pod, ANNOTATION_INSTALLATION)?
        .parse::<i64>()
        .ok()?;
    Some((
        installation,
        RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
    ))
}

/// Whether a listed pod's owner/repo annotations match `repo` (the LIST selector
/// spans every repo + installation, so this scopes it to the one being reconciled).
fn pod_matches_repo(pod: &Pod, repo: &RepoRef) -> bool {
    annotation(pod, ANNOTATION_OWNER) == Some(repo.owner.as_str())
        && annotation(pod, ANNOTATION_REPO) == Some(repo.name.as_str())
}

/// Project the coarse liveness from the pod phase + deletion state: a set
/// `deletionTimestamp` always wins (Terminating); else Pending→Starting,
/// Running→Live, Succeeded/Failed→Terminal, anything else (Unknown / not-yet-set)
/// → Starting (not yet observed running).
fn phase_to_liveness(phase: Option<&str>, terminating: bool) -> PodLiveness {
    if terminating {
        return PodLiveness::Terminating;
    }
    match phase {
        Some("Running") => PodLiveness::Live,
        Some("Succeeded") | Some("Failed") => PodLiveness::Terminal,
        _ => PodLiveness::Starting,
    }
}

/// Project one pod into a [`LivePod`]. `None` when the pod carries no session-id
/// label (not one of ours / malformed) — such a pod is skipped, never planned on.
fn pod_to_live(pod: &Pod) -> Option<LivePod> {
    let session_id = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(SESSION_ID_LABEL))
        .cloned()?;

    let trigger_issue = annotation(pod, ANNOTATION_TRIGGER_ISSUE)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let terminating = pod.metadata.deletion_timestamp.is_some();
    let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
    let liveness = phase_to_liveness(phase, terminating);

    // creationTimestamp is always present on a real pod; default to now so a
    // malformed pod is treated as freshly created (shielded from idle-kill) rather
    // than instantly idle.
    let created_at = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|Time(t)| *t)
        .unwrap_or_else(Utc::now);

    let last_pending_at = annotation(pod, ANNOTATION_LAST_PENDING_AT)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let config_hash = annotation(pod, ANNOTATION_CONFIG_HASH).map(str::to_string);

    Some(LivePod {
        session_id,
        trigger_issue,
        liveness,
        created_at,
        last_pending_at,
        config_hash,
    })
}

#[cfg(test)]
#[path = "repo_tests.rs"]
mod tests;
