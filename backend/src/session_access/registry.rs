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
    pub fn begin_generation(&self, expected: HashSet<RepoKey>) {
        let mut inner = self.write();
        inner.generation = inner.generation.saturating_add(1);
        if expected.is_empty() {
            // An App installed nowhere is an authoritatively empty projection,
            // not an unknown one.
            inner.contexts.clear();
            inner.staging = None;
            inner.state = RegistryState::Ready;
            return;
        }
        inner.staging = Some(Staging {
            pending: expected,
            contexts: HashMap::new(),
        });
        if !inner.state.is_ready() {
            inner.state = RegistryState::Recovering;
        }
    }

    /// Drop a staged generation without publishing it.
    ///
    /// Used when discovery could not complete. The previously published
    /// generation (if any) stays exactly as it was — a failed refresh must never
    /// downgrade a projection that is already known complete, and must never
    /// publish a half-built one.
    pub fn abandon_generation(&self) {
        let mut inner = self.write();
        if inner.staging.take().is_some() && !inner.state.is_ready() {
            inner.state = RegistryState::Cold;
        }
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
            staging.pending.remove(&(installation_id, repo.clone()));
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
