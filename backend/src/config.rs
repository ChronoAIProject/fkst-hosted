//! Typed runtime configuration loaded from environment variables.
//!
//! Several envy passes over the same snapshot of variables, one per prefix: the
//! `FKST_HOSTED_*` HTTP/server settings, the `FKST_POD_*` pod-dispatch settings,
//! the `FKST_LLM_*` static LLM-provider settings, and the bare `FKST_*`
//! (`FKST_GITHUB_API_BASE_URL`).
//!
//! The control plane is API-only and datastore-free: there is no in-process
//! session execution, no worker fleet, no journaling, and no MongoDB, so none of
//! the dispatch/worker/journal knobs survive. Identity is the HMAC-verified
//! GitHub webhook actor — there is no application-level auth to configure.

use secrecy::SecretString;
use serde::Deserialize;

use crate::env_config::EnvConfig;
use crate::error::AppError;
use crate::reconcile_config::ReconcileConfig;

/// Prefix shared by every HTTP/server configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_";

/// Prefix for the bare `FKST_*` settings (currently only
/// `FKST_GITHUB_API_BASE_URL`); the envy pass reads them with the `FKST_`
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
        "gpt-5.5".to_string()
    }

    pub(super) fn llm_base_url() -> String {
        // Base URL of the LLM provider the session codex talks to.
        "https://llm.aelf.dev/v1".to_string()
    }

    pub(super) fn llm_wire_api() -> String {
        // The codex `wire_api`. MUST default to `chat`: chrono-llm serves only
        // `/chat/completions`; `responses` returns 502 (a verified bug). Never
        // default to `responses`.
        "chat".to_string()
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
        // The ServiceAccount the session pods run as (minimal identity).
        "fkst-session-runner".to_string()
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

/// Bare `FKST_*` settings (currently only `FKST_GITHUB_API_BASE_URL`); envy
/// pass with the `FKST_` prefix.
#[derive(Debug, Deserialize)]
struct WebhookVars {
    /// Base URL the per-user-store GitHub-token identity check calls
    /// (`GET {base}/user`). Env: `FKST_GITHUB_API_BASE_URL`. Default
    /// `https://api.github.com`; non-blank (tests point it at a mock server).
    #[serde(default = "defaults::github_api_base_url")]
    github_api_base_url: String,
}

/// `FKST_POD_*`-prefixed variables (pod-per-session dispatch, milestone #9).
/// kube-client owns these.
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
    /// Comma-separated external DNS resolvers for the isolated pod. Parsed as a
    /// String (not a Vec) to avoid envy's Vec quirks; split in `from_vars`.
    #[serde(default = "defaults::pod_dns_nameservers_raw")]
    dns_nameservers: String,
    /// Optional `runtimeClassName` for the session/validation pods. Absent (or
    /// blank) means the cluster default runtime (runc); split/trimmed in
    /// `from_vars`.
    #[serde(default)]
    runtime_class: Option<String>,
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
    /// The image session pods run (the control-plane image, `run-substrate`
    /// mode). Env: `FKST_POD_IMAGE`. Required when `dispatch=true`.
    pub image: Option<String>,
    /// ServiceAccount the session pods run as. Env: `FKST_POD_SERVICE_ACCOUNT`.
    /// Default `fkst-session-runner`.
    pub service_account: String,
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
    /// The pod `runtimeClassName` for both the session and the env-validation
    /// pod. Env: `FKST_POD_RUNTIME_CLASS`. Default **unset = runc** (the cluster
    /// default runtime, so local/docker-desktop keeps working). Set to e.g.
    /// `kata` in prod to run every session under a sandboxed runtime (Kata
    /// Containers) — the nodes must have the Kata runtime installed and nested
    /// virtualization enabled. Session and validation pods share this value.
    pub runtime_class: Option<String>,
}

impl Default for PodConfig {
    fn default() -> Self {
        Self {
            dispatch: false,
            namespace: defaults::pod_namespace(),
            image: None,
            service_account: defaults::pod_service_account(),
            llm_base_url: defaults::llm_base_url(),
            llm_model: defaults::llm_model(),
            llm_wire_api: defaults::llm_wire_api(),
            dns_nameservers: defaults::pod_dns_nameservers(),
            runtime_class: None,
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
    /// Model B reconciler knobs (`FKST_*`, issue #359 §4). Config surface only —
    /// no behaviour reads these yet (PR5b wires the loop; PR6 flips Model B on).
    pub reconcile: ReconcileConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            bind_addr: defaults::bind_addr(),
            log_level: defaults::log_level(),
            request_timeout_secs: defaults::request_timeout_secs(),
            github_api_base_url: defaults::github_api_base_url(),
            vault_value_byte_cap: defaults::vault_value_byte_cap(),
            vault_entries_per_scope_cap: defaults::vault_entries_per_scope_cap(),
            llm_api_key: SecretString::from(String::new()),
            pod: PodConfig::default(),
            env: EnvConfig::default(),
            reconcile: ReconcileConfig::default(),
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

        // Model B reconciler knobs (FKST_*). Built BEFORE the pod-dispatch block so
        // the dispatch-on `FKST_GITHUB_BOT_LOGIN` requirement (issue #359 §8, the
        // PR6 flip) can read it. Shares the same `vars` snapshot; fails closed on
        // its own cadence / token-refresh bounds internally.
        let reconcile = ReconcileConfig::from_vars(&vars)?;

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
            // A session that actually runs needs a real LLM credential, or the
            // engine 401s on every call. Fail closed when dispatch is on but no
            // key is configured. (Checked last so the image/namespace/time-bound
            // errors above surface first for an otherwise-empty dispatch config.)
            if llm_api_key.is_none() {
                return Err(AppError::Config(
                    "FKST_LLM_API_KEY must be set when FKST_POD_DISPATCH=true".to_string(),
                ));
            }
            // Model B posts feedback + drives sessions as its bot identity; the
            // reconciler needs the bot's login to attribute its own comments (and
            // skip them). PR5a deferred this requirement to the flip — enforce it
            // now that dispatch means Model B is live.
            if reconcile.github_bot_login.is_none() {
                return Err(AppError::Config(
                    "FKST_GITHUB_BOT_LOGIN must be set when FKST_POD_DISPATCH=true".to_string(),
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
            llm_base_url: llm.base_url,
            llm_model: llm.model,
            llm_wire_api: llm.wire_api,
            dns_nameservers,
            // Blank (or an empty ConfigMap value) means the cluster default
            // runtime (runc); only a real name selects a sandboxed RuntimeClass.
            runtime_class: pod
                .runtime_class
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
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
            github_api_base_url: webhook.github_api_base_url.trim().to_string(),
            pod,
            env,
            reconcile,
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
        assert!(config.pod.image.is_none());
        assert_eq!(config.request_timeout_secs, 30);
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
            ("FKST_LLM_API_KEY", "sk-test"),
            // Required by the PR6 flip whenever dispatch is on.
            ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
        ]))
        .expect("valid dispatch config should load");
        assert!(config.pod.dispatch);
        assert_eq!(
            config.pod.image.as_deref(),
            Some("registry/fkst-control-plane:1.0")
        );
        assert_eq!(config.pod.namespace, "sessions-prod");
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
    fn pod_dispatch_on_requires_a_github_bot_login() {
        // The LLM key is set so this passes the earlier LLM check and reaches the
        // bot-login requirement the PR6 flip added.
        let err = Config::from_vars(vars(&[
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "img"),
            ("FKST_LLM_API_KEY", "sk-test"),
        ]))
        .expect_err("dispatch with no bot login must fail closed");
        assert!(err.to_string().contains("FKST_GITHUB_BOT_LOGIN"));
    }

    #[test]
    fn pod_dispatch_on_with_bot_login_and_key_loads() {
        // The full happy path: dispatch on with an image, an LLM key, and a bot
        // login loads cleanly and surfaces the login on the reconcile config.
        let config = Config::from_vars(vars(&[
            ("FKST_POD_DISPATCH", "true"),
            ("FKST_POD_IMAGE", "img"),
            ("FKST_LLM_API_KEY", "sk-test"),
            ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
        ]))
        .expect("valid dispatch config with a bot login should load");
        assert_eq!(
            config.reconcile.github_bot_login.as_deref(),
            Some("fkst-bot")
        );
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

    // ---- pod runtime-class (Kata) tests ---------------------------------------

    #[test]
    fn pod_runtime_class_defaults_to_none() {
        // Unset means the cluster default runtime (runc) — local docker-desktop
        // has no Kata RuntimeClass, so the default must not select one.
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(config.pod.runtime_class, None);
    }

    #[test]
    fn pod_runtime_class_override_is_kept() {
        let config =
            Config::from_vars(vars(&[("FKST_POD_RUNTIME_CLASS", "kata")])).expect("override");
        assert_eq!(config.pod.runtime_class.as_deref(), Some("kata"));
    }

    #[test]
    fn blank_pod_runtime_class_is_treated_as_none() {
        // A blank ConfigMap value must fall back to runc, not to an empty (and
        // therefore invalid) runtimeClassName.
        let config = Config::from_vars(vars(&[("FKST_POD_RUNTIME_CLASS", "   ")]))
            .expect("blank runtime class");
        assert_eq!(config.pod.runtime_class, None);
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

    // ---- Model B reconciler (FKST_*) wiring tests ------------------------------

    #[test]
    fn reconcile_config_defaults_are_wired_into_config() {
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(
            config.reconcile.substrate_trigger_label,
            "fkst-substrate-trigger"
        );
        assert_eq!(config.reconcile.reconcile_interval_secs, 30);
        assert_eq!(config.reconcile.github_bot_login, None);
    }

    #[test]
    fn reconcile_config_override_surfaces_through_config_from_vars() {
        let config = Config::from_vars(vars(&[("FKST_RECONCILE_INTERVAL_SECS", "5")]))
            .expect("override should surface");
        assert_eq!(config.reconcile.reconcile_interval_secs, 5);
    }

    #[test]
    fn reconcile_config_bound_violation_surfaces_through_config_from_vars() {
        let err = Config::from_vars(vars(&[("FKST_POD_TOKEN_REFRESH_SECS", "3600")]))
            .expect_err("token refresh at TTL must fail closed through Config");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_POD_TOKEN_REFRESH_SECS"));
    }

    // ---- static LLM provider configuration tests -------------------------------

    #[test]
    fn llm_defaults_apply_when_unset() {
        let config = Config::from_vars(vars(&[])).expect("defaults");
        assert_eq!(config.pod.llm_model, "gpt-5.5");
        assert_eq!(config.pod.llm_base_url, "https://llm.aelf.dev/v1");
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
