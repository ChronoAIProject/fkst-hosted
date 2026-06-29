//! Shared application state passed to every handler.

use crate::auth::AuthMode;
use crate::authz::Authorizer;
use crate::config::Config;
use crate::github_app::GithubAppTokens;
use crate::goals::GoalIssueStore;
use crate::sessions::SessionService;
use crate::vault::VaultService;

/// Clonable state shared across the router. Every member is cheap to clone
/// (the session service is an `Arc` handle). The control plane is API-only and
/// datastore-free: there is no database handle, no claim authority, and no
/// worker registry here — a goal trigger only records a `Pending` session that
/// pod-per-session execution will later run (milestone #9).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Session bookkeeping (sessions module): HTTP handlers create/read/stop
    /// session documents through this. It records sessions only; it never runs
    /// an engine in-process.
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
    /// Durable per-owner NyxID broker-binding store (connect-at-install, #297).
    /// Always present (empty until an owner connects); shared by the connect
    /// routes and, later, the webhook trigger + token refresh.
    pub binding_store: crate::nyxid_connect::BrokerBindingStore,
}

impl AppState {
    /// The NyxID issuer base URL when auth is enabled (used by the connect flow).
    pub fn nyxid_base_url(&self) -> Option<String> {
        match &self.auth_mode {
            AuthMode::Enabled(settings) => Some(settings.base_url.clone()),
            AuthMode::Disabled => None,
        }
    }
}
