//! Typed configuration for the OpenSandbox session-execution backend.
//!
//! A single envy pass over the `FKST_OSB_*` prefix, mirroring the extracted-module
//! and fail-closed style of [`crate::storage::config`] and [`crate::reconcile_config`].
//! These knobs point the control plane at an OpenSandbox lifecycle server and shape
//! the per-session sandbox the [`crate::session_backend::opensandbox::OsbBackend`]
//! launches: the lifecycle base URL, the lifecycle API key, the seed the per-session
//! execd access token is derived from, and the sandbox's cpu / memory / entrypoint.
//!
//! Unlike the always-optional chrono-storage feature, these vars are validated ONLY
//! when they are actually needed — i.e. when pod dispatch is ON **and**
//! `FKST_POD_MODE=opensandbox`. In every other posture the whole block is skipped
//! (`Ok(None)`), so a k8s-customized deploy (the default) never has to set an
//! `FKST_OSB_*` var. When they ARE required, each is validated fail-closed with a
//! precise message naming the offending variable (mirroring the neighbouring
//! `FKST_POD_IMAGE` / `FKST_POD_NAMESPACE` errors in [`crate::config`]).
//!
//! Secrets discipline: the API key + execd seed are held in [`SecretString`]s and the
//! hand-written [`OpensandboxConfig`] `Debug` renders them as `<redacted>` (the
//! config-module convention, mirroring [`crate::storage::config::ChronoStorageConfig`]).
//!
//! Both secrets resolve FILE-FIRST: `FKST_OSB_API_KEY_FILE` /
//! `FKST_OSB_EXECD_TOKEN_SEED_FILE` point at CSI-mounted secret files (how the
//! production platform delivers them — GCP Secret Manager → GKE Secrets Store CSI);
//! when a `*_FILE` var is set the file WINS over the inline var, its contents are
//! trimmed (mounted secrets carry a trailing newline), and an unreadable or empty
//! file fails closed rather than silently falling back. The inline vars remain the
//! local-dev fallback. See [`resolve_secret_file_first`].

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;

/// Prefix shared by every OpenSandbox-backend configuration variable. The keys read
/// naturally (`FKST_OSB_BASE_URL`, `FKST_OSB_EXECD_TOKEN_SEED`). envy drops every
/// field it does not recognize, so this pass sees only the `FKST_OSB_*` keys.
const OSB_ENV_PREFIX: &str = "FKST_OSB_";

/// `FKST_POD_*` knobs the OpenSandbox sandbox template owns, so a value set here is
/// IGNORED in opensandbox mode. `FKST_POD_NAMESPACE` and `FKST_POD_IMAGE` are
/// DELIBERATELY absent: the namespace still binds the env-store KubeClient in BOTH
/// modes, and the image is the sandbox image too. See
/// [`ignored_pod_knobs_in_opensandbox`].
const IGNORED_POD_KNOBS: [&str; 4] = [
    "FKST_POD_SERVICE_ACCOUNT",
    "FKST_POD_DNS_NAMESERVERS",
    "FKST_POD_RUNTIME_CLASS",
    "FKST_POD_TERMINATION_GRACE_SECS",
];

/// The `FKST_OSB_*`-prefixed variables. All optional at the envy layer; the
/// presence/validity policy is applied in [`from_vars`].
#[derive(Debug, Deserialize)]
struct OsbVars {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    /// Path to a file holding the lifecycle API key (production: a CSI-mounted
    /// GCP Secret Manager secret). File-first: when set it WINS over `api_key`.
    #[serde(default)]
    api_key_file: Option<String>,
    #[serde(default)]
    execd_token_seed: Option<String>,
    /// Path to a file holding the execd token seed (same CSI channel as the API
    /// key). File-first: when set it WINS over `execd_token_seed`.
    #[serde(default)]
    execd_token_seed_file: Option<String>,
    #[serde(default)]
    session_cpu: Option<String>,
    #[serde(default)]
    session_memory: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    /// Whether execd is reached through the lifecycle server proxy. Validate-and-
    /// discard: only `true` (the default) is supported, so this is never stored.
    #[serde(default)]
    use_server_proxy: Option<bool>,
    /// Reserved health-inspection toggle (see [`OpensandboxConfig::inspect_health`]).
    #[serde(default)]
    inspect_health: Option<bool>,
}

/// Resolved OpenSandbox-backend configuration. Present only when pod dispatch is on
/// AND `FKST_POD_MODE=opensandbox`; otherwise the feature is skipped (`None`).
///
/// Named distinctly from the backend's `OsbConfig`
/// ([`crate::session_backend::opensandbox::backend::OsbConfig`]) to avoid an ident
/// clash: this is the CONFIG-LAYER view (raw env values), which `main.rs` maps into
/// the backend's launch config when it constructs the backend.
#[derive(Clone)]
pub struct OpensandboxConfig {
    /// The OpenSandbox lifecycle server base URL. Env: `FKST_OSB_BASE_URL`. Parsed
    /// into a [`reqwest::Url`] so an unroutable value fails closed at startup.
    pub base_url: reqwest::Url,
    /// The lifecycle API key (`OPEN-SANDBOX-API-KEY` header). Env: `FKST_OSB_API_KEY`,
    /// or file-first via `FKST_OSB_API_KEY_FILE` (production: a CSI-mounted GCP
    /// Secret Manager secret; the file wins and its contents are trimmed). A
    /// [`SecretString`]; never logged, redacted in `Debug`.
    pub api_key: SecretString,
    /// The long-lived seed the per-session execd access token is derived from (HMAC,
    /// see [`crate::session_backend::opensandbox::derive_execd_token`]). Env:
    /// `FKST_OSB_EXECD_TOKEN_SEED`, or file-first via
    /// `FKST_OSB_EXECD_TOKEN_SEED_FILE` (same CSI channel as the API key). A
    /// [`SecretString`]; never logged, redacted in `Debug`.
    pub execd_token_seed: SecretString,
    /// The sandbox `resourceLimits.cpu` value (e.g. `500m`). Env:
    /// `FKST_OSB_SESSION_CPU`.
    pub session_cpu: String,
    /// The sandbox `resourceLimits.memory` value (e.g. `512Mi`). Env:
    /// `FKST_OSB_SESSION_MEMORY`.
    pub session_memory: String,
    /// The absolute path of the in-sandbox control-plane binary the entrypoint runs
    /// (OpenSandbox has no image-default fallback, so it is always spelled out). Env:
    /// `FKST_OSB_ENTRYPOINT`; must be absolute.
    pub entrypoint: String,
    /// Reserved health-inspection toggle. Env: `FKST_OSB_INSPECT_HEALTH`. Default
    /// `false`. Stored (and surfaced in `Debug`) so it is a live config knob rather
    /// than dead code; no behaviour reads it yet.
    pub inspect_health: bool,
    /// The `FKST_POD_*` knobs the operator set that OpenSandbox mode IGNORES (the
    /// sandbox template owns them). Computed here (main.rs has no access to the raw
    /// vars) so `main.rs` can emit one WARN per ignored knob without config.rs
    /// logging. Never includes `FKST_POD_NAMESPACE` / `FKST_POD_IMAGE`.
    pub ignored_pod_knobs: Vec<&'static str>,
}

// Manual `Debug` that renders both secrets as `<redacted>` (the config-module
// convention, mirroring `ChronoStorageConfig`) so an accidental `{:?}` on the config
// — or on the `Config` embedding it — can never spill a credential into a log.
impl std::fmt::Debug for OpensandboxConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpensandboxConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("execd_token_seed", &"<redacted>")
            .field("session_cpu", &self.session_cpu)
            .field("session_memory", &self.session_memory)
            .field("entrypoint", &self.entrypoint)
            .field("inspect_health", &self.inspect_health)
            .field("ignored_pod_knobs", &self.ignored_pod_knobs)
            .finish()
    }
}

/// Trim a raw env value; a blank string is treated as absent so a stray empty
/// ConfigMap value never masquerades as a real setting.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// File-first secret resolution: when `file_path` is set (non-blank) the file MUST
/// be readable and non-empty after trimming — any failure is a hard, fail-closed
/// config error naming the path (a typo'd path silently falling back to a stale env
/// var is worse than failing). The trim matters: CSI-mounted secrets commonly carry
/// a trailing newline that would otherwise corrupt the outgoing
/// `OPEN-SANDBOX-API-KEY` header. Only when `file_path` is absent does the inline
/// env value apply. Neither → an error naming BOTH vars.
fn resolve_secret_file_first(
    inline: Option<String>,
    file_path: Option<String>,
    inline_var: &str,
    file_var: &str,
) -> Result<SecretString, AppError> {
    if let Some(path) = file_path {
        let contents = std::fs::read_to_string(&path).map_err(|e| {
            AppError::Config(format!(
                "{file_var} points at {path} but the file could not be read: {e}"
            ))
        })?;
        let trimmed = contents.trim();
        if trimmed.is_empty() {
            return Err(AppError::Config(format!(
                "{file_var} points at {path} but the file is empty"
            )));
        }
        return Ok(SecretString::from(trimmed.to_string()));
    }
    match inline {
        Some(value) => Ok(SecretString::from(value)),
        None => Err(AppError::Config(format!(
            "{inline_var} or {file_var} must be set when FKST_POD_MODE=opensandbox"
        ))),
    }
}

/// Deserialize the OpenSandbox-backend configuration from environment-style pairs.
///
/// `required` is the caller's `pod.dispatch && pod_mode == Opensandbox` gate: when
/// `false` (dispatch off OR a different backend) the whole `FKST_OSB_*` block is
/// skipped and this returns `Ok(None)` — a k8s-customized deploy never validates an
/// OSB var. When `true`, every required var is validated fail-closed, naming the
/// offending variable, and this returns `Ok(Some(_))`.
///
/// Testable seam: unit tests feed explicit pairs instead of mutating the process
/// environment; shares the caller's already-collected `vars` snapshot (see
/// [`crate::config::Config::from_vars`]).
pub(crate) fn from_vars(
    vars: &[(String, String)],
    required: bool,
) -> Result<Option<OpensandboxConfig>, AppError> {
    // Not required (dispatch off OR mode != opensandbox): the OSB vars are never
    // validated, so a staged/half-set block cannot fail an unrelated deploy.
    if !required {
        return Ok(None);
    }

    let raw: OsbVars = envy::prefixed(OSB_ENV_PREFIX)
        .from_iter(vars.iter().cloned())
        .map_err(|e| AppError::Config(e.to_string()))?;

    // Base URL: present + non-blank + parseable. A missing OR malformed value gives
    // the SAME "must be a valid URL" message (both are the operator's error to fix).
    let base_url = match non_blank(raw.base_url).map(|s| reqwest::Url::parse(&s)) {
        Some(Ok(url)) => url,
        _ => {
            return Err(AppError::Config(
                "FKST_OSB_BASE_URL must be a valid URL when FKST_POD_MODE=opensandbox".to_string(),
            ))
        }
    };

    let api_key = resolve_secret_file_first(
        non_blank(raw.api_key),
        non_blank(raw.api_key_file),
        "FKST_OSB_API_KEY",
        "FKST_OSB_API_KEY_FILE",
    )?;

    // A blank seed is rejected exactly like a missing one (`non_blank` + the
    // helper's empty-after-trim check): an empty seed would derive a constant,
    // guessable execd token for every session.
    let execd_token_seed = resolve_secret_file_first(
        non_blank(raw.execd_token_seed),
        non_blank(raw.execd_token_seed_file),
        "FKST_OSB_EXECD_TOKEN_SEED",
        "FKST_OSB_EXECD_TOKEN_SEED_FILE",
    )?;

    let session_cpu = match non_blank(raw.session_cpu) {
        Some(value) => value,
        None => {
            return Err(AppError::Config(
                "FKST_OSB_SESSION_CPU must be set when FKST_POD_MODE=opensandbox".to_string(),
            ))
        }
    };

    let session_memory = match non_blank(raw.session_memory) {
        Some(value) => value,
        None => {
            return Err(AppError::Config(
                "FKST_OSB_SESSION_MEMORY must be set when FKST_POD_MODE=opensandbox".to_string(),
            ))
        }
    };

    // Entrypoint must be an absolute path (OpenSandbox has no image-default fallback).
    let entrypoint = match non_blank(raw.entrypoint) {
        Some(value) if value.starts_with('/') => value,
        _ => {
            return Err(AppError::Config(
                "FKST_OSB_ENTRYPOINT must be an absolute path when FKST_POD_MODE=opensandbox"
                    .to_string(),
            ))
        }
    };

    // The lifecycle proxy is the ONLY supported execd transport; reject an explicit
    // opt-out rather than silently ignoring it.
    if !raw.use_server_proxy.unwrap_or(true) {
        return Err(AppError::Config(
            "FKST_OSB_USE_SERVER_PROXY=false is not supported yet (execd is always reached \
             through the lifecycle proxy)"
                .to_string(),
        ));
    }

    let inspect_health = raw.inspect_health.unwrap_or(false);
    let ignored_pod_knobs = ignored_pod_knobs_in_opensandbox(vars);

    Ok(Some(OpensandboxConfig {
        base_url,
        api_key,
        execd_token_seed,
        session_cpu,
        session_memory,
        entrypoint,
        inspect_health,
        ignored_pod_knobs,
    }))
}

/// True when `url`'s host cannot be reached from a production sandbox: the
/// sandbox-lockdown NetworkPolicy allows egress ONLY to kube-dns + the public
/// internet, blocking RFC1918, loopback, link-local (incl. the GCP metadata IP
/// `169.254.169.254`), and — by virtue of blocking the cluster CIDRs — every
/// Kubernetes cluster-internal DNS name (`*.svc`, `*.svc.cluster.local`,
/// `*.cluster.local`). Static classification only: no DNS resolution, no probing.
fn is_sandbox_unreachable_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return true; // a hostless URL cannot work either
    };
    // reqwest/url brackets IPv6 hosts (`[::1]`); strip for the address parse.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<std::net::Ipv4Addr>() {
        return ip.is_private() || ip.is_loopback() || ip.is_link_local();
    }
    if let Ok(ip) = bare.parse::<std::net::Ipv6Addr>() {
        return ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local();
    }
    let domain = host.trim_end_matches('.').to_ascii_lowercase();
    domain == "localhost"
        || domain == "cluster.local"
        || domain.ends_with(".svc")
        || domain.ends_with(".svc.cluster.local")
        || domain.ends_with(".cluster.local")
}

/// Fail-closed egress-safety check for endpoints HANDED TO SESSIONS in opensandbox
/// mode (session env / creds files — the LLM base URL, the chrono-storage URLs). A
/// cluster-internal or private value passes config parsing but black-holes inside a
/// sandbox mid-run; failing at startup names the misconfiguration instead.
/// Deliberately NOT applied to `FKST_OSB_BASE_URL` itself — that is the
/// backend→server URL, dialed from the control plane and intentionally
/// cluster-internal.
pub(crate) fn ensure_sandbox_endpoints_reachable(
    endpoints: &[(&str, &str)],
) -> Result<(), AppError> {
    for (var, value) in endpoints {
        let parsed = reqwest::Url::parse(value).map_err(|e| {
            AppError::Config(format!(
                "{var} must be a valid URL in opensandbox mode (got {value:?}: {e})"
            ))
        })?;
        if is_sandbox_unreachable_host(&parsed) {
            return Err(AppError::Config(format!(
                "{var} points at a cluster-internal/private host ({value}) that a sandbox \
                 cannot reach: sandbox egress is locked to the PUBLIC internet (RFC1918, \
                 loopback, link-local/metadata, and *.svc / *.cluster.local are blocked). \
                 Use the endpoint's public URL in opensandbox mode."
            )));
        }
    }
    Ok(())
}

/// The `FKST_POD_*` knobs the operator EXPLICITLY set (non-blank raw value) that
/// OpenSandbox mode ignores because the sandbox template owns them. Presence in the
/// raw vars is the only reliable set-vs-defaulted signal (several of these knobs are
/// not `Option` on [`crate::config::PodConfig`], so a defaulted value is
/// indistinguishable from an explicit one once parsed). `FKST_POD_NAMESPACE` and
/// `FKST_POD_IMAGE` are deliberately NOT scanned: the namespace binds the env-store
/// KubeClient in both modes, and the image is the sandbox image too.
pub(crate) fn ignored_pod_knobs_in_opensandbox(vars: &[(String, String)]) -> Vec<&'static str> {
    IGNORED_POD_KNOBS
        .iter()
        .copied()
        .filter(|name| {
            vars.iter()
                .any(|(key, value)| key == name && !value.trim().is_empty())
        })
        .collect()
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

    /// A fully-valid `FKST_OSB_*` block; individual tests drop one var to assert its
    /// fail-closed message.
    fn full() -> Vec<(String, String)> {
        vars(&[
            ("FKST_OSB_BASE_URL", "https://sandbox.example/api"),
            ("FKST_OSB_API_KEY", "osb-key"),
            ("FKST_OSB_EXECD_TOKEN_SEED", "execd-seed"),
            ("FKST_OSB_SESSION_CPU", "500m"),
            ("FKST_OSB_SESSION_MEMORY", "512Mi"),
            ("FKST_OSB_ENTRYPOINT", "/usr/local/bin/fkst-control-plane"),
        ])
    }

    /// `full()` minus the one named var — for the required-var-missing assertions.
    fn full_without(skip: &str) -> Vec<(String, String)> {
        full().into_iter().filter(|(k, _)| k != skip).collect()
    }

    #[test]
    fn not_required_resolves_to_none() {
        // Dispatch off OR mode != opensandbox: the OSB block is skipped entirely, so
        // even a garbage base URL does not fail the load.
        assert!(
            from_vars(&vars(&[("FKST_OSB_BASE_URL", "not a url")]), false)
                .expect("skipped when not required")
                .is_none()
        );
    }

    #[test]
    fn fully_configured_resolves_to_some() {
        let config = from_vars(&full(), true)
            .expect("full config is valid")
            .expect("feature enabled");
        assert_eq!(config.base_url.as_str(), "https://sandbox.example/api");
        assert_eq!(config.api_key.expose_secret(), "osb-key");
        assert_eq!(config.execd_token_seed.expose_secret(), "execd-seed");
        assert_eq!(config.session_cpu, "500m");
        assert_eq!(config.session_memory, "512Mi");
        assert_eq!(config.entrypoint, "/usr/local/bin/fkst-control-plane");
        // The reserved toggle defaults false; no pod knobs were set to ignore.
        assert!(!config.inspect_health);
        assert!(config.ignored_pod_knobs.is_empty());
    }

    #[test]
    fn missing_base_url_fails_closed_naming_the_var() {
        let err = from_vars(&full_without("FKST_OSB_BASE_URL"), true)
            .expect_err("missing base url must fail closed");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err
            .to_string()
            .contains("FKST_OSB_BASE_URL must be a valid URL when FKST_POD_MODE=opensandbox"));
    }

    #[test]
    fn garbage_base_url_fails_closed() {
        let mut v = full_without("FKST_OSB_BASE_URL");
        v.push(("FKST_OSB_BASE_URL".to_string(), "not a url".to_string()));
        let err = from_vars(&v, true).expect_err("unparseable base url must fail closed");
        assert!(err.to_string().contains("FKST_OSB_BASE_URL"));
    }

    #[test]
    fn missing_api_key_fails_closed_naming_both_vars() {
        let err = from_vars(&full_without("FKST_OSB_API_KEY"), true)
            .expect_err("missing api key must fail closed");
        assert!(err.to_string().contains(
            "FKST_OSB_API_KEY or FKST_OSB_API_KEY_FILE must be set when FKST_POD_MODE=opensandbox"
        ));
    }

    #[test]
    fn missing_or_blank_execd_seed_fails_closed_naming_both_vars() {
        // Missing.
        let err = from_vars(&full_without("FKST_OSB_EXECD_TOKEN_SEED"), true)
            .expect_err("missing seed must fail closed");
        assert!(err.to_string().contains(
            "FKST_OSB_EXECD_TOKEN_SEED or FKST_OSB_EXECD_TOKEN_SEED_FILE must be set when \
             FKST_POD_MODE=opensandbox"
        ));
        // Blank is rejected exactly like missing (a blank seed derives a constant
        // token for every session).
        let mut v = full_without("FKST_OSB_EXECD_TOKEN_SEED");
        v.push(("FKST_OSB_EXECD_TOKEN_SEED".to_string(), "   ".to_string()));
        let err = from_vars(&v, true).expect_err("blank seed must fail closed");
        assert!(err.to_string().contains("FKST_OSB_EXECD_TOKEN_SEED"));
    }

    /// Write `contents` to a fresh temp file and return its path (kept alive by
    /// returning the guard alongside).
    fn secret_file(contents: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("secret");
        std::fs::write(&path, contents).expect("write secret file");
        (dir, path.to_string_lossy().into_owned())
    }

    #[test]
    fn api_key_file_wins_over_inline_env() {
        let (_guard, path) = secret_file("file-key\n");
        let mut v = full(); // full() already sets FKST_OSB_API_KEY=osb-key
        v.push(("FKST_OSB_API_KEY_FILE".to_string(), path));
        let config = from_vars(&v, true).expect("valid").expect("enabled");
        assert_eq!(config.api_key.expose_secret(), "file-key");
    }

    #[test]
    fn secret_file_content_is_trimmed() {
        // CSI-mounted secrets commonly carry a trailing newline; it must never
        // reach the outgoing header.
        let (_guard, path) = secret_file("  seed-value\n");
        let mut v = full_without("FKST_OSB_EXECD_TOKEN_SEED");
        v.push(("FKST_OSB_EXECD_TOKEN_SEED_FILE".to_string(), path));
        let config = from_vars(&v, true).expect("valid").expect("enabled");
        assert_eq!(config.execd_token_seed.expose_secret(), "seed-value");
    }

    #[test]
    fn unreadable_secret_file_fails_closed_without_env_fallback() {
        // The file var is set but points nowhere: fail closed naming the file var
        // and the path — do NOT silently fall back to the (also set) inline value.
        let mut v = full();
        v.push((
            "FKST_OSB_API_KEY_FILE".to_string(),
            "/nonexistent/fkst-osb-test/api-key".to_string(),
        ));
        let err = from_vars(&v, true).expect_err("unreadable file must fail closed");
        let msg = err.to_string();
        assert!(msg.contains("FKST_OSB_API_KEY_FILE"), "{msg}");
        assert!(msg.contains("/nonexistent/fkst-osb-test/api-key"), "{msg}");
        assert!(msg.contains("could not be read"), "{msg}");
    }

    #[test]
    fn empty_secret_file_fails_closed() {
        let (_guard, path) = secret_file("  \n");
        let mut v = full();
        v.push(("FKST_OSB_API_KEY_FILE".to_string(), path.clone()));
        let err = from_vars(&v, true).expect_err("empty file must fail closed");
        let msg = err.to_string();
        assert!(msg.contains("FKST_OSB_API_KEY_FILE"), "{msg}");
        assert!(msg.contains("is empty"), "{msg}");
    }

    #[test]
    fn seed_file_errors_name_the_seed_vars() {
        // Same helper, seed pair: the error names the SEED vars, not the key vars.
        let mut v = full();
        v.push((
            "FKST_OSB_EXECD_TOKEN_SEED_FILE".to_string(),
            "/nonexistent/fkst-osb-test/seed".to_string(),
        ));
        let err = from_vars(&v, true).expect_err("unreadable seed file must fail closed");
        assert!(err.to_string().contains("FKST_OSB_EXECD_TOKEN_SEED_FILE"));
    }

    #[test]
    fn inline_only_still_works() {
        // Regression guard: no *_FILE vars set → the inline env values resolve
        // exactly as before (covered broadly by fully_configured_resolves_to_some,
        // asserted here against the secrets specifically).
        let config = from_vars(&full(), true).expect("valid").expect("enabled");
        assert_eq!(config.api_key.expose_secret(), "osb-key");
        assert_eq!(config.execd_token_seed.expose_secret(), "execd-seed");
    }

    #[test]
    fn missing_session_cpu_or_memory_fails_closed_naming_the_var() {
        let err = from_vars(&full_without("FKST_OSB_SESSION_CPU"), true)
            .expect_err("missing cpu must fail closed");
        assert!(err
            .to_string()
            .contains("FKST_OSB_SESSION_CPU must be set when FKST_POD_MODE=opensandbox"));
        let err = from_vars(&full_without("FKST_OSB_SESSION_MEMORY"), true)
            .expect_err("missing memory must fail closed");
        assert!(err
            .to_string()
            .contains("FKST_OSB_SESSION_MEMORY must be set when FKST_POD_MODE=opensandbox"));
    }

    #[test]
    fn non_absolute_entrypoint_fails_closed() {
        let mut v = full_without("FKST_OSB_ENTRYPOINT");
        v.push((
            "FKST_OSB_ENTRYPOINT".to_string(),
            "fkst-control-plane".to_string(),
        ));
        let err = from_vars(&v, true).expect_err("relative entrypoint must fail closed");
        assert!(err.to_string().contains(
            "FKST_OSB_ENTRYPOINT must be an absolute path when FKST_POD_MODE=opensandbox"
        ));
    }

    #[test]
    fn use_server_proxy_false_is_rejected() {
        let mut v = full();
        v.push(("FKST_OSB_USE_SERVER_PROXY".to_string(), "false".to_string()));
        let err = from_vars(&v, true).expect_err("proxy opt-out must fail closed");
        assert!(err
            .to_string()
            .contains("FKST_OSB_USE_SERVER_PROXY=false is not supported yet"));
    }

    #[test]
    fn inspect_health_is_stored_when_set() {
        let mut v = full();
        v.push(("FKST_OSB_INSPECT_HEALTH".to_string(), "true".to_string()));
        let config = from_vars(&v, true).expect("valid").expect("enabled");
        assert!(config.inspect_health);
    }

    #[test]
    fn ignored_pod_knobs_lists_set_knobs_but_excludes_namespace_and_image() {
        let mut v = full();
        v.push(("FKST_POD_SERVICE_ACCOUNT".to_string(), "sa".to_string()));
        v.push(("FKST_POD_RUNTIME_CLASS".to_string(), "kata".to_string()));
        // NAMESPACE + IMAGE are meaningful in opensandbox mode too — never ignored.
        v.push(("FKST_POD_NAMESPACE".to_string(), "sessions".to_string()));
        v.push(("FKST_POD_IMAGE".to_string(), "img".to_string()));
        // A blank knob is treated as unset (not ignored-and-warned).
        v.push(("FKST_POD_DNS_NAMESERVERS".to_string(), "   ".to_string()));
        let config = from_vars(&v, true).expect("valid").expect("enabled");
        assert!(config
            .ignored_pod_knobs
            .contains(&"FKST_POD_SERVICE_ACCOUNT"));
        assert!(config.ignored_pod_knobs.contains(&"FKST_POD_RUNTIME_CLASS"));
        assert!(!config.ignored_pod_knobs.contains(&"FKST_POD_NAMESPACE"));
        assert!(!config.ignored_pod_knobs.contains(&"FKST_POD_IMAGE"));
        // The blank DNS value is not counted as set.
        assert!(!config
            .ignored_pod_knobs
            .contains(&"FKST_POD_DNS_NAMESERVERS"));
    }

    #[test]
    fn sandbox_endpoint_classifier_table() {
        // Unreachable from a sandbox: RFC1918 / loopback / link-local (incl. the
        // GCP metadata IP) / cluster-internal DNS / localhost.
        for value in [
            "http://10.1.2.3/v1",
            "http://172.16.0.1:8080/v1",
            "http://192.168.1.1/v1",
            "http://127.0.0.1:9000/v1",
            "http://169.254.169.254/computeMetadata",
            "http://[::1]:9000/v1",
            "http://[fd00::1]/v1",
            "http://localhost:8080/v1",
            "http://foo.chronoai-fkst.svc/v1",
            "http://foo.ns.svc.cluster.local/v1",
            "http://anything.cluster.local/v1",
        ] {
            let err = ensure_sandbox_endpoints_reachable(&[("FKST_LLM_BASE_URL", value)])
                .expect_err(&format!("{value} must be classified unreachable"));
            assert!(err.to_string().contains("FKST_LLM_BASE_URL"), "{value}");
        }
        // Reachable: real public endpoints.
        for value in [
            "https://api.github.com",
            "https://nyx-api.chrono-ai.fun/oauth/token",
            "https://storage.example/proxy",
            "https://llm.aelf.dev/v1",
            "http://8.8.8.8/x",
        ] {
            ensure_sandbox_endpoints_reachable(&[("FKST_LLM_BASE_URL", value)])
                .unwrap_or_else(|e| panic!("{value} must be classified reachable: {e}"));
        }
        // Unparseable is an error too (it cannot work anyway), naming the var.
        let err = ensure_sandbox_endpoints_reachable(&[("FKST_STORAGE_BASE_URL", "not a url")])
            .expect_err("garbage must fail");
        assert!(err.to_string().contains("FKST_STORAGE_BASE_URL"));
        assert!(err.to_string().contains("valid URL"));
    }

    #[test]
    fn debug_never_leaks_the_secrets() {
        let config = from_vars(&full(), true).expect("valid").expect("enabled");
        let debug = format!("{config:?}");
        assert!(
            !debug.contains("osb-key"),
            "Debug leaked the api key: {debug}"
        );
        assert!(
            !debug.contains("execd-seed"),
            "Debug leaked the execd seed: {debug}"
        );
        assert!(debug.contains("<redacted>"), "{debug}");
        // Non-secret fields stay visible for diagnostics.
        assert!(debug.contains("512Mi"), "{debug}");
    }
}
