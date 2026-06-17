//! fkst-control-plane server entrypoint: JSON tracing init, config load, router
//! build, and serving with graceful shutdown (SIGTERM / Ctrl-C).

use std::process::ExitCode;

use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::controller::{ClaimMap, ControllerHandle, InternalAuth, WorkerRegistry};
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::journal_config::journal_config_from_app;
use fkst_control_plane::nyxid::NyxIdClient;
use fkst_control_plane::reconcile::{reconcile_orphans, ReconcileConfig};
use fkst_control_plane::router::{build_router, mount_internal};
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use tokio_util::sync::CancellationToken;
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

    // 3. The controller is datastore-free (#143): there is no database to
    //    connect to, no indexes to ensure, and no journal/goals collection.
    //    Goals are GitHub-Issue + in-memory backed (#137); the journal's SOLE
    //    machine-truth is the committed GitHub file (#139); sessions live in an
    //    in-memory store (#198-i). The store-bearing wiring below is built next.

    // 4c. Load the engine configuration (fail-closed: a zero timeout or a
    //     malformed value must never reach a live session).
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

    // 4c-bis. Load the orphan-reconcile configuration (fail-closed on a
    //     malformed value — the SWEEP itself is fail-open later, but a bad
    //     env value is a misconfiguration that should be caught loudly).
    //     A clone of the engine config carries `temp_root` into the sweep,
    //     since `engine_config` is later moved into the session service.
    let reconcile_config = match ReconcileConfig::load_from_env() {
        Ok(reconcile_config) => reconcile_config,
        Err(error) => {
            tracing::error!(error = %error, "failed to load reconcile configuration");
            return ExitCode::FAILURE;
        }
    };
    let reconcile_engine_config = engine_config.clone();

    // 4d. The per-worker max-load cap used by worker-dispatch placement
    //     (`FKST_PLACEMENT_MAX_LOAD`, 0 = uncapped). Carried over from the
    //     deleted distribution layer (#198-ii). Only consulted under dispatch
    //     mode; the in-process claim path never rejects on load.
    let dispatch_max_load = config.placement_max_load;

    // 4f. Build the session service (in-memory store) and sweep orphans BEFORE
    //     binding: any pre-terminal session left in the in-memory store from a
    //     prior incarnation must be failed before clients can observe stale
    //     "running" state. A fresh controller has none, but the call is kept for
    //     parity and the logged outcome.
    let sessions = SessionService::new(SessionRepo::new(), engine_config);
    // Session-progress journaling (issue #25): the committed GitHub file is
    // the sole machine-truth (#139). GitHub sync per the FKST_JOURNAL_* config
    // (absent repo/token degrades to no-durable-floor with a warn).
    sessions.enable_journaling(journal_config_from_app(&config));

    match sessions.repo().fail_orphans().await {
        Ok(count) => tracing::info!(count, "orphan sweep completed"),
        Err(error) => {
            tracing::error!(error = %error, "orphan sweep failed");
            return ExitCode::FAILURE;
        }
    }

    // 4f-bis. Sweep orphan engine RUNTIME dirs (fkst-rt-*) left by a prior
    //     HARD-KILLED incarnation of THIS pod (TempDir RAII cleans every
    //     normal path; only a kill -9 / OOM leaks them — issue #26 reduced
    //     scope). Runtime dirs are fenced against live sessions' runtime_dir
    //     values and an mtime safety threshold. Package dirs (fkst-pkg-*) are
    //     NOT deleted — their path is not persisted, so they cannot be fenced
    //     (counted as skipped_unfenceable). The live fence is now OS truth
    //     (#136): live = owner pid alive & leads its group & breadcrumb present
    //     — no Mongo query. FAIL-OPEN + infallible: an unreadable temp_root just
    //     yields an empty sweep; cleaning is best-effort.
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

    // 4g. (removed, #198-ii) No takeover reaper: a single authoritative
    //     controller owns every claim, so there is no cross-pod takeover to
    //     drive. Worker reassignment (dispatch mode only) is handled by the
    //     registry sweeper + the reassignment driver wired in step 5b.

    // 5. Build the router.
    let addr = format!("{}:{}", config.bind_addr, config.port);
    let auth_mode = config.auth.clone();

    // Build the NyxID client ONCE and share it across the Authorizer and the
    // LLM gateway. Only construct a NyxIdClient when auth is enabled AND both
    // service-account credentials are present.
    let nyxid_client: Option<NyxIdClient> = match (
        &auth_mode,
        &config.nyxid_client_id,
        &config.nyxid_client_secret,
    ) {
        (fkst_control_plane::auth::AuthMode::Enabled(settings), Some(id), Some(secret)) => {
            match NyxIdClient::new(
                &settings.base_url,
                &config.nyxid_github_proxy_slug,
                id.clone(),
                secret.clone(),
                std::time::Duration::from_secs(config.nyxid_org_cache_ttl_secs),
            ) {
                Ok(client) => {
                    tracing::info!("NyxID org features enabled");
                    Some(client)
                }
                Err(error) => {
                    tracing::error!(error = %error, "failed to build NyxID client");
                    return ExitCode::FAILURE;
                }
            }
        }
        (fkst_control_plane::auth::AuthMode::Enabled(_), None, None) => {
            tracing::warn!("NyxID org features disabled: NYXID_CLIENT_ID/SECRET not configured");
            None
        }
        _ => None,
    };

    // The Authorizer is given the (optional) NyxID client. Share-aware package
    // policy was removed with the package store (#115); ownership/org checks
    // remain.
    let authz = Authorizer::new(nyxid_client.clone());

    // 5a. Load the GitHub App configuration (fail-closed: a bad PEM must
    //     never reach a live session). Installation resolution is stateless
    //     (#141): the token service resolves on demand and caches in memory —
    //     no durable installation store. The webhook secret (if set) is lifted
    //     out into AppState so the router can mount the signature-verified
    //     webhook route.
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

    // 5a-bis-pre-pre. Build the session token minter for the controller's mid-run
    //         credential-refresh channel (#151). Cloned from `github_app` BEFORE
    //         it moves into AppState; `None` when the App is unconfigured (the
    //         credential-refresh route then answers 503). This is the internal
    //         router's `minter` argument below.
    let session_minter: Option<
        std::sync::Arc<dyn fkst_control_plane::controller::SessionTokenMinter>,
    > = github_app.clone().map(|tokens| {
        std::sync::Arc::new(fkst_control_plane::controller::GithubAppMinter::new(tokens))
            as std::sync::Arc<dyn fkst_control_plane::controller::SessionTokenMinter>
    });

    // 5a-bis-pre. Build the goal store (#137): goals are GitHub-Issue +
    //         in-memory backed. It mints the App installation token to write
    //         the goal issues; without the App configured it is in-memory only
    //         (the authoritative read path still works, no GitHub mirror).
    let goals = GoalIssueStore::new(github_app.clone());

    // 5a-ter. Build the in-memory vault service (issue #100, database-free #138).
    //         Secrets are supplied inline at goal trigger and held by the
    //         controller in memory only — no at-rest key, no Mongo collection,
    //         no index. The `FKST_HOSTED_VAULT_*` caps still bound a single
    //         inline value's size and an owner's per-scope entry count.
    let vault =
        fkst_control_plane::vault::VaultService::new(fkst_control_plane::vault::VaultLimits {
            value_byte_cap: config.vault_value_byte_cap,
            entries_per_scope_cap: config.vault_entries_per_scope_cap,
        });
    tracing::info!("vault enabled (in-memory)");

    // 5a-ter-bis. Wire the vault into the session driver (issue #102): every
    //         driver this service spawns resolves the session's inline-secret
    //         scope into an `env_profile` and injects it into the engine run.
    //         The VaultService is Clone (it joins AppState below too).
    sessions.enable_vault(vault.clone());

    // 5a-ter-quater. Wire per-session codex LLM-provider config into the driver
    //         (issue #112): every session renders a per-session CODEX_HOME
    //         config.toml selecting the provider — default the NyxID-proxied
    //         chrono-llm, with RAW/STRUCTURED vault overrides — so the engine's
    //         codex reaches a working LLM backend. The operator-pinned chrono-llm
    //         DEFAULT model + base URL are fail-closed config values (validated
    //         non-blank at load). Rendering also requires the vault (wired above);
    //         without it the driver skips CODEX_HOME (legacy behaviour).
    sessions.enable_codex(
        config.codex_model.clone(),
        config.chrono_llm_base_url.clone(),
    );
    tracing::info!("per-session codex provider config enabled");

    // 5a-ter-ter. Wire per-session NyxID token provisioning into the driver
    //         (issue #111): every session mints a per-session agent key on the
    //         triggering user's behalf and injects NYXID_ACCESS_TOKEN +
    //         NYXID_URL into the engine env, then revokes it at teardown. The
    //         origin is the NyxID issuer base URL (the SAME host that issues the
    //         inbound user JWTs we mint against). Requires the NyxID service
    //         client (built above) AND an enabled auth mode (which carries the
    //         base URL); when either is absent, provisioning stays disabled and
    //         the driver behaves exactly as pre-#111.
    match (&nyxid_client, &auth_mode) {
        (Some(client), fkst_control_plane::auth::AuthMode::Enabled(settings)) => {
            sessions.enable_nyxid_token(client.clone(), settings.base_url.clone());
            tracing::info!("per-session nyxid token provisioning enabled");
        }
        _ => {
            tracing::info!(
                "per-session nyxid token provisioning disabled \
                 (requires auth enabled with NYXID_CLIENT_ID/SECRET)"
            );
        }
    }

    // 5a-ter-quinquies. Wire the Ornn skill-registry client (issue #114): when
    //         a session pins Ornn skills/skillsets, the driver fetches them as
    //         the session user (via the #111 NyxID token through the `ornn-api`
    //         proxy) and installs them into the per-session CODEX_HOME (#112)
    //         before the engine spawns. The catalog API consumes the same
    //         client. Requires the NyxID service client (the proxy host); when
    //         absent, injection stays disabled and the catalog answers 503.
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
            tracing::info!(
                "ornn skill injection + catalog disabled (requires NYXID_CLIENT_ID/SECRET)"
            );
            None
        }
    };

    // 5a-bis. Enable goal support in the session service: goal-status sync
    //         writes + token refresh. Requires both the goals repo and the
    //         GitHub App tokens service.
    if let Some(ref gh_app) = github_app {
        sessions.enable_goal_support(goals.clone(), gh_app.clone());
        tracing::info!("goal support enabled in session service");
    } else {
        tracing::info!("github app not configured; goal support disabled in session service");
    }

    // Capture the internal worker-protocol config before `config` moves into
    // AppState (issue #134). `dispatch_mode_enabled` (#151 i7b) is captured the
    // same way: it gates whether the controller is enabled below.
    let internal_auth_token = config.internal_auth_token.clone();
    let worker_liveness_ttl_secs = config.worker_liveness_ttl_secs;
    let dispatch_mode_enabled = config.dispatch_mode_enabled;

    // 5a-quater. Controller-backed placement (#135, #198-ii): the in-memory
    //     `ClaimMap` is the SINGLE claim authority for placement, so it is built
    //     and enabled UNCONDITIONALLY — the in-process default path claims
    //     through it, and dispatch mode places workers through it. Built BEFORE
    //     the router so the SAME `Arc<ClaimMap>` + registry can be handed to the
    //     observability surface (`AppState.claims` / `worker_registry`, #144) as
    //     well as to the session service and `mount_internal` (below): a dispatch
    //     queued via the handle is drained by the heartbeat handler, the fence a
    //     worker echoes is checked against the live claim map, and the admin /
    //     metrics routes read that same live state. `dispatch_mode` rides on the
    //     handle so `create_for_goal` picks in-process vs worker-dispatch off it.
    let ttl = std::time::Duration::from_secs(worker_liveness_ttl_secs);
    let registry = WorkerRegistry::new(ttl);
    let claims = std::sync::Arc::new(ClaimMap::new());
    sessions.enable_controller(ControllerHandle::new(
        claims.clone(),
        registry.clone(),
        dispatch_max_load,
        dispatch_mode_enabled,
    ));
    tracing::info!(
        dispatch_mode = dispatch_mode_enabled,
        max_load = dispatch_max_load,
        "controller-backed placement enabled (in-memory claim authority)"
    );

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
        // The observability surface (#144) reads the SAME live claim authority +
        // registry the controller placement uses, so the admin/metrics view is
        // exact (never a copy that can drift).
        claims: Some(claims.clone()),
        worker_registry: Some(registry.clone()),
    }) {
        Ok(router) => router,
        Err(error) => {
            tracing::error!(error = %error, "failed to build router");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("router built");

    // 5b. Internal worker protocol (issue #134): when the shared secret is set,
    //     spawn the stale-worker expiry sweeper (cancelled on shutdown) and merge
    //     the shared-secret-guarded internal routes onto the top-level router,
    //     reusing the SAME registry + claims wired above. When the secret is
    //     absent the internal surface stays closed (not mounted), but the
    //     in-process claim authority is still live.
    let registry_sweeper_shutdown = CancellationToken::new();
    let (app, registry_sweeper_handle) = match internal_auth_token {
        Some(token) => {
            // Workers must heartbeat several times per TTL window to avoid a
            // false expiry; the controller is authoritative for this cadence.
            let heartbeat_interval_secs = (worker_liveness_ttl_secs / 3).max(1);
            let sweep_interval_secs = (worker_liveness_ttl_secs / 2).max(1);
            tracing::info!(
                route_prefix = "/internal/v1",
                "internal worker protocol enabled"
            );
            // Dispatch mode (#151 i7b / #140, default OFF): only when the operator
            // opts in do we wire the worker-reassignment driver. The driver MUST
            // share the SAME registry + claims so a redo lands on the live
            // outbound queue and re-fences the live claim. When OFF the reassign
            // driver stays None (goal sessions run in-process; the sweeper is
            // log-only).
            let reassign: Option<std::sync::Arc<fkst_control_plane::controller::ReassignDriver>> =
                if dispatch_mode_enabled {
                    // The real re-dispatch seam (#140): re-resolves a reassigned
                    // session's dispatch + queues it to the new worker. Shares the
                    // SAME registry so the redo lands on the live outbound queue.
                    let redispatch = sessions.make_redispatch(registry.clone());
                    let driver =
                        std::sync::Arc::new(fkst_control_plane::controller::ReassignDriver::new(
                            claims.clone(),
                            registry.clone(),
                            dispatch_max_load,
                            redispatch,
                        ));
                    tracing::info!(
                        max_load = dispatch_max_load,
                        "dispatch mode ENABLED: goal sessions are dispatched to workers; \
                         dead/drained workers are reassigned"
                    );
                    Some(driver)
                } else {
                    None
                };
            // The sweeper expires stale workers; under dispatch mode it ALSO
            // reassigns each expired worker's in-flight work onto a live worker
            // (the abrupt-death path: fence-bump + git-idempotency keep the redo
            // safe). With dispatch OFF (`reassign` is None) it stays log-only,
            // byte-identical to before.
            let sweep_registry = registry.clone();
            let sweeper_cancel = registry_sweeper_shutdown.clone();
            let sweep_reassign = reassign.clone();
            let handle = tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(sweep_interval_secs));
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            let expired = sweep_registry.expire_stale().await;
                            if !expired.is_empty() {
                                tracing::info!(count = expired.len(), "swept stale workers");
                                if let Some(reassign) = &sweep_reassign {
                                    for worker_id in &expired {
                                        let n = reassign.on_worker_dead(worker_id).await;
                                        if n > 0 {
                                            tracing::info!(
                                                worker_id = %worker_id,
                                                reassigned = n,
                                                "reassigned a dead worker's in-flight sessions"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        _ = sweeper_cancel.cancelled() => break,
                    }
                }
            });
            (
                mount_internal(
                    app,
                    registry,
                    InternalAuth::new(token),
                    heartbeat_interval_secs,
                    claims,
                    session_minter,
                    reassign,
                ),
                Some(handle),
            )
        }
        None => {
            tracing::warn!(
                "FKST_INTERNAL_AUTH_TOKEN not set; internal worker protocol disabled \
                 (internal routes not mounted)"
            );
            (app, None)
        }
    };

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
        // Still stop the worker-registry sweeper and drain the session drivers:
        // a serve error must not orphan live engine processes without a SIGTERM.
        registry_sweeper_shutdown.cancel();
        if let Some(handle) = registry_sweeper_handle {
            let _ = handle.await;
        }
        sessions.shutdown().await;
        return ExitCode::FAILURE;
    }

    // 7. Stop the worker-registry sweeper, then drain the session drivers
    //    (SIGTERM live engines, bounded wait).
    registry_sweeper_shutdown.cancel();
    if let Some(handle) = registry_sweeper_handle {
        let _ = handle.await;
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
