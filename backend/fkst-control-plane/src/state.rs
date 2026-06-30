//! Shared application state passed to every handler.

use crate::auth::AuthMode;
use crate::authz::Authorizer;
use crate::config::Config;
use crate::github_app::GithubAppTokens;

/// Clonable state shared across the router. The control plane is API-only and
/// datastore-free: a session IS a Kubernetes Job (read/stopped via the K8s API
/// in `routes/sessions.rs`), so there is no in-memory session/goal/vault store
/// here — only configuration, the auth layer, the GitHub App token service, the
/// webhook secret, and the durable NyxID binding store.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Authentication mode: disabled (local dev) or enabled with NyxID
    /// settings. Determines whether the JWT middleware is active.
    pub auth_mode: AuthMode,
    /// Authorization facade: wraps the NyxID client (if configured) for
    /// ownership enforcement and the `allows()` policy.
    pub authz: Authorizer,
    /// GitHub App token service: `None` when `FKST_GITHUB_APP_ID` is unset
    /// (module disabled). Mints installation tokens for the webhook trigger, the
    /// Job watcher, and the session read/stop endpoints.
    pub github_app: Option<GithubAppTokens>,
    /// GitHub App webhook HMAC secret (issue #108): `None` when
    /// `FKST_GITHUB_APP_WEBHOOK_SECRET` is unset — the webhook route is then NOT
    /// mounted. Held in a `SecretString` and never logged; the webhook handler
    /// uses it to verify `X-Hub-Signature-256` over the raw body before any parse.
    pub github_app_webhook_secret: Option<secrecy::SecretString>,
    /// Durable per-owner NyxID broker-binding store (connect-at-install, #297).
    /// Always present (empty until an owner connects); shared by the connect
    /// routes and the webhook trigger + token refresh.
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
