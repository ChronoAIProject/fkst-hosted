//! fkst-hosted-api server entrypoint: JSON tracing init, config load, router
//! build, and serving with graceful shutdown (SIGTERM / Ctrl-C).

use std::process::ExitCode;

use std::sync::Arc;

use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::{redact_mongodb_uri, Db};
use fkst_hosted_api::distribution::{DistributionConfig, Distributor, DriverHost, SelfOnlyHealth};
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::journal::store::MongoProgressStore;
use fkst_hosted_api::journal::JournalConfig;
use fkst_hosted_api::leases::LeaseStore;
use fkst_hosted_api::packages::PackageRepository;
use fkst_hosted_api::reconcile::{reconcile_orphans, ReconcileConfig};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
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
    let app = match build_router(AppState {
        config,
        db,
        packages,
        sessions: sessions.clone(),
        auth_mode,
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
