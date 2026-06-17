//! Shared application state passed to every handler.

use std::sync::Arc;

use crate::auth::AuthMode;
use crate::authz::Authorizer;
use crate::config::Config;
use crate::controller::{ClaimMap, WorkerRegistry};
use crate::github_app::GithubAppTokens;
use crate::goals::GoalIssueStore;
use crate::sessions::SessionService;
use crate::vault::VaultService;

/// Clonable state shared across the router. Every member is cheap to clone
/// (the session service is an `Arc` handle). The controller is datastore-free
/// (#143): there is no database handle here.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Single-pod session orchestration (sessions module); HTTP handlers go
    /// through this, never the engine runner directly.
    pub sessions: SessionService,
    /// Authentication mode: disabled (local dev) or enabled with NyxID
    /// settings. Determines whether the JWT middleware is active.
    pub auth_mode: AuthMode,
    /// Authorization facade: wraps NyxID client (if configured) for org-role
    /// lookups, ownership enforcement, and the `allows()` policy.
    pub authz: Authorizer,
    /// GitHub App token service: `None` when `FKST_GITHUB_APP_ID` is unset
    /// (module disabled). Wired into `AppState` so a bad PEM fails at deploy
    /// time and the trigger issue consumes it with zero re-plumbing.
    pub github_app: Option<GithubAppTokens>,
    /// GitHub App webhook HMAC secret (issue #108): `None` when
    /// `FKST_GITHUB_APP_WEBHOOK_SECRET` is unset — the webhook route is then NOT
    /// mounted and installation resolution degrades to on-demand. Held in a
    /// `SecretString` and never logged; the webhook handler uses it to verify
    /// `X-Hub-Signature-256` over the raw body before any parse.
    pub github_app_webhook_secret: Option<secrecy::SecretString>,
    /// Goal store (GitHub-Issue + in-memory backed, domain layer owned by the
    /// goals module). Goal CRUD handlers go through this.
    pub goals: GoalIssueStore,
    /// Per-session secret/variable vault (issue #100), in-memory (#138).
    /// Secrets are supplied inline at goal trigger and held by the controller
    /// in memory only — no at-rest key and no persistence.
    pub vault: VaultService,
    /// Ornn skill-registry client for the catalog API (issue #114): `None` when
    /// NyxID is not configured (auth disabled / no service client) — the catalog
    /// endpoints then answer `503`. The catalog forwards the caller's NyxID
    /// token to Ornn, which enforces all visibility; fkst-hosted adds no policy.
    pub ornn: Option<crate::ornn::OrnnClient>,
    /// The controller's in-memory claim authority (#135/#198-ii), shared with the
    /// observability surface (`GET /api/v1/admin/state` + `GET /metrics`, #144).
    /// `None` in a minimal/test build that never enabled the controller — the
    /// admin/metrics routes then report an empty claim set. The same `Arc` the
    /// session service and the internal router hold, so the live view is exact.
    pub claims: Option<Arc<ClaimMap>>,
    /// The controller's in-memory worker registry (#134), shared with the
    /// observability surface (#144). `None` in a minimal/test build with no
    /// controller wired. Snapshotted (liveness only, never the dispatch queue)
    /// by the admin/metrics routes.
    pub worker_registry: Option<WorkerRegistry>,
}
