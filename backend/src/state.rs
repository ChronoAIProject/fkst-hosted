//! Shared application state passed to every handler.

use std::sync::Arc;

use crate::config::Config;
use crate::disposable_environment::DisposableEnvironmentRegistry;
use crate::github_app::GithubAppTokens;
use crate::log_access::LogAccessRegistry;
use crate::log_bundle_cache::LogBundleCache;
use crate::reconcile::ReconcileDispatcher;
use crate::recovery::RecoveryMonitor;
use crate::session_backend::SessionBackend;
use crate::storage::ChronoStorageClient;

/// Clonable state shared across the router. The control plane is API-only and
/// durable-datastore-free: a session IS its Kubernetes Pod / OpenSandbox sandbox
/// (created and retired by the reconciler through the session backend), so
/// there is no in-memory session/goal/vault database here. The one exception is
/// the short-lived disposable-environment handoff documented below. Identity is the
/// HMAC-verified GitHub webhook actor; there is no application-level auth
/// layer.
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Startup/full-resync recovery projection. `/ready` and `/metrics` only read
    /// snapshots; the serialized full-resync coordinator is its sole writer.
    pub recovery: RecoveryMonitor,
    /// GitHub App token service: `None` when `FKST_GITHUB_APP_ID` is unset
    /// (module disabled). Mints installation tokens for the webhook trigger, the
    /// reconciler's GitHub operations, and session-pod token rotation.
    pub github_app: Option<GithubAppTokens>,
    /// GitHub App webhook HMAC secret (issue #108): `None` when
    /// `FKST_GITHUB_APP_WEBHOOK_SECRET` is unset — the webhook route is then NOT
    /// mounted. Held in a `SecretString` and never logged; the webhook handler
    /// uses it to verify `X-Hub-Signature-256` over the raw body before any parse.
    pub github_app_webhook_secret: Option<secrecy::SecretString>,
    /// Dynamic Model B reconcile dispatcher: `Some` only when
    /// `FKST_POD_DISPATCH` is on and the reconciler supervisor spawned. Webhook
    /// hints forward to the current leader generation; followers have no active
    /// queue and drop them because acquisition/full resync is authoritative.
    pub reconciler: Option<ReconcileDispatcher>,
    /// The session runtime the env-validation REST path drives (issue #413): built
    /// ONCE at startup, UN-gated on pod dispatch so the `PUT` validate path stays
    /// available. `None` when the Kubernetes client could not be built — the validate
    /// path then reports 503. Held as `Arc<dyn SessionBackend>` so no concrete
    /// Kubernetes type touches the REST layer.
    pub session_backend: Option<Arc<dyn SessionBackend>>,
    /// chrono-storage object client for minting presigned GET URLs on the log
    /// download endpoint (READ-scoped SA). `None` when log storage is unconfigured
    /// (`FKST_STORAGE_*` unset) — the endpoint then reports the feature disabled.
    /// Shared behind an `Arc` (the client holds a connection pool + token cache).
    pub storage: Option<Arc<ChronoStorageClient>>,
    /// The in-memory `session_id -> log-access context` registry: the reverse map
    /// the identity-gated `/api/v1/logs/{session_id}` endpoint authorizes against.
    /// Populated by the reconciler each sweep; a cheap `Arc`-backed handle.
    pub log_registry: LogAccessRegistry,
    /// TTL-bounded cache of each session's redacted log bundle (the gzip'd `tar.gz`
    /// fetched from chrono-storage). Lets the log viewer's manifest + per-file reads
    /// and the whole-bundle download share one storage fetch per ~30s window instead
    /// of re-downloading + re-gunzipping on every request. A cheap `Arc`-backed handle.
    pub log_bundle_cache: LogBundleCache,
    /// Private, process-local create-request handoff for disposable session
    /// environments. Shared with the active reconciler; never exposed by a read
    /// route or backed by durable storage.
    pub disposable_environments: DisposableEnvironmentRegistry,
}
