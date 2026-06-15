//! Typed runtime configuration loaded from environment variables.
//!
//! Two envy passes over the same snapshot of variables:
//! 1. the `FKST_HOSTED_*`-prefixed HTTP/server settings, and
//! 2. the unprefixed MongoDB settings (`MONGODB_URI` is required, fail-closed).

use std::fmt;

use secrecy::SecretString;
use serde::Deserialize;

use crate::auth::{AuthMode, NyxIdAuthSettings};
use crate::db::redact_mongodb_uri;
use crate::error::AppError;

/// Prefix shared by every HTTP/server configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_";

/// Prefix of the journaling variables (`FKST_JOURNAL_*` / `FKST_RAISED_*`).
const JOURNAL_ENV_PREFIX: &str = "FKST_";

/// Default values, shared by serde defaults and `Config::default`.
mod defaults {
    pub(super) fn port() -> u16 {
        8080
    }

    pub(super) fn bind_addr() -> String {
        "0.0.0.0".to_string()
    }

    pub(super) fn log_level() -> String {
        "info".to_string()
    }

    pub(super) fn request_timeout_secs() -> u64 {
        30
    }

    pub(super) fn mongodb_uri_placeholder() -> String {
        "mongodb://localhost:27017".to_string()
    }

    pub(super) fn mongodb_db() -> String {
        "fkst_hosted".to_string()
    }

    pub(super) fn mongodb_server_selection_timeout_ms() -> u64 {
        5000
    }

    pub(super) fn journal_flush_interval_ms() -> u64 {
        2000
    }

    pub(super) fn journal_flush_max_batch() -> usize {
        50
    }

    pub(super) fn journal_github_enabled() -> bool {
        true
    }

    pub(super) fn journal_issue_comments() -> bool {
        false
    }

    pub(super) fn journal_cas_max_retries() -> u32 {
        5
    }

    pub(super) fn journal_github_branch() -> String {
        "main".to_string()
    }

    pub(super) fn raised_identity_pointers() -> String {
        "/department,/source,/name,/corr".to_string()
    }

    pub(super) fn raised_max_line_bytes() -> usize {
        1_048_576
    }

    pub(super) fn auth_enabled() -> bool {
        // Default true: fail-closed at startup. Explicit `false` is a conscious
        // local-dev choice (the operator must set `FKST_AUTH_ENABLED=false` to
        // disable authentication).
        true
    }

    pub(super) fn auth_issuer() -> String {
        "nyxid".to_string()
    }

    pub(super) fn auth_jwks_cache_ttl_secs() -> u64 {
        300
    }

    pub(super) fn nyxid_org_cache_ttl_secs() -> u64 {
        30
    }

    pub(super) fn nyxid_github_proxy_slug() -> String {
        // NyxID `main`/v0.7.0 seeds its GitHub OAuth proxy under slug
        // `api-github` (`backend/src/services/provider_service.rs`,
        // `DefaultServiceSeed`); kept in sync with
        // `crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG`.
        crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG.to_string()
    }

    pub(super) fn llm_timeout_secs() -> u64 {
        20
    }

    pub(super) fn llm_max_output_bytes() -> usize {
        1_048_576
    }

    pub(super) fn vault_value_byte_cap() -> usize {
        65_536
    }

    pub(super) fn vault_entries_per_scope_cap() -> usize {
        100
    }

    pub(super) fn codex_model() -> String {
        // The model the chrono-llm DEFAULT codex provider serves (#112). The
        // operator pins it to whatever chrono-llm currently serves; this is a
        // sensible non-empty default, never a literal placeholder.
        "gpt-5-codex".to_string()
    }

    pub(super) fn chrono_llm_base_url() -> String {
        // The NyxID proxy slug for the admin-seeded chrono-llm service (#112).
        "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm".to_string()
    }
}

/// `FKST_HOSTED_*`-prefixed variables (HTTP/server settings).
#[derive(Debug, Deserialize)]
struct HttpVars {
    #[serde(default = "defaults::port")]
    port: u16,
    #[serde(default = "defaults::bind_addr")]
    bind_addr: String,
    #[serde(default = "defaults::log_level")]
    log_level: String,
    #[serde(default = "defaults::request_timeout_secs")]
    request_timeout_secs: u64,
    /// NyxID LLM-gateway base URL for package generation. Absent => generation
    /// is disabled (the endpoint answers 503). Env: `FKST_HOSTED_LLM_GATEWAY_URL`.
    #[serde(default)]
    llm_gateway_url: Option<String>,
    /// LLM model name routed by the gateway. Required when the gateway URL is
    /// set. Env: `FKST_HOSTED_LLM_MODEL`.
    #[serde(default)]
    llm_model: Option<String>,
    /// Per-request timeout (seconds) for one LLM completion call.
    /// Env: `FKST_HOSTED_LLM_TIMEOUT_SECS`. Default 20, zero rejected.
    #[serde(default = "defaults::llm_timeout_secs")]
    llm_timeout_secs: u64,
    /// Max bytes accepted from a single LLM completion before the draft is
    /// rejected. Env: `FKST_HOSTED_LLM_MAX_OUTPUT_BYTES`. Default 1 MiB,
    /// zero rejected.
    #[serde(default = "defaults::llm_max_output_bytes")]
    llm_max_output_bytes: usize,
    /// Vault KEK master key, base64-encoded 32 bytes (SECRET). One of this or
    /// `_PATH` must be set (vault is always-on, fail-closed). Env:
    /// `FKST_HOSTED_VAULT_MASTER_KEY`.
    #[serde(default)]
    vault_master_key: Option<String>,
    /// Path to a file holding the base64 vault master key; mutually exclusive
    /// with the inline key. Env: `FKST_HOSTED_VAULT_MASTER_KEY_PATH`.
    #[serde(default)]
    vault_master_key_path: Option<String>,
    /// Max bytes for a single vault value. Env:
    /// `FKST_HOSTED_VAULT_VALUE_BYTE_CAP`. Default 65536, zero rejected.
    #[serde(default = "defaults::vault_value_byte_cap")]
    vault_value_byte_cap: usize,
    /// Max vault entries an owner may hold per scope. Env:
    /// `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP`. Default 100, zero rejected.
    #[serde(default = "defaults::vault_entries_per_scope_cap")]
    vault_entries_per_scope_cap: usize,
    /// Operator-pinned model the chrono-llm DEFAULT codex provider serves
    /// (#112). Env: `FKST_HOSTED_CODEX_MODEL`. Default `gpt-5-codex`; blank
    /// rejected at load (a misconfigured default must fail closed, never render
    /// an unusable codex config).
    #[serde(default = "defaults::codex_model")]
    codex_model: String,
    /// NyxID proxy base URL for the chrono-llm DEFAULT codex provider (#112).
    /// Env: `FKST_HOSTED_CHRONO_LLM_BASE_URL`. Default the seeded chrono-llm
    /// slug; blank rejected at load. Non-secret (it is a route, like
    /// `llm_gateway_url`).
    #[serde(default = "defaults::chrono_llm_base_url")]
    chrono_llm_base_url: String,
}

/// Unprefixed MongoDB variables. `MONGODB_URI` has no default: a backend
/// without a store is misconfigured and must fail closed at startup.
/// `GITHUB_TOKEN` rides this unprefixed pass too (secret; optional —
/// without it GitHub journaling degrades to Mongo-only with a warn).
/// `NYXID_CLIENT_ID` / `NYXID_CLIENT_SECRET` also ride this pass (platform
/// credentials are unprefixed, following the `GITHUB_TOKEN` precedent).
/// Both-or-neither: only one set is a config error naming the missing var.
#[derive(Deserialize)]
struct MongoVars {
    mongodb_uri: String,
    #[serde(default = "defaults::mongodb_db")]
    mongodb_db: String,
    #[serde(default = "defaults::mongodb_server_selection_timeout_ms")]
    mongodb_server_selection_timeout_ms: u64,
    #[serde(default)]
    github_token: Option<String>,
    #[serde(default)]
    nyxid_client_id: Option<String>,
    #[serde(default)]
    nyxid_client_secret: Option<String>,
}

/// `FKST_JOURNAL_*` / `FKST_RAISED_*` variables (journaling settings; envy
/// pass with the `FKST_` prefix).
#[derive(Debug, Deserialize)]
struct JournalVars {
    #[serde(default = "defaults::journal_flush_interval_ms")]
    journal_flush_interval_ms: u64,
    #[serde(default = "defaults::journal_flush_max_batch")]
    journal_flush_max_batch: usize,
    #[serde(default = "defaults::journal_github_enabled")]
    journal_github_enabled: bool,
    #[serde(default = "defaults::journal_issue_comments")]
    journal_issue_comments: bool,
    #[serde(default = "defaults::journal_cas_max_retries")]
    journal_cas_max_retries: u32,
    #[serde(default = "defaults::journal_github_branch")]
    journal_github_branch: String,
    #[serde(default)]
    journal_github_repo: Option<String>,
    #[serde(default = "defaults::raised_identity_pointers")]
    raised_identity_pointers: String,
    #[serde(default = "defaults::raised_max_line_bytes")]
    raised_max_line_bytes: usize,
}

/// `FKST_AUTH_*`-prefixed variables (authentication settings; envy pass with
/// the `FKST_` prefix).
#[derive(Debug, Deserialize)]
struct AuthVars {
    #[serde(default = "defaults::auth_enabled")]
    auth_enabled: bool,
    #[serde(default)]
    auth_nyxid_base_url: Option<String>,
    #[serde(default = "defaults::auth_issuer")]
    auth_issuer: String,
    #[serde(default)]
    auth_audience: Option<String>,
    #[serde(default = "defaults::auth_jwks_cache_ttl_secs")]
    auth_jwks_cache_ttl_secs: u64,
    #[serde(default = "defaults::nyxid_org_cache_ttl_secs")]
    nyxid_org_cache_ttl_secs: u64,
    #[serde(default = "defaults::nyxid_github_proxy_slug")]
    nyxid_github_proxy_slug: String,
}

/// Runtime configuration assembled from both envy passes.
#[derive(Clone)]
pub struct Config {
    /// TCP port the HTTP server binds. Env: `FKST_HOSTED_PORT`. Default 8080.
    pub port: u16,
    /// Bind address. Env: `FKST_HOSTED_BIND_ADDR`. Default "0.0.0.0".
    pub bind_addr: String,
    /// tracing-subscriber `EnvFilter` directive. Env: `FKST_HOSTED_LOG_LEVEL`.
    /// Default "info".
    pub log_level: String,
    /// Per-request timeout in seconds for the tower-http `TimeoutLayer`.
    /// Env: `FKST_HOSTED_REQUEST_TIMEOUT_SECS`. Default 30.
    pub request_timeout_secs: u64,
    /// MongoDB connection string (may embed credentials — never log it in
    /// full). Env: `MONGODB_URI`. Required, fail-closed.
    pub mongodb_uri: String,
    /// Logical MongoDB database name. Env: `MONGODB_DB`.
    /// Default "fkst_hosted".
    pub mongodb_db: String,
    /// Driver server-selection timeout in milliseconds; bounds the startup
    /// ping and `/health` so an unreachable Mongo fails fast.
    /// Env: `MONGODB_SERVER_SELECTION_TIMEOUT_MS`. Default 5000.
    pub mongodb_server_selection_timeout_ms: u64,
    /// Max debounce (ms) before flushing buffered completions to GitHub.
    /// Env: `FKST_JOURNAL_FLUSH_INTERVAL_MS`. Default 2000.
    pub journal_flush_interval_ms: u64,
    /// Flush early when this many new completions are buffered.
    /// Env: `FKST_JOURNAL_FLUSH_MAX_BATCH`. Default 50.
    pub journal_flush_max_batch: usize,
    /// Master switch for GitHub journaling (Mongo journaling is always on).
    /// Env: `FKST_JOURNAL_GITHUB_ENABLED`. Default true.
    pub journal_github_enabled: bool,
    /// Enable the optional issue-comment mirroring (dormant by default).
    /// Env: `FKST_JOURNAL_ISSUE_COMMENTS`. Default false.
    pub journal_issue_comments: bool,
    /// Max optimistic-concurrency retries on the GitHub Contents write per
    /// flush. Env: `FKST_JOURNAL_CAS_MAX_RETRIES`. Default 5.
    pub journal_cas_max_retries: u32,
    /// Branch the journal file lives on.
    /// Env: `FKST_JOURNAL_GITHUB_BRANCH`. Default "main".
    pub journal_github_branch: String,
    /// `owner/name` of the journal repo; absent => GitHub journaling is
    /// disabled (Mongo-only, warn). Env: `FKST_JOURNAL_GITHUB_REPO`.
    pub journal_github_repo: Option<String>,
    /// Comma-separated JSON pointers forming raised-event identity.
    /// Env: `FKST_RAISED_IDENTITY_POINTERS`.
    /// Default "/department,/source,/name,/corr".
    pub raised_identity_pointers: String,
    /// Max stdout line length parsed; longer lines are truncated + counted
    /// as malformed. Env: `FKST_RAISED_MAX_LINE_BYTES`. Default 1048576.
    pub raised_max_line_bytes: usize,
    /// GitHub API token (SECRET — env/secret manager only; never logged,
    /// redacted from Debug). Env: `GITHUB_TOKEN`. Optional: absent =>
    /// GitHub journaling is disabled (Mongo-only, warn).
    pub github_token: Option<SecretString>,
    /// Authentication mode: disabled (local dev) or enabled with NyxID
    /// settings. Env: `FKST_AUTH_ENABLED` (default true = fail-closed).
    pub auth: AuthMode,
    /// NyxID service-account client ID for org APIs. Env: `NYXID_CLIENT_ID`.
    /// Both-or-neither with `nyxid_client_secret`. Optional: absent means
    /// org features degrade gracefully (owner-only policy).
    pub nyxid_client_id: Option<String>,
    /// NyxID service-account client secret (SECRET). Env:
    /// `NYXID_CLIENT_SECRET`. Both-or-neither with `nyxid_client_id`.
    pub nyxid_client_secret: Option<SecretString>,
    /// TTL in seconds for the NyxID org-role and user-orgs caches.
    /// Env: `FKST_NYXID_ORG_CACHE_TTL_SECS`. Default 30, zero rejected.
    pub nyxid_org_cache_ttl_secs: u64,
    /// Downstream-service slug NyxID resolves to inject the user's GitHub
    /// credential on proxied requests; the client builds the proxy base path
    /// `/api/v1/proxy/{slug}` from it. Env: `FKST_NYXID_GITHUB_PROXY_SLUG`.
    /// Default `api-github` (the slug NyxID `main`/v0.7.0 seeds). Rejected when
    /// blank (fail-closed: an empty slug yields an unresolvable proxy route).
    pub nyxid_github_proxy_slug: String,
    /// NyxID LLM-gateway base URL for package generation. `None` => the
    /// generate endpoint is disabled (answers 503). Env:
    /// `FKST_HOSTED_LLM_GATEWAY_URL`. Non-secret (the route is logged): a set
    /// URL requires both NyxID service-account credentials and a model name.
    pub llm_gateway_url: Option<String>,
    /// LLM model name the gateway routes by; required when the gateway URL is
    /// set (fail-closed). Env: `FKST_HOSTED_LLM_MODEL`.
    pub llm_model: Option<String>,
    /// Per-request timeout (seconds) for one LLM completion call.
    /// Env: `FKST_HOSTED_LLM_TIMEOUT_SECS`. Default 20, zero rejected.
    pub llm_timeout_secs: u64,
    /// Max bytes accepted from a single LLM completion before the generated
    /// draft is rejected (a retry budget guard against a runaway model).
    /// Env: `FKST_HOSTED_LLM_MAX_OUTPUT_BYTES`. Default 1 MiB, zero rejected.
    pub llm_max_output_bytes: usize,
    /// Vault KEK master key (SECRET): the base64-encoded 32 bytes, resolved
    /// from `FKST_HOSTED_VAULT_MASTER_KEY` or read from
    /// `FKST_HOSTED_VAULT_MASTER_KEY_PATH`. `None` only in the `Config::default`
    /// fixture (tests); production startup fails closed when the vault key
    /// source is absent. Never logged (redacted from `Debug`).
    pub vault_master_key: Option<SecretString>,
    /// Max bytes for a single vault value. Env:
    /// `FKST_HOSTED_VAULT_VALUE_BYTE_CAP`. Default 65536, zero rejected.
    pub vault_value_byte_cap: usize,
    /// Max vault entries an owner may hold per scope. Env:
    /// `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP`. Default 100, zero rejected.
    pub vault_entries_per_scope_cap: usize,
    /// Operator-pinned model the chrono-llm DEFAULT codex provider serves
    /// (#112). Env: `FKST_HOSTED_CODEX_MODEL`. Default `gpt-5-codex`; blank
    /// rejected at load (fail-closed). Non-secret routing config.
    pub codex_model: String,
    /// NyxID proxy base URL for the chrono-llm DEFAULT codex provider (#112).
    /// Env: `FKST_HOSTED_CHRONO_LLM_BASE_URL`. Default the seeded chrono-llm
    /// slug; blank rejected at load. Non-secret routing config.
    pub chrono_llm_base_url: String,
}

// Hand-written so the URI (which may embed credentials) is always printed
// through the redaction helper — `{:?}` can never leak a password.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("port", &self.port)
            .field("bind_addr", &self.bind_addr)
            .field("log_level", &self.log_level)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("mongodb_uri", &redact_mongodb_uri(&self.mongodb_uri))
            .field("mongodb_db", &self.mongodb_db)
            .field(
                "mongodb_server_selection_timeout_ms",
                &self.mongodb_server_selection_timeout_ms,
            )
            .field("journal_flush_interval_ms", &self.journal_flush_interval_ms)
            .field("journal_flush_max_batch", &self.journal_flush_max_batch)
            .field("journal_github_enabled", &self.journal_github_enabled)
            .field("journal_issue_comments", &self.journal_issue_comments)
            .field("journal_cas_max_retries", &self.journal_cas_max_retries)
            .field("journal_github_branch", &self.journal_github_branch)
            .field("journal_github_repo", &self.journal_github_repo)
            .field("raised_identity_pointers", &self.raised_identity_pointers)
            .field("raised_max_line_bytes", &self.raised_max_line_bytes)
            // The token value never reaches any Debug/log output.
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "<redacted>"),
            )
            .field("auth", &self.auth)
            .field("nyxid_client_id", &self.nyxid_client_id)
            .field(
                "nyxid_client_secret",
                &self.nyxid_client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("nyxid_org_cache_ttl_secs", &self.nyxid_org_cache_ttl_secs)
            .field("nyxid_github_proxy_slug", &self.nyxid_github_proxy_slug)
            // URL/model/numbers are non-secret routing config — show them.
            .field("llm_gateway_url", &self.llm_gateway_url)
            .field("llm_model", &self.llm_model)
            .field("llm_timeout_secs", &self.llm_timeout_secs)
            .field("llm_max_output_bytes", &self.llm_max_output_bytes)
            // The vault master key never reaches any Debug/log output.
            .field(
                "vault_master_key",
                &self.vault_master_key.as_ref().map(|_| "<redacted>"),
            )
            .field("vault_value_byte_cap", &self.vault_value_byte_cap)
            .field(
                "vault_entries_per_scope_cap",
                &self.vault_entries_per_scope_cap,
            )
            // Model name + proxy route are non-secret config — show them.
            .field("codex_model", &self.codex_model)
            .field("chrono_llm_base_url", &self.chrono_llm_base_url)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            bind_addr: defaults::bind_addr(),
            log_level: defaults::log_level(),
            request_timeout_secs: defaults::request_timeout_secs(),
            mongodb_uri: defaults::mongodb_uri_placeholder(),
            mongodb_db: defaults::mongodb_db(),
            mongodb_server_selection_timeout_ms: defaults::mongodb_server_selection_timeout_ms(),
            journal_flush_interval_ms: defaults::journal_flush_interval_ms(),
            journal_flush_max_batch: defaults::journal_flush_max_batch(),
            journal_github_enabled: defaults::journal_github_enabled(),
            journal_issue_comments: defaults::journal_issue_comments(),
            journal_cas_max_retries: defaults::journal_cas_max_retries(),
            journal_github_branch: defaults::journal_github_branch(),
            journal_github_repo: None,
            raised_identity_pointers: defaults::raised_identity_pointers(),
            raised_max_line_bytes: defaults::raised_max_line_bytes(),
            github_token: None,
            auth: AuthMode::Disabled,
            nyxid_client_id: None,
            nyxid_client_secret: None,
            nyxid_org_cache_ttl_secs: defaults::nyxid_org_cache_ttl_secs(),
            nyxid_github_proxy_slug: defaults::nyxid_github_proxy_slug(),
            llm_gateway_url: None,
            llm_model: None,
            llm_timeout_secs: defaults::llm_timeout_secs(),
            llm_max_output_bytes: defaults::llm_max_output_bytes(),
            vault_master_key: None,
            vault_value_byte_cap: defaults::vault_value_byte_cap(),
            vault_entries_per_scope_cap: defaults::vault_entries_per_scope_cap(),
            codex_model: defaults::codex_model(),
            chrono_llm_base_url: defaults::chrono_llm_base_url(),
        }
    }
}

impl Config {
    /// Deserialize a `Config` from environment-style key/value pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment. The pairs are collected once and fed to both
    /// envy passes (prefixed HTTP vars, unprefixed Mongo vars).
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Result<Config, AppError> {
        let vars: Vec<(String, String)> = vars.into_iter().collect();

        let http: HttpVars = envy::prefixed(ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        // A zero timeout would make every request time out (408) — a total
        // outage from a one-character misconfiguration. Reject it loudly.
        if http.request_timeout_secs == 0 {
            return Err(AppError::Config(
                "FKST_HOSTED_REQUEST_TIMEOUT_SECS must be at least 1".to_string(),
            ));
        }
        // A zero LLM timeout would fail every completion instantly and a zero
        // output cap would reject every draft — reject both loudly, mirroring
        // the request-timeout guard above.
        if http.llm_timeout_secs == 0 {
            return Err(AppError::Config(
                "FKST_HOSTED_LLM_TIMEOUT_SECS must be at least 1".to_string(),
            ));
        }
        if http.llm_max_output_bytes == 0 {
            return Err(AppError::Config(
                "FKST_HOSTED_LLM_MAX_OUTPUT_BYTES must be at least 1".to_string(),
            ));
        }

        let journal: JournalVars = envy::prefixed(JOURNAL_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        // A zero interval would force a GitHub round-trip per record and a
        // zero retry budget would fail every flush instantly — reject both
        // loudly, mirroring the timeout guards above.
        if journal.journal_flush_interval_ms == 0 {
            return Err(AppError::Config(
                "FKST_JOURNAL_FLUSH_INTERVAL_MS must be at least 1".to_string(),
            ));
        }
        if journal.journal_flush_max_batch == 0 {
            return Err(AppError::Config(
                "FKST_JOURNAL_FLUSH_MAX_BATCH must be at least 1".to_string(),
            ));
        }
        if journal.journal_cas_max_retries == 0 {
            return Err(AppError::Config(
                "FKST_JOURNAL_CAS_MAX_RETRIES must be at least 1".to_string(),
            ));
        }
        if journal.raised_max_line_bytes == 0 {
            return Err(AppError::Config(
                "FKST_RAISED_MAX_LINE_BYTES must be at least 1".to_string(),
            ));
        }

        let mongo: MongoVars = envy::from_iter(vars.clone()).map_err(|e| match e {
            // Name the exact env var so the fail-closed startup error is
            // actionable (envy reports the lowercase field name).
            envy::Error::MissingValue(field) => {
                AppError::Config(format!("{} must be set", field.to_uppercase()))
            }
            other => AppError::Config(other.to_string()),
        })?;
        // A zero selection timeout would make every Mongo operation fail
        // instantly (or fall back to a driver default) — reject it loudly,
        // mirroring the request-timeout guard above.
        if mongo.mongodb_server_selection_timeout_ms == 0 {
            return Err(AppError::Config(
                "MONGODB_SERVER_SELECTION_TIMEOUT_MS must be at least 1".to_string(),
            ));
        }
        // Both-or-neither: NYXID_CLIENT_ID and NYXID_CLIENT_SECRET.
        let (nyxid_client_id, nyxid_client_secret) =
            match (mongo.nyxid_client_id, mongo.nyxid_client_secret) {
                (Some(id), Some(secret)) => (Some(id), Some(SecretString::from(secret))),
                (None, None) => (None, None),
                (Some(_), None) => {
                    return Err(AppError::Config(
                        "NYXID_CLIENT_SECRET must be set when NYXID_CLIENT_ID is set".to_string(),
                    ));
                }
                (None, Some(_)) => {
                    return Err(AppError::Config(
                        "NYXID_CLIENT_ID must be set when NYXID_CLIENT_SECRET is set".to_string(),
                    ));
                }
            };

        // Authentication settings pass (FKST_AUTH_* with the FKST_ prefix).
        let auth: AuthVars = envy::prefixed(JOURNAL_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        if auth.auth_jwks_cache_ttl_secs == 0 {
            return Err(AppError::Config(
                "FKST_AUTH_JWKS_CACHE_TTL_SECS must be at least 1".to_string(),
            ));
        }
        if auth.nyxid_org_cache_ttl_secs == 0 {
            return Err(AppError::Config(
                "FKST_NYXID_ORG_CACHE_TTL_SECS must be at least 1".to_string(),
            ));
        }
        // Fail-closed: a blank slug builds `/api/v1/proxy/` which NyxID cannot
        // resolve to a downstream GitHub credential, so reject it loudly rather
        // than degrade GitHub proxying silently. `trim` also rejects whitespace.
        if auth.nyxid_github_proxy_slug.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_NYXID_GITHUB_PROXY_SLUG must not be blank".to_string(),
            ));
        }
        let auth_mode = if auth.auth_enabled {
            let base_url = match auth.auth_nyxid_base_url {
                Some(url) => url.trim_end_matches('/').to_string(),
                None => {
                    return Err(AppError::Config(
                        "FKST_AUTH_NYXID_BASE_URL must be set when FKST_AUTH_ENABLED=true"
                            .to_string(),
                    ));
                }
            };
            let audience = auth.auth_audience.unwrap_or_else(|| base_url.clone());
            AuthMode::Enabled(NyxIdAuthSettings {
                base_url,
                issuer: auth.auth_issuer,
                audience,
                jwks_cache_ttl: std::time::Duration::from_secs(auth.auth_jwks_cache_ttl_secs),
            })
        } else {
            AuthMode::Disabled
        };

        // Fail-closed wiring for LLM package generation: a configured gateway
        // is useless without the service-account credentials that mint its
        // bearer token, and the gateway routes by model name, so an unset model
        // would be an unroutable call. Reject both at startup, naming the var.
        if http.llm_gateway_url.is_some()
            && (nyxid_client_id.is_none() || nyxid_client_secret.is_none())
        {
            return Err(AppError::Config(
                "FKST_HOSTED_LLM_GATEWAY_URL requires NYXID_CLIENT_ID and NYXID_CLIENT_SECRET"
                    .to_string(),
            ));
        }
        if http.llm_gateway_url.is_some() && http.llm_model.is_none() {
            return Err(AppError::Config(
                "FKST_HOSTED_LLM_MODEL must be set when FKST_HOSTED_LLM_GATEWAY_URL is set"
                    .to_string(),
            ));
        }

        // Vault key/cap validation (fail-closed): the vault is always-on, so a
        // missing or invalid master-key source is a startup error. The inline
        // key and the path are mutually exclusive. The base64 bytes are NOT
        // validated for length here (that is the crypto provider's job at
        // boot); the config only resolves the source into a SecretString and
        // names the missing/conflicting env vars.
        if http.vault_value_byte_cap == 0 {
            return Err(AppError::Config(
                "FKST_HOSTED_VAULT_VALUE_BYTE_CAP must be at least 1".to_string(),
            ));
        }
        if http.vault_entries_per_scope_cap == 0 {
            return Err(AppError::Config(
                "FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP must be at least 1".to_string(),
            ));
        }
        // Codex chrono-llm DEFAULT (fail-closed): both values have serde
        // defaults so the default path works out of the box, but a blank
        // override would render an unusable codex config.toml (no model /
        // unroutable base_url). Reject it loudly at startup, naming the var.
        if http.codex_model.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_HOSTED_CODEX_MODEL must not be blank".to_string(),
            ));
        }
        if http.chrono_llm_base_url.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_HOSTED_CHRONO_LLM_BASE_URL must not be blank".to_string(),
            ));
        }
        let vault_master_key = match (&http.vault_master_key, &http.vault_master_key_path) {
            (Some(_), Some(_)) => {
                return Err(AppError::Config(
                    "both FKST_HOSTED_VAULT_MASTER_KEY and FKST_HOSTED_VAULT_MASTER_KEY_PATH set; \
                     provide exactly one"
                        .to_string(),
                ));
            }
            (Some(inline), None) => Some(SecretString::from(inline.clone())),
            (None, Some(path)) => {
                let contents = std::fs::read_to_string(path).map_err(|e| {
                    AppError::Config(format!(
                        "FKST_HOSTED_VAULT_MASTER_KEY_PATH: failed to read {path}: {e}"
                    ))
                })?;
                Some(SecretString::from(contents))
            }
            // No key source: leave None. Whether that is fatal is decided at
            // boot (production: fail closed; the Disabled-auth local-dev path
            // may run without the vault). Tests build Config directly.
            (None, None) => None,
        };

        Ok(Config {
            port: http.port,
            bind_addr: http.bind_addr,
            log_level: http.log_level,
            request_timeout_secs: http.request_timeout_secs,
            mongodb_uri: mongo.mongodb_uri,
            mongodb_db: mongo.mongodb_db,
            mongodb_server_selection_timeout_ms: mongo.mongodb_server_selection_timeout_ms,
            journal_flush_interval_ms: journal.journal_flush_interval_ms,
            journal_flush_max_batch: journal.journal_flush_max_batch,
            journal_github_enabled: journal.journal_github_enabled,
            journal_issue_comments: journal.journal_issue_comments,
            journal_cas_max_retries: journal.journal_cas_max_retries,
            journal_github_branch: journal.journal_github_branch,
            journal_github_repo: journal.journal_github_repo,
            raised_identity_pointers: journal.raised_identity_pointers,
            raised_max_line_bytes: journal.raised_max_line_bytes,
            github_token: mongo.github_token.map(SecretString::from),
            auth: auth_mode,
            nyxid_client_id,
            nyxid_client_secret,
            nyxid_org_cache_ttl_secs: auth.nyxid_org_cache_ttl_secs,
            nyxid_github_proxy_slug: auth.nyxid_github_proxy_slug,
            llm_gateway_url: http.llm_gateway_url,
            llm_model: http.llm_model,
            llm_timeout_secs: http.llm_timeout_secs,
            llm_max_output_bytes: http.llm_max_output_bytes,
            vault_master_key,
            vault_value_byte_cap: http.vault_value_byte_cap,
            vault_entries_per_scope_cap: http.vault_entries_per_scope_cap,
            codex_model: http.codex_model,
            chrono_llm_base_url: http.chrono_llm_base_url,
        })
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<Config, AppError> {
        Self::from_vars(std::env::vars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid URI: every `from_vars` test input needs `MONGODB_URI`.
    const URI: (&str, &str) = ("MONGODB_URI", "mongodb://localhost:27017");

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_only_mongodb_uri_is_set() {
        let config = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")]))
            .expect("defaults should deserialize");
        assert_eq!(config.port, 8080);
        assert_eq!(config.bind_addr, "0.0.0.0");
        assert_eq!(config.log_level, "info");
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.mongodb_uri, "mongodb://localhost:27017");
        assert_eq!(config.mongodb_db, "fkst_hosted");
        assert_eq!(config.mongodb_server_selection_timeout_ms, 5000);
        assert!(matches!(config.auth, AuthMode::Disabled));
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")]))
            .expect("defaults should deserialize");
        let from_default = Config::default();
        assert_eq!(from_default.port, from_env.port);
        assert_eq!(from_default.bind_addr, from_env.bind_addr);
        assert_eq!(from_default.log_level, from_env.log_level);
        assert_eq!(
            from_default.request_timeout_secs,
            from_env.request_timeout_secs
        );
        assert_eq!(from_default.mongodb_uri, from_env.mongodb_uri);
        assert_eq!(from_default.mongodb_db, from_env.mongodb_db);
        assert_eq!(
            from_default.mongodb_server_selection_timeout_ms,
            from_env.mongodb_server_selection_timeout_ms
        );
    }

    #[test]
    fn missing_mongodb_uri_is_a_config_error_naming_the_env_var() {
        let err = Config::from_vars(vars(&[])).expect_err("missing MONGODB_URI must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("MONGODB_URI"),
            "error must name MONGODB_URI, got: {err}"
        );
    }

    #[test]
    fn port_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_PORT", "9090"),
        ]))
        .unwrap();
        assert_eq!(config.port, 9090);
    }

    #[test]
    fn bind_addr_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_BIND_ADDR", "127.0.0.1"),
        ]))
        .unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1");
    }

    #[test]
    fn log_level_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_LOG_LEVEL", "debug"),
        ]))
        .unwrap();
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn request_timeout_secs_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "5"),
        ]))
        .unwrap();
        assert_eq!(config.request_timeout_secs, 5);
    }

    #[test]
    fn mongodb_uri_is_read_from_env() {
        let config = Config::from_vars(vars(&[
            ("MONGODB_URI", "mongodb://mongo.svc:27017"),
            ("FKST_AUTH_ENABLED", "false"),
        ]))
        .unwrap();
        assert_eq!(config.mongodb_uri, "mongodb://mongo.svc:27017");
    }

    #[test]
    fn mongodb_db_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("MONGODB_DB", "other_db"),
        ]))
        .unwrap();
        assert_eq!(config.mongodb_db, "other_db");
    }

    #[test]
    fn mongodb_server_selection_timeout_ms_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "750"),
        ]))
        .unwrap();
        assert_eq!(config.mongodb_server_selection_timeout_ms, 750);
    }

    #[test]
    fn non_numeric_mongodb_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "soon"),
        ]))
        .expect_err("non-numeric timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn zero_mongodb_selection_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "0"),
        ]))
        .expect_err("zero selection timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string()
                .contains("MONGODB_SERVER_SELECTION_TIMEOUT_MS"),
            "error must name the env var, got: {err}"
        );
    }

    #[test]
    fn zero_request_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "0"),
        ]))
        .expect_err("zero timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_HOSTED_REQUEST_TIMEOUT_SECS"));
    }

    #[test]
    fn non_numeric_port_is_a_config_error() {
        let err = Config::from_vars(vars(&[URI, ("FKST_HOSTED_PORT", "abc")]))
            .expect_err("non-numeric port must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn journal_defaults_apply_when_unset() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.journal_flush_interval_ms, 2000);
        assert_eq!(config.journal_flush_max_batch, 50);
        assert!(config.journal_github_enabled);
        assert!(!config.journal_issue_comments);
        assert_eq!(config.journal_cas_max_retries, 5);
        assert_eq!(config.journal_github_branch, "main");
        assert_eq!(config.journal_github_repo, None);
        assert_eq!(
            config.raised_identity_pointers,
            "/department,/source,/name,/corr"
        );
        assert_eq!(config.raised_max_line_bytes, 1_048_576);
        assert!(config.github_token.is_none());
    }

    #[test]
    fn journal_vars_are_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_JOURNAL_FLUSH_INTERVAL_MS", "500"),
            ("FKST_JOURNAL_FLUSH_MAX_BATCH", "10"),
            ("FKST_JOURNAL_GITHUB_ENABLED", "false"),
            ("FKST_JOURNAL_ISSUE_COMMENTS", "true"),
            ("FKST_JOURNAL_CAS_MAX_RETRIES", "9"),
            ("FKST_JOURNAL_GITHUB_BRANCH", "journal"),
            ("FKST_JOURNAL_GITHUB_REPO", "acme/pkg-repo"),
            ("FKST_RAISED_IDENTITY_POINTERS", "/dept,/evt"),
            ("FKST_RAISED_MAX_LINE_BYTES", "2048"),
        ]))
        .expect("overrides");
        assert_eq!(config.journal_flush_interval_ms, 500);
        assert_eq!(config.journal_flush_max_batch, 10);
        assert!(!config.journal_github_enabled);
        assert!(config.journal_issue_comments);
        assert_eq!(config.journal_cas_max_retries, 9);
        assert_eq!(config.journal_github_branch, "journal");
        assert_eq!(config.journal_github_repo.as_deref(), Some("acme/pkg-repo"));
        assert_eq!(config.raised_identity_pointers, "/dept,/evt");
        assert_eq!(config.raised_max_line_bytes, 2048);
    }

    #[test]
    fn zero_journal_knobs_are_config_errors_naming_the_var() {
        for (var, value) in [
            ("FKST_JOURNAL_FLUSH_INTERVAL_MS", "0"),
            ("FKST_JOURNAL_FLUSH_MAX_BATCH", "0"),
            ("FKST_JOURNAL_CAS_MAX_RETRIES", "0"),
            ("FKST_RAISED_MAX_LINE_BYTES", "0"),
        ] {
            let err = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false"), (var, value)]))
                .expect_err("zero must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }

    #[test]
    fn non_boolean_journal_switch_is_a_config_error() {
        let err = Config::from_vars(vars(&[URI, ("FKST_JOURNAL_GITHUB_ENABLED", "yep")]))
            .expect_err("non-boolean must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn github_token_is_read_and_never_appears_in_debug() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("GITHUB_TOKEN", "ghp_sneaky_value"),
        ]))
        .expect("token config");
        assert!(config.github_token.is_some());
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("ghp_sneaky_value"), "token leaked");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn debug_output_redacts_mongodb_credentials() {
        let config = Config {
            mongodb_uri: "mongodb://user:hunter2@mongo.svc:27017".to_string(),
            ..Config::default()
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("hunter2"), "debug leaked the password");
        assert!(rendered.contains("mongodb://<redacted>@mongo.svc:27017"));
    }

    // ---- auth configuration tests ----------------------------------------------

    #[test]
    fn auth_enabled_without_base_url_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "true")]))
            .expect_err("enabled without base URL must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_AUTH_NYXID_BASE_URL"),
            "error must name the variable, got: {err}"
        );
    }

    #[test]
    fn auth_enabled_with_base_url_builds_enabled_mode() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com/"),
        ]))
        .expect("enabled with base URL");
        match config.auth {
            AuthMode::Enabled(ref settings) => {
                // Trailing slash must be trimmed.
                assert_eq!(settings.base_url, "https://nyxid.example.com");
                assert_eq!(settings.issuer, "nyxid");
                // Audience defaults to base_url (after trim).
                assert_eq!(settings.audience, "https://nyxid.example.com");
                assert_eq!(settings.jwks_cache_ttl, std::time::Duration::from_secs(300));
            }
            AuthMode::Disabled => panic!("expected Enabled, got Disabled"),
        }
    }

    #[test]
    fn auth_issuer_and_audience_are_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com"),
            ("FKST_AUTH_ISSUER", "custom-issuer"),
            ("FKST_AUTH_AUDIENCE", "my-audience"),
            ("FKST_AUTH_JWKS_CACHE_TTL_SECS", "600"),
        ]))
        .expect("auth overrides");
        match config.auth {
            AuthMode::Enabled(ref settings) => {
                assert_eq!(settings.issuer, "custom-issuer");
                assert_eq!(settings.audience, "my-audience");
                assert_eq!(settings.jwks_cache_ttl, std::time::Duration::from_secs(600));
            }
            AuthMode::Disabled => panic!("expected Enabled"),
        }
    }

    #[test]
    fn zero_jwks_cache_ttl_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com"),
            ("FKST_AUTH_JWKS_CACHE_TTL_SECS", "0"),
        ]))
        .expect_err("zero JWKS cache TTL must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_AUTH_JWKS_CACHE_TTL_SECS"),
            "error must name the variable, got: {err}"
        );
    }

    // ---- NyxID client credential tests ----------------------------------------

    #[test]
    fn nyxid_creds_both_set_are_accepted() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("NYXID_CLIENT_ID", "sa_test"),
            ("NYXID_CLIENT_SECRET", "sas_test"),
        ]))
        .expect("both set");
        assert_eq!(config.nyxid_client_id.as_deref(), Some("sa_test"));
        assert!(config.nyxid_client_secret.is_some());
    }

    #[test]
    fn nyxid_creds_neither_set_is_accepted() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("neither set");
        assert!(config.nyxid_client_id.is_none());
        assert!(config.nyxid_client_secret.is_none());
    }

    #[test]
    fn nyxid_client_id_without_secret_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("NYXID_CLIENT_ID", "sa_test"),
        ]))
        .expect_err("id without secret must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("NYXID_CLIENT_SECRET"),
            "error must name the missing variable, got: {err}"
        );
    }

    #[test]
    fn nyxid_client_secret_without_id_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("NYXID_CLIENT_SECRET", "sas_test"),
        ]))
        .expect_err("secret without id must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("NYXID_CLIENT_ID"),
            "error must name the missing variable, got: {err}"
        );
    }

    #[test]
    fn nyxid_client_secret_never_appears_in_debug() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("NYXID_CLIENT_ID", "sa_test"),
            ("NYXID_CLIENT_SECRET", "sas_should_not_leak"),
        ]))
        .expect("both set");
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("sas_should_not_leak"),
            "secret leaked in debug"
        );
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn nyxid_org_cache_ttl_defaults_to_30() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.nyxid_org_cache_ttl_secs, 30);
    }

    #[test]
    fn nyxid_org_cache_ttl_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_NYXID_ORG_CACHE_TTL_SECS", "60"),
        ]))
        .expect("override");
        assert_eq!(config.nyxid_org_cache_ttl_secs, 60);
    }

    #[test]
    fn zero_nyxid_org_cache_ttl_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_NYXID_ORG_CACHE_TTL_SECS", "0"),
        ]))
        .expect_err("zero TTL must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_NYXID_ORG_CACHE_TTL_SECS"),
            "error must name the variable, got: {err}"
        );
    }

    #[test]
    fn nyxid_github_proxy_slug_defaults_to_api_github() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.nyxid_github_proxy_slug, "api-github");
    }

    #[test]
    fn nyxid_github_proxy_slug_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_NYXID_GITHUB_PROXY_SLUG", "api-github-pat"),
        ]))
        .expect("override");
        assert_eq!(config.nyxid_github_proxy_slug, "api-github-pat");
    }

    #[test]
    fn blank_nyxid_github_proxy_slug_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_NYXID_GITHUB_PROXY_SLUG", "   "),
        ]))
        .expect_err("blank slug must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_NYXID_GITHUB_PROXY_SLUG"),
            "error must name the variable, got: {err}"
        );
    }

    // ---- LLM gateway configuration tests --------------------------------------

    /// Service-account credentials any test enabling the gateway needs.
    const NYXID_CREDS: [(&str, &str); 2] = [
        ("NYXID_CLIENT_ID", "sa_test"),
        ("NYXID_CLIENT_SECRET", "sas_test"),
    ];

    #[test]
    fn llm_defaults_when_gateway_unset() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert!(config.llm_gateway_url.is_none());
        assert!(config.llm_model.is_none());
        assert_eq!(config.llm_timeout_secs, 20);
        assert_eq!(config.llm_max_output_bytes, 1_048_576);
    }

    #[test]
    fn llm_vars_are_all_overridable() {
        let mut pairs = vec![
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            (
                "FKST_HOSTED_LLM_GATEWAY_URL",
                "https://nyxid.example.com/llm",
            ),
            ("FKST_HOSTED_LLM_MODEL", "claude-sonnet"),
            ("FKST_HOSTED_LLM_TIMEOUT_SECS", "45"),
            ("FKST_HOSTED_LLM_MAX_OUTPUT_BYTES", "2048"),
        ];
        pairs.extend_from_slice(&NYXID_CREDS);
        let config = Config::from_vars(vars(&pairs)).expect("overrides");
        assert_eq!(
            config.llm_gateway_url.as_deref(),
            Some("https://nyxid.example.com/llm")
        );
        assert_eq!(config.llm_model.as_deref(), Some("claude-sonnet"));
        assert_eq!(config.llm_timeout_secs, 45);
        assert_eq!(config.llm_max_output_bytes, 2048);
    }

    #[test]
    fn zero_llm_timeout_is_a_config_error_naming_the_var() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_LLM_TIMEOUT_SECS", "0"),
        ]))
        .expect_err("zero LLM timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_HOSTED_LLM_TIMEOUT_SECS"),
            "error must name the var, got: {err}"
        );
    }

    #[test]
    fn zero_llm_max_output_bytes_is_a_config_error_naming_the_var() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_LLM_MAX_OUTPUT_BYTES", "0"),
        ]))
        .expect_err("zero LLM output cap must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_HOSTED_LLM_MAX_OUTPUT_BYTES"),
            "error must name the var, got: {err}"
        );
    }

    #[test]
    fn llm_gateway_without_nyxid_creds_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            (
                "FKST_HOSTED_LLM_GATEWAY_URL",
                "https://nyxid.example.com/llm",
            ),
            ("FKST_HOSTED_LLM_MODEL", "claude-sonnet"),
        ]))
        .expect_err("gateway without creds must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("NYXID_CLIENT_ID"),
            "error must name the credential vars, got: {err}"
        );
    }

    #[test]
    fn llm_gateway_without_model_is_a_config_error() {
        let mut pairs = vec![
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            (
                "FKST_HOSTED_LLM_GATEWAY_URL",
                "https://nyxid.example.com/llm",
            ),
        ];
        pairs.extend_from_slice(&NYXID_CREDS);
        let err = Config::from_vars(vars(&pairs)).expect_err("gateway without model must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_HOSTED_LLM_MODEL"),
            "error must name the var, got: {err}"
        );
    }

    #[test]
    fn llm_gateway_with_creds_and_model_is_accepted() {
        let mut pairs = vec![
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            (
                "FKST_HOSTED_LLM_GATEWAY_URL",
                "https://nyxid.example.com/llm",
            ),
            ("FKST_HOSTED_LLM_MODEL", "claude-sonnet"),
        ];
        pairs.extend_from_slice(&NYXID_CREDS);
        let config = Config::from_vars(vars(&pairs)).expect("fully configured gateway");
        assert_eq!(
            config.llm_gateway_url.as_deref(),
            Some("https://nyxid.example.com/llm")
        );
        assert_eq!(config.llm_model.as_deref(), Some("claude-sonnet"));
    }

    // ---- vault configuration tests --------------------------------------------

    /// A valid base64-32-byte vault key for the tests below.
    const VAULT_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

    #[test]
    fn vault_caps_default_and_master_key_optional() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.vault_value_byte_cap, 65_536);
        assert_eq!(config.vault_entries_per_scope_cap, 100);
        // No key source set => None at the config layer (boot decides fatality).
        assert!(config.vault_master_key.is_none());
    }

    #[test]
    fn vault_caps_are_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_VAULT_VALUE_BYTE_CAP", "1024"),
            ("FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP", "5"),
        ]))
        .expect("overrides");
        assert_eq!(config.vault_value_byte_cap, 1024);
        assert_eq!(config.vault_entries_per_scope_cap, 5);
    }

    #[test]
    fn zero_vault_caps_are_config_errors_naming_the_var() {
        for (var, value) in [
            ("FKST_HOSTED_VAULT_VALUE_BYTE_CAP", "0"),
            ("FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP", "0"),
        ] {
            let err = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false"), (var, value)]))
                .expect_err("zero cap must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }

    #[test]
    fn vault_master_key_inline_is_read() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_VAULT_MASTER_KEY", VAULT_KEY),
        ]))
        .expect("inline key");
        assert!(config.vault_master_key.is_some());
    }

    #[test]
    fn vault_master_key_never_appears_in_debug() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_VAULT_MASTER_KEY", VAULT_KEY),
        ]))
        .expect("inline key");
        let rendered = format!("{config:?}");
        assert!(!rendered.contains(VAULT_KEY), "vault key leaked in debug");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn vault_master_key_and_path_both_set_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_VAULT_MASTER_KEY", VAULT_KEY),
            ("FKST_HOSTED_VAULT_MASTER_KEY_PATH", "/some/path"),
        ]))
        .expect_err("both sources must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_HOSTED_VAULT_MASTER_KEY"),
            "error must name the vault key vars, got: {err}"
        );
    }

    #[test]
    fn vault_master_key_path_reads_the_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("vault.key");
        std::fs::write(&path, VAULT_KEY).expect("write key");
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_VAULT_MASTER_KEY_PATH", path.to_str().unwrap()),
        ]))
        .expect("key from path");
        assert!(config.vault_master_key.is_some());
    }

    #[test]
    fn vault_master_key_path_missing_file_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            (
                "FKST_HOSTED_VAULT_MASTER_KEY_PATH",
                "/nonexistent/vault.key",
            ),
        ]))
        .expect_err("missing file must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string()
                .contains("FKST_HOSTED_VAULT_MASTER_KEY_PATH"),
            "error must name the path var, got: {err}"
        );
    }

    // ---- codex chrono-llm DEFAULT configuration tests (#112) ------------------

    #[test]
    fn codex_defaults_apply_when_unset() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.codex_model, "gpt-5-codex");
        assert_eq!(
            config.chrono_llm_base_url,
            "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm"
        );
    }

    #[test]
    fn codex_vars_are_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_CODEX_MODEL", "gpt-4.1"),
            (
                "FKST_HOSTED_CHRONO_LLM_BASE_URL",
                "https://proxy.example/s/chrono-llm",
            ),
        ]))
        .expect("overrides");
        assert_eq!(config.codex_model, "gpt-4.1");
        assert_eq!(
            config.chrono_llm_base_url,
            "https://proxy.example/s/chrono-llm"
        );
    }

    #[test]
    fn blank_codex_vars_are_config_errors_naming_the_var() {
        for var in ["FKST_HOSTED_CODEX_MODEL", "FKST_HOSTED_CHRONO_LLM_BASE_URL"] {
            let err = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false"), (var, "   ")]))
                .expect_err("blank must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }
}
