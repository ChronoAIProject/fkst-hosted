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

use crate::goals::trigger_parse::PackageRef;
use crate::models::RepoRef;
use crate::reconcile::reachability;
use crate::reconcile_config::ReconcileConfig;

// The pure content hashes live in the sibling `hashing` module; re-exported here so
// the planner (and its attached test modules) reach them as `desired::…` unchanged.
pub use crate::reconcile::hashing::{config_hash, full_config_hash};

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
    pub work_label: Option<String>,
    /// The optional named environment parsed from `### Environment`.
    pub environment: Option<String>,
    /// The optional session output locale parsed from `### Output Language`
    /// (rendered into the session as `FKST_OUTPUT_LANG`). Part of BOTH hashes:
    /// it changes the pod env, so editing it after registration is a rejected
    /// config change like any other launch input.
    pub output_lang: Option<String>,
    /// The validated `### Engine Config` map (allowlisted engine tunables the
    /// launcher injects as session env). Part of BOTH hashes: it changes the
    /// pod env, so editing it after registration is a rejected config change.
    pub engine_config: std::collections::BTreeMap<String, String>,
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
    /// The issue author's GitHub LOGIN. Identity metadata like
    /// [`trigger_author_id`](Self::trigger_author_id) — EXCLUDED from both config
    /// hashes. Injected (author-first) into `FKST_GITHUB_AUTHORIZED_LOGINS` so the
    /// packages' github author policy always trusts the person who opened the
    /// trigger.
    pub trigger_author_login: String,
    /// The launch inputs.
    pub def: SessionDef,
    /// The deterministic session id (see [`crate::session_spec::derive_session_id`]).
    pub session_id: String,
    /// A stable hash over the launch inputs; a live pod whose recorded hash differs
    /// is running a stale config and must be re-spawned (see [`plan_repo`]).
    pub config_hash: String,
    /// Per-session opt-in (from the trigger issue's `### Auto-merge`) for the
    /// reconciler to auto-merge the App bot's mergeable PRs on this repo. NOT part
    /// of `config_hash` — a pod runs identically regardless, so toggling it never
    /// respawns the pod; it only gates the reconcile-side merge step.
    pub auto_merge: bool,
    /// Per-session log-download allow-list (from the trigger issue's `### Log
    /// Access`): the GitHub logins/ids permitted to pull this session's redacted
    /// logs, IN ADDITION to the issue author + the global admins. Like the two
    /// opt-ins it is NOT part of `config_hash` (a pod runs identically regardless),
    /// but it IS part of [`full_config_hash`] so config-immutability FREEZES it — the
    /// allow-list cannot be edited after registration to grant access retroactively.
    pub log_access: Vec<String>,
    /// Per-session work-item COLLABORATORS (from the trigger issue's `### Session
    /// Collaborators`): the GitHub logins granted authority over this session's
    /// work issues, IN ADDITION to the trigger author. Like [`log_access`](Self::log_access)
    /// it is NOT part of `config_hash` (a pod runs identically regardless) but IS
    /// part of [`full_config_hash`], so config-immutability FREEZES it — the list
    /// cannot be edited after registration to grant authority retroactively. F3
    /// carries + freezes the list only; the authority gate is a later PR.
    pub collaborators: Vec<String>,
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
    /// The session's GitHub work label, recorded on the pod (from its
    /// `fkst.chrono-ai.fun/work-label` annotation). Carried so that when this pod is
    /// orphaned (its trigger issue closed) the planner can retire-notify the still-open
    /// work issues that share this label. `None` when the annotation is absent (an
    /// older pod predating the annotation), in which case no retire-notify is emitted.
    pub work_label: Option<String>,
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
    Spawn {
        /// The registration to spawn.
        reg: SessionRegistration,
        /// The session's FULL effective work-label set (explicit `### Work Label` ∪
        /// its packages' auto-declared labels) — the set that actually wakes it.
        /// Threaded (I2, epic #594) as a foundation for a later PR; carried but NOT
        /// yet consumed, so the spawned pod spec stays byte-identical.
        detected_work_labels: Vec<String>,
    },
    /// Refresh the pod's `last-pending-at` (it is live and reported pending).
    TouchPending { session_id: String },
    /// Delete the pod for the given reason.
    Kill {
        session_id: String,
        reason: KillReason,
    },
    /// GC a terminal pod (+ its owned Secret).
    CleanupTerminal { session_id: String },
    /// Retire-notify the still-OPEN work issues of a session whose trigger issue was
    /// closed (session retired). Emitted from the orphan-pod branch alongside the
    /// `Kill { TriggerClosed }`, carrying the orphan pod's `work_label` so the executor
    /// can list that label's open issues, comment "session retired, no longer worked",
    /// latch [`crate::reconcile::SUBSTRATE_RETIRED_LABEL`], and drop the now-stale
    /// picked-up label — leaving each issue OPEN.
    RetireWorkIssues { work_label: Option<String> },
    /// Flag an invalid trigger issue (comment + label), first observation only.
    FlagInvalid { trigger_issue: i64, detail: String },
    /// Clear the invalid flag from an issue that now parses.
    ClearInvalid { trigger_issue: i64 },
    /// Announce a freshly-registered VALID session on its trigger issue (comment +
    /// durable latch label), first observation only. Carries the pre-rendered public
    /// metadata the comment shows so the executor renders a pure body. Independent of
    /// Spawn/pending — a session is announced on registration whether or not it has
    /// queued work yet.
    AnnounceSession {
        trigger_issue: i64,
        /// The deterministic session id, so the executor can build the identity-gated
        /// log-download link (`<base>/api/v1/logs/<session_id>`) the comment carries.
        session_id: String,
        /// The session name (`### Session Name`).
        session_name: String,
        /// The explicit GitHub work label whose open issues queue this session's
        /// work, or `None` for a label-less session (wake labels auto-discovered
        /// from its packages) — the announce comment omits the work-label line.
        work_label: Option<String>,
        /// The session's FULL effective work-label set (explicit ∪ package-discovered)
        /// — the set that actually wakes it. Threaded (I2, epic #594) as a foundation
        /// for a later PR; carried but NOT yet rendered, so the announcement body stays
        /// byte-identical.
        detected_work_labels: Vec<String>,
        /// The package refs rendered back to `owner/repo@ref:path`, in author order.
        packages: Vec<String>,
        /// The named environment, or `None` for a no-environment session.
        environment: Option<String>,
        /// Whether this trigger opted into reconcile-side PR auto-merge.
        auto_merge: bool,
        /// The registration's [`full_config_hash`], latched as a hidden marker in the
        /// announcement comment so a later config edit can be detected + rejected.
        full_config_hash: String,
    },
    /// Reject an attempted config change on an already-triggered issue (config is
    /// immutable once a session exists). Emitted ONCE per change transition — the
    /// executor comments "config changes aren't accepted; close + reopen to change"
    /// and latches [`crate::reconcile::SUBSTRATE_CONFIG_REJECTED_LABEL`] so it never
    /// re-comments. The edit is separately prevented from respawning the pod (see
    /// [`plan_repo`]); this action is purely the user-facing feedback.
    RejectConfigChange { trigger_issue: i64 },
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

/// True when a registration's CURRENT [`full_config_hash`] differs from the ORIGINAL
/// hash latched (in the announcement marker) for its trigger issue — i.e. the author
/// edited some config after the session was triggered. Config is immutable once a
/// session exists, so such an edit is rejected.
///
/// A trigger with no latched original (`latched_config_hash` has no entry) is
/// PRE-announce: it has never been announced, so there is nothing to change against
/// yet — never a rejection. This bounds the check to already-triggered issues, and it
/// is why [`crate::reconcile::repo`] fetches comments only for announced triggers.
fn config_change_rejected(
    reg: &SessionRegistration,
    latched_config_hash: &HashMap<i64, String>,
) -> bool {
    match latched_config_hash.get(&reg.trigger_issue) {
        Some(original) => &full_config_hash(reg) != original,
        None => false,
    }
}

/// Plan the reconciliation of ONE repository: diff the desired registrations
/// against the observed pods and invalid-flag state, returning the ordered actions
/// that reconcile them. Pure and deterministic — the output depends only on the
/// inputs (`HashMap`/`HashSet` iteration order does not leak into it).
///
/// Precedence for a live pod: a config-drift kill takes priority over an idle kill;
/// a `Terminating` pod is always left alone; a `Terminal` pod is always cleaned up.
///
/// CONFIG IMMUTABILITY: config is immutable once a session is triggered. When a
/// registration's CURRENT [`full_config_hash`] differs from the ORIGINAL latched for
/// its trigger (`latched_config_hash`), the edit is REJECTED: the planner (a) treats
/// the registration's pod-affecting state as UNCHANGED — it suppresses the Spawn and
/// the `Kill { ConfigChanged }` the edit would otherwise cause, so the running pod is
/// left serving its original config and is never respawned on the edit — and (b)
/// emits a one-time [`ReconcileAction::RejectConfigChange`] (deduped by
/// `latched_config_rejected`) so the author is told the edit was ignored.
// Each argument is one distinct axis of the desired/observed snapshot the planner
// diffs; bundling them into a struct would only rename the same fields at every
// call site (the tests drive this directly) without reducing the real input set.
#[allow(clippy::too_many_arguments)]
pub fn plan_repo(
    regs: &[SessionRegistration],
    // The per-session FULL effective work-label set (explicit UNION package-discovered),
    // keyed by `session_id`, resolved once per pass by the driver. Read-only here: it
    // populates the carried `detected_work_labels` on Spawn/AnnounceSession (I2, epic
    // #594) and drives NO planning decision, so it never affects which actions emit.
    work_labels_by_session: &HashMap<String, Vec<String>>,
    invalid: &[(i64, String)],
    live: &[LivePod],
    pending: &HashMap<String, bool>,
    latched_invalid: &HashSet<i64>,
    latched_announced: &HashSet<i64>,
    latched_config_hash: &HashMap<i64, String>,
    latched_config_rejected: &HashSet<i64>,
    now: DateTime<Utc>,
    cfg: &ReconcileConfig,
) -> Vec<ReconcileAction> {
    let mut actions = Vec::new();

    // Index the observed pods by session id so a registration can find its pod.
    let live_by_session: HashMap<&str, &LivePod> =
        live.iter().map(|p| (p.session_id.as_str(), p)).collect();
    // The set of session ids that ARE desired (have an open registration).
    let desired_sessions: HashSet<&str> = regs.iter().map(|r| r.session_id.as_str()).collect();
    // The FULL effective work-label set carried onto a session's Spawn/AnnounceSession
    // (I2, epic #594); an unmapped session (shouldn't happen — the driver fills every
    // one) defaults to empty. Read-only: it never influences which actions emit.
    let detected = |session_id: &str| -> Vec<String> {
        work_labels_by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    };

    // --- 1. Registration-driven actions (desired state present) ---------------
    for reg in regs {
        let pod = live_by_session.get(reg.session_id.as_str()).copied();
        let liveness = pod.map(|p| p.liveness).unwrap_or(PodLiveness::Absent);
        let is_pending = pending.get(&reg.session_id).copied().unwrap_or(false);
        // A rejected config edit must not drive any pod change: the pod keeps running
        // its ORIGINAL config and is never respawned on the edit. We gate the two
        // edit-driven actions (Spawn a new pod, Kill { ConfigChanged } to respawn) on
        // this so the immutability guarantee holds regardless of what the edit touched.
        let rejected = config_change_rejected(reg, latched_config_hash);

        match liveness {
            // Desired but no pod: spawn only once the session reports pending
            // (the pending signal is what turns a registration into a live need).
            // A rejected edit suppresses the spawn — we cannot spawn the ORIGINAL
            // config (only its hash is latched) and must not spawn the edited one, so
            // the session stays frozen until the author closes + reopens it.
            PodLiveness::Absent => {
                if is_pending && !rejected {
                    actions.push(ReconcileAction::Spawn {
                        reg: reg.clone(),
                        detected_work_labels: detected(&reg.session_id),
                    });
                }
            }
            // A running/starting pod: drift beats idle; pending refreshes the
            // clock; otherwise idle-kill once both clocks pass. A rejected edit
            // suppresses the drift kill (treat the pod as un-drifted) so the running
            // pod keeps serving; it still touches-pending / idle-kills normally.
            PodLiveness::Starting | PodLiveness::Live => {
                let pod = pod.expect("Starting/Live liveness implies a pod is present");
                if config_drifted(pod, reg) && !rejected {
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

    // --- 1b. Announce newly-registered valid sessions -> comment once ---------
    // Emitted for every VALID registration whose trigger issue is not already
    // latched-announced. Independent of the pod lifecycle above: a session is
    // announced the moment it registers, whether or not it has a pod or queued work
    // yet. Invalid/flagged triggers are never here (they are not in `regs`).
    for reg in regs {
        if !latched_announced.contains(&reg.trigger_issue) {
            actions.push(ReconcileAction::AnnounceSession {
                trigger_issue: reg.trigger_issue,
                session_id: reg.session_id.clone(),
                session_name: reg.def.name.clone(),
                work_label: reg.def.work_label.clone(),
                detected_work_labels: detected(&reg.session_id),
                packages: reg
                    .def
                    .packages
                    .iter()
                    .map(reachability::render_ref)
                    .collect(),
                environment: reg.def.environment.clone(),
                auto_merge: reg.auto_merge,
                full_config_hash: full_config_hash(reg),
            });
        }
    }

    // --- 1c. Reject config edits on already-triggered issues -> comment once ---
    // Config is immutable once a session exists. A registration whose CURRENT full
    // config hash differs from the ORIGINAL latched for its trigger has been edited;
    // emit the one-time rejection feedback (the pod actions above already ignored the
    // edit). Deduped by the durable `fkst-config-rejected` latch so it comments only
    // on the transition, mirroring the invalid-flag latch.
    for reg in regs {
        if config_change_rejected(reg, latched_config_hash)
            && !latched_config_rejected.contains(&reg.trigger_issue)
        {
            actions.push(ReconcileAction::RejectConfigChange {
                trigger_issue: reg.trigger_issue,
            });
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
                // Same cycle as the kill: retire-notify the still-open work issues so
                // they no longer look claimed (a retired session is no longer working
                // them). Only when the pod recorded its work label — an older pod
                // without the annotation carries no label to list.
                if pod.work_label.is_some() {
                    actions.push(ReconcileAction::RetireWorkIssues {
                        work_label: pod.work_label.clone(),
                    });
                }
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
// fixtures, the `plan_repo` matrix, the session-announcement + determinism cases,
// and the `config_hash` cases.
#[cfg(test)]
#[path = "desired_announce_tests.rs"]
mod desired_announce_tests;
#[cfg(test)]
#[path = "desired_collision_tests.rs"]
mod desired_collision_tests;
#[cfg(test)]
#[path = "desired_config_reject_tests.rs"]
mod desired_config_reject_tests;
#[cfg(test)]
#[path = "desired_full_hash_tests.rs"]
mod desired_full_hash_tests;
#[cfg(test)]
#[path = "desired_hash_tests.rs"]
mod desired_hash_tests;
#[cfg(test)]
#[path = "desired_plan_tests.rs"]
mod desired_plan_tests;
#[cfg(test)]
#[path = "desired_retire_tests.rs"]
mod desired_retire_tests;
#[cfg(test)]
#[path = "desired_test_fixtures.rs"]
mod desired_test_fixtures;
