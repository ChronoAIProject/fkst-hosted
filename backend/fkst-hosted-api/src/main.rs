//! fkst-hosted-api server entrypoint: JSON tracing init, config load, router
//! build, and serving with graceful shutdown (SIGTERM / Ctrl-C).

use std::process::ExitCode;

use std::sync::Arc;

use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::{redact_mongodb_uri, Db};
use fkst_hosted_api::distribution::{DistributionConfig, Distributor, DriverHost, SelfOnlyHealth};
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::journal::store::MongoProgressStore;
use fkst_hosted_api::journal::JournalConfig;
use fkst_hosted_api::leases::LeaseStore;
use fkst_hosted_api::llm::gateway::NyxLlmGateway;
use fkst_hosted_api::llm::{LlmConfig, LlmGateway};
use fkst_hosted_api::nyxid::NyxIdClient;
use fkst_hosted_api::packages::{PackageRepository, ShareRepo};
use fkst_hosted_api::reconcile::{reconcile_orphans, ReconcileConfig};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use secrecy::ExposeSecret;
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
        mongodb_db = %config.mongodb_db,
        // Redacted host only — the full URI may embed credentials.
        mongodb_host = %redact_mongodb_uri(&config.mongodb_uri),
        "config loaded"
    );

    // 3. Connect to MongoDB (fail-closed: never serve without the store).
    let db = match Db::connect(&config).await {
        Ok(db) => db,
        Err(error) => {
            tracing::error!(error = %error, "failed to connect to mongodb");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("mongo connected");

    // 4. Ensure indexes (idempotent; fail-closed on error).
    if let Err(error) = db.ensure_indexes().await {
        tracing::error!(error = %error, "failed to ensure mongodb indexes");
        return ExitCode::FAILURE;
    }
    tracing::info!("indexes ensured");

    // 4a. Ensure the journal-collection indexes (idempotent; fail-closed on
    //     error) — `session_progress` + `run_journals`, incl. the unique
    //     partial `sp_run_idem_uniq` idempotency index.
    if let Err(error) = fkst_hosted_api::journal::index::ensure_journal_indexes(&db.database).await
    {
        tracing::error!(error = %error, "failed to ensure journal indexes");
        return ExitCode::FAILURE;
    }
    tracing::info!("journal indexes ensured");

    // 4b. Ensure the packages-collection indexes via the domain repository's
    //     own startup hook (idempotent; fail-closed on error). The same
    //     repository instance then joins AppState for the HTTP handlers.
    let packages = PackageRepository::new(&db.database);
    if let Err(error) = packages.ensure_indexes().await {
        tracing::error!(error = %error, "failed to ensure packages indexes");
        return ExitCode::FAILURE;
    }

    // 4b-bis. Ensure the package_shares-collection indexes (idempotent;
    //         fail-closed on error). Share-aware policy checks depend on these.
    let shares = ShareRepo::new(&db.database);
    if let Err(error) = shares.ensure_indexes().await {
        tracing::error!(error = %error, "failed to ensure package_shares indexes");
        return ExitCode::FAILURE;
    }

    // 4b-ter. Ensure the goals-collection indexes (idempotent; fail-closed on
    //          error). Goal CRUD and list queries depend on these.
    let goals = GoalRepo::new(&db.database);
    if let Err(error) = goals.ensure_indexes().await {
        tracing::error!(error = %error, "failed to ensure goals indexes");
        return ExitCode::FAILURE;
    }

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
    // The generate endpoint's conformance dry-run reaches the engine plumbing
    // through AppState; clone the config here because `engine_config` is moved
    // into the session service below.
    let state_engine_config = engine_config.clone();

    // 4d. Load the distribution configuration (fail-closed: a bad cadence
    //     or pod identity must never reach the lease layer).
    let distribution_config = match DistributionConfig::load_from_env() {
        Ok(distribution_config) => distribution_config,
        Err(error) => {
            tracing::error!(error = %error, "failed to load distribution configuration");
            return ExitCode::FAILURE;
        }
    };

    // 4e. Lease store + its indexes (idempotent; fail-closed on error), then
    //     the distributor over the self-only health view.
    let lease_store = LeaseStore::new(&db, &distribution_config.pool);
    if let Err(error) = lease_store.ensure_indexes().await {
        tracing::error!(error = %error, "failed to ensure lease indexes");
        return ExitCode::FAILURE;
    }
    let health = Arc::new(SelfOnlyHealth::new(
        db.clone(),
        distribution_config.pool.pod_id.clone(),
    ));
    let distributor = Distributor::new(db.clone(), lease_store, health, distribution_config);

    // 4f. Build the session service (lease-fenced drivers) and sweep
    //     orphans BEFORE binding: any pre-terminal session in Mongo refers
    //     to an engine process that died with the previous pod and must be
    //     failed — and its lease released — before clients can observe
    //     stale "running" state.
    let sessions = SessionService::with_distribution(
        SessionRepo::new(&db),
        packages.clone(),
        engine_config,
        distributor.clone(),
    );
    // Session-progress journaling (issue #25): Mongo floor always on;
    // GitHub sync per the FKST_JOURNAL_* config (absent repo/token degrades
    // to Mongo-only with a warn from the journaler).
    sessions.enable_journaling(
        JournalConfig::from_config(&config),
        MongoProgressStore::new(&db.database),
    );

    match distributor.fail_orphans_at_boot().await {
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
    //     (counted as skipped_unfenceable). FAIL-OPEN: a sweep error logs WARN
    //     and never blocks startup — cleaning is best-effort, unlike the
    //     fail-closed config/index steps above.
    match reconcile_orphans(&db, &reconcile_engine_config, &reconcile_config).await {
        Ok(report) => tracing::info!(
            scanned = report.scanned,
            swept = report.swept_count(),
            skipped_live = report.skipped_live,
            skipped_too_new = report.skipped_too_new,
            skipped_unfenceable = report.skipped_unfenceable,
            errors = report.error_count(),
            "orphan temp-dir reconciliation completed"
        ),
        Err(error) => tracing::warn!(
            error = %error,
            "orphan temp-dir reconciliation failed (non-fatal, continuing startup)"
        ),
    }

    // 4g. Spawn the takeover reaper, cancelled on shutdown.
    let reaper_shutdown = CancellationToken::new();
    let reaper_handle = tokio::spawn(Arc::new(distributor).run_reaper(
        Arc::new(sessions.clone()) as Arc<dyn DriverHost>,
        reaper_shutdown.clone(),
    ));

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
        (fkst_hosted_api::auth::AuthMode::Enabled(settings), Some(id), Some(secret)) => {
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
        (fkst_hosted_api::auth::AuthMode::Enabled(_), None, None) => {
            tracing::warn!("NyxID org features disabled: NYXID_CLIENT_ID/SECRET not configured");
            None
        }
        _ => None,
    };

    // The Authorizer is given the (optional) NyxID client and the ShareRepo for
    // share-aware policy checks.
    let authz = Authorizer::with_shares(nyxid_client.clone(), shares.clone());

    // Build the LLM gateway for package generation. When the gateway URL is set
    // (config has already verified the credentials + model are present), the
    // NyxID client MUST exist (auth enabled with service credentials); a missing
    // one is a fail-closed startup error. When the URL is unset, generation is
    // disabled and the endpoint answers 503.
    let llm: Option<Arc<dyn LlmGateway>> = match &config.llm_gateway_url {
        Some(url) => {
            let client = match &nyxid_client {
                Some(client) => client.clone(),
                None => {
                    tracing::error!(
                        "FKST_HOSTED_LLM_GATEWAY_URL set but NyxID service client unavailable \
                         (auth must be enabled with NYXID_CLIENT_ID/SECRET)"
                    );
                    return ExitCode::FAILURE;
                }
            };
            let model = config.llm_model.clone().unwrap_or_default();
            let llm_config = LlmConfig {
                gateway_url: url.clone(),
                model,
                timeout: std::time::Duration::from_secs(config.llm_timeout_secs),
                max_output_bytes: config.llm_max_output_bytes,
            };
            match NyxLlmGateway::new(client, llm_config) {
                Ok(gateway) => {
                    tracing::info!("llm package generation enabled");
                    Some(Arc::new(gateway))
                }
                Err(error) => {
                    tracing::error!(error = %error, "failed to build llm gateway");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => {
            tracing::info!("llm package generation disabled (FKST_HOSTED_LLM_GATEWAY_URL not set)");
            None
        }
    };

    // 5a. Load the GitHub App configuration (fail-closed: a bad PEM must
    //     never reach a live session). The token service is built WITH the
    //     Mongo-backed installation store (issue #108) so installation
    //     resolution reads persistence before probing GitHub and survives a pod
    //     restart. The webhook secret (if set) is lifted out into AppState so
    //     the router can mount the signature-verified webhook route.
    let mut github_app_webhook_secret: Option<secrecy::SecretString> = None;
    let github_app = match fkst_hosted_api::github_app::GithubAppConfig::load_from_env() {
        Ok(Some(config)) => {
            let app_id = config.app_id;
            github_app_webhook_secret = config.webhook_secret.clone();
            let store = std::sync::Arc::new(
                fkst_hosted_api::github_app::MongoInstallationStore::new(&db),
            );
            match fkst_hosted_api::github_app::GithubAppTokens::new_with_store(&config, store) {
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

    // 5a-ter. Build the vault service (issue #100). The vault is always-on: a
    //         missing OR invalid master-key source is a fail-closed startup
    //         error (mirrors the github_app PEM pattern), so the vault routes
    //         never run without an at-rest encryption key. The base64 key was
    //         resolved from FKST_HOSTED_VAULT_MASTER_KEY / _PATH by Config; here
    //         it is decoded + length-validated into the KEK and the unique index
    //         is ensured.
    let Some(vault_key) = &config.vault_master_key else {
        tracing::error!(
            "vault master key not configured; set FKST_HOSTED_VAULT_MASTER_KEY or \
             FKST_HOSTED_VAULT_MASTER_KEY_PATH (base64-encoded 32 bytes)"
        );
        return ExitCode::FAILURE;
    };
    let vault = match fkst_hosted_api::vault::VaultService::with_local_key(
        &db.database,
        vault_key.expose_secret(),
        fkst_hosted_api::vault::VaultLimits {
            value_byte_cap: config.vault_value_byte_cap,
            entries_per_scope_cap: config.vault_entries_per_scope_cap,
        },
    ) {
        Ok(vault) => vault,
        Err(error) => {
            tracing::error!(error = %error, "invalid vault master key");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = vault.repo().ensure_indexes().await {
        tracing::error!(error = %error, "failed to ensure vault indexes");
        return ExitCode::FAILURE;
    }
    tracing::info!("vault enabled");

    // 5a-ter-bis. Wire the vault into the session driver (issue #102): every
    //         driver this service spawns now resolves the session's vault
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
        (Some(client), fkst_hosted_api::auth::AuthMode::Enabled(settings)) => {
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
    let ornn_client: Option<fkst_hosted_api::ornn::OrnnClient> = match &nyxid_client {
        Some(client) => match fkst_hosted_api::ornn::OrnnClient::with_nyxid(
            client.clone(),
            fkst_hosted_api::ornn::DEFAULT_ORNN_SLUG,
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

    let app = match build_router(AppState {
        config,
        db,
        packages,
        shares,
        sessions: sessions.clone(),
        auth_mode,
        authz,
        github_app,
        github_app_webhook_secret,
        goals,
        engine: state_engine_config,
        llm,
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
        // Still stop the reaper and drain the session drivers: a serve
        // error must not orphan live engine processes without a SIGTERM.
        reaper_shutdown.cancel();
        let _ = reaper_handle.await;
        sessions.shutdown().await;
        return ExitCode::FAILURE;
    }

    // 7. Stop the reaper, then drain the session drivers (SIGTERM live
    //    engines, bounded wait).
    reaper_shutdown.cancel();
    let _ = reaper_handle.await;
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
