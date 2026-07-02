//! The per-pod INSTANCE identity + the branch-root documents.
//!
//! A session's log branch (`fkst-logs/issue-<N>`) is shared across every pod that
//! ever serves the session; each pod lifetime is ONE instance, written under its
//! own `instances/<INSTANCE>/` dir so a revived session only ever ADDS a dir and
//! never rewrites an earlier one. This module is pure: it computes the instance id
//! from the pod's identity + a clock, and renders the branch `README.md` and the
//! per-instance `meta.json` — all as unit-testable string builders.

use k8s_openapi::chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Compute the instance id `<UTC-rfc3339-basic>Z-<pod-uid8>`: a compact UTC stamp
/// (no separators) taken at pod start, joined to a short pod identifier. The
/// identifier is the first 8 chars of the downward-API pod UID when present, else
/// the first 8 hex of a SHA-256 over the pod name (stable per pod). One instance ==
/// one pod lifetime.
pub fn compute_instance_id(now: DateTime<Utc>, pod_uid: &str, pod_name: &str) -> String {
    let stamp = now.format("%Y%m%dT%H%M%S");
    format!("{stamp}Z-{}", pod_short_id(pod_uid, pod_name))
}

/// The 8-char pod short id: the UID prefix when a non-empty UID is present,
/// otherwise a SHA-256 prefix over the pod name (so it is still deterministic and
/// collision-resistant across pods). Never empty — a blank name still hashes.
fn pod_short_id(pod_uid: &str, pod_name: &str) -> String {
    let uid = pod_uid.trim();
    if !uid.is_empty() {
        return uid.chars().take(8).collect();
    }
    let digest = Sha256::digest(pod_name.as_bytes());
    hex8(&digest)
}

/// First 8 lowercase-hex chars of a byte digest.
fn hex8(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(8);
    for byte in bytes.iter().take(4) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The non-secret facts recorded once per instance in `meta.json`. None of these
/// are credentials, so the file is safe to push verbatim.
#[derive(Debug, Clone, Serialize)]
pub struct InstanceMeta {
    /// The instance id (dir name).
    pub instance: String,
    /// Pod start time (RFC3339, UTC).
    pub start_time: String,
    /// The downward-API pod UID (empty when unavailable).
    pub pod_uid: String,
    /// The workspace/engine git ref the pod cloned.
    pub engine_ref: String,
    /// The registration config-hash the pod launched with.
    pub config_hash: String,
    /// The trigger issue number.
    pub trigger_issue: i64,
    /// The `owner/name` repository the session works.
    pub repo: String,
}

impl InstanceMeta {
    /// Build the meta for an instance from the pod-injected facts + the start clock.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        instance: String,
        start: DateTime<Utc>,
        pod_uid: String,
        engine_ref: String,
        config_hash: String,
        trigger_issue: i64,
        repo: String,
    ) -> Self {
        Self {
            instance,
            start_time: start.to_rfc3339_opts(SecondsFormat::Secs, true),
            pod_uid,
            engine_ref,
            config_hash,
            trigger_issue,
            repo,
        }
    }

    /// Render the `meta.json` document (pretty, trailing newline). Serialization of a
    /// plain non-secret struct cannot fail; a defensive fallback keeps this infallible
    /// for the best-effort collector.
    pub fn to_json(&self) -> String {
        match serde_json::to_string_pretty(self) {
            Ok(mut json) => {
                json.push('\n');
                json
            }
            Err(_) => "{}\n".to_string(),
        }
    }
}

/// Render the branch `README.md`: the session's trigger link + a loud "generated,
/// redacted" notice so a human who stumbles onto the branch understands what it is.
pub fn readme_markdown(repo: &str, trigger_issue: i64, branch: &str) -> String {
    let issue_url = format!("https://github.com/{repo}/issues/{trigger_issue}");
    format!(
        "# Session logs — `{repo}` issue #{trigger_issue}\n\
         \n\
         Branch `{branch}` holds the **redacted** logs for the substrate session \
         triggered by [issue #{trigger_issue}]({issue_url}).\n\
         \n\
         > These files are **auto-generated** and **redacted**: every credential-shaped \
         run is masked before it is written. Do not treat any value here as sensitive, \
         and do not edit this branch by hand — each pod lifetime appends a new \
         `instances/<id>/` dir and never rewrites an earlier one.\n"
    )
}

#[cfg(test)]
#[path = "instance_tests.rs"]
mod tests;
