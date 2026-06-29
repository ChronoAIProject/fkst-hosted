//! Typed runtime configuration loaded from environment variables.
//!
//! Two envy passes over the same snapshot of variables:
//! 1. the `FKST_HOSTED_*`-prefixed HTTP/server settings, and
//! 2. the `FKST_*`-prefixed auth settings.
//!
//! The control plane is API-only and datastore-free: there is no in-process
//! session execution, no worker fleet, no journaling, and no MongoDB, so none of
//! the dispatch/worker/journal knobs survive. The owner-only NyxID client (#257)
//! needs no service-account credential.

use serde::Deserialize;

use crate::auth::{AuthMode, NyxIdAuthSettings};
use crate::error::AppError;

/// Prefix shared by every HTTP/server configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_";

/// Prefix of the auth variables (`FKST_AUTH_*` / `FKST_NYXID_*` /
/// `FKST_SESSION_*`); the auth envy pass reads them with the `FKST_` prefix.
const AUTH_ENV_PREFIX: &str = "FKST_";

/// Prefix for the pod-dispatch settings (`FKST_POD_*`). kube-client is the sole
/// owner of these knobs; later issues read them but never redefine them.
const POD_ENV_PREFIX: &str = "FKST_POD_";

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

    pub(super) fn auth_enabled() -> bool {
        // Default true: fail-closed at startup. Explicit `false` is a conscious
        // local-dev choice (the operator must set `FKST_AUTH_ENABLED=false` to
        // disable authentication).
        true
    }

    pub(super) fn nyxid_org_cache_ttl_secs() -> u64 {
        30
    }

    /// TTL (seconds) for a per-session NyxID agent key (#216). The key carries
    /// `expires_at = now + this`, so it SELF-EXPIRES rather than relying on a
    /// service-account revoke NyxID rejects. Default 24h: long enough for any
    /// realistic engine run, short enough to bound a leaked key's blast radius.
    /// Must be > 0.
    pub(super) fn session_key_ttl_secs() -> u64 {
        86_400
    }

    pub(super) fn nyxid_github_proxy_slug() -> String {
        // NyxID `main`/v0.7.0 seeds its GitHub OAuth proxy under slug
        // `api-github` (`backend/src/services/provider_service.rs`,
        // `DefaultServiceSeed`); kept in sync with
        // `crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG`.
        crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG.to_string()
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

    pub(super) fn pod_namespace() -> String {
        // The namespace per-session Jobs + Secrets live in (milestone #9).
        "fkst-sessions".to_string()
    }

    pub(super) fn pod_service_account() -> String {
        // The ServiceAccount the session Job pods run as (minimal identity).
        "fkst-session-runner".to_string()
    }

    pub(super) fn pod_run_ttl_secs() -> i32 {
        // `ttlSecondsAfterFinished`: K8s GCs a finished Job (+ its pod and the
        // owner-referenced Secret) this long after completion. 10 min.
        600
    }

    pub(super) fn pod_active_deadline_secs() -> i64 {
        // `activeDeadlineSeconds`: hard wall after which a still-running Job is
        // failed. 1 hour — generous for a realistic engine run.
        3600
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
    /// Max bytes for a single inline vault value (#138). Env:
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
    /// slug; blank rejected at load. Non-secret (it is a route).
    #[serde(default = "defaults::chrono_llm_base_url")]
    chrono_llm_base_url: String,
}

/// `FKST_AUTH_*`-prefixed variables (authentication settings; envy pass with
/// the `FKST_` prefix).
#[derive(Debug, Deserialize)]
struct AuthVars {
    #[serde(default = "defaults::auth_enabled")]
    auth_enabled: bool,
    #[serde(default)]
    auth_nyxid_base_url: Option<String>,
    #[serde(default = "defaults::nyxid_org_cache_ttl_secs")]
    nyxid_org_cache_ttl_secs: u64,
    #[serde(default = "defaults::session_key_ttl_secs")]
    session_key_ttl_secs: u64,
    #[serde(default = "defaults::nyxid_github_proxy_slug")]
    nyxid_github_proxy_slug: String,
}

/// `FKST_POD_*`-prefixed variables (pod-per-session dispatch, milestone #9).
/// kube-client owns these; `i32`/`i64` match `k8s-openapi`'s `JobSpec` fields.
#[derive(Debug, Deserialize)]
struct PodVars {
    #[serde(default)]
    dispatch: bool,
    #[serde(default = "defaults::pod_namespace")]
    namespace: String,
    #[serde(default)]
    image: Option<String>,
    #[serde(default = "defaults::pod_service_account")]
    service_account: String,
    #[serde(default = "defaults::pod_run_ttl_secs")]
    run_ttl_secs: i32,
    #[serde(default = "defaults::pod_active_deadline_secs")]
    active_deadline_secs: i64,
}

/// Pod-per-session dispatch configuration (milestone #9). When `dispatch` is
/// false (the default) the control plane never touches Kubernetes.
#[derive(Clone, Debug)]
pub struct PodConfig {
    /// Master switch. Env: `FKST_POD_DISPATCH`. Default false.
    pub dispatch: bool,
    /// Namespace for per-session Jobs + Secrets. Env: `FKST_POD_NAMESPACE`.
    /// Default `fkst-sessions`.
    pub namespace: String,
    /// The image session Job pods run (the control-plane image, `run-session`
    /// mode). Env: `FKST_POD_IMAGE`. Required when `dispatch=true`.
    pub image: Option<String>,
    /// ServiceAccount the Job pods run as. Env: `FKST_POD_SERVICE_ACCOUNT`.
    /// Default `fkst-session-runner`.
    pub service_account: String,
    /// `ttlSecondsAfterFinished` for the Job. Env: `FKST_POD_RUN_TTL_SECS`.
    /// Default 600; must be > 0 when `dispatch=true`.
    pub run_ttl_secs: i32,
    /// `activeDeadlineSeconds` for the Job. Env: `FKST_POD_ACTIVE_DEADLINE_SECS`.
    /// Default 3600; must be > 0 when `dispatch=true`.
    pub active_deadline_secs: i64,
}

impl Default for PodConfig {
    fn default() -> Self {
        Self {
            dispatch: false,
            namespace: defaults::pod_namespace(),
            image: None,
            service_account: defaults::pod_service_account(),
            run_ttl_secs: defaults::pod_run_ttl_secs(),
            active_deadline_secs: defaults::pod_active_deadline_secs(),
        }
    }
}

/// Runtime configuration assembled from both envy passes.
#[derive(Clone, Debug)]
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
    /// Authentication mode: disabled (local dev) or enabled with NyxID
    /// settings. Env: `FKST_AUTH_ENABLED` (default true = fail-closed).
    pub auth: AuthMode,
    /// TTL in seconds for the NyxID user-orgs cache.
    /// Env: `FKST_NYXID_ORG_CACHE_TTL_SECS`. Default 30, zero rejected.
    pub nyxid_org_cache_ttl_secs: u64,
    /// TTL in seconds for a per-session NyxID agent key (#216): the key is
    /// minted with `expires_at = now + this`, so it self-expires (the primary
    /// cleanup mechanism — the service-account revoke route NyxID rejects).
    /// Env: `FKST_SESSION_KEY_TTL_SECS`. Default 86400 (24h), zero rejected.
    pub session_key_ttl_secs: u64,
    /// Downstream-service slug NyxID resolves to inject the user's GitHub
    /// credential on proxied requests; the client builds the proxy base path
    /// `/api/v1/proxy/{slug}` from it. Env: `FKST_NYXID_GITHUB_PROXY_SLUG`.
    /// Default `api-github` (the slug NyxID `main`/v0.7.0 seeds). Rejected when
    /// blank (fail-closed: an empty slug yields an unresolvable proxy route).
    pub nyxid_github_proxy_slug: String,
    /// Max bytes for a single inline vault value (#138). Env:
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
    /// Pod-per-session dispatch settings (milestone #9). `dispatch=false` by
    /// default: the control plane is Kubernetes-free until an operator opts in.
    pub pod: PodConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            bind_addr: defaults::bind_addr(),
            log_level: defaults::log_level(),
            request_timeout_secs: defaults::request_timeout_secs(),
            auth: AuthMode::Disabled,
            nyxid_org_cache_ttl_secs: defaults::nyxid_org_cache_ttl_secs(),
            session_key_ttl_secs: defaults::session_key_ttl_secs(),
            nyxid_github_proxy_slug: defaults::nyxid_github_proxy_slug(),
            vault_value_byte_cap: defaults::vault_value_byte_cap(),
            vault_entries_per_scope_cap: defaults::vault_entries_per_scope_cap(),
            codex_model: defaults::codex_model(),
            chrono_llm_base_url: defaults::chrono_llm_base_url(),
            pod: PodConfig::default(),
        }
    }
}

impl Config {
    /// Deserialize a `Config` from environment-style key/value pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment. The pairs are collected once and fed to every
    /// envy pass (prefixed HTTP vars, prefixed auth vars).
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

        // Authentication settings pass (FKST_AUTH_* with the FKST_ prefix).
        let auth: AuthVars = envy::prefixed(AUTH_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        if auth.nyxid_org_cache_ttl_secs == 0 {
            return Err(AppError::Config(
                "FKST_NYXID_ORG_CACHE_TTL_SECS must be at least 1".to_string(),
            ));
        }
        // A zero session-key TTL would mint an already-expired key, breaking
        // every engine run at startup — reject it loudly, mirroring the guards
        // above. The key self-expires after this many seconds (#216).
        if auth.session_key_ttl_secs == 0 {
            return Err(AppError::Config(
                "FKST_SESSION_KEY_TTL_SECS must be at least 1".to_string(),
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
            // No JWKS / issuer / audience: fkst-hosted trusts the proxy and never
            // verifies a user token (#113). `base_url` is the NyxID issuer host,
            // used by the org-lookup client and per-session token provisioning.
            AuthMode::Enabled(NyxIdAuthSettings { base_url })
        } else {
            AuthMode::Disabled
        };

        // Vault cap validation (fail-closed): the vault is always-on, so a zero
        // cap is a startup error.
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

        // Pod-per-session dispatch settings (FKST_POD_*). Off by default; when
        // an operator turns it on, the image + namespace must be real and the
        // Job time bounds positive, or the control plane would emit unspawnable
        // Jobs. Fail closed, naming the offending var.
        let pod: PodVars = envy::prefixed(POD_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        let pod_image = pod
            .image
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if pod.dispatch {
            if pod_image.is_none() {
                return Err(AppError::Config(
                    "FKST_POD_IMAGE must be set when FKST_POD_DISPATCH=true".to_string(),
                ));
            }
            if pod.namespace.trim().is_empty() {
                return Err(AppError::Config(
                    "FKST_POD_NAMESPACE must not be blank when FKST_POD_DISPATCH=true".to_string(),
                ));
            }
            if pod.run_ttl_secs <= 0 {
                return Err(AppError::Config(
                    "FKST_POD_RUN_TTL_SECS must be at least 1 when FKST_POD_DISPATCH=true"
                        .to_string(),
                ));
            }
            if pod.active_deadline_secs <= 0 {
                return Err(AppError::Config(
                    "FKST_POD_ACTIVE_DEADLINE_SECS must be at least 1 when FKST_POD_DISPATCH=true"
                        .to_string(),
                ));
            }
        }
        let pod = PodConfig {
            dispatch: pod.dispatch,
            namespace: pod.namespace,
            image: pod_image,
            service_account: pod.service_account,
            run_ttl_secs: pod.run_ttl_secs,
            active_deadline_secs: pod.active_deadline_secs,
        };

        Ok(Config {
            port: http.port,
            bind_addr: http.bind_addr,
            log_level: http.log_level,
            request_timeout_secs: http.request_timeout_secs,
            auth: auth_mode,
            nyxid_org_cache_ttl_secs: auth.nyxid_org_cache_ttl_secs,
            session_key_ttl_secs: auth.session_key_ttl_secs,
            nyxid_github_proxy_slug: auth.nyxid_github_proxy_slug,
            vault_value_byte_cap: http.vault_value_byte_cap,
            vault_entries_per_scope_cap: http.vault_entries_per_scope_cap,
            codex_model: http.codex_model,
            chrono_llm_base_url: http.chrono_llm_base_url,
            pod,
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

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        // The control plane is datastore-free: no MONGODB_* var is required at
        // startup; an otherwise-empty environment loads cleanly.
        let config = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")]))
            .expect("defaults should deserialize");
        assert_eq!(config.port, 8080);
        assert_eq!(config.bind_addr, "0.0.0.0");
        assert_eq!(config.log_level, "info");
        // Pod dispatch is OFF by default; the control plane never touches k8s.
        assert!(!config.pod.dispatch);
        assert_eq!(config.pod.namespace, "fkst-sessions");
        assert_eq!(config.pod.service_account, "fkst-session-runner");
        assert_eq!(config.pod.run_ttl_secs, 600);
        assert_eq!(config.pod.active_deadline_secs, 3600);
        assert!(config.pod.image.is_none());
        assert_eq!(config.request_timeout_secs, 30);
        assert!(matches!(config.auth, AuthMode::Disabled));
    }

    #[test]
    fn pod_dispatch_on_requires_an_image() {
        let err = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_POD_DISPATCH", "true"),
        ]))
        .expect_err("dispatch with no image must fail closed");
        assert!(err.to_string().contains("FKST_POD_IMAGE"));
    }

    #[test]
    fn pod_dispatch_on_with_image_parses_and_keeps_overrides() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "registry/fkst-control-plane:1.0"),
            ("FKST_POD_NAMESPACE", "sessions-prod"),
            ("FKST_POD_RUN_TTL_SECS", "900"),
            ("FKST_POD_ACTIVE_DEADLINE_SECS", "7200"),
        ]))
        .expect("valid dispatch config should load");
        assert!(config.pod.dispatch);
        assert_eq!(
            config.pod.image.as_deref(),
            Some("registry/fkst-control-plane:1.0")
        );
        assert_eq!(config.pod.namespace, "sessions-prod");
        assert_eq!(config.pod.run_ttl_secs, 900);
        assert_eq!(config.pod.active_deadline_secs, 7200);
    }

    #[test]
    fn pod_dispatch_on_rejects_nonpositive_time_bounds() {
        let ttl = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "img"),
            ("FKST_POD_RUN_TTL_SECS", "0"),
        ]))
        .expect_err("zero ttl must fail");
        assert!(ttl.to_string().contains("FKST_POD_RUN_TTL_SECS"));

        let deadline = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "img"),
            ("FKST_POD_ACTIVE_DEADLINE_SECS", "0"),
        ]))
        .expect_err("zero deadline must fail");
        assert!(deadline
            .to_string()
            .contains("FKST_POD_ACTIVE_DEADLINE_SECS"));
    }

    #[test]
    fn pod_image_blank_is_treated_as_absent_when_dispatch_on() {
        let err = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "   "),
        ]))
        .expect_err("blank image must fail closed");
        assert!(err.to_string().contains("FKST_POD_IMAGE"));
    }

    #[test]
    fn no_mongodb_var_is_required_at_startup() {
        // Regression guard: with no MONGODB_URI set, the store-free control plane
        // must still load — there is no mandatory datastore config. (Auth is the
        // only other fail-closed gate, disabled here to isolate the assertion.)
        Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")]))
            .expect("loads without any MONGODB_* var");
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")]))
            .expect("defaults should deserialize");
        let from_default = Config::default();
        assert_eq!(from_default.port, from_env.port);
        assert_eq!(from_default.bind_addr, from_env.bind_addr);
        assert_eq!(from_default.log_level, from_env.log_level);
        assert_eq!(
            from_default.request_timeout_secs,
            from_env.request_timeout_secs
        );
    }

    #[test]
    fn port_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_PORT", "9090"),
        ]))
        .unwrap();
        assert_eq!(config.port, 9090);
    }

    #[test]
    fn bind_addr_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_BIND_ADDR", "127.0.0.1"),
        ]))
        .unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1");
    }

    #[test]
    fn log_level_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_LOG_LEVEL", "debug"),
        ]))
        .unwrap();
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn request_timeout_secs_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "5"),
        ]))
        .unwrap();
        assert_eq!(config.request_timeout_secs, 5);
    }

    #[test]
    fn zero_request_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "0"),
        ]))
        .expect_err("zero timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_HOSTED_REQUEST_TIMEOUT_SECS"));
    }

    #[test]
    fn non_numeric_port_is_a_config_error() {
        let err = Config::from_vars(vars(&[("FKST_HOSTED_PORT", "abc")]))
            .expect_err("non-numeric port must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    // ---- auth configuration tests ----------------------------------------------

    #[test]
    fn auth_enabled_without_base_url_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "true")]))
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
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com/"),
        ]))
        .expect("enabled with base URL");
        match config.auth {
            AuthMode::Enabled(ref settings) => {
                // Trailing slash must be trimmed. No issuer/audience/JWKS: the
                // proxy is trusted (#113) and no user token is ever verified.
                assert_eq!(settings.base_url, "https://nyxid.example.com");
            }
            AuthMode::Disabled => panic!("expected Enabled, got Disabled"),
        }
    }

    #[test]
    fn legacy_jwks_issuer_audience_env_vars_are_ignored() {
        // The old verification settings no longer exist; supplying them must be
        // harmless (envy ignores unknown vars) — `base_url` is all that matters.
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com"),
            ("FKST_AUTH_ISSUER", "ignored"),
            ("FKST_AUTH_AUDIENCE", "ignored"),
            ("FKST_AUTH_JWKS_CACHE_TTL_SECS", "0"),
        ]))
        .expect("legacy verification vars are ignored, not errors");
        match config.auth {
            AuthMode::Enabled(ref settings) => {
                assert_eq!(settings.base_url, "https://nyxid.example.com");
            }
            AuthMode::Disabled => panic!("expected Enabled"),
        }
    }

    #[test]
    fn nyxid_org_cache_ttl_defaults_to_30() {
        let config = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.nyxid_org_cache_ttl_secs, 30);
    }

    #[test]
    fn nyxid_org_cache_ttl_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_NYXID_ORG_CACHE_TTL_SECS", "60"),
        ]))
        .expect("override");
        assert_eq!(config.nyxid_org_cache_ttl_secs, 60);
    }

    #[test]
    fn zero_nyxid_org_cache_ttl_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
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
    fn session_key_ttl_defaults_to_one_day() {
        let config = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.session_key_ttl_secs, 86_400);
    }

    #[test]
    fn session_key_ttl_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_SESSION_KEY_TTL_SECS", "3600"),
        ]))
        .expect("override");
        assert_eq!(config.session_key_ttl_secs, 3600);
    }

    #[test]
    fn zero_session_key_ttl_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_SESSION_KEY_TTL_SECS", "0"),
        ]))
        .expect_err("zero TTL must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_SESSION_KEY_TTL_SECS"),
            "error must name the variable, got: {err}"
        );
    }

    #[test]
    fn nyxid_github_proxy_slug_defaults_to_api_github() {
        let config = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.nyxid_github_proxy_slug, "api-github");
    }

    #[test]
    fn nyxid_github_proxy_slug_is_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_NYXID_GITHUB_PROXY_SLUG", "api-github-pat"),
        ]))
        .expect("override");
        assert_eq!(config.nyxid_github_proxy_slug, "api-github-pat");
    }

    #[test]
    fn blank_nyxid_github_proxy_slug_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
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

    // ---- vault configuration tests --------------------------------------------

    #[test]
    fn vault_caps_default() {
        let config = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.vault_value_byte_cap, 65_536);
        assert_eq!(config.vault_entries_per_scope_cap, 100);
    }

    #[test]
    fn vault_caps_are_overridable() {
        let config = Config::from_vars(vars(&[
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
            let err = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false"), (var, value)]))
                .expect_err("zero cap must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }

    // ---- codex chrono-llm DEFAULT configuration tests (#112) ------------------

    #[test]
    fn codex_defaults_apply_when_unset() {
        let config = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.codex_model, "gpt-5-codex");
        assert_eq!(
            config.chrono_llm_base_url,
            "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm"
        );
    }

    #[test]
    fn codex_vars_are_overridable() {
        let config = Config::from_vars(vars(&[
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
            let err = Config::from_vars(vars(&[("FKST_AUTH_ENABLED", "false"), (var, "   ")]))
                .expect_err("blank must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }
}
