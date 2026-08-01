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

use std::collections::BTreeMap;

use secrecy::SecretString;
use serde::Deserialize;

use crate::audit::AuditConfig;
use crate::env_config::EnvConfig;
use crate::error::AppError;
use crate::leader_config::LeaderElectionConfig;
use crate::log_config::LogConfig;
use crate::operations::{ActivityQueryConfig, SandboxInventoryConfig};
use crate::osb_config::OpensandboxConfig;
use crate::reconcile_config::ReconcileConfig;
use crate::storage::ChronoStorageConfig;

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
        // non-empty default, never a literal placeholder. (gpt-5.5 → gpt-5.6-sol,
        // issue #3393.)
        "gpt-5.6-sol".to_string()
    }

    pub(super) fn llm_reasoning_effort() -> String {
        // The codex `model_reasoning_effort` every session runs at unless the
        // trigger's `### Engine Config` overrides it (issue #3393). Platform
        // default is the deepest tier.
        "max".to_string()
    }

    pub(super) fn llm_base_url() -> String {
        // Base URL of the LLM provider the session codex talks to.
        "https://llm.aelf.dev/v1".to_string()
    }

    pub(super) fn llm_wire_api() -> String {
        // The codex `wire_api`. Defaults to `responses`: codex 0.139+ dropped
        // `wire_api = "chat"` (it errors at config load), and the LLM backend
        // (verified on llm.aelf.dev) serves the `/responses` API. Operators pin a
        // chat-only backend via `FKST_LLM_WIRE_API`.
        "responses".to_string()
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
    /// Raw `FKST_POD_MODE` value; parsed into [`PodMode`] in `from_vars` so a
    /// bad value yields our own precise error (envy would swallow the text).
    #[serde(default)]
    mode: Option<String>,
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
    /// Raw `FKST_POD_RATE_POOLS` value (operator-default engine rate pools);
    /// parsed + validated in `from_vars` so a bad token yields our own precise
    /// error. Space-separated `NAME=<burst>,<refill_per_minute>` tokens.
    #[serde(default)]
    rate_pools: Option<String>,
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
    /// `FKST_LLM_REASONING_EFFORT` — the codex `model_reasoning_effort` for
    /// every session (default `max`). Normalized + validated in `from_vars`
    /// against [`LLM_REASONING_EFFORTS`]; a bad value fails closed.
    #[serde(default = "defaults::llm_reasoning_effort")]
    reasoning_effort: String,
    /// The static LLM API key. Optional at parse time; REQUIRED non-blank when
    /// `FKST_POD_DISPATCH=true` (an engine with no LLM credential 401s).
    #[serde(default)]
    api_key: Option<String>,
}

/// The accepted `FKST_LLM_REASONING_EFFORT` / `### Engine Config`
/// `FKST_LLM_REASONING_EFFORT` values — the codex `model_reasoning_effort`
/// tiers. ONE list shared by the startup validation here and the trigger-side
/// 422 ([`crate::goals::engine_config`]) so the two can never drift.
pub(crate) const LLM_REASONING_EFFORTS: [&str; 5] = ["minimal", "low", "medium", "high", "max"];

/// Normalize a reasoning-effort value (trim + ASCII-lowercase) and accept it
/// only when it names one of [`LLM_REASONING_EFFORTS`]. `None` = invalid; the
/// caller renders its own error (startup Config error vs trigger 422). A blank
/// value is NOT accepted here — blank-means-default is the caller's rule.
pub(crate) fn normalize_llm_reasoning_effort(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    LLM_REASONING_EFFORTS
        .contains(&normalized.as_str())
        .then_some(normalized)
}

/// Which session-execution backend the control plane drives. Selected by
/// `FKST_POD_MODE`. `K8sCustomized` is the default (one Kubernetes Job per
/// session); `Opensandbox` drives one OpenSandbox sandbox per session (issue #420)
/// and requires the `FKST_OSB_*` block. Deliberately NOT
/// `serde::Deserialize`/`FromStr`: it is parsed by hand in `from_vars` so a bad
/// value surfaces our own precise error text rather than an envy message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PodMode {
    /// The k8s-customized backend (one Kubernetes Job per session). Default.
    K8sCustomized,
    /// The OpenSandbox backend (one sandbox per session). Requires the
    /// `FKST_OSB_*` config block when pod dispatch is on.
    Opensandbox,
}

/// One operator-default engine rate pool (`FKST_POD_RATE_POOLS` token
/// `NAME=<burst>,<refill_per_minute>`), injected into every session as
/// `FKST_RATE_POOL_<NAME>` so the engine throttles matching external commands
/// (`gh`, `git`, …) — including calls made inside the codex coding agent, via
/// the engine's PATH shims. Protects the shared installation-scoped GitHub App
/// API budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RatePool {
    pub burst: u64,
    pub refill_per_minute: u64,
}

/// Parse + validate the raw `FKST_POD_RATE_POOLS` value: space-separated
/// `NAME=<burst>,<refill_per_minute>` tokens. Fail closed on any malformed
/// token, naming it — a silently-dropped pool would leave sessions unthrottled,
/// the exact failure this knob exists to prevent. `NAME` is restricted to the
/// engine-safe uppercase subset and may not be `ROOT` (that would render as
/// `FKST_RATE_POOL_ROOT`, the engine's ledger-directory env, not a pool).
/// Values must be positive u64s — overflow must fail here, not pod-side.
fn parse_rate_pools(raw: &str) -> Result<BTreeMap<String, RatePool>, AppError> {
    fn err(token: &str, reason: &str) -> AppError {
        AppError::Config(format!(
            "FKST_POD_RATE_POOLS token {token:?} is invalid: {reason}; expected \
             space-separated NAME=<burst>,<refill_per_minute> with NAME matching \
             ^[A-Z0-9_]+$ (not ROOT) and both values positive integers"
        ))
    }
    let mut pools = BTreeMap::new();
    for token in raw.split_whitespace() {
        let (name, value) = token
            .split_once('=')
            .ok_or_else(|| err(token, "missing `=`"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(err(token, "the pool NAME must match ^[A-Z0-9_]+$"));
        }
        if name == "ROOT" {
            return Err(err(
                token,
                "ROOT is reserved (FKST_RATE_POOL_ROOT is the ledger dir, not a pool)",
            ));
        }
        let (burst_raw, refill_raw) = value
            .split_once(',')
            .ok_or_else(|| err(token, "missing `,` between burst and refill"))?;
        let parse_positive = |part: &str, which: &str| -> Result<u64, AppError> {
            let n: u64 = part
                .parse()
                .map_err(|_| err(token, &format!("the {which} must be a u64")))?;
            if n == 0 {
                return Err(err(token, &format!("the {which} must be >= 1")));
            }
            Ok(n)
        };
        let pool = RatePool {
            burst: parse_positive(burst_raw, "burst")?,
            refill_per_minute: parse_positive(refill_raw, "refill_per_minute")?,
        };
        if pools.insert(name.to_string(), pool).is_some() {
            return Err(err(token, "duplicate pool NAME"));
        }
    }
    Ok(pools)
}

/// Pod-per-session dispatch configuration (milestone #9). When `dispatch` is
/// false (the default) the control plane never touches Kubernetes.
#[derive(Clone, Debug)]
pub struct PodConfig {
    /// Master switch. Env: `FKST_POD_DISPATCH`. Default false.
    pub dispatch: bool,
    /// The session-execution backend. Env: `FKST_POD_MODE`. Default
    /// `k8s-customized`; `opensandbox` drives one OpenSandbox sandbox per session
    /// (requires the `FKST_OSB_*` block when dispatch is on).
    pub mode: PodMode,
    /// Namespace for per-session Jobs + Secrets. Env: `FKST_POD_NAMESPACE`.
    /// Default `fkst-sessions`. REQUIRED non-blank when dispatch is on in BOTH
    /// modes: even in opensandbox mode the env-store `KubeClient`
    /// (`KubeClient::from_inferred`) binds to this namespace to read per-session
    /// environment/secret objects, so it is never an opensandbox-ignored knob.
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
    /// Env: `FKST_LLM_WIRE_API`. Default `responses` (codex 0.139+ dropped
    /// `chat`; see [`defaults::llm_wire_api`]) — operators pin a chat-only
    /// backend explicitly.
    pub llm_wire_api: String,
    /// codex `model_reasoning_effort` injected into the session pod as
    /// `FKST_LLM_REASONING_EFFORT`. Env: `FKST_LLM_REASONING_EFFORT`. Default
    /// `max` (issue #3393); one of [`LLM_REASONING_EFFORTS`], normalized
    /// lowercase, fail-closed on anything else. A trigger's `### Engine Config`
    /// may override it per session.
    pub llm_reasoning_effort: String,
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
    /// Operator-default engine rate pools, keyed by pool NAME (rendered into
    /// every session as `FKST_RATE_POOL_<NAME>`). Env: `FKST_POD_RATE_POOLS`,
    /// space-separated `NAME=<burst>,<refill_per_minute>` tokens. Default empty
    /// (no pools, no PATH shims — exactly the pre-knob behavior).
    pub rate_pools: BTreeMap<String, RatePool>,
}

impl Default for PodConfig {
    fn default() -> Self {
        Self {
            dispatch: false,
            mode: PodMode::K8sCustomized,
            namespace: defaults::pod_namespace(),
            image: None,
            service_account: defaults::pod_service_account(),
            llm_base_url: defaults::llm_base_url(),
            llm_model: defaults::llm_model(),
            llm_wire_api: defaults::llm_wire_api(),
            llm_reasoning_effort: defaults::llm_reasoning_effort(),
            dns_nameservers: defaults::pod_dns_nameservers(),
            runtime_class: None,
            rate_pools: BTreeMap::new(),
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
    /// Deployment-wide GitHub-identity access policy (`FKST_ACCESS_ALLOWED_USERS`
    /// and `FKST_GLOBAL_ADMINS`, comma-separated logins and/or numeric ids).
    /// Unset access list = open; a set list admits listed identities, while global
    /// admins are always admitted and receive App-wide read visibility. See
    /// [`crate::access_policy`].
    pub access: crate::access_policy::AccessPolicy,
    /// Exact operator-owned cross-repository delivery routes. Empty by default;
    /// see [`crate::delivery_grants::DeliveryGrantPolicy`].
    pub delivery_grants: crate::delivery_grants::DeliveryGrantPolicy,
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
    /// OpenSandbox session-backend config (`FKST_OSB_*`, issue #420). `Some` only
    /// when pod dispatch is on AND `FKST_POD_MODE=opensandbox`; `None` for the
    /// default k8s-customized backend (or dispatch off). A required var missing
    /// under opensandbox fails closed at startup (see [`crate::osb_config::from_vars`]).
    pub opensandbox: Option<OpensandboxConfig>,
    /// Named-environment storage and install-validation knobs (`FKST_ENV_*`).
    pub env: EnvConfig,
    /// Model B reconciler knobs (`FKST_*`, issue #359 §4). Config surface only —
    /// no behaviour reads these yet (PR5b wires the loop; PR6 flips Model B on).
    pub reconcile: ReconcileConfig,
    /// Optional Kubernetes Lease ownership for the reconciler task group.
    pub leader: LeaderElectionConfig,
    /// Optional chrono-storage log-streaming config (`FKST_STORAGE_*` /
    /// `FKST_NYXID_*`). `None` when the feature is unset (log streaming
    /// disabled); a partial config fails closed at startup (see
    /// [`ChronoStorageConfig::from_vars`]).
    pub storage: Option<ChronoStorageConfig>,
    /// On-demand session-log download config (`FKST_LOG_ADMINS`,
    /// `FKST_PUBLIC_BASE_URL`, `FKST_GITHUB_OAUTH_*`): the global-admin allow-list,
    /// the public base URL the announce comment links, and the browser-mode OAuth
    /// creds. All optional; a half-configured OAuth pair fails closed (see
    /// [`LogConfig::from_vars`]).
    pub log: LogConfig,
    /// Chat-concierge config (`FKST_CHAT_*`). `None` when `FKST_CHAT_ENABLED` is
    /// not true — the feature is then entirely dark (no route mounted, no provider
    /// credential required). See [`crate::chat::config::from_vars`].
    pub chat: Option<crate::chat::config::ChatConfig>,
    /// Audit capture config (`FKST_POSTHOG_*` + `FKST_DEPLOYMENT_ENVIRONMENT`).
    /// Always present; `enabled == false` (the default) installs the no-op sink
    /// and starts no delivery worker. See [`crate::audit::config`].
    pub audit: AuditConfig,
    /// Activity-QUERY config (`FKST_POSTHOG_PROJECT_ID` / `FKST_POSTHOG_QUERY_*`
    /// / `FKST_POSTHOG_ACTIVITY_*`). Always present; deliberately separate from
    /// [`Config::audit`] because capture and query are two credentials with two
    /// blast radii and the ingestion token may never stand in for the read key.
    /// See [`crate::operations::config`].
    pub activity_query: ActivityQueryConfig,
    /// Audit DELIVERY config (`FKST_AUDIT_DELIVERY_MODE`, `FKST_AUDIT_RELAY_*`,
    /// `FKST_AUDIT_INCOMPLETE_GRACE_SECS`). Always present; the default mode is
    /// `disabled`, which preserves the pre-relay behaviour exactly. See
    /// [`crate::audit::relay::config`].
    pub audit_delivery: crate::audit::relay::AuditDeliveryConfig,
    /// Live sandbox inventory config (`FKST_OPERATIONS_SANDBOX_*`). Always
    /// present; whether a deployment can answer depends on the runtime backend,
    /// not on this block. See [`crate::operations::sandbox::config`].
    pub sandbox: SandboxInventoryConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            bind_addr: defaults::bind_addr(),
            log_level: defaults::log_level(),
            request_timeout_secs: defaults::request_timeout_secs(),
            github_api_base_url: defaults::github_api_base_url(),
            access: crate::access_policy::AccessPolicy::default(),
            delivery_grants: crate::delivery_grants::DeliveryGrantPolicy::default(),
            vault_value_byte_cap: defaults::vault_value_byte_cap(),
            vault_entries_per_scope_cap: defaults::vault_entries_per_scope_cap(),
            llm_api_key: SecretString::from(String::new()),
            pod: PodConfig::default(),
            opensandbox: None,
            env: EnvConfig::default(),
            reconcile: ReconcileConfig::default(),
            leader: LeaderElectionConfig::default(),
            storage: None,
            log: LogConfig::default(),
            chat: None,
            audit: AuditConfig::default(),
            activity_query: ActivityQueryConfig::default(),
            audit_delivery: crate::audit::relay::AuditDeliveryConfig::default(),
            sandbox: SandboxInventoryConfig::default(),
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
        let leader = LeaderElectionConfig::from_vars(&vars)?;

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
        // Session-execution backend. Parsed UNCONDITIONALLY (even with dispatch
        // off) so a bad value fails closed at startup regardless of dispatch.
        // Blank/unset means the default k8s-customized backend, matching the
        // repo's blank-as-absent convention.
        let pod_mode = match pod.mode.as_deref().map(str::trim) {
            None | Some("") | Some("k8s-customized") => PodMode::K8sCustomized,
            Some("opensandbox") => PodMode::Opensandbox,
            Some(other) => {
                return Err(AppError::Config(format!(
                    "FKST_POD_MODE must be one of \"k8s-customized\" | \"opensandbox\" (got \"{other}\")"
                )))
            }
        };
        if pod.dispatch {
            // FKST_POD_IMAGE + FKST_POD_NAMESPACE are required in BOTH backend modes:
            // the image is the session pod / sandbox image, and the namespace binds
            // the env-store KubeClient (mode-independent). The OpenSandbox-specific
            // FKST_OSB_* vars are validated separately below via osb_config::from_vars.
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
        // Normalize + validate the reasoning effort (unset ⇒ the serde default
        // `max`). A SET value must name a known tier — an unknown value would
        // crash codex at session start, and a blanked var is an explicit
        // operator action — so both fail closed with the accepted list spelled
        // out, matching the blank-LLM-var convention of the model/URL/wire trio.
        let llm_reasoning_effort = normalize_llm_reasoning_effort(&llm.reasoning_effort)
            .ok_or_else(|| {
                AppError::Config(format!(
                    "FKST_LLM_REASONING_EFFORT must be one of {} (got {:?})",
                    LLM_REASONING_EFFORTS.join(" | "),
                    llm.reasoning_effort.trim()
                ))
            })?;
        let pod = PodConfig {
            dispatch: pod.dispatch,
            mode: pod_mode,
            namespace: pod.namespace,
            image: pod_image,
            service_account: pod.service_account,
            llm_base_url: llm.base_url,
            llm_model: llm.model,
            llm_wire_api: llm.wire_api,
            llm_reasoning_effort,
            dns_nameservers,
            // Blank (or an empty ConfigMap value) means the cluster default
            // runtime (runc); only a real name selects a sandboxed RuntimeClass.
            runtime_class: pod
                .runtime_class
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            // Parsed UNCONDITIONALLY (like the mode) so a malformed pool fails
            // closed at startup even with dispatch off.
            rate_pools: parse_rate_pools(pod.rate_pools.as_deref().unwrap_or(""))?,
        };
        if leader.enabled && !pod.dispatch {
            return Err(AppError::Config(
                "FKST_LEADER_ELECTION_ENABLED=true requires FKST_POD_DISPATCH=true".to_string(),
            ));
        }

        // OpenSandbox session-backend config (FKST_OSB_*). Validated ONLY when pod
        // dispatch is on AND the selected backend is opensandbox; otherwise skipped
        // (`None`), so a k8s-customized deploy never sets an FKST_OSB_* var. Shares
        // the same `vars` snapshot; fails closed naming any missing/invalid var.
        let opensandbox =
            crate::osb_config::from_vars(&vars, pod.dispatch && pod.mode == PodMode::Opensandbox)?;

        // Named-environment / install-validation knobs (FKST_ENV_*). Shares the
        // same `vars` snapshot; fails closed on its own zero bounds internally.
        let env = EnvConfig::from_vars(&vars)?;
        if env
            .store_namespace
            .as_deref()
            .is_some_and(|namespace| namespace == pod.namespace.trim())
        {
            return Err(AppError::Config(
                "FKST_ENV_STORE_NAMESPACE must differ from FKST_POD_NAMESPACE so \
                 environment profiles survive application-namespace loss"
                    .to_string(),
            ));
        }

        // Optional chrono-storage log-streaming config (FKST_STORAGE_* /
        // FKST_NYXID_*). Shares the same `vars` snapshot; `None` when unset,
        // and fails closed on a partial config (naming the missing vars).
        let storage = ChronoStorageConfig::from_vars(&vars)?;

        // Egress-safety gate (opensandbox mode only): every endpoint HANDED TO
        // SESSIONS must be publicly reachable — the sandbox-lockdown NetworkPolicy
        // blocks RFC1918 / cluster-internal egress, so a private value here would
        // black-hole mid-session. Runs AFTER storage resolves so the storage URLs
        // (which ride the per-session creds files) are vetted too. Deliberately
        // NOT vetted: FKST_OSB_BASE_URL (backend→server, intentionally in-cluster).
        if opensandbox.is_some() {
            let mut sandbox_endpoints: Vec<(&str, &str)> =
                vec![("FKST_LLM_BASE_URL", pod.llm_base_url.as_str())];
            if let Some(storage) = &storage {
                sandbox_endpoints.push(("FKST_STORAGE_BASE_URL", storage.base_url.as_str()));
                sandbox_endpoints.push(("FKST_NYXID_TOKEN_URL", storage.nyxid_token_url.as_str()));
            }
            crate::osb_config::ensure_sandbox_endpoints_reachable(&sandbox_endpoints)?;
        }

        // On-demand session-log download config (FKST_LOG_ADMINS,
        // FKST_PUBLIC_BASE_URL, FKST_GITHUB_OAUTH_*). Shares the same `vars`
        // snapshot; fails closed only on a half-configured OAuth id/secret pair.
        let log = LogConfig::from_vars(&vars)?;

        // Chat concierge (FKST_CHAT_*). `None` unless FKST_CHAT_ENABLED=true — with
        // the feature off no chat variable is validated at all, so a half-staged
        // block cannot fail an unrelated deploy. When on, it inherits the FKST_LLM_*
        // provider values above unless overridden, and fails closed naming the
        // exact variable. Shares the same `vars` snapshot.
        let chat = crate::chat::config::from_vars(&vars)?;

        // Audit capture (FKST_POSTHOG_* + FKST_DEPLOYMENT_ENVIRONMENT). Always
        // resolved: the feature is off by default, but its numeric bounds are
        // validated unconditionally so a typo surfaces at deploy time instead of
        // at the moment an operator flips it on. Shares the same `vars` snapshot.
        let audit = AuditConfig::from_vars(&vars)?;
        let activity_query = ActivityQueryConfig::from_vars(&vars)?;
        // Audit DELIVERY (FKST_AUDIT_*). Fails closed when a mode that promises
        // durability has no relay to talk to: a `required` deployment that
        // silently degraded to best-effort would make its central claim false.
        let audit_delivery = crate::audit::relay::AuditDeliveryConfig::from_vars(&vars)?;
        // Live-inventory ceilings + route budget. Validated unconditionally for
        // the same reason: a zero ceiling would silently take the operations
        // sandbox view down at the first request, not at deploy time.
        let sandbox = SandboxInventoryConfig::from_vars(&vars)?;
        // The inventory budget is meant to sit BELOW the ceiling the route runs
        // under, so a slow fleet read fails as an explicit `503` rather than as a
        // bare request timeout the caller cannot interpret. That ceiling is the
        // `/api/v1` subtree's, not the top-level one — see `api_subtree_timeout`.
        sandbox.warn_unless_below(crate::router::api_subtree_timeout(&env));
        // The incomplete grace must OUTLAST the longest audited request, or the
        // relay force-closes a request that is still running. The widest ceiling
        // is whichever is larger: the `/api/v1` subtree's budget (which bounds
        // every audited route) or the top-level request timeout (which bounds the
        // handful of routes outside that nest). Fail-closed rather than a warning
        // — the failure mode is a fabricated terminal state, not slow reads.
        audit_delivery.ensure_grace_covers(std::cmp::max(
            crate::router::api_subtree_timeout(&env),
            std::time::Duration::from_secs(http.request_timeout_secs),
        ))?;
        // The two capture shapes are mutually exclusive, and until now that was
        // only asserted in the deployment docs. Running both means two writers
        // into one PostHog project — duplicated logical events whose dedup uuid
        // only covers ONE of the writers — and, worse, it legitimises putting
        // FKST_POSTHOG_PROJECT_TOKEN back into the control-plane record, which
        // is the exact credential boundary the relay exists to draw (epic
        // `OPS-02`). Fail closed naming both variables; the operator picks one.
        if audit.enabled && audit_delivery.mode.uses_relay() {
            return Err(AppError::Config(format!(
                "FKST_POSTHOG_ENABLED=true and FKST_AUDIT_DELIVERY_MODE={} are mutually \
                 exclusive: the relay is then the capture writer, so the control plane must \
                 not also capture directly (and must not hold FKST_POSTHOG_PROJECT_TOKEN)",
                audit_delivery.mode.as_str()
            )));
        }

        // Deployment-wide access policy (FKST_ACCESS_ALLOWED_USERS +
        // FKST_ACCESS_BLOCKED_USERS + FKST_GLOBAL_ADMINS + FKST_AUTH_MODEL).
        // Derived default: no list = open, allowed list = enforced allowlist
        // (set-but-empty = enforced deny-all), blocked list = enforced denylist.
        // Fails closed (naming the vars) on an unrecognized FKST_AUTH_MODEL,
        // both lists set without an explicit model, or a denylist whose set
        // blocklist yields zero valid entries.
        let access = crate::access_policy::AccessPolicy::from_vars(&vars)?;
        let delivery_grants = crate::delivery_grants::DeliveryGrantPolicy::from_vars(&vars)?;

        Ok(Config {
            port: http.port,
            bind_addr: http.bind_addr,
            log_level: http.log_level,
            request_timeout_secs: http.request_timeout_secs,
            vault_value_byte_cap: http.vault_value_byte_cap,
            vault_entries_per_scope_cap: http.vault_entries_per_scope_cap,
            llm_api_key: SecretString::from(llm_api_key.unwrap_or_default()),
            github_api_base_url: webhook.github_api_base_url.trim().to_string(),
            access,
            delivery_grants,
            pod,
            opensandbox,
            env,
            reconcile,
            leader,
            storage,
            log,
            chat,
            audit,
            activity_query,
            audit_delivery,
            sandbox,
        })
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<Config, AppError> {
        Self::from_vars(std::env::vars())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
