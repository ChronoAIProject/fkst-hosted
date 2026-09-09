//! Observes and records the staging PostHog's server version and API behaviour.
//!
//! The issue requires the evidence artifact to carry "PostHog server
//! version/API behavior ... without credentials/host-private details", and the
//! reason is concrete: this deployment pins no PostHog version, so a round trip
//! that passed says nothing unless the artifact records WHICH server it passed
//! against. A future regression report needs to be able to answer "was that on
//! the same server behaviour we validated?".
//!
//! Everything recorded here is a shape or a status:
//!
//! - the version string PostHog itself advertises, when it advertises one;
//! - which probe endpoints answered, by HTTP status only;
//! - the query API's response envelope (its top-level keys, and whether the rows
//!   come back positionally), which is the contract the product's row decoder
//!   depends on;
//! - whether parameterized HogQL placeholders were honoured;
//! - the observed capture-to-visibility lag.
//!
//! The host, the project id, and every credential are deliberately absent: the
//! artifact is attached to a milestone record that leaves the deployment.

use std::time::Duration;

use serde_json::{json, Value};

use super::gate::GateEnvironment;

/// What one staging run observed about the server it ran against.
#[derive(Debug, Default)]
pub struct PostHogProfile {
    /// The version PostHog advertises, or `None` when it advertises none to an
    /// unprivileged caller (the common self-hosted case).
    pub version: Option<String>,
    /// `endpoint -> HTTP status`, for the probes that were attempted.
    pub probes: Vec<(&'static str, u16)>,
    /// The query response envelope's top-level keys, sorted.
    pub query_envelope: Vec<String>,
    /// Whether a row came back as a positional array (PostHog's HogQL shape)
    /// rather than an object.
    pub rows_are_positional: Option<bool>,
    /// Whether `{placeholder}` + `values` was honoured rather than interpolated.
    pub honours_placeholders: Option<bool>,
    /// Seconds between capture acceptance and first query visibility.
    pub visibility_lag_secs: Option<u64>,
}

impl PostHogProfile {
    /// The artifact document. Contains no host, project id, or credential.
    pub fn to_json(&self) -> Value {
        json!({
            "kind": "fkst-acceptance-posthog-staging",
            "note": "Server behaviour observed by backend/tests/acceptance_staging.rs. \
                     No host, project id, credential, or event payload is recorded.",
            "server_version": self.version.clone().unwrap_or_else(|| {
                "not advertised to an unprivileged caller".to_string()
            }),
            "probes": self
                .probes
                .iter()
                .map(|(endpoint, status)| json!({ "endpoint": endpoint, "status": status }))
                .collect::<Vec<_>>(),
            "query_response_keys": self.query_envelope,
            "rows_are_positional": self.rows_are_positional,
            "honours_parameter_placeholders": self.honours_placeholders,
            "capture_to_visibility_lag_secs": self.visibility_lag_secs,
        })
    }
}

/// A version-looking value anywhere in a JSON document.
///
/// PostHog has moved this key between releases (`ph_version`,
/// `posthog_version`, `version`), so the probe looks for the family rather than
/// pinning one name — pinning would make the probe report "unknown" on the next
/// upgrade, which is the moment the record matters most.
fn find_version(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                if key.to_ascii_lowercase().contains("version") {
                    if let Some(text) = nested.as_str() {
                        return Some(text.to_string());
                    }
                }
                if let Some(found) = find_version(nested) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_version),
        _ => None,
    }
}

/// Probe the endpoints that can advertise a version to an unprivileged caller.
///
/// Every probe is best-effort: a self-hosted PostHog may lock all of them down,
/// and a locked-down instance is a fact to record, not a test failure.
pub async fn observe_server(environment: &GateEnvironment, profile: &mut PostHogProfile) {
    let host = super::host(environment);
    let key = environment.get("FKST_ACCEPTANCE_POSTHOG_QUERY_KEY");
    let client = reqwest::Client::new();
    for endpoint in ["/_health", "/api/instance_status", "/api/instance_settings"] {
        let response = client
            .get(format!("{host}{endpoint}"))
            .bearer_auth(key)
            .timeout(Duration::from_secs(15))
            .send()
            .await;
        let Ok(response) = response else {
            // A refused connection on an OPTIONAL probe is recorded as such
            // rather than failing the tier; the round trip itself is what has to
            // work, and it is asserted elsewhere.
            profile.probes.push((endpoint, 0));
            continue;
        };
        let status = response.status().as_u16();
        profile.probes.push((endpoint, status));
        if profile.version.is_none() && response.status().is_success() {
            if let Ok(body) = response.json::<Value>().await {
                profile.version = find_version(&body);
            }
        }
    }
}

/// Record the query API's envelope from one real response body.
pub fn observe_query_envelope(body: &Value, profile: &mut PostHogProfile) {
    if let Some(map) = body.as_object() {
        profile.query_envelope = map.keys().cloned().collect();
        profile.query_envelope.sort();
    }
    profile.rows_are_positional = body["results"]
        .as_array()
        .and_then(|rows| rows.first())
        .map(|row| row.is_array());
}
