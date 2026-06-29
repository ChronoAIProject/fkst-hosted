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
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::nyxid::NyxIdClient;
use fkst_control_plane::reconcile::{reconcile_orphans, ReconcileConfig};
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
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

    // 3. The control plane is datastore-free and API-only: there is no database
    //    to connect to and no in-process execution to wire. Goals are
    //    GitHub-Issue + in-memory backed (#137); sessions live in an in-memory
    //    store (#198-i). The store-bearing wiring below is built next.

    // 4. Load the engine configuration (it carries the temp root the orphan
    //    reconciler sweeps; fail-closed on a malformed value).
    let engine_config = match EngineConfig::load_from_env() {
        Ok(engine_config) => engine_config,
        Err(error) => {
            tracing::error!(error = %error, "failed to load engine configuration");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        framework_bin = %engine_config.framework_bin.display(),
        "engine config loaded"
    );

    // 4b. Load the orphan-reconcile configuration (fail-closed on a malformed
    //     value — the SWEEP itself is fail-open later, but a bad env value is a
    //     misconfiguration that should be caught loudly). A clone of the engine
    //     config carries `temp_root` into the sweep, since `engine_config` is
    //     later moved into the session service.
    let reconcile_config = match ReconcileConfig::load_from_env() {
        Ok(reconcile_config) => reconcile_config,
        Err(error) => {
            tracing::error!(error = %error, "failed to load reconcile configuration");
            return ExitCode::FAILURE;
        }
    };
    let reconcile_engine_config = engine_config.clone();

    // 4c. Build the session service (in-memory store) and sweep orphans BEFORE
    //     binding: any pre-terminal session left in the in-memory store from a
    //     prior incarnation must be failed before clients can observe stale
    //     "running" state. A fresh store has none, but the call is kept for
    //     parity and the logged outcome.
    let sessions = SessionService::new(SessionRepo::new(), engine_config);

    match sessions.repo().fail_orphans().await {
        Ok(count) => tracing::info!(count, "orphan sweep completed"),
        Err(error) => {
            tracing::error!(error = %error, "orphan sweep failed");
            return ExitCode::FAILURE;
        }
    }

    // 4d. Sweep orphan engine RUNTIME dirs (fkst-rt-*) left by a prior
    //     HARD-KILLED incarnation of THIS pod. Runtime dirs are fenced against
    //     live sessions' runtime_dir values and an mtime safety threshold.
    //     Package dirs (fkst-pkg-*) are NOT deleted (counted as
    //     skipped_unfenceable). FAIL-OPEN + infallible: an unreadable temp_root
    //     just yields an empty sweep; cleaning is best-effort.
    let report = reconcile_orphans(&reconcile_engine_config, &reconcile_config);
    tracing::info!(
        scanned = report.scanned,
        swept = report.swept_count(),
        skipped_live = report.skipped_live,
        skipped_too_new = report.skipped_too_new,
        skipped_unfenceable = report.skipped_unfenceable,
        errors = report.error_count(),
        "orphan temp-dir reconciliation completed"
    );

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

    // 5b. Build the goal store (#137): goals are GitHub-Issue + in-memory backed.
    //     It mints the App installation token to write the goal issues; without
    //     the App configured it is in-memory only (the authoritative read path
    //     still works, no GitHub mirror).
    let goals = GoalIssueStore::new(github_app.clone());

    // 5c. Build the in-memory vault service (issue #100, database-free #138).
    //     Secrets are supplied inline at goal trigger and held in memory only.
    let vault =
        fkst_control_plane::vault::VaultService::new(fkst_control_plane::vault::VaultLimits {
            value_byte_cap: config.vault_value_byte_cap,
            entries_per_scope_cap: config.vault_entries_per_scope_cap,
        });
    tracing::info!("vault enabled (in-memory)");

    // 5d. Wire the session-execution setups (retained for pod-per-session run,
    //     milestone #9): the vault, the per-session codex provider config, the
    //     per-session NyxID token provisioning, the Ornn skill client, and goal
    //     support. They are recorded on the service now so pod-per-session can
    //     reuse them without re-plumbing; the API-only control plane does not run
    //     an engine in-process.
    sessions.enable_vault(vault.clone());

    sessions.enable_codex(
        config.codex_model.clone(),
        config.chrono_llm_base_url.clone(),
    );
    tracing::info!("per-session codex provider config enabled");

    // Per-session NyxID token provisioning is gated on AUTH BEING ENABLED — the
    // key is minted with the user's own token, and the control plane carries no
    // service-account credential.
    match (&nyxid_client, &auth_mode) {
        (Some(client), fkst_control_plane::auth::AuthMode::Enabled(settings)) => {
            sessions.enable_nyxid_token(
                client.clone(),
                settings.base_url.clone(),
                std::time::Duration::from_secs(config.session_key_ttl_secs),
            );
            tracing::info!("per-session nyxid token provisioning enabled");
        }
        _ => {
            tracing::info!("per-session nyxid token provisioning disabled (requires auth enabled)");
        }
    }

    // The Ornn skill-registry client also backs the catalog API (issue #114);
    // owner-only (#219), so it needs only the NyxID client. When auth is
    // disabled, the catalog answers 503.
    let ornn_client: Option<fkst_control_plane::ornn::OrnnClient> = match &nyxid_client {
        Some(client) => match fkst_control_plane::ornn::OrnnClient::with_nyxid(
            client.clone(),
            fkst_control_plane::ornn::DEFAULT_ORNN_SLUG,
        ) {
            Ok(ornn) => {
                sessions.enable_ornn(ornn.clone());
                tracing::info!("ornn skill injection + catalog enabled");
                Some(ornn)
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to build ornn client");
                return ExitCode::FAILURE;
            }
        },
        None => {
            tracing::info!("ornn skill injection + catalog disabled (requires auth enabled)");
            None
        }
    };

    // Goal support: goal-status sync writes + token refresh. Requires both the
    // goals repo and the GitHub App tokens service.
    if let Some(ref gh_app) = github_app {
        sessions.enable_goal_support(goals.clone(), gh_app.clone());
        tracing::info!("goal support enabled in session service");
    } else {
        tracing::info!("github app not configured; goal support disabled in session service");
    }

    let app = match build_router(AppState {
        config,
        sessions: sessions.clone(),
        auth_mode,
        authz,
        github_app,
        github_app_webhook_secret,
        goals,
        vault,
        ornn: ornn_client,
    }) {
        Ok(router) => router,
        Err(error) => {
            tracing::error!(error = %error, "failed to build router");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("router built");

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
        sessions.shutdown().await;
        return ExitCode::FAILURE;
    }

    sessions.shutdown().await;

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
