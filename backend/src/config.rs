//! Typed runtime configuration loaded from environment variables.
//!
//! Several envy passes over the same snapshot of variables, one per prefix: the
//! `FKST_HOSTED_*` HTTP/server settings, the `FKST_POD_*` pod-dispatch settings,
//! the `FKST_LLM_*` static LLM-provider settings, and the bare
//! `FKST_WEBHOOK_TRIGGER_LABEL`.
//!
//! The control plane is API-only and datastore-free: there is no in-process
//! session execution, no worker fleet, no journaling, and no MongoDB, so none of
//! the dispatch/worker/journal knobs survive. Identity is the HMAC-verified
//! GitHub webhook actor — there is no application-level auth to configure.

use secrecy::SecretString;
use serde::Deserialize;

use crate::env_config::EnvConfig;
use crate::error::AppError;

/// Prefix shared by every HTTP/server configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_";

/// Prefix for the bare `FKST_*` settings (currently only
/// `FKST_WEBHOOK_TRIGGER_LABEL`); the envy pass reads them with the `FKST_`
/// prefix and ignores the more specific `FKST_HOSTED_`/`FKST_POD_`/`FKST_LLM_`
/// variables (envy drops fields it does not recognize).
const WEBHOOK_ENV_PREFIX: &str = "FKST_";

/// Prefix for the pod-dispatch settings (`FKST_POD_*`). kube-client is the sole
/// owner of these knobs; later issues read them but never redefine them.
const POD_ENV_PREFIX: &str = "FKST_POD_";

/// Prefix for the static LLM-provider settings (`FKST_LLM_*`). The per-session
/// codex provider is config-driven: the model/base URL/wire_api are injected into
/// the session pod and the API key rides the per-session Secret.
const LLM_ENV_PREFIX: &str = "FKST_LLM_";

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

    pub(super) fn vault_value_byte_cap() -> usize {
        65_536
    }

    pub(super) fn vault_entries_per_scope_cap() -> usize {
        100
    }

    pub(super) fn llm_model() -> String {
        // The model the per-session codex provider serves. The operator pins it
        // to whatever the LLM backend currently serves; this is a sensible
        // non-empty default, never a literal placeholder.
        "gpt-5-codex".to_string()
    }

    pub(super) fn llm_base_url() -> String {
        // Base URL of the LLM provider the session codex talks to.
        "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm".to_string()
    }

    pub(super) fn llm_wire_api() -> String {
        // The codex `wire_api`. MUST default to `chat`: chrono-llm serves only
        // `/chat/completions`; `responses` returns 502 (a verified bug). Never
        // default to `responses`.
        "chat".to_string()
    }

    pub(super) fn webhook_trigger_label() -> String {
        // Only issues carrying this label auto-trigger a session.
        "fkst".to_string()
    }

    pub(super) fn github_api_base_url() -> String {
        // Base URL the per-user-store identity check calls (`GET {base}/user`).
        // Overridable so tests can point at a wiremock server.
        "https://api.github.com".to_string()
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

    pub(super) fn pod_dns_nameservers() -> Vec<String> {
        // The isolated session/validation pod's external-only DNS. Public
        // resolvers so the pod can reach GitHub/LLM without cluster DNS.
        vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()]
    }

    pub(super) fn pod_dns_nameservers_raw() -> String {
        // Parsed as a single comma-separated String to sidestep envy's Vec
        // handling; split into `dns_nameservers` in `from_vars`.
        "1.1.1.1,8.8.8.8".to_string()
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
}

/// Bare `FKST_*` settings (currently only `FKST_WEBHOOK_TRIGGER_LABEL`); envy
/// pass with the `FKST_` prefix.
#[derive(Debug, Deserialize)]
struct WebhookVars {
    #[serde(default = "defaults::webhook_trigger_label")]
    webhook_trigger_label: String,
    /// Base URL the per-user-store GitHub-token identity check calls
    /// (`GET {base}/user`). Env: `FKST_GITHUB_API_BASE_URL`. Default
    /// `https://api.github.com`; non-blank (tests point it at a mock server).
    #[serde(default = "defaults::github_api_base_url")]
    github_api_base_url: String,
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
    /// Comma-separated external DNS resolvers for the isolated pod. Parsed as a
    /// String (not a Vec) to avoid envy's Vec quirks; split in `from_vars`.
    #[serde(default = "defaults::pod_dns_nameservers_raw")]
    dns_nameservers: String,
}

/// `FKST_LLM_*`-prefixed variables (static LLM-provider config). The session
/// codex provider is config-driven (model/base URL/wire_api injected into the
/// pod) with a static API key (`FKST_LLM_API_KEY`) that rides the per-session
/// Secret.
#[derive(Debug, Deserialize)]
struct LlmVars {
    #[serde(default = "defaults::llm_model")]
    model: String,
    #[serde(default = "defaults::llm_base_url")]
    base_url: String,
    #[serde(default = "defaults::llm_wire_api")]
    wire_api: String,
    /// The static LLM API key. Optional at parse time; REQUIRED non-blank when
    /// `FKST_POD_DISPATCH=true` (an engine with no LLM credential 401s).
    #[serde(default)]
    api_key: Option<String>,
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
    /// LLM provider base URL injected into the session pod as `FKST_LLM_BASE_URL`
    /// (session pods do NOT inherit the control-plane ConfigMap, so build_job
    /// injects it explicitly). Env: `FKST_LLM_BASE_URL`.
    pub llm_base_url: String,
    /// LLM model injected into the session pod as `FKST_LLM_MODEL`.
    /// Env: `FKST_LLM_MODEL`.
    pub llm_model: String,
    /// codex `wire_api` injected into the session pod as `FKST_LLM_WIRE_API`.
    /// Env: `FKST_LLM_WIRE_API`. Default `chat`.
    pub llm_wire_api: String,
    /// External DNS resolvers for the isolated session/validation pod's
    /// `dnsConfig.nameservers`. Env: `FKST_POD_DNS_NAMESERVERS`, comma-separated.
    /// Default `["1.1.1.1", "8.8.8.8"]`; a session with no DNS cannot resolve
    /// GitHub/LLM, so a blank list is rejected.
    pub dns_nameservers: Vec<String>,
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
            llm_base_url: defaults::llm_base_url(),
            llm_model: defaults::llm_model(),
            llm_wire_api: defaults::llm_wire_api(),
            dns_nameservers: defaults::pod_dns_nameservers(),
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
    /// Only `issues.opened` carrying this label auto-triggers a session.
    /// Env: `FKST_WEBHOOK_TRIGGER_LABEL`. Default `fkst`.
    pub webhook_trigger_label: String,
    /// Base URL the per-user-store identity check calls (`GET {base}/user`) to
    /// trade a caller's GitHub token for the verified `{login, id}`. The numeric
    /// `id` (never a client-supplied value) keys the user's `fkst-user-<id>`
    /// objects. Env: `FKST_GITHUB_API_BASE_URL`. Default `https://api.github.com`.
    pub github_api_base_url: String,
    /// Max bytes for a single inline vault value (#138). Env:
    /// `FKST_HOSTED_VAULT_VALUE_BYTE_CAP`. Default 65536, zero rejected.
    pub vault_value_byte_cap: usize,
    /// Max vault entries an owner may hold per scope. Env:
    /// `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP`. Default 100, zero rejected.
    pub vault_entries_per_scope_cap: usize,
    /// The static LLM API key the session engine authenticates with (read by the
    /// webhook trigger into the per-session Secret). Env: `FKST_LLM_API_KEY`.
    /// Empty when unset; REQUIRED non-blank when `FKST_POD_DISPATCH=true`. Never
    /// logged. The model/base URL/wire_api live on [`PodConfig`] (pod-injected).
    pub llm_api_key: SecretString,
    /// Pod-per-session dispatch settings (milestone #9). `dispatch=false` by
    /// default: the control plane is Kubernetes-free until an operator opts in.
    pub pod: PodConfig,
    /// Named-environment / install-validation knobs (`FKST_ENV_*`, issue #338
    /// §6.1). Config surface only — no behaviour reads these yet.
    pub env: EnvConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            bind_addr: defaults::bind_addr(),
            log_level: defaults::log_level(),
            request_timeout_secs: defaults::request_timeout_secs(),
            webhook_trigger_label: defaults::webhook_trigger_label(),
            github_api_base_url: defaults::github_api_base_url(),
            vault_value_byte_cap: defaults::vault_value_byte_cap(),
            vault_entries_per_scope_cap: defaults::vault_entries_per_scope_cap(),
            llm_api_key: SecretString::from(String::new()),
            pod: PodConfig::default(),
            env: EnvConfig::default(),
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

        // Webhook trigger label pass (the bare `FKST_WEBHOOK_TRIGGER_LABEL`) plus
        // the GitHub API base used by the per-user-store identity check.
        let webhook: WebhookVars = envy::prefixed(WEBHOOK_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        // A blank base would make every user-store identity check call a malformed
        // URL (and 503 the whole user surface). Reject it loudly, naming the var.
        if webhook.github_api_base_url.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_GITHUB_API_BASE_URL must not be blank".to_string(),
            ));
        }

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
        // Static LLM provider config (FKST_LLM_*). The model/base URL/wire_api
        // have serde defaults so the default path works out of the box, but a
        // blank override would render an unusable codex config.toml (no model /
        // unroutable base_url / empty wire_api). Reject it loudly, naming the
        // var. The API key requirement is enforced in the pod-dispatch block
        // below (it is only mandatory when sessions actually run).
        let llm: LlmVars = envy::prefixed(LLM_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        if llm.model.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_LLM_MODEL must not be blank".to_string(),
            ));
        }
        if llm.base_url.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_LLM_BASE_URL must not be blank".to_string(),
            ));
        }
        if llm.wire_api.trim().is_empty() {
            return Err(AppError::Config(
                "FKST_LLM_WIRE_API must not be blank".to_string(),
            ));
        }
        let llm_api_key = llm.api_key.filter(|s| !s.trim().is_empty());

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
            // A session that actually runs needs a real LLM credential, or the
            // engine 401s on every call. Fail closed when dispatch is on but no
            // key is configured. (Checked last so the image/namespace/time-bound
            // errors above surface first for an otherwise-empty dispatch config.)
            if llm_api_key.is_none() {
                return Err(AppError::Config(
                    "FKST_LLM_API_KEY must be set when FKST_POD_DISPATCH=true".to_string(),
                ));
            }
        }
        // Split the comma-separated DNS list, trimming and dropping empties. An
        // empty result means the operator blanked the var: the isolated pod
        // would have no resolver and could not reach GitHub/LLM, so fail closed.
        let dns_nameservers: Vec<String> = pod
            .dns_nameservers
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if dns_nameservers.is_empty() {
            return Err(AppError::Config(
                "FKST_POD_DNS_NAMESERVERS must list at least one resolver".to_string(),
            ));
        }
        let pod = PodConfig {
            dispatch: pod.dispatch,
            namespace: pod.namespace,
            image: pod_image,
            service_account: pod.service_account,
            run_ttl_secs: pod.run_ttl_secs,
            active_deadline_secs: pod.active_deadline_secs,
            llm_base_url: llm.base_url,
            llm_model: llm.model,
            llm_wire_api: llm.wire_api,
            dns_nameservers,
        };

        // Named-environment / install-validation knobs (FKST_ENV_*). Shares the
        // same `vars` snapshot; fails closed on its own zero bounds internally.
        let env = EnvConfig::from_vars(&vars)?;

        Ok(Config {
            port: http.port,
            bind_addr: http.bind_addr,
            log_level: http.log_level,
            request_timeout_secs: http.request_timeout_secs,
            vault_value_byte_cap: http.vault_value_byte_cap,
            vault_entries_per_scope_cap: http.vault_entries_per_scope_cap,
            llm_api_key: SecretString::from(llm_api_key.unwrap_or_default()),
            webhook_trigger_label: webhook.webhook_trigger_label,
            github_api_base_url: webhook.github_api_base_url.trim().to_string(),
            pod,
            env,
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
    use secrecy::ExposeSecret;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        // The control plane is datastore-free and has no application-level auth:
        // an otherwise-empty environment loads cleanly.
        let config = Config::from_vars(vars(&[])).expect("defaults should deserialize");
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
        assert_eq!(config.webhook_trigger_label, "fkst");
    }

    #[test]
    fn pod_dispatch_on_requires_an_image() {
        let err = Config::from_vars(vars(&[("FKST_POD_DISPATCH", "true")]))
            .expect_err("dispatch with no image must fail closed");
        assert!(err.to_string().contains("FKST_POD_IMAGE"));
    }

    #[test]
    fn pod_dispatch_on_with_image_parses_and_keeps_overrides() {
        let config = Config::from_vars(vars(&[
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "registry/fkst-control-plane:1.0"),
            ("FKST_POD_NAMESPACE", "sessions-prod"),
            ("FKST_POD_RUN_TTL_SECS", "900"),
            ("FKST_POD_ACTIVE_DEADLINE_SECS", "7200"),
            ("FKST_LLM_API_KEY", "sk-test"),
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
        assert_eq!(config.llm_api_key.expose_secret(), "sk-test");
    }

    #[test]
    fn pod_dispatch_on_requires_an_llm_api_key() {
        let err = Config::from_vars(vars(&[
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "img"),
        ]))
        .expect_err("dispatch with no llm api key must fail closed");
        assert!(err.to_string().contains("FKST_LLM_API_KEY"));
    }

    #[test]
    fn pod_dispatch_on_rejects_nonpositive_time_bounds() {
        let ttl = Config::from_vars(vars(&[
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "img"),
            ("FKST_POD_RUN_TTL_SECS", "0"),
        ]))
        .expect_err("zero ttl must fail");
        assert!(ttl.to_string().contains("FKST_POD_RUN_TTL_SECS"));

        let deadline = Config::from_vars(vars(&[
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
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "   "),
        ]))
        .expect_err("blank image must fail closed");
        assert!(err.to_string().contains("FKST_POD_IMAGE"));
    }

    // ---- pod DNS nameserver tests ---------------------------------------------

    #[test]
    fn pod_dns_nameservers_default() {
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(config.pod.dns_nameservers, vec!["1.1.1.1", "8.8.8.8"]);
    }

    #[test]
    fn pod_dns_nameservers_override_is_split_and_trimmed() {
        let config = Config::from_vars(vars(&[("FKST_POD_DNS_NAMESERVERS", "9.9.9.9, 1.0.0.1")]))
            .expect("override");
        assert_eq!(config.pod.dns_nameservers, vec!["9.9.9.9", "1.0.0.1"]);
    }

    #[test]
    fn blank_pod_dns_nameservers_is_a_config_error_naming_the_var() {
        let err = Config::from_vars(vars(&[("FKST_POD_DNS_NAMESERVERS", "   ")]))
            .expect_err("blank nameservers must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_POD_DNS_NAMESERVERS"));
    }

    #[test]
    fn no_mongodb_var_is_required_at_startup() {
        // Regression guard: with no MONGODB_URI set, the store-free control plane
        // must still load — there is no mandatory datastore config.
        Config::from_vars(vars(&[])).expect("loads without any MONGODB_* var");
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = Config::from_vars(vars(&[])).expect("defaults should deserialize");
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
        let config = Config::from_vars(vars(&[("FKST_HOSTED_PORT", "9090")])).unwrap();
        assert_eq!(config.port, 9090);
    }

    #[test]
    fn bind_addr_is_overridable() {
        let config = Config::from_vars(vars(&[("FKST_HOSTED_BIND_ADDR", "127.0.0.1")])).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1");
    }

    #[test]
    fn log_level_is_overridable() {
        let config = Config::from_vars(vars(&[("FKST_HOSTED_LOG_LEVEL", "debug")])).unwrap();
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn request_timeout_secs_is_overridable() {
        let config = Config::from_vars(vars(&[("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "5")])).unwrap();
        assert_eq!(config.request_timeout_secs, 5);
    }

    #[test]
    fn zero_request_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "0")]))
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

    // ---- webhook trigger label tests -------------------------------------------

    #[test]
    fn webhook_trigger_label_defaults_and_overrides() {
        let default = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(default.webhook_trigger_label, "fkst");
        let overridden = Config::from_vars(vars(&[("FKST_WEBHOOK_TRIGGER_LABEL", "fkst-cloud")]))
            .expect("override");
        assert_eq!(overridden.webhook_trigger_label, "fkst-cloud");
    }

    // ---- github api base (per-user store identity) tests ----------------------

    #[test]
    fn github_api_base_defaults_and_overrides() {
        let default = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(default.github_api_base_url, "https://api.github.com");
        let overridden = Config::from_vars(vars(&[(
            "FKST_GITHUB_API_BASE_URL",
            "http://127.0.0.1:8080",
        )]))
        .expect("override");
        assert_eq!(overridden.github_api_base_url, "http://127.0.0.1:8080");
    }

    #[test]
    fn blank_github_api_base_is_a_config_error_naming_the_var() {
        let err = Config::from_vars(vars(&[("FKST_GITHUB_API_BASE_URL", "   ")]))
            .expect_err("blank base must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_GITHUB_API_BASE_URL"));
    }

    // ---- vault configuration tests --------------------------------------------

    #[test]
    fn vault_caps_default() {
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(config.vault_value_byte_cap, 65_536);
        assert_eq!(config.vault_entries_per_scope_cap, 100);
    }

    #[test]
    fn vault_caps_are_overridable() {
        let config = Config::from_vars(vars(&[
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
            let err = Config::from_vars(vars(&[(var, value)])).expect_err("zero cap must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }

    // ---- named-environment (FKST_ENV_*) wiring tests ---------------------------

    #[test]
    fn env_config_defaults_are_wired_into_config() {
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(config.env.max_per_user, 20);
        assert_eq!(config.env.validate_max_concurrent, 4);
    }

    #[test]
    fn env_config_zero_bound_surfaces_through_config_from_vars() {
        let err = Config::from_vars(vars(&[("FKST_ENV_MAX_PER_USER", "0")]))
            .expect_err("zero env bound must fail closed through Config");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_ENV_MAX_PER_USER"));
    }

    // ---- static LLM provider configuration tests -------------------------------

    #[test]
    fn llm_defaults_apply_when_unset() {
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(config.pod.llm_model, "gpt-5-codex");
        assert_eq!(
            config.pod.llm_base_url,
            "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm"
        );
        // The wire_api MUST default to `chat` (chrono-llm 502s on `responses`).
        assert_eq!(config.pod.llm_wire_api, "chat");
        // No key configured (dispatch off) => empty, never a placeholder.
        assert_eq!(config.llm_api_key.expose_secret(), "");
    }

    #[test]
    fn llm_vars_are_overridable() {
        let config = Config::from_vars(vars(&[
            ("FKST_LLM_MODEL", "gpt-4.1"),
            ("FKST_LLM_BASE_URL", "https://proxy.example/s/llm"),
            ("FKST_LLM_WIRE_API", "responses"),
            ("FKST_LLM_API_KEY", "sk-abc"),
        ]))
        .expect("overrides");
        assert_eq!(config.pod.llm_model, "gpt-4.1");
        assert_eq!(config.pod.llm_base_url, "https://proxy.example/s/llm");
        assert_eq!(config.pod.llm_wire_api, "responses");
        assert_eq!(config.llm_api_key.expose_secret(), "sk-abc");
    }

    #[test]
    fn blank_llm_vars_are_config_errors_naming_the_var() {
        for var in ["FKST_LLM_MODEL", "FKST_LLM_BASE_URL", "FKST_LLM_WIRE_API"] {
            let err = Config::from_vars(vars(&[(var, "   ")])).expect_err("blank must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }
}
