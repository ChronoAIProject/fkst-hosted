//! Session-scoped identity, visibility, and capability policy.
//!
//! One module owns everything that answers "who is calling, and what may they see
//! about this session":
//!
//! ```text
//! verified GithubUser
//!   -> AuthenticatedViewer            [viewer]    + global-admin role
//!        -> ViewerScope               [viewer]    server-resolved, sealed
//!             -> ActivityVisibilityConstraint     [activity] source predicate
//!
//! reconciler SessionRegistration
//!   -> SessionAccessContext           [context]   credential-free facts
//!        -> SessionAccessRegistry     [registry]  atomic, readiness-aware
//!             -> decide(capability)   [policy]    pure tiers
//!                  -> authorize_session_visibility  [gate]  the HTTP contract
//! ```
//!
//! Why one module rather than a policy per surface: log download, engine observe,
//! work authority, and operations visibility all read the same trigger-issue
//! facts, and three route-local interpretations of "creator" is exactly how a
//! capability silently widens. The tiers still differ — [`policy`] holds that
//! table — but they differ in ONE place, under test.
//!
//! The registry is an ephemeral projection of GitHub issues, never durable
//! application state (epic `OPS-03`), and nothing in this module ever holds a
//! token, an issue body, or a credential fingerprint.

pub mod activity;
pub mod context;
pub mod gate;
pub mod metrics;
pub mod policy;
pub mod registry;
pub mod viewer;

/// Shared, credential-free fixtures for this module's unit tests.
#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;

pub use activity::{
    authorize_lifecycle_session, ActivityVisibilityConstraint, AuthorizedSessionId,
    GlobalActivityScope, PersonalActivityScope,
};
pub use context::{SessionAccessContext, SessionAuthorizationFacts};
pub use gate::authorize_session_visibility;
pub use metrics::{ScopeMetrics, ScopeMetricsSnapshot, ScopeOutcome};
pub use policy::{
    decide, AccessBasis, PolicyEnvironment, SessionAccessDecision, SessionAccessRequest,
    SessionCapability, VerifiedCaller,
};
pub use registry::{
    ContextLookup, RegistrySnapshot, RegistryState, RepoKey, SessionAccessRegistry,
};
pub use viewer::{
    resolve_operations_scope, AuthenticatedViewer, GlobalAdmin, GlobalScope, PersonalScope,
    RequestedScope, ScopeDenialReason, ScopeRequest, ViewerScope,
};

/// The session-access state carried on [`crate::state::AppState`].
///
/// The projection and its scope telemetry travel together because every
/// operations route needs both, and bundling them keeps the application state
/// from growing a new field per counter.
#[derive(Clone, Debug, Default)]
pub struct SessionAccessState {
    /// The `session_id -> context` projection the reconciler publishes.
    pub registry: SessionAccessRegistry,
    /// Bounded scope-decision counters rendered by `/metrics`.
    pub scope_metrics: ScopeMetrics,
}

impl SessionAccessState {
    /// Build the state around a registry (dispatch-aware; see
    /// [`SessionAccessRegistry::new`]).
    pub fn new(registry: SessionAccessRegistry) -> Self {
        Self {
            registry,
            scope_metrics: ScopeMetrics::new(),
        }
    }
}
