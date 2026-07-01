//! fkst-control-plane server entrypoint: JSON tracing init, config load, router
//! build, and serving with graceful shutdown (SIGTERM / Ctrl-C).
//!
//! The control plane is API-only: it records sessions but never runs an engine
//! in-process. There is no controller, no worker fleet, no internal worker
//! protocol, and no journaling. A goal trigger records a `Pending` session that
//! pod-per-session execution will later run (milestone #9).

use std::process::ExitCode;

use fkst_control_plane::config::Config;
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

    // 1b. Subcommand dispatch: `run-session` is the in-pod, pod-per-session
    //     runner (milestone #9). It drives ONE substrate engine session to
    //     completion and exits with the session disposition — it never binds a
    //     socket or builds the server router. Dispatched here (after the
    //     subscriber init so its logs are structured) so the default arg-less
    //     invocation keeps the existing API-server behaviour unchanged.
    if std::env::args().nth(1).as_deref() == Some("run-session") {
        return fkst_control_plane::runner::run_session_from_env().await;
    }

    // 1c. Subcommand dispatch: `validate-env` is the in-pod, isolated
    //     install-validation runner (issue #338 §3.4). It executes a named
    //     environment's ordered install commands, prints a single-line JSON
    //     verdict as the final stdout line, and exits SUCCESS/FAILURE — it never
    //     binds a socket or builds the server router. Mirrors the `run-session`
    //     arm so the default arg-less invocation stays the API server unchanged.
    if std::env::args().nth(1).as_deref() == Some("validate-env") {
        return fkst_control_plane::install::run_validate_env().await;
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

    // 2b. Pod-per-session dispatch (milestone #9): when enabled, prove the
    //     Kubernetes API is reachable at startup so a misconfigured cluster
    //     surfaces in the logs immediately. Non-fatal — a transient API blip
    //     should not crash the control plane; the Job-spawn path surfaces hard
    //     errors per session. The control plane is Kubernetes-free when off.
    if config.pod.dispatch {
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

    // Capture what the Job watcher needs BEFORE `config`/`github_app` move into
    // `AppState`. The watcher drives no per-session credential refresh (the LLM
    // key is static config), so it only needs the namespace + the App tokens.
    let pod_dispatch = config.pod.dispatch;
    let pod_namespace = config.pod.namespace.clone();
    let watcher_github_app = github_app.clone();
    // The env-validation GC sweep (below) needs its own copy of the namespace +
    // the validation deadline, captured before `config` moves into `AppState`.
    let sweep_namespace = config.pod.namespace.clone();
    let sweep_deadline = config.env.validate_deadline_secs;

    let app = match build_router(AppState {
        config,
        github_app,
        github_app_webhook_secret,
    }) {
        Ok(router) => router,
        Err(error) => {
            tracing::error!(error = %error, "failed to build router");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("router built");

    // Pod-per-session: spawn the Job watcher (maps Job terminal status -> goal
    // issue labels + summary comment). Requires the GitHub App (issue mutations
    // go through the App token) + a reachable cluster.
    if pod_dispatch {
        match (
            watcher_github_app,
            fkst_control_plane::k8s::KubeClient::from_inferred(&pod_namespace).await,
        ) {
            (Some(github_app), Ok(kube)) => {
                let watcher = fkst_control_plane::k8s::JobWatcher::new(
                    kube.client().clone(),
                    pod_namespace,
                    github_app,
                );
                tokio::spawn(async move { watcher.run().await });
                tracing::info!("job watcher spawned");
            }
            (None, _) => tracing::warn!(
                "pod dispatch on but github app not configured; job watcher not started"
            ),
            (_, Err(error)) => tracing::warn!(
                error = %error,
                "pod dispatch on but kubernetes client unavailable; job watcher not started"
            ),
        }
    }

    // Pod-per-session: spawn the env-validation GC sweep. A validation pod is a
    // bare Pod (no `ttlSecondsAfterFinished`), so a control-plane crash mid-run
    // can orphan one; this periodic sweep reaps any older than the deadline. Only
    // when dispatch is on and a cluster is reachable (mirrors the watcher above).
    if pod_dispatch {
        match fkst_control_plane::k8s::KubeClient::from_inferred(&sweep_namespace).await {
            Ok(kube) => {
                // Sweep at least once per deadline window, never faster than 30s.
                let interval = std::time::Duration::from_secs(
                    u64::try_from(sweep_deadline).unwrap_or(300).max(30),
                );
                tokio::spawn(async move {
                    fkst_control_plane::k8s::env_validator::run_sweep_loop(
                        kube,
                        sweep_deadline,
                        interval,
                    )
                    .await;
                });
                tracing::info!("env-validation gc sweep spawned");
            }
            Err(error) => tracing::warn!(
                error = %error,
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
