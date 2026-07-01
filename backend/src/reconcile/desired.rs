//! Pure desired-state types + the Model B reconcile planner (issue #359 §4.3).
//!
//! This is the heart of the reconciler expressed as a **pure function**: given a
//! snapshot of the desired state (the valid + invalid trigger registrations) and
//! the observed state (the live pods, which sessions report themselves pending,
//! and which invalid issues are already flagged), [`plan_repo`] returns the
//! ordered list of [`ReconcileAction`]s that would drive the two into agreement.
//!
//! It performs NO Kubernetes or GitHub I/O and holds no clock of its own — `now`
//! is injected — so the full event→action matrix is exhaustively unit-testable
//! without a cluster. The effectful loop that executes these actions (spawns the
//! pods, deletes them, comments on the issues, refreshes tokens) is PR5b.

use std::collections::{HashMap, HashSet};

use k8s_openapi::chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::goals::trigger_parse::PackageRef;
use crate::models::RepoRef;
use crate::reconcile_config::ReconcileConfig;

/// The launch inputs one substrate session needs, distilled from a parsed trigger
/// issue. This is the non-identifying "what to run" half of a
/// [`SessionRegistration`] (the identifying half — installation, repo, issue — sits
/// on the registration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDef {
    /// The session name (a DNS-1123-label token) parsed from `### Session Name`.
    pub name: String,
    /// The fully-qualified package references parsed from `### Packages`, in
    /// author order.
    pub packages: Vec<PackageRef>,
    /// The single GitHub work label parsed from `### Work Label`.
    pub work_label: String,
    /// The optional named environment parsed from `### Environment`.
    pub environment: Option<String>,
}

/// One valid trigger issue resolved to everything the reconciler needs to spawn
/// (and later drift-check) a session: the identity keys, the launch [`SessionDef`],
/// the deterministic `session_id`, and the `config_hash` over the launch inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRegistration {
    /// The GitHub App installation the session's token is minted from.
    pub installation_id: i64,
    /// The `owner/name` repository the session works.
    pub repo: RepoRef,
    /// The issue number that triggered the session (progress reports go back here).
    pub trigger_issue: i64,
    /// The numeric GitHub id of the issue author (the control-path authz subject).
    pub trigger_author_id: i64,
    /// The launch inputs.
    pub def: SessionDef,
    /// The deterministic session id (see [`crate::session_spec::derive_session_id`]).
    pub session_id: String,
    /// A stable hash over the launch inputs; a live pod whose recorded hash differs
    /// is running a stale config and must be re-spawned (see [`plan_repo`]).
    pub config_hash: String,
}

/// The lifecycle phase of a live session pod, as the reconciler observes it. This
/// is the reconciler's own coarse projection of the Kubernetes pod phase +
/// deletion state, not a raw `PodStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodLiveness {
    /// No pod exists for the session.
    Absent,
    /// A pod exists but has not yet reached a running/ready state.
    Starting,
    /// A pod is running.
    Live,
    /// A pod is being deleted (a `deletionTimestamp` is set); leave it alone.
    Terminating,
    /// A pod has reached a terminal phase (Succeeded/Failed) and needs cleanup.
    Terminal,
}

/// The reconciler's observation of one live (or terminal) session pod, keyed by
/// its deterministic `session_id`. Mirrors the annotations the session-pod builder
/// stamps (`config-hash`, `last-pending-at`, `trigger-issue-number`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LivePod {
    /// The deterministic session id the pod name embeds.
    pub session_id: String,
    /// The trigger issue the pod was launched for (from its annotation).
    pub trigger_issue: i64,
    /// The observed lifecycle phase.
    pub liveness: PodLiveness,
    /// When the pod was created (drives the min-lifetime idle shield).
    pub created_at: DateTime<Utc>,
    /// When the session last reported itself pending (drives idle detection).
    /// `None` when the pod has never reported pending.
    pub last_pending_at: Option<DateTime<Utc>>,
    /// The `config_hash` recorded on the pod, if any. `None` means unknown (no
    /// drift decision can be made), which is treated as "no drift".
    pub config_hash: Option<String>,
}

/// Why a pod is being killed. Carried on [`ReconcileAction::Kill`] so the executor
/// (PR5b) can comment/log the reason and so tests can assert intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillReason {
    /// The session sat non-pending past the idle grace (and its min lifetime).
    Idle,
    /// The pod's config hash no longer matches its registration.
    ConfigChanged,
    /// The pod's trigger issue no longer has a matching open registration.
    TriggerClosed,
}

/// One reconciliation action. The output of [`plan_repo`]; PR5b's executor turns
/// each into the corresponding Kubernetes/GitHub call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileAction {
    /// Spawn a session pod for this registration (it is desired but absent).
    Spawn(SessionRegistration),
    /// Refresh the pod's `last-pending-at` (it is live and reported pending).
    TouchPending { session_id: String },
    /// Delete the pod for the given reason.
    Kill {
        session_id: String,
        reason: KillReason,
    },
    /// GC a terminal pod (+ its owned Secret).
    CleanupTerminal { session_id: String },
    /// Flag an invalid trigger issue (comment + label), first observation only.
    FlagInvalid { trigger_issue: i64, detail: String },
    /// Clear the invalid flag from an issue that now parses.
    ClearInvalid { trigger_issue: i64 },
}

/// A stable content hash over a session's launch inputs: its ordered package
/// references, its work label, and its optional environment. Mirrors
/// [`crate::k8s::env_store_meta::content_hash`] (canonical JSON → SHA-256 hex) so a
/// live pod's recorded hash can be compared for drift. Stable and, for a fixed
/// package ORDER, deterministic (packages are author-ordered, so order is part of
/// the identity).
pub fn config_hash(packages: &[PackageRef], work_label: &str, environment: Option<&str>) -> String {
    // A borrow-only projection of each package so `PackageRef` need not itself be
    // `Serialize`; the field set + order is the canonical package identity.
    #[derive(Serialize)]
    struct CanonPackage<'a> {
        owner: &'a str,
        repo: &'a str,
        git_ref: &'a str,
        path: &'a str,
    }
    #[derive(Serialize)]
    struct Canonical<'a> {
        packages: Vec<CanonPackage<'a>>,
        work_label: &'a str,
        environment: Option<&'a str>,
    }
    let canonical = Canonical {
        packages: packages
            .iter()
            .map(|p| CanonPackage {
                owner: &p.owner,
                repo: &p.repo,
                git_ref: &p.git_ref,
                path: &p.path,
            })
            .collect(),
        work_label,
        environment,
    };
    let json = serde_json::to_vec(&canonical).expect("canonical config-hash json is infallible");
    let digest = Sha256::digest(&json);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decide whether a live, non-pending pod is due for an idle-kill.
///
/// A non-pending pod is treated as idle (see the §4.3 matrix note). It is killed
/// only once BOTH clocks pass: it has been idle at least `session_idle_grace_secs`
/// AND alive at least `pod_min_lifetime_secs` (the shield that keeps a slow
/// startup from being mistaken for idleness). When the pod has never reported
/// pending, the idle clock runs from its creation time.
fn idle_kill_due(pod: &LivePod, now: DateTime<Utc>, cfg: &ReconcileConfig) -> bool {
    let idle_since = pod.last_pending_at.unwrap_or(pod.created_at);
    let idle_for = now - idle_since;
    let alive_for = now - pod.created_at;
    idle_for >= Duration::seconds(cfg.session_idle_grace_secs as i64)
        && alive_for >= Duration::seconds(cfg.pod_min_lifetime_secs as i64)
}

/// True when the live pod is running a config that no longer matches its
/// registration. A pod with no recorded hash (`None`) yields no drift decision.
fn config_drifted(pod: &LivePod, reg: &SessionRegistration) -> bool {
    matches!(&pod.config_hash, Some(h) if h != &reg.config_hash)
}

/// Plan the reconciliation of ONE repository: diff the desired registrations
/// against the observed pods and invalid-flag state, returning the ordered actions
/// that reconcile them. Pure and deterministic — the output depends only on the
/// inputs (`HashMap`/`HashSet` iteration order does not leak into it).
///
/// Precedence for a live pod: a config-drift kill takes priority over an idle kill;
/// a `Terminating` pod is always left alone; a `Terminal` pod is always cleaned up.
pub fn plan_repo(
    regs: &[SessionRegistration],
    invalid: &[(i64, String)],
    live: &[LivePod],
    pending: &HashMap<String, bool>,
    latched_invalid: &HashSet<i64>,
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Vec<ReconcileAction> {
    let mut actions = Vec::new();

    // Index the observed pods by session id so a registration can find its pod.
    let live_by_session: HashMap<&str, &LivePod> =
        live.iter().map(|p| (p.session_id.as_str(), p)).collect();
    // The set of session ids that ARE desired (have an open registration).
    let desired_sessions: HashSet<&str> = regs.iter().map(|r| r.session_id.as_str()).collect();

    // --- 1. Registration-driven actions (desired state present) ---------------
    for reg in regs {
        let pod = live_by_session.get(reg.session_id.as_str()).copied();
        let liveness = pod.map(|p| p.liveness).unwrap_or(PodLiveness::Absent);
        let is_pending = pending.get(&reg.session_id).copied().unwrap_or(false);

        match liveness {
            // Desired but no pod: spawn only once the session reports pending
            // (the pending signal is what turns a registration into a live need).
            PodLiveness::Absent => {
                if is_pending {
                    actions.push(ReconcileAction::Spawn(reg.clone()));
                }
            }
            // A running/starting pod: drift beats idle; pending refreshes the
            // clock; otherwise idle-kill once both clocks pass.
            PodLiveness::Starting | PodLiveness::Live => {
                let pod = pod.expect("Starting/Live liveness implies a pod is present");
                if config_drifted(pod, reg) {
                    actions.push(ReconcileAction::Kill {
                        session_id: reg.session_id.clone(),
                        reason: KillReason::ConfigChanged,
                    });
                } else if is_pending {
                    actions.push(ReconcileAction::TouchPending {
                        session_id: reg.session_id.clone(),
                    });
                } else if idle_kill_due(pod, now, cfg) {
                    actions.push(ReconcileAction::Kill {
                        session_id: reg.session_id.clone(),
                        reason: KillReason::Idle,
                    });
                }
            }
            // Being deleted already: nothing to do.
            PodLiveness::Terminating => {}
            // Finished: GC it (+ its owned Secret).
            PodLiveness::Terminal => {
                actions.push(ReconcileAction::CleanupTerminal {
                    session_id: reg.session_id.clone(),
                });
            }
        }
    }

    // --- 2. Orphan pods (observed but no matching registration) ---------------
    // A pod whose trigger issue closed (or whose label was removed) loses its
    // registration; a live/starting orphan is killed, a terminal orphan is GC'd.
    for pod in live {
        if desired_sessions.contains(pod.session_id.as_str()) {
            continue;
        }
        match pod.liveness {
            PodLiveness::Starting | PodLiveness::Live => {
                actions.push(ReconcileAction::Kill {
                    session_id: pod.session_id.clone(),
                    reason: KillReason::TriggerClosed,
                });
            }
            PodLiveness::Terminal => {
                actions.push(ReconcileAction::CleanupTerminal {
                    session_id: pod.session_id.clone(),
                });
            }
            PodLiveness::Absent | PodLiveness::Terminating => {}
        }
    }

    // --- 3. Invalid trigger issues -> flag once (not already latched) ---------
    for (issue, detail) in invalid {
        if !latched_invalid.contains(issue) {
            actions.push(ReconcileAction::FlagInvalid {
                trigger_issue: *issue,
                detail: detail.clone(),
            });
        }
    }

    // --- 4. Latched-invalid issues that now parse -> clear the flag -----------
    // An issue that once failed to parse but now appears as a registration has
    // been fixed. Sorted so the output order is independent of the set's
    // iteration order (determinism guarantee).
    let reg_issues: HashSet<i64> = regs.iter().map(|r| r.trigger_issue).collect();
    let mut cleared: Vec<i64> = latched_invalid
        .iter()
        .copied()
        .filter(|issue| reg_issues.contains(issue))
        .collect();
    cleared.sort_unstable();
    for issue in cleared {
        actions.push(ReconcileAction::ClearInvalid {
            trigger_issue: issue,
        });
    }

    actions
}

// Tests are split across files to keep each under the 500-line limit: shared
// fixtures, the `plan_repo` matrix, and the `config_hash` cases.
#[cfg(test)]
#[path = "desired_hash_tests.rs"]
mod desired_hash_tests;
#[cfg(test)]
#[path = "desired_plan_tests.rs"]
mod desired_plan_tests;
#[cfg(test)]
#[path = "desired_test_fixtures.rs"]
mod desired_test_fixtures;
