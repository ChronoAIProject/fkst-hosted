//! fkst-control-plane server entrypoint: JSON tracing init, config load, router
//! build, and serving with graceful shutdown (SIGTERM / Ctrl-C).
//!
//! The control plane is API-only: it records sessions but never runs an engine
//! in-process. There is no controller, no worker fleet, no internal worker
//! protocol, and no journaling. A goal trigger records a `Pending` session that
//! pod-per-session execution will later run (milestone #9).

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use fkst_control_plane::access_policy::AuthModel;
use fkst_control_plane::config::{Config, PodMode};
use fkst_control_plane::error::AppError;
use fkst_control_plane::github_app::HttpGithubListing;
use fkst_control_plane::osb_config::OpensandboxConfig;
use fkst_control_plane::reconcile::{
    reconcile_channel, run_full_resync_loop, run_reconcile_loop, run_sweep_loop, ReconcileCtx,
    ReconcileHandle,
};
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::AppState;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    // 1. Install the JSON subscriber FIRST so even config-load failures are
    //    logged structurally. The raw directive is read directly from the
    //    environment because the subscriber must exist before Config loads.
    let raw_directive =
        std::env::var("FKST_HOSTED_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let (filter, directive_ok) = match EnvFilter::try_new(&raw_directive) {
        Ok(filter) => (filter, true),
        Err(_) => (EnvFilter::new("info"), false),
    };
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
    tracing::info!("subscriber initialized");
    if !directive_ok {
        tracing::warn!(directive = %raw_directive, "invalid log directive; falling back to info");
    }

    // 1c. Subcommand dispatch: `validate-env` is the in-pod, isolated
    //     install-validation runner (issue #338 §3.4). It executes a named
    //     environment's ordered install commands, prints a single-line JSON
    //     verdict as the final stdout line, and exits SUCCESS/FAILURE — it never
    //     binds a socket or builds the server router, so the default arg-less
    //     invocation stays the API server unchanged.
    if std::env::args().nth(1).as_deref()
        == Some(fkst_control_plane::install::VALIDATE_ENV_SUBCOMMAND)
    {
        return fkst_control_plane::install::run_validate_env().await;
    }

    // 1d. Subcommand dispatch: `run-substrate` is the in-pod, Model B substrate
    //     session entrypoint (issue #359 §5). It fetches the workspace packages +
    //     the target repo, wires the rotating GitHub token into git + gh, renders
    //     the codex config, and execs `fkst-framework supervise` (forwarding
    //     SIGTERM) — it never binds a socket or builds the server router. Mirrors
    //     the `validate-env` arm so the default arg-less invocation stays the API
    //     server unchanged.
    if std::env::args().nth(1).as_deref() == Some("run-substrate") {
        return fkst_control_plane::session_pod::run_substrate_from_env().await;
    }

    // 2. Load the configuration from the environment.
    let config = match Config::load_from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(error = %error, "failed to load configuration");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        port = config.port,
        bind_addr = %config.bind_addr,
        request_timeout_secs = config.request_timeout_secs,
        log_level = %config.log_level,
        "config loaded"
    );
    // Log the RESOLVED auth model (FKST_AUTH_MODEL + FKST_ACCESS_ALLOWED_USERS),
    // never the entries themselves. An explicit `all` reads as open; an explicit
    // `allowlist` and a legacy set list both read as an enforced allowlist.
    match config.access.model() {
        Some(AuthModel::All) => {
            tracing::info!(
                "auth model: all (FKST_AUTH_MODEL=all; every authenticated user allowed)"
            );
        }
        _ if config.access.enforced() => {
            tracing::info!(
                allowed_users = config.access.entry_count(),
                "auth model: allowlist (selected users only)"
            );
        }
        _ => {
            tracing::info!(
                "auth model: open (unset; FKST_AUTH_MODEL / FKST_ACCESS_ALLOWED_USERS not set)"
            );
        }
    }
    tracing::info!(
        global_admins = config.access.global_admin_count(),
        "global admin policy loaded"
    );

    // 2b. Pod-per-session dispatch (milestone #9): when enabled IN K8S MODE, prove
    //     the Kubernetes API is reachable at startup so a misconfigured cluster
    //     surfaces in the logs immediately. Non-fatal — a transient API blip
    //     should not crash the control plane; the Job-spawn path surfaces hard
    //     errors per session. In OPENSANDBOX mode this apiserver probe would be
    //     misleading (session dispatch never touches the k8s pod surface — the
    //     control plane holds ZERO pod RBAC by design), so it is skipped: OSB
    //     reachability is probed by `build_osb_backend` below. The env-store
    //     KubeClient built in `spawn_reconciler` is a separate, mode-independent
    //     concern and is untouched by this gate.
    if config.pod.dispatch && config.pod.mode == PodMode::K8sCustomized {
        match fkst_control_plane::k8s::KubeClient::from_inferred(&config.pod.namespace).await {
            Ok(kube) => match kube.check_reachable().await {
                Ok(version) => tracing::info!(
                    namespace = %config.pod.namespace,
                    apiserver_version = %version,
                    "pod dispatch enabled (kubernetes reachable)"
                ),
                Err(error) => tracing::warn!(
                    error = %error,
                    namespace = %config.pod.namespace,
                    "pod dispatch enabled but the kubernetes apiserver is unreachable"
                ),
            },
            Err(error) => tracing::warn!(
                error = %error,
                "pod dispatch enabled but the kubernetes client could not be built"
            ),
        }
    } else if config.pod.dispatch {
        tracing::info!(
            "pod dispatch enabled (opensandbox mode; lifecycle reachability probed by the OSB backend)"
        );
    } else {
        tracing::info!("pod dispatch disabled (FKST_POD_DISPATCH not set)");
    }

    // 3. The control plane is API-only and datastore-free: a session IS a
    //    Kubernetes Job, so there is no in-memory session/goal/vault store and
    //    no in-process engine to wire here.

    // 5. Build the router.
    let addr = format!("{}:{}", config.bind_addr, config.port);

    // 5a. Load the GitHub App configuration (fail-closed: a bad PEM must never
    //     reach a session). Installation resolution is stateless (#141): the
    //     token service resolves on demand and caches in memory. The webhook
    //     secret (if set) is lifted out into AppState so the router can mount the
    //     signature-verified webhook route.
    let mut github_app_webhook_secret: Option<secrecy::SecretString> = None;
    let github_app = match fkst_control_plane::github_app::GithubAppConfig::load_from_env() {
        Ok(Some(config)) => {
            let app_id = config.app_id;
            github_app_webhook_secret = config.webhook_secret.clone();
            match fkst_control_plane::github_app::GithubAppTokens::new(&config) {
                Ok(tokens) => {
                    tracing::info!(
                        app_id,
                        webhook = github_app_webhook_secret.is_some(),
                        "github app enabled"
                    );
                    Some(tokens)
                }
                Err(error) => {
                    tracing::error!(error = %error, "failed to initialize github app tokens");
                    return ExitCode::FAILURE;
                }
            }
        }
        Ok(None) => {
            tracing::info!("github app disabled (FKST_GITHUB_APP_ID not set)");
            None
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to load github app configuration");
            return ExitCode::FAILURE;
        }
    };

    // Capture what the post-router background loops need BEFORE `config` moves
    // into `AppState`. The env-validation GC sweep (below) needs its own copy of
    // the namespace + the validation deadline; the reconciler gate reads dispatch.
    let pod_dispatch = config.pod.dispatch;
    let sweep_deadline = config.env.validate_deadline_secs;

    // The shared `session_id -> log-access context` registry the reconciler writes
    // each sweep and the log-download endpoint reads. Built here so ONE registry is
    // shared between the background loops and the API router.
    let log_registry = fkst_control_plane::log_access::LogAccessRegistry::new();

    // The chrono-storage client the log-download endpoint mints presigned GET URLs
    // with. Built from the already-parsed + validated `config.storage` (None when
    // the feature is unconfigured); shared behind an `Arc`.
    let storage = config
        .storage
        .clone()
        .map(|c| std::sync::Arc::new(fkst_control_plane::storage::client_from_config(c)));
    if storage.is_some() {
        tracing::info!("log-download endpoint enabled (chrono-storage configured)");
    }

    // Model B reconciler (issue #359, PR5b): when pod dispatch is on AND the GitHub
    // App + a cluster are available, spawn the reconcile queue consumer + the two
    // producer loops (sweep, full-resync) + the token-rotation loop, and hand the
    // enqueue handle to AppState. ADDITIVE + GATED: the webhook is NOT rewired to
    // enqueue here yet (that is the PR6 flip); with zero trigger issues this is a
    // harmless idle set of loops. Runs BEFORE build_router so the handle rides on
    // AppState.
    // The single session backend the env-validation REST path + the reconciler loops
    // drive (issue #413). Built ONCE, UN-gated on pod dispatch so the `PUT` validate
    // path stays available even when the dynamic-session loops are off; `None` (no
    // cluster) is non-fatal — the validate path then reports 503 and the loops do not
    // spawn.
    let session_backend = match build_session_backend(&config).await {
        Ok(backend) => backend,
        Err(error) => {
            tracing::error!(error = %error, "failed to build session backend");
            return ExitCode::FAILURE;
        }
    };

    let reconciler = if pod_dispatch {
        match session_backend.clone() {
            Some(backend) => {
                spawn_reconciler(&config, github_app.clone(), log_registry.clone(), backend).await
            }
            None => {
                tracing::warn!(
                    "pod dispatch on but session backend unavailable; reconciler not started"
                );
                None
            }
        }
    } else {
        None
    };

    // Clone the backend for the GC sweep (spawned after the router build consumes it).
    let sweep_backend = session_backend.clone();

    let app = match build_router(AppState {
        config,
        github_app,
        github_app_webhook_secret,
        reconciler,
        session_backend,
        storage,
        log_registry,
        log_bundle_cache: fkst_control_plane::log_bundle_cache::LogBundleCache::new(),
    }) {
        Ok(router) => router,
        Err(error) => {
            tracing::error!(error = %error, "failed to build router");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("router built");

    // Pod-per-session: spawn the env-validation GC sweep. A validation pod is a
    // bare Pod (no `ttlSecondsAfterFinished`), so a control-plane crash mid-run
    // can orphan one; this periodic sweep reaps any older than the deadline. Only
    // when dispatch is on and a cluster is reachable (mirrors the watcher above).
    if pod_dispatch {
        match sweep_backend {
            Some(backend) => {
                // Sweep at least once per deadline window, never faster than 30s.
                let interval = std::time::Duration::from_secs(
                    u64::try_from(sweep_deadline).unwrap_or(300).max(30),
                );
                tokio::spawn(async move {
                    fkst_control_plane::k8s::env_validator::run_sweep_loop(backend, interval).await;
                });
                tracing::info!("env-validation gc sweep spawned");
            }
            None => tracing::warn!(
                "pod dispatch on but kubernetes client unavailable; env-validation gc sweep not started"
            ),
        }
    }

    // 6. Bind and serve with graceful shutdown.
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(error = %error, addr = %addr, "failed to bind listener");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(addr = %addr, "server listening");

    if let Err(error) = axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!(error = %error, "server error");
        return ExitCode::FAILURE;
    }

    tracing::info!("server stopped");
    ExitCode::SUCCESS
}

/// Build the single session backend for the configured [`PodMode`], UN-gated on pod
/// dispatch so the env-validation REST path can drive it even when the dynamic-session
/// loops are off. For `K8sCustomized`, `Ok(None)` (the client could not be built) is
/// non-fatal: the validate path then reports 503 and the reconciler/sweep do not spawn.
/// For `Opensandbox`, `Ok(None)` when the `FKST_OSB_*` block is absent (dispatch-off
/// staging) — the same degraded path; otherwise the OpenSandbox backend is built.
async fn build_session_backend(
    config: &Config,
) -> Result<Option<Arc<dyn fkst_control_plane::session_backend::SessionBackend>>, AppError> {
    match config.pod.mode {
        PodMode::K8sCustomized => {
            match fkst_control_plane::k8s::KubeClient::from_inferred(&config.pod.namespace).await {
                Ok(kube) => {
                    let backend = fkst_control_plane::session_backend::k8s::K8sBackend::new(
                        kube,
                        config.pod.clone(),
                        config.reconcile.pod_termination_grace_secs,
                        config.env.validate_deadline_secs,
                        config.env.validate_poll_interval_secs,
                    );
                    Ok(Some(Arc::new(backend)))
                }
                Err(error) => {
                    tracing::warn!(error = %error, "session backend unavailable (kubernetes client could not be built); pod-driven features degraded");
                    Ok(None)
                }
            }
        }
        PodMode::Opensandbox => match &config.opensandbox {
            Some(osb) => Ok(Some(build_osb_backend(config, osb).await?)),
            // Dispatch-off staging (mode set, FKST_OSB_* not yet configured): mirror
            // the K8s "cluster unreachable -> Ok(None)" degraded path so the process
            // still boots (the validate path reports 503, the loops do not spawn).
            None => {
                tracing::warn!(
                    "FKST_POD_MODE=opensandbox but FKST_OSB_* not configured; session backend unavailable"
                );
                Ok(None)
            }
        },
    }
}

/// Construct the OpenSandbox [`fkst_control_plane::session_backend::opensandbox::OsbBackend`]
/// from the resolved config-layer [`OpensandboxConfig`]: one shared rustls HTTP client
/// drives the lifecycle client and every per-session execd client (the factory derives
/// the session-scoped execd token from the seed). The startup reachability probe is
/// WARN-not-fatal, mirroring the K8s apiserver probe.
async fn build_osb_backend(
    config: &Config,
    osb: &OpensandboxConfig,
) -> Result<Arc<dyn fkst_control_plane::session_backend::SessionBackend>, AppError> {
    use fkst_control_plane::session_backend::opensandbox::backend::{
        ExecdFactory, OsbConfig, DEFAULT_EXECD_TOKEN_ENV_KEY,
    };
    use fkst_control_plane::session_backend::opensandbox::{
        derive_execd_token, ExecdClient, ImageSpec, OsbBackend, OsbLifecycleClient, ResourceLimits,
    };
    use fkst_control_plane::session_backend::SessionBackend;

    // Warn once per FKST_POD_* knob the operator set that opensandbox mode ignores (the
    // sandbox template owns them). Computed in osb_config::from_vars (config.rs does not
    // log); FKST_POD_NAMESPACE / FKST_POD_IMAGE are never in this set (both still apply).
    for knob in &osb.ignored_pod_knobs {
        tracing::warn!(
            var = knob,
            "{knob} is ignored in opensandbox mode (the sandbox template owns it)"
        );
    }

    // One shared HTTP client (reqwest is built rustls-only, per Cargo.toml), reused by
    // the lifecycle client and every per-session execd client. Only the CONNECT
    // timeout lives here — per-request budgets are per-verb (client-owned): a
    // client-wide request timeout would sever the long-lived SSE command stream
    // and the synchronous 330s-budget create.
    let http = reqwest::Client::builder()
        .user_agent("fkst-hosted-api")
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Config(format!("failed to build opensandbox http client: {e}")))?;

    let lifecycle =
        OsbLifecycleClient::new(osb.base_url.clone(), osb.api_key.clone(), http.clone());

    // The execd factory: one closure captures the shared inputs and mints a per-sandbox
    // execd client, deriving the session-scoped execd token (HMAC of the seed) per call.
    let base = osb.base_url.clone();
    let key = osb.api_key.clone();
    let seed = osb.execd_token_seed.clone();
    let factory_http = http.clone();
    let factory: ExecdFactory = Arc::new(move |sandbox_id: &str, session_id: &str| {
        ExecdClient::new(
            base.clone(),
            key.clone(),
            sandbox_id.to_string(),
            derive_execd_token(&seed, session_id),
            factory_http.clone(),
        )
    });

    // The static launch config every sandbox is spawned with. The image is the SAME
    // control-plane image (required in both modes); the entrypoint runs the in-sandbox
    // binary in `run-substrate` mode (parity with the k8s Job re-exec).
    let image_uri = config.pod.image.clone().ok_or_else(|| {
        AppError::Config("FKST_POD_IMAGE must be set when FKST_POD_DISPATCH=true".to_string())
    })?;
    let osb_backend_config = OsbConfig {
        image: ImageSpec {
            uri: image_uri,
            auth: None,
        },
        entrypoint: vec![osb.entrypoint.clone(), "run-substrate".to_string()],
        resource_limits: ResourceLimits(BTreeMap::from([
            ("cpu".to_string(), osb.session_cpu.clone()),
            ("memory".to_string(), osb.session_memory.clone()),
        ])),
        execd_seed: osb.execd_token_seed.clone(),
        execd_token_env_key: DEFAULT_EXECD_TOKEN_ENV_KEY.to_string(),
        // The respawn-shield window: one reconcile cadence, so a just-stopped session
        // stays reported `Terminating` across the next reconcile tick (the window in
        // which the planner would otherwise re-observe it `Absent` and re-spawn).
        reconcile_window: Duration::from_secs(config.reconcile.reconcile_interval_secs),
        validate_deadline_secs: config.env.validate_deadline_secs,
        validate_poll_interval_secs: config.env.validate_poll_interval_secs,
    };

    let backend = OsbBackend::new(lifecycle, factory, config.pod.clone(), osb_backend_config);

    // Startup reachability probe: WARN-not-fatal (a transient blip must not crash the
    // control plane; per-session spawns surface hard errors), mirroring the K8s probe.
    if let Err(error) = backend.check_reachable().await {
        tracing::warn!(
            error = %error,
            "opensandbox lifecycle server not reachable at startup; continuing"
        );
    }

    Ok(Arc::new(backend))
}

/// Build the Model B reconcile context and spawn its loops, returning the enqueue
/// handle for `AppState` (or `None` if any prerequisite is missing). The concrete
/// session backend is passed in (built once, un-gated). Every failure is a WARN,
/// never fatal: a misconfigured/unreachable reconciler must not stop the API server
/// (Model A stays fully functional).
async fn spawn_reconciler(
    config: &Config,
    github_app: Option<fkst_control_plane::github_app::GithubAppTokens>,
    log_registry: fkst_control_plane::log_access::LogAccessRegistry,
    backend: Arc<dyn fkst_control_plane::session_backend::SessionBackend>,
) -> Option<ReconcileHandle> {
    let Some(github) = github_app else {
        tracing::warn!("pod dispatch on but github app not configured; reconciler not started");
        return None;
    };
    let kube = match fkst_control_plane::k8s::KubeClient::from_inferred(&config.pod.namespace).await
    {
        Ok(kube) => kube,
        Err(error) => {
            tracing::warn!(error = %error, "pod dispatch on but kubernetes client unavailable; reconciler not started");
            return None;
        }
    };
    // The read-side listing transport + the unauthenticated reachability probe
    // client both target the configured GitHub REST base.
    let listing = match HttpGithubListing::new(&config.github_api_base_url) {
        Ok(listing) => Arc::new(listing),
        Err(error) => {
            tracing::warn!(error = %error, "reconciler listing transport build failed; reconciler not started");
            return None;
        }
    };
    let http = match reqwest::Client::builder()
        .user_agent("fkst-hosted-api")
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(error = %error, "reconciler http client build failed; reconciler not started");
            return None;
        }
    };

    let ctx = ReconcileCtx {
        backend,
        // The env-store seam the spawn pre-flight reads; the reconciler shares one
        // namespace-bound client with it (the backend owns its own).
        env_store: std::sync::Arc::new(fkst_control_plane::k8s::env_store::EnvStore::from_kube(
            kube,
        )),
        github,
        listing,
        http,
        config: config.clone(),
        active_repos: fkst_control_plane::reconcile::new_active_repos(),
        ensured_templates: fkst_control_plane::reconcile::new_ensured_templates(),
        log_registry,
    };

    let (handle, rx) = reconcile_channel(1024);

    // Consumer: one serial reconcile worker draining the queue.
    let consumer_ctx = ctx.clone();
    tokio::spawn(async move { run_reconcile_loop(rx, consumer_ctx).await });
    // Producer 1: the periodic pod sweep.
    let sweep_ctx = ctx.clone();
    let sweep_handle = handle.clone();
    tokio::spawn(async move { run_sweep_loop(sweep_ctx, sweep_handle).await });
    // Producer 2: the periodic full resync (installations -> repos).
    let resync_ctx = ctx.clone();
    let resync_handle = handle.clone();
    tokio::spawn(async move { run_full_resync_loop(resync_ctx, resync_handle).await });
    // The in-place per-session token rotation loop (drives the fleet through the backend).
    let rot_backend = ctx.backend.clone();
    let rot_github = ctx.github.clone();
    let rot_cfg = ctx.config.reconcile.clone();
    let rot_handle = handle.clone();
    tokio::spawn(async move {
        fkst_control_plane::k8s::run_token_rotation_loop(
            rot_backend,
            rot_github,
            rot_cfg,
            rot_handle,
        )
        .await
    });
    // The package-agnostic session-health scrape loop (pod status + framework log
    // severity → flag/clear the degraded label on the trigger issue).
    let health_backend = ctx.backend.clone();
    let health_github = ctx.github.clone();
    let health_cfg = ctx.config.reconcile.clone();
    let health_handle = handle.clone();
    tokio::spawn(async move {
        fkst_control_plane::k8s::run_health_scrape_loop(
            health_backend,
            health_github,
            health_cfg,
            health_handle,
        )
        .await
    });

    tracing::info!(
        "model B reconciler spawned (reconcile + sweep + full-resync + token rotation + health scrape)"
    );
    Some(handle)
}

/// Resolve when either SIGTERM (how Kubernetes terminates pods) or Ctrl-C
/// (SIGINT) arrives; axum then stops accepting new connections and drains
/// the in-flight requests before the server future resolves.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
