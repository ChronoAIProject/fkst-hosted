//! The in-memory `session_id -> `[`SessionAccessContext`] reverse index.
//!
//! A `session_id` is a one-way UUIDv5 over `(installation, owner, repo, issue)`
//! (see [`crate::session_spec::derive_session_id`]) — it cannot be reversed into
//! the trigger context an authorization decision needs. The reconciler already
//! resolves every open trigger issue into a
//! [`SessionRegistration`](crate::reconcile::desired::SessionRegistration) on each
//! per-repository sweep, so it publishes each one's context here as a cheap side
//! effect and every session-scoped route reads it.
//!
//! This registry is an EPHEMERAL, RECONSTRUCTABLE projection of GitHub issues
//! (epic `OPS-03`). It is not a domain database, it never persists to a
//! control-plane volume, and losing it costs nothing but a bounded delay.
//!
//! ## Readiness, and why it is not just "is the map non-empty"
//!
//! A fresh process starts with an empty map. "Empty" is indistinguishable from
//! "this deployment genuinely has no sessions", so an operations list that
//! filtered on an empty cold registry would return a confident, *wrong*, empty
//! answer. That is the failure mode readiness exists to prevent:
//!
//! - [`RegistryState::Cold`] — no authoritative discovery has completed in this
//!   process yet. Fail closed.
//! - [`RegistryState::Recovering`] — a complete installation/repository
//!   enumeration succeeded and its per-repository contexts are still being
//!   published. Fail closed: the generation is known to be incomplete.
//! - [`RegistryState::Ready`] — at least one full generation published. Later
//!   per-repository sweeps maintain it in place, so a periodic resync never
//!   flaps a working deployment back into `503`.
//!
//! The staged generation is invisible to readers by construction: it lives in a
//! separate buffer and is swapped in under the write lock, so a concurrent reader
//! observes either the previous generation or the new one, never a mixture.
//!
//! ## Two write paths, one invariant
//!
//! - [`SessionAccessRegistry::replace_repo`] is the steady-state write: one
//!   successful per-repository reconciliation REPLACES that repository's complete
//!   set, so a closed/retired/deleted registration disappears instead of
//!   surviving as a stale grant.
//! - [`SessionAccessRegistry::begin_generation`] +
//!   [`SessionAccessRegistry::abandon_generation`] wrap the full resync. While a
//!   cold generation is staged, `replace_repo` writes into the staging buffer and
//!   publication happens exactly once, when the last expected repository lands.
//!
//! ## Why a staged generation can always be degraded
//!
//! Staging only pays off if it ends. A repository that can never report — its
//! Issues tab disabled (`410` from the issues API), its installation revoked, or
//! simply an enqueue dropped by a full queue — would otherwise hold `pending`
//! non-empty forever, and since every subsequent `replace_repo` is diverted into
//! that buffer, the LIVE map would freeze: new sessions permanently invisible to
//! the shipped log/observe routes, retired ones permanently granted. Each
//! subsequent full resync would re-open the same doomed generation.
//!
//! So a generation that is known never to complete is DEGRADED
//! ([`SessionAccessRegistry::record_repo_failure`],
//! [`SessionAccessRegistry::abandon_generation`], and a superseding
//! `begin_generation`): its collected contexts are folded into the live map — each
//! one is an authoritative per-repository replacement in its own right, exactly
//! what the steady-state path applies — and writes go live again. Only the
//! COMPLETENESS claim is lost, and that is expressed by readiness: a projection
//! that was never complete stays [`RegistryState::Cold`] and keeps failing closed.
//!
//! Nothing sensitive lives here: the lists are the public trigger-issue content
//! and the ids are public GitHub numeric ids — never a token, body, or secret.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::models::RepoRef;

use super::context::SessionAccessContext;

/// A reconcile key: one repository under one installation.
pub type RepoKey = (i64, RepoRef);

/// The bounded lifecycle state of the projection. The only values that ever
/// become a metric label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryState {
    /// No authoritative discovery has completed in this process.
    Cold,
    /// A complete enumeration is being published; the generation is incomplete.
    Recovering,
    /// A complete generation is published and maintained.
    Ready,
}

impl RegistryState {
    /// The stable wire string; safe as a closed-enum metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            RegistryState::Cold => "cold",
            RegistryState::Recovering => "recovering",
            RegistryState::Ready => "ready",
        }
    }

    /// Whether session-scoped authorization may trust the projection.
    pub fn is_ready(self) -> bool {
        matches!(self, RegistryState::Ready)
    }
}

/// The result of a readiness-aware context lookup.
///
/// The three arms are distinct on purpose: a route must answer `503` while
/// completeness is unknown but `404` for a genuinely unknown session, and it may
/// never confuse the two (epic `SBOX-06`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextLookup {
    /// The projection is ready and knows this session.
    Found(Box<SessionAccessContext>),
    /// The projection is ready and this session is not in it.
    Unknown,
    /// The projection is cold or incomplete; nothing can be concluded.
    Unavailable,
}

/// A bounded read projection for `/metrics` and diagnostics.
///
/// Counts, state, and generation only — never an entry, a session id, a
/// repository, or a user value (epic `OPS-04`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegistrySnapshot {
    pub sessions: usize,
    pub state: RegistryState,
    pub generation: u64,
    /// Repositories still to publish before the staged generation completes.
    pub pending_repositories: usize,
}

/// The staged, not-yet-visible generation.
struct Staging {
    /// Repositories whose contexts this generation still expects.
    pending: HashSet<RepoKey>,
    /// Repositories that HAVE reported into this generation. Needed to degrade it:
    /// a repository that reported an empty set contributes no context, yet its
    /// live entries must still be dropped — "reported nothing" and "said nothing"
    /// are opposite instructions.
    reported: HashSet<RepoKey>,
    /// Contexts collected so far. Invisible to every reader.
    contexts: HashMap<String, SessionAccessContext>,
}

struct RegistryInner {
    /// The published generation. The only map any reader ever observes.
    contexts: HashMap<String, SessionAccessContext>,
    staging: Option<Staging>,
    state: RegistryState,
    generation: u64,
}

impl RegistryInner {
    /// Fold the staged generation (if any) into the live map and drop the buffer,
    /// so per-repository writes are immediately visible again.
    ///
    /// Every staged entry arrived as one repository's complete, authoritative set,
    /// so applying them is the same operation the steady-state path performs — no
    /// half-built generation is published, because there is no generation left to
    /// publish. Returns whether anything was staged.
    fn degrade_staging(&mut self) -> bool {
        let Some(staged) = self.staging.take() else {
            return false;
        };
        self.contexts.retain(|_, ctx| {
            !staged
                .reported
                .contains(&(ctx.installation_id, ctx.repo.clone()))
        });
        self.contexts.extend(staged.contexts);
        true
    }
}

/// A shared, in-memory session authorization projection. Cloning shares the one
/// backing store, so the reconciler (writer) and the routes (readers) hold
/// independent handles onto a single registry.
#[derive(Clone)]
pub struct SessionAccessRegistry {
    inner: Arc<RwLock<RegistryInner>>,
}

impl SessionAccessRegistry {
    /// A fresh registry.
    ///
    /// `dispatch_enabled` mirrors [`crate::recovery::RecoveryMonitor::new`]: a
    /// deployment with session dispatch switched off has no reconciler to
    /// discover anything, so its authoritative session set is empty by
    /// construction and the projection starts ready. With dispatch on, the
    /// projection starts cold and fails closed until the first generation lands.
    pub fn new(dispatch_enabled: bool) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner {
                contexts: HashMap::new(),
                staging: None,
                state: if dispatch_enabled {
                    RegistryState::Cold
                } else {
                    RegistryState::Ready
                },
                generation: 0,
            })),
        }
    }

    /// Begin a generation covering exactly `expected` repositories.
    ///
    /// Called by the full-resync coordinator once installation/repository
    /// enumeration has *completely* succeeded — a partial enumeration is not
    /// authoritative and must not start a generation, or an unreachable
    /// installation's sessions would silently vanish from the projection.
    ///
    /// An already-ready registry keeps serving its published generation while the
    /// new one is collected: readiness is about knowing the set is complete, and
    /// that knowledge is not lost by a refresh.
    ///
    /// A generation left staged by the previous pass is degraded first: it can
    /// never complete now that its successor owns the expected set, and its
    /// collected per-repository sets are worth more folded into the live map than
    /// discarded. This is also the periodic backstop for a repository that vanished
    /// from the queue without failing (a dropped enqueue), which no error path can
    /// report.
    pub fn begin_generation(&self, expected: HashSet<RepoKey>) {
        let mut inner = self.write();
        if inner.degrade_staging() {
            tracing::warn!(
                generation = inner.generation,
                "session access registry: a previous generation never completed; folding it into the live projection"
            );
        }
        inner.generation = inner.generation.saturating_add(1);
        if expected.is_empty() {
            // An App installed nowhere is an authoritatively empty projection,
            // not an unknown one.
            inner.contexts.clear();
            inner.state = RegistryState::Ready;
            return;
        }
        inner.staging = Some(Staging {
            pending: expected,
            reported: HashSet::new(),
            contexts: HashMap::new(),
        });
        if !inner.state.is_ready() {
            inner.state = RegistryState::Recovering;
        }
    }

    /// Give up on the staged generation without publishing it as complete.
    ///
    /// Used when discovery could not complete. Nothing half-built is ever
    /// published: the generation is degraded (its authoritative per-repository sets
    /// fold into the live map, see [`RegistryInner::degrade_staging`]) and the
    /// COMPLETENESS claim is dropped. A projection that was already known complete
    /// keeps its readiness — a failed refresh must never flap a healthy deployment
    /// into `503` — while one that never completed stays fail-closed.
    pub fn abandon_generation(&self) {
        let mut inner = self.write();
        Self::degrade(&mut inner, "discovery could not complete");
    }

    /// Record that one repository could not be reconciled this pass.
    ///
    /// The reconcile loop calls this on every per-repository failure. If that
    /// repository is one the staged generation is waiting for, the generation can
    /// never complete — a repository whose Issues tab is disabled, or whose
    /// installation was revoked, fails every single pass — so the projection
    /// degrades to incremental maintenance instead of freezing the live map behind
    /// a buffer that will never be published. A failure outside the expected set is
    /// ignored: the generation is still on track.
    pub fn record_repo_failure(&self, installation_id: i64, repo: &RepoRef) {
        let mut inner = self.write();
        let expected = inner
            .staging
            .as_ref()
            .is_some_and(|staging| staging.pending.contains(&(installation_id, repo.clone())));
        if !expected {
            return;
        }
        Self::degrade(
            &mut inner,
            "a repository of the staged generation could not report",
        );
    }

    /// Fold the staged generation into the live map and drop its completeness
    /// claim. `reason` is a fixed, bounded string — never a repository or session
    /// value.
    fn degrade(inner: &mut RegistryInner, reason: &'static str) {
        if !inner.degrade_staging() {
            return;
        }
        if !inner.state.is_ready() {
            inner.state = RegistryState::Cold;
        }
        tracing::warn!(
            generation = inner.generation,
            sessions = inner.contexts.len(),
            state = inner.state.as_str(),
            reason,
            "session access registry: staged generation degraded to incremental maintenance"
        );
    }

    /// Publish one repository's complete context set.
    ///
    /// While a generation is staged this fills the staging buffer and publishes
    /// atomically once the last expected repository arrives. Otherwise it edits
    /// the live map in place, replacing that repository's entries so retired
    /// sessions are removed rather than lingering as stale grants.
    pub fn replace_repo(
        &self,
        installation_id: i64,
        repo: &RepoRef,
        contexts: Vec<(String, SessionAccessContext)>,
    ) {
        let mut inner = self.write();
        if let Some(staging) = inner.staging.as_mut() {
            staging
                .contexts
                .retain(|_, ctx| !ctx.belongs_to(installation_id, repo));
            staging.contexts.extend(contexts);
            let key = (installation_id, repo.clone());
            staging.pending.remove(&key);
            staging.reported.insert(key);
            if staging.pending.is_empty() {
                // The swap: readers see the previous generation until this
                // assignment and the new one after it — never a mixture.
                let staged = inner.staging.take().expect("staging present");
                inner.contexts = staged.contexts;
                inner.state = RegistryState::Ready;
                tracing::info!(
                    generation = inner.generation,
                    sessions = inner.contexts.len(),
                    "session access registry: generation published"
                );
            }
            return;
        }
        inner
            .contexts
            .retain(|_, ctx| !ctx.belongs_to(installation_id, repo));
        inner.contexts.extend(contexts);
    }

    /// Look the context up without consulting readiness.
    ///
    /// This is the capability lookup the pre-existing session-scoped routes use:
    /// they have always been fail-closed on an unknown session (`404`) and their
    /// behaviour must not change. Operations surfaces that need to distinguish
    /// "unknown" from "not yet known" use [`Self::lookup`] instead.
    pub fn get(&self, session_id: &str) -> Option<SessionAccessContext> {
        let inner = self.read();
        inner.contexts.get(session_id).cloned()
    }

    /// Readiness-aware lookup for the operations surfaces.
    pub fn lookup(&self, session_id: &str) -> ContextLookup {
        let inner = self.read();
        if !inner.state.is_ready() {
            return ContextLookup::Unavailable;
        }
        match inner.contexts.get(session_id) {
            Some(context) => ContextLookup::Found(Box::new(context.clone())),
            None => ContextLookup::Unknown,
        }
    }

    /// The bounded projection for `/metrics` and diagnostics.
    pub fn snapshot(&self) -> RegistrySnapshot {
        let inner = self.read();
        RegistrySnapshot {
            sessions: inner.contexts.len(),
            state: inner.state,
            generation: inner.generation,
            pending_repositories: inner
                .staging
                .as_ref()
                .map(|staging| staging.pending.len())
                .unwrap_or(0),
        }
    }

    /// The number of published sessions (diagnostics + tests).
    pub fn len(&self) -> usize {
        self.read().contexts.len()
    }

    /// Whether the published generation is empty (diagnostics + tests).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether session-scoped authorization may trust the projection.
    pub fn is_ready(&self) -> bool {
        self.read().state.is_ready()
    }

    /// Poison-safe read: a panic elsewhere while the lock was held recovers the
    /// guard rather than wedging every subsequent authorization decision.
    fn read(&self) -> std::sync::RwLockReadGuard<'_, RegistryInner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Poison-safe write; see [`Self::read`].
    fn write(&self) -> std::sync::RwLockWriteGuard<'_, RegistryInner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for SessionAccessRegistry {
    /// Fail closed: an unconfigured registry is cold, never silently ready.
    fn default() -> Self {
        Self::new(true)
    }
}

impl std::fmt::Debug for SessionAccessRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Bounded counts and state only. A `{:?}` of the application state must
        // never dump session ids, logins, or allow-list entries into a log line.
        let snapshot = self.snapshot();
        f.debug_struct("SessionAccessRegistry")
            .field("sessions", &snapshot.sessions)
            .field("state", &snapshot.state.as_str())
            .field("generation", &snapshot.generation)
            .field("pending_repositories", &snapshot.pending_repositories)
            .finish()
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
