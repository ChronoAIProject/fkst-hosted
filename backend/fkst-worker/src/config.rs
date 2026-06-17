//! Worker configuration, loaded fail-closed from the environment.
//!
//! A worker that cannot reach its controller is useless, so `controller_url`
//! and the internal-auth secret are REQUIRED and a missing/blank value is a
//! hard error that stops the process from starting. Mirrors the control-plane's
//! fail-closed config style.

use std::collections::HashMap;

use secrecy::SecretString;

/// Default worker-local HTTP port for the liveness server.
const DEFAULT_PORT: u16 = 8090;
/// Default work-pull cadence (seconds).
const DEFAULT_PULL_INTERVAL_SECS: u64 = 5;
/// Default max concurrent engine sessions the worker accepts.
const DEFAULT_CAPACITY: u32 = 4;
/// Default graceful-drain deadline (seconds) for SIGTERM / preStop. MUST stay
/// strictly below the k8s `terminationGracePeriodSeconds` (owned by #144) so the
/// kubelet's own SIGKILL never races the drain mid-checkpoint; 25 leaves a
/// margin under the common 30s default.
const DEFAULT_DRAIN_GRACE_SECS: u64 = 25;

/// Validated worker configuration.
#[derive(Clone)]
pub struct WorkerConfig {
    /// Stable controller Service DNS, e.g. `http://fkst-hosted:80`. Required.
    pub controller_url: String,
    /// Shared secret on every internal request. Required. Redacted in Debug.
    pub internal_auth_token: SecretString,
    /// This worker's unique id. Resolved from `FKST_POD_ID` (the k8s downward-API
    /// pod name) when set, else the container `HOSTNAME` (also the pod name).
    /// Always non-empty.
    pub worker_id: String,
    /// Bind address for the worker-local HTTP server. Default `0.0.0.0`.
    pub bind_addr: String,
    /// Port for the worker-local HTTP server. Default 8090.
    pub port: u16,
    /// Work-pull cadence (seconds). Default 5; must be > 0.
    pub pull_interval_secs: u64,
    /// Max concurrent engine sessions accepted. Default 4 (0 = derive later).
    pub capacity: u32,
    /// The worker's engine temp dir, reported on registration.
    pub engine_temp_root: String,
    /// Graceful-drain deadline (seconds) for SIGTERM / preStop (#140a). Bounds
    /// how long the worker spends checkpointing + stopping in-flight sessions
    /// before it gives up and exits. Default 25; MUST stay below the k8s
    /// `terminationGracePeriodSeconds` (owned by #144) so the kubelet never
    /// SIGKILLs mid-drain.
    pub worker_drain_grace_secs: u64,
}

// Hand-written so the internal-auth secret is never printed.
impl std::fmt::Debug for WorkerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerConfig")
            .field("controller_url", &self.controller_url)
            .field("internal_auth_token", &"<redacted>")
            .field("worker_id", &self.worker_id)
            .field("bind_addr", &self.bind_addr)
            .field("port", &self.port)
            .field("pull_interval_secs", &self.pull_interval_secs)
            .field("capacity", &self.capacity)
            .field("engine_temp_root", &self.engine_temp_root)
            .field("worker_drain_grace_secs", &self.worker_drain_grace_secs)
            .finish()
    }
}

impl WorkerConfig {
    /// Load from the process environment (fail-closed).
    pub fn load_from_env() -> anyhow::Result<Self> {
        Self::from_vars(std::env::vars())
    }

    /// Build from environment-style pairs (testable).
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> anyhow::Result<Self> {
        let map: HashMap<String, String> = vars.into_iter().collect();

        let required = |key: &str| -> anyhow::Result<String> {
            non_empty(&map, key)
                .ok_or_else(|| anyhow::anyhow!("{key} must be set (non-empty) for the worker role"))
        };

        let controller_url = required("CONTROLLER_URL")?;
        let internal_auth_token = SecretString::from(required("FKST_INTERNAL_AUTH_TOKEN")?);

        // Worker identity. Prefer the explicit `FKST_POD_ID` (in Kubernetes the
        // downward-API pod name — exact even when the name exceeds the 63-char
        // hostname limit). When it is absent — e.g. a deployment that can only
        // edit the ConfigMap/Secret and cannot wire the downward API into the Pod
        // spec — fall back to the container `HOSTNAME`, which every mainstream
        // runtime sets to the pod name, so each replica still gets a unique,
        // stable id. Fail closed only when NEITHER is available (a static shared
        // id would collide across replicas in the controller's registry).
        let worker_id = match non_empty(&map, "FKST_POD_ID") {
            Some(id) => id,
            None => match non_empty(&map, "HOSTNAME") {
                Some(host) => {
                    tracing::info!(
                        worker_id = %host,
                        "FKST_POD_ID not set; derived worker id from the container HOSTNAME"
                    );
                    host
                }
                None => anyhow::bail!(
                    "worker id unresolved: set FKST_POD_ID (non-empty) for the worker role — \
                     in Kubernetes inject it from the downward API (env valueFrom fieldRef \
                     metadata.name); the container HOSTNAME is used as a fallback when present"
                ),
            },
        };

        let bind_addr = map
            .get("FKST_WORKER_BIND_ADDR")
            .filter(|v| !v.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "0.0.0.0".to_string());

        let port = parse_or("FKST_WORKER_PORT", &map, DEFAULT_PORT)?;

        let pull_interval_secs = parse_or(
            "FKST_WORKER_PULL_INTERVAL_SECS",
            &map,
            DEFAULT_PULL_INTERVAL_SECS,
        )?;
        if pull_interval_secs == 0 {
            anyhow::bail!("FKST_WORKER_PULL_INTERVAL_SECS must be at least 1");
        }

        let capacity = parse_or("FKST_WORKER_CAPACITY", &map, DEFAULT_CAPACITY)?;

        let worker_drain_grace_secs = parse_or(
            "FKST_WORKER_DRAIN_GRACE_SECS",
            &map,
            DEFAULT_DRAIN_GRACE_SECS,
        )?;

        // The engine temp root the image sets (FKST_RUNTIME_ROOT); fall back to
        // the OS temp dir so a local `cargo run` still produces a valid value.
        let engine_temp_root = map
            .get("FKST_RUNTIME_ROOT")
            .filter(|v| !v.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());

        Ok(Self {
            controller_url,
            internal_auth_token,
            worker_id,
            bind_addr,
            port,
            pull_interval_secs,
            capacity,
            engine_temp_root,
            worker_drain_grace_secs,
        })
    }
}

/// Return the value of `key` from `map` when present and not blank (the
/// original, untrimmed string), or `None` when absent or whitespace-only.
fn non_empty(map: &HashMap<String, String>, key: &str) -> Option<String> {
    map.get(key).filter(|v| !v.trim().is_empty()).cloned()
}

/// Parse a numeric env var or fall back to `default`; a malformed value is a
/// fail-closed error naming the variable.
fn parse_or<T>(key: &str, map: &HashMap<String, String>, default: T) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match map.get(key).filter(|v| !v.trim().is_empty()) {
        None => Ok(default),
        Some(v) => v
            .parse::<T>()
            .map_err(|e| anyhow::anyhow!("{key} is not a valid value: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn base() -> Vec<(String, String)> {
        vec![
            ("CONTROLLER_URL".into(), "http://controller:80".into()),
            ("FKST_INTERNAL_AUTH_TOKEN".into(), "secret".into()),
            ("FKST_POD_ID".into(), "worker-0".into()),
        ]
    }

    #[test]
    fn loads_with_defaults() {
        let c = WorkerConfig::from_vars(base()).unwrap();
        assert_eq!(c.controller_url, "http://controller:80");
        assert_eq!(c.worker_id, "worker-0");
        assert_eq!(c.bind_addr, "0.0.0.0");
        assert_eq!(c.port, DEFAULT_PORT);
        assert_eq!(c.pull_interval_secs, DEFAULT_PULL_INTERVAL_SECS);
        assert_eq!(c.capacity, DEFAULT_CAPACITY);
        assert_eq!(c.worker_drain_grace_secs, DEFAULT_DRAIN_GRACE_SECS);
        assert_eq!(c.internal_auth_token.expose_secret(), "secret");
    }

    #[test]
    fn drain_grace_defaults_to_25_and_is_overridable() {
        // Default when the var is absent.
        let c = WorkerConfig::from_vars(base()).unwrap();
        assert_eq!(c.worker_drain_grace_secs, 25);

        // Overridable via the env var.
        let mut vars = base();
        vars.push(("FKST_WORKER_DRAIN_GRACE_SECS".into(), "40".into()));
        let c = WorkerConfig::from_vars(vars).unwrap();
        assert_eq!(c.worker_drain_grace_secs, 40);
    }

    #[test]
    fn malformed_drain_grace_is_rejected() {
        let mut vars = base();
        vars.push(("FKST_WORKER_DRAIN_GRACE_SECS".into(), "soon".into()));
        let err = WorkerConfig::from_vars(vars).expect_err("malformed must fail");
        assert!(err.to_string().contains("FKST_WORKER_DRAIN_GRACE_SECS"));
    }

    #[test]
    fn missing_required_vars_fail_closed_naming_the_var() {
        for missing in ["CONTROLLER_URL", "FKST_INTERNAL_AUTH_TOKEN", "FKST_POD_ID"] {
            let vars: Vec<(String, String)> =
                base().into_iter().filter(|(k, _)| k != missing).collect();
            let err = WorkerConfig::from_vars(vars).expect_err("must fail");
            assert!(
                err.to_string().contains(missing),
                "error must name {missing}"
            );
        }
    }

    #[test]
    fn worker_id_falls_back_to_hostname_when_pod_id_absent() {
        // A deployment that cannot wire the downward API still gets a unique id
        // from the runtime-provided HOSTNAME (= the pod name).
        let vars: Vec<(String, String)> = base()
            .into_iter()
            .filter(|(k, _)| k != "FKST_POD_ID")
            .chain([("HOSTNAME".to_string(), "fkst-worker-7d9-abcde".to_string())])
            .collect();
        let c = WorkerConfig::from_vars(vars).unwrap();
        assert_eq!(c.worker_id, "fkst-worker-7d9-abcde");
    }

    #[test]
    fn explicit_pod_id_wins_over_hostname() {
        // base() sets FKST_POD_ID = worker-0; an also-present HOSTNAME must lose.
        let mut vars = base();
        vars.push(("HOSTNAME".into(), "fkst-worker-7d9-abcde".into()));
        let c = WorkerConfig::from_vars(vars).unwrap();
        assert_eq!(c.worker_id, "worker-0", "explicit FKST_POD_ID must win");
    }

    #[test]
    fn blank_pod_id_falls_back_to_hostname() {
        let vars: Vec<(String, String)> = base()
            .into_iter()
            .map(|(k, v)| {
                if k == "FKST_POD_ID" {
                    (k, "   ".into())
                } else {
                    (k, v)
                }
            })
            .chain([("HOSTNAME".to_string(), "pod-xyz".to_string())])
            .collect();
        let c = WorkerConfig::from_vars(vars).unwrap();
        assert_eq!(c.worker_id, "pod-xyz");
    }

    #[test]
    fn missing_both_pod_id_and_hostname_fails_closed() {
        // Neither FKST_POD_ID nor HOSTNAME present (base() carries no HOSTNAME).
        let vars: Vec<(String, String)> = base()
            .into_iter()
            .filter(|(k, _)| k != "FKST_POD_ID")
            .collect();
        let err = WorkerConfig::from_vars(vars).expect_err("must fail closed");
        let msg = err.to_string();
        assert!(
            msg.contains("FKST_POD_ID"),
            "error must name FKST_POD_ID: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("hostname"),
            "error should mention the HOSTNAME fallback: {msg}"
        );
    }

    #[test]
    fn blank_required_var_is_rejected() {
        let blanked: Vec<(String, String)> = base()
            .into_iter()
            .map(|(k, v)| {
                if k == "FKST_POD_ID" {
                    (k, "   ".into())
                } else {
                    (k, v)
                }
            })
            .collect();
        let err = WorkerConfig::from_vars(blanked).expect_err("blank must fail");
        assert!(err.to_string().contains("FKST_POD_ID"));
    }

    #[test]
    fn zero_pull_interval_is_rejected() {
        let mut vars = base();
        vars.push(("FKST_WORKER_PULL_INTERVAL_SECS".into(), "0".into()));
        let err = WorkerConfig::from_vars(vars).expect_err("zero must fail");
        assert!(err.to_string().contains("FKST_WORKER_PULL_INTERVAL_SECS"));
    }

    #[test]
    fn malformed_number_is_rejected() {
        let mut vars = base();
        vars.push(("FKST_WORKER_CAPACITY".into(), "lots".into()));
        let err = WorkerConfig::from_vars(vars).expect_err("malformed must fail");
        assert!(err.to_string().contains("FKST_WORKER_CAPACITY"));
    }
}
