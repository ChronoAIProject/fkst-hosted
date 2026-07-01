//! Shared application state passed to every handler.

use crate::config::Config;
use crate::github_app::GithubAppTokens;
use crate::reconcile::ReconcileHandle;

/// Clonable state shared across the router. The control plane is API-only and
/// datastore-free: a session IS a Kubernetes Job (read/stopped via the K8s API
/// in `routes/sessions.rs`), so there is no in-memory session/goal/vault store
/// here — only configuration, the GitHub App token service, and the webhook
/// secret. Identity is the HMAC-verified GitHub webhook actor; there is no
/// application-level auth layer.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// GitHub App token service: `None` when `FKST_GITHUB_APP_ID` is unset
    /// (module disabled). Mints installation tokens for the webhook trigger, the
    /// Job watcher, and the session read/stop endpoints.
    pub github_app: Option<GithubAppTokens>,
    /// GitHub App webhook HMAC secret (issue #108): `None` when
    /// `FKST_GITHUB_APP_WEBHOOK_SECRET` is unset — the webhook route is then NOT
    /// mounted. Held in a `SecretString` and never logged; the webhook handler
    /// uses it to verify `X-Hub-Signature-256` over the raw body before any parse.
    pub github_app_webhook_secret: Option<secrecy::SecretString>,
    /// Model B reconcile queue handle (issue #359, PR5b): `Some` only when
    /// `FKST_POD_DISPATCH` is on AND the reconciler spawned. The webhook will
    /// enqueue repos through it at the PR6 flip; today nothing reads it (Model A
    /// is unchanged), so it defaults to `None`.
    pub reconciler: Option<ReconcileHandle>,
}
