//! The authenticated operations viewer, and the server-resolved scope.
//!
//! [`AuthenticatedViewer`] wraps the already-verified
//! [`GithubUser`](crate::github_identity::GithubUser) — same bearer token, same
//! `GET /user` check, same deployment [`AccessPolicy`] gate — and adds exactly one
//! derived fact: whether the caller is a deployment global administrator.
//!
//! It exists because the operations surfaces are NOT admin-only. Every admitted
//! user opens `/operations`; only *selecting* the global scope or another actor is
//! a `403`. [`GlobalAdmin`] remains available for genuinely admin-only routes.
//!
//! ## Why the scope types have private fields
//!
//! A scope is a *capability*, not a request parameter. If `ViewerScope::All` were
//! constructible anywhere, a query DTO could mint one from caller-supplied input
//! and the whole row-level authorization argument would collapse. So the payload
//! structs keep private fields and only [`AuthenticatedViewer::resolve_scope`]
//! — which runs on a server-verified identity — can build them. The enum shape
//! from the epic is preserved; only the constructors are sealed.
//!
//! ## What the deny path must not reveal
//!
//! A rejected scope request answers the same way whatever the reason class, and
//! never states whether some other login/id is configured as an administrator.
//! The denial log carries the bounded reason only — never the probed actor value.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::access_policy::AccessPolicy;
use crate::error::AppError;
use crate::github_identity::GithubUser;
use crate::state::AppState;

use super::metrics::{ScopeMetrics, ScopeOutcome};

/// A verified caller admitted by the deployment access policy, plus their
/// global-admin role.
#[derive(Clone, Debug)]
pub struct AuthenticatedViewer {
    user: GithubUser,
    global_admin: bool,
}

impl AuthenticatedViewer {
    /// Wrap an already-verified identity and derive the role from `access`.
    ///
    /// The role is computed here, once, from the SAME policy the extractor
    /// applied — never from a request header, a query flag, or an
    /// `overview.global_admin` value a browser sent back.
    pub fn new(user: GithubUser, access: &AccessPolicy) -> Self {
        let global_admin = access.is_global_admin(user.id, &user.login);
        Self { user, global_admin }
    }

    /// The verified identity.
    pub fn user(&self) -> &GithubUser {
        &self.user
    }

    /// The immutable GitHub numeric id — the only ownership proof.
    pub fn id(&self) -> i64 {
        self.user.id
    }

    /// The login snapshot. Display and list matching only.
    pub fn login(&self) -> &str {
        &self.user.login
    }

    /// Whether the deployment configures this caller as a global administrator.
    pub fn is_global_admin(&self) -> bool {
        self.global_admin
    }

    /// Resolve the effective scope for one operations request.
    ///
    /// Pure and metric-free so the whole decision table is unit-testable; the
    /// route wrapper [`resolve_operations_scope`] adds telemetry and the HTTP
    /// mapping.
    pub fn resolve_scope(&self, request: ScopeRequest) -> Result<ViewerScope, ScopeDenialReason> {
        // A cross-actor filter is a global-only capability regardless of the
        // requested scope: "my activity, but for that other person" is exactly
        // the probe row-level authorization exists to stop.
        if request.cross_actor_filter && !self.global_admin {
            return Err(ScopeDenialReason::CrossActorFilter);
        }
        match request.requested {
            Some(RequestedScope::Global) if !self.global_admin => {
                Err(ScopeDenialReason::GlobalScope)
            }
            Some(RequestedScope::Global) => Ok(self.global_scope()),
            Some(RequestedScope::Personal) => Ok(self.personal_scope()),
            // Omission resolves to the caller's natural default: personal for a
            // regular user, global for an administrator.
            None if self.global_admin => Ok(self.global_scope()),
            None => Ok(self.personal_scope()),
        }
    }

    fn personal_scope(&self) -> ViewerScope {
        ViewerScope::Mine(PersonalScope {
            viewer_id: self.user.id,
            viewer_login: self.user.login.clone(),
        })
    }

    fn global_scope(&self) -> ViewerScope {
        ViewerScope::All(GlobalScope {
            admin_id: self.user.id,
            admin_login: self.user.login.clone(),
        })
    }
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for AuthenticatedViewer {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Reuse the one verification path: missing/invalid token stays `401` with
        // `WWW-Authenticate: Bearer`, and a verified-but-not-admitted identity
        // stays the existing canonical `403`, both BEFORE any scope resolution.
        let user = GithubUser::from_request_parts(parts, state).await?;
        Ok(Self::new(user, &state.config.access))
    }
}

/// A strict gate for genuinely administrator-only routes.
#[derive(Clone, Debug)]
pub struct GlobalAdmin {
    user: GithubUser,
}

impl GlobalAdmin {
    /// The verified administrator identity.
    pub fn user(&self) -> &GithubUser {
        &self.user
    }
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for GlobalAdmin {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let viewer = AuthenticatedViewer::from_request_parts(parts, state).await?;
        if !viewer.is_global_admin() {
            // Log the bounded fact, not the probe: the id is already in the
            // request's own tracing span, and the message must not hint at who IS
            // configured.
            tracing::info!("operations: global-admin route refused a regular caller");
            return Err(AppError::Forbidden(
                "this operation requires a deployment global administrator".to_string(),
            ));
        }
        Ok(Self {
            user: viewer.user.clone(),
        })
    }
}

/// The closed scope a caller may ask for.
///
/// Routes map their own vocabulary onto it (`mine`/`all` for activity,
/// `accessible`/`all` for sandboxes) so the authorization decision has exactly
/// two cases whatever the surface calls them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestedScope {
    /// `mine` / `accessible`.
    Personal,
    /// `all`.
    Global,
}

/// One request's scope inputs, already normalized from the route's query DTO.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScopeRequest {
    /// `None` when the caller omitted the parameter.
    pub requested: Option<RequestedScope>,
    /// Whether the caller supplied any cross-user actor filter (`actor_id` /
    /// `actor_login`). A regular user may never do that, even in personal scope.
    pub cross_actor_filter: bool,
}

impl ScopeRequest {
    /// A request with no cross-actor filter.
    pub fn new(requested: Option<RequestedScope>) -> Self {
        Self {
            requested,
            cross_actor_filter: false,
        }
    }

    /// The same request, marked as carrying a cross-actor filter.
    pub fn with_cross_actor_filter(mut self) -> Self {
        self.cross_actor_filter = true;
        self
    }
}

/// The server-resolved scope. Constructible only through
/// [`AuthenticatedViewer::resolve_scope`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewerScope {
    /// Personal scope: rows must carry this exact verified actor id.
    Mine(PersonalScope),
    /// Global scope: an administrator may see every actor and record kind.
    All(GlobalScope),
}

impl ViewerScope {
    /// The bounded label for metrics and structured logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            ViewerScope::Mine(_) => "mine",
            ViewerScope::All(_) => "all",
        }
    }

    /// Whether this is the global-administrator scope.
    pub fn is_global(&self) -> bool {
        matches!(self, ViewerScope::All(_))
    }

    /// The verified id behind the scope (the viewer's, or the administrator's).
    pub fn identity_id(&self) -> i64 {
        match self {
            ViewerScope::Mine(scope) => scope.viewer_id,
            ViewerScope::All(scope) => scope.admin_id,
        }
    }
}

/// The payload of [`ViewerScope::Mine`]. Private fields: only a verified viewer
/// can mint one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonalScope {
    viewer_id: i64,
    viewer_login: String,
}

impl PersonalScope {
    /// The immutable id every personal row predicate compares against.
    pub fn viewer_id(&self) -> i64 {
        self.viewer_id
    }

    /// The login snapshot. Display only — never an authorization input.
    pub fn viewer_login(&self) -> &str {
        &self.viewer_login
    }
}

/// The payload of [`ViewerScope::All`]. Private fields; see [`PersonalScope`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalScope {
    admin_id: i64,
    admin_login: String,
}

impl GlobalScope {
    /// The verified administrator's immutable id (audit correlation).
    pub fn admin_id(&self) -> i64 {
        self.admin_id
    }

    /// The administrator's login snapshot. Display only.
    pub fn admin_login(&self) -> &str {
        &self.admin_login
    }
}

/// Why a scope request was refused. A closed enum: the only value that reaches a
/// metric label or a log line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeDenialReason {
    /// A regular caller asked for the global scope.
    GlobalScope,
    /// A regular caller supplied a cross-user actor filter.
    CrossActorFilter,
}

impl ScopeDenialReason {
    /// The stable wire string.
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeDenialReason::GlobalScope => "global_scope_forbidden",
            ScopeDenialReason::CrossActorFilter => "cross_actor_forbidden",
        }
    }
}

/// Resolve the scope, record the bounded decision, and map a refusal to the
/// stable `403 operations_scope_forbidden`.
///
/// Called before PostHog, the relay, or the runtime backend is touched, so a
/// refused probe cannot cost the deployment an upstream call — nor learn anything
/// from a timing difference between "denied" and "denied after a query".
pub fn resolve_operations_scope(
    viewer: &AuthenticatedViewer,
    request: ScopeRequest,
    metrics: &ScopeMetrics,
) -> Result<ViewerScope, AppError> {
    let resolved = viewer.resolve_scope(request);
    metrics.record(ScopeOutcome::of(request, &resolved));
    match resolved {
        Ok(scope) => Ok(scope),
        Err(reason) => {
            tracing::info!(
                reason = reason.as_str(),
                "operations: scope selection refused"
            );
            Err(AppError::ScopeForbidden(
                "this scope is available to deployment global administrators only".to_string(),
            ))
        }
    }
}

#[cfg(test)]
#[path = "viewer_tests.rs"]
mod tests;
