//! fkst-control-plane server entrypoint: JSON tracing init, config load, router
//! build, and serving with graceful shutdown (SIGTERM / Ctrl-C).
//!
//! The control plane is API-only: it records sessions but never runs an engine
//! in-process. There is no controller, no worker fleet, no internal worker
//! protocol, and no journaling. A goal trigger records a `Pending` session that
//! pod-per-session execution will later run (milestone #9).

use std::process::ExitCode;

use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::nyxid::NyxIdClient;
use fkst_control_plane::router::build_router;
use fkst_control_plane::startup::build_nyxid_client;
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
    let auth_mode = config.auth.clone();

    // Build the NyxID client ONCE for the Authorizer and the user-token paths.
    // Owner-only model (#257): the client is built from `AuthMode::Enabled`
    // ALONE (which carries the NyxID base URL), because every feature it drives
    // — per-session key mint, Ornn proxy, github_hub, repo-create —
    // authenticates with the FORWARDED USER TOKEN. There is no service account.
    let nyxid_client: Option<NyxIdClient> = match build_nyxid_client(
        &auth_mode,
        &config.nyxid_github_proxy_slug,
        std::time::Duration::from_secs(config.nyxid_org_cache_ttl_secs),
    ) {
        Ok(client) => {
            if client.is_some() {
                tracing::info!("NyxID client built (owner-only, forwarded user token)");
            }
            client
        }
        Err(error) => {
            tracing::error!(error = %error, "failed to build NyxID client");
            return ExitCode::FAILURE;
        }
    };

    // The Authorizer is given the (optional) NyxID client. Ownership/org checks
    // remain.
    let authz = Authorizer::new(nyxid_client.clone());

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

    // Capture what the Job watcher needs BEFORE `config`/`github_app`/`auth_mode`
    // move into `AppState`. The binding store is shared (one instance) between
    // the connect routes (via AppState) and the watcher's refresh driver.
    let binding_store = fkst_control_plane::nyxid_connect::BrokerBindingStore::new();
    let pod_dispatch = config.pod.dispatch;
    let pod_namespace = config.pod.namespace.clone();
    let broker_client = config.broker_client();
    let watcher_base_url = match &auth_mode {
        fkst_control_plane::auth::AuthMode::Enabled(settings) => Some(settings.base_url.clone()),
        fkst_control_plane::auth::AuthMode::Disabled => None,
    };
    let watcher_github_app = github_app.clone();

    let app = match build_router(AppState {
        binding_store: binding_store.clone(),
        config,
        auth_mode,
        authz,
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
    // issue labels + summary comment, drives the NyxID refresh). Requires the
    // GitHub App (issue mutations go through the App token) + a reachable cluster.
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
                    binding_store,
                    broker_client,
                    watcher_base_url,
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
