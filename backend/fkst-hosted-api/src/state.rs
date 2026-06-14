//! Shared application state passed to every handler.

use crate::auth::AuthMode;
use crate::authz::Authorizer;
use crate::config::Config;
use crate::db::Db;
use crate::github_app::GithubAppTokens;
use crate::goals::GoalRepo;
use crate::packages::{PackageRepository, ShareRepo};
use crate::sessions::SessionService;

/// Clonable state shared across the router. Every member is cheap to clone
/// (`Db` and the repository's `Collection` are `Arc`-backed inside the
/// driver; the session service is an `Arc` handle).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Db,
    /// Repository over the `packages` collection (domain layer owned by the
    /// packages module); HTTP handlers go through this, never raw Mongo.
    pub packages: PackageRepository,
    /// Repository over the `package_shares` collection (domain layer owned by
    /// the packages module). Share-aware policy checks and the share HTTP
    /// handlers go through this.
    pub shares: ShareRepo,
    /// Single-pod session orchestration (sessions module); HTTP handlers go
    /// through this, never raw Mongo or the engine runner.
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
    /// Repository over the `goals` collection (domain layer owned by the goals
    /// module). Goal CRUD handlers go through this, never raw Mongo.
    pub goals: GoalRepo,
    /// Cloned engine config so the generate-endpoint's conformance dry-run can
    /// reach the engine plumbing (materialize + conformance) WITHOUT coupling
    /// to `SessionService` (which owns the long-lived session lifecycle).
    pub engine: crate::engine::EngineConfig,
    /// LLM gateway for package generation: `None` when `FKST_HOSTED_LLM_GATEWAY_URL`
    /// is unset → `POST /api/v1/packages/generate` answers 503. The trait object
    /// is the only LLM seam, so the concrete gateway stays swappable/mockable.
    pub llm: Option<std::sync::Arc<dyn crate::llm::LlmGateway>>,
}
