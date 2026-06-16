//! Shared application state passed to every handler.

use crate::auth::AuthMode;
use crate::authz::Authorizer;
use crate::config::Config;
use crate::db::Db;
use crate::github_app::GithubAppTokens;
use crate::goals::GoalIssueStore;
use crate::sessions::SessionService;
use crate::vault::VaultService;

/// Clonable state shared across the router. Every member is cheap to clone
/// (`Db` and the repository's `Collection` are `Arc`-backed inside the
/// driver; the session service is an `Arc` handle).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Db,
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
    /// GitHub App webhook HMAC secret (issue #108): `None` when
    /// `FKST_GITHUB_APP_WEBHOOK_SECRET` is unset — the webhook route is then NOT
    /// mounted and installation resolution degrades to on-demand. Held in a
    /// `SecretString` and never logged; the webhook handler uses it to verify
    /// `X-Hub-Signature-256` over the raw body before any parse.
    pub github_app_webhook_secret: Option<secrecy::SecretString>,
    /// Repository over the `goals` collection (domain layer owned by the goals
    /// module). Goal CRUD handlers go through this, never raw Mongo.
    pub goals: GoalIssueStore,
    /// Per-session secret/variable vault (issue #100). Always present: the
    /// `KeyProvider` is built fail-closed at boot, so the vault routes never
    /// run without an at-rest encryption key.
    pub vault: VaultService,
    /// Ornn skill-registry client for the catalog API (issue #114): `None` when
    /// NyxID is not configured (auth disabled / no service client) — the catalog
    /// endpoints then answer `503`. The catalog forwards the caller's NyxID
    /// token to Ornn, which enforces all visibility; fkst-hosted adds no policy.
    pub ornn: Option<crate::ornn::OrnnClient>,
}
