//! Shared scaffolding for the OpenSandbox backend tests: an [`OsbBackend`] pointed at
//! a wiremock base (both the lifecycle client AND the execd factory target it), plus
//! the sample spec / config builders. Kept in one place so the five `*_tests.rs`
//! siblings stay small and consistent.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;
use serde_json::{json, Value};

use crate::config::PodConfig;
use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::session_backend::opensandbox::dto::{ImageSpec, ResourceLimits};
use crate::session_backend::opensandbox::{ExecdClient, OsbLifecycleClient};

use super::{ExecdFactory, OsbBackend, OsbConfig, DEFAULT_EXECD_TOKEN_ENV_KEY};

/// The API key every mock asserts is present on lifecycle + execd requests.
pub(crate) const API_KEY: &str = "osb_test_key";
/// The long-lived execd-token seed the backend derives per-session create-env tokens
/// from; tests recompute the expected token from it.
pub(crate) const EXECD_SEED: &str = "execd-seed-under-test";

/// A UUID-shaped (label-valid) session id.
pub(crate) const SESSION_ID: &str = "11111111-2222-3333-4444-555555555555";

/// A canonical 64-hex config hash whose halves are trivially checkable (32 `a`s each).
pub(crate) fn config_hash() -> String {
    "a".repeat(64)
}

/// The sample launch spec: `acme/site`, issue 7, work label `fkst-work`.
pub(crate) fn spec() -> SessionPodSpec {
    SessionPodSpec {
        session_id: SESSION_ID.to_string(),
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue_number: 7,
        package_roots: vec!["web".to_string()],
        work_label: "fkst-work".to_string(),
        bot_login: "fkst-bot[bot]".to_string(),
        config_hash: config_hash(),
    }
}

/// The static launch config, with a generous shield window.
pub(crate) fn osb_config() -> OsbConfig {
    OsbConfig {
        image: ImageSpec {
            uri: "registry/fkst-control-plane:1".to_string(),
            auth: None,
        },
        entrypoint: vec!["run-substrate".to_string()],
        resource_limits: ResourceLimits(BTreeMap::from([("cpu".to_string(), "500m".to_string())])),
        execd_seed: SecretString::from(EXECD_SEED.to_string()),
        execd_token_env_key: DEFAULT_EXECD_TOKEN_ENV_KEY.to_string(),
        reconcile_window: Duration::from_secs(300),
    }
}

/// Build a backend whose lifecycle client AND execd factory both target `base`. The
/// execd token is a fixed placeholder (the mock does not validate it).
pub(crate) fn backend(base: &str, config: OsbConfig) -> OsbBackend {
    let url = reqwest::Url::parse(base).expect("base url");
    let http = reqwest::Client::new();
    let lifecycle = OsbLifecycleClient::new(
        url.clone(),
        SecretString::from(API_KEY.to_string()),
        http.clone(),
    );
    let factory_url = url;
    let factory_http = http;
    let factory: ExecdFactory = Arc::new(move |sandbox_id: &str, _session_id: &str| {
        ExecdClient::new(
            factory_url.clone(),
            SecretString::from(API_KEY.to_string()),
            sandbox_id.to_string(),
            SecretString::from("execd-token".to_string()),
            factory_http.clone(),
        )
    });
    OsbBackend::new(lifecycle, factory, PodConfig::default(), config)
}

/// A one-page list envelope wrapping `items` (no further pages).
pub(crate) fn list_page(items: Value) -> Value {
    json!({
        "items": items,
        "pagination": {
            "page": 1, "pageSize": 100, "totalItems": 0, "totalPages": 1, "hasNextPage": false
        }
    })
}

/// A sandbox response body with the given id, state, createdAt, and metadata map.
pub(crate) fn sandbox_json(id: &str, state: &str, created_at: &str, metadata: Value) -> Value {
    json!({
        "id": id,
        "status": { "state": state },
        "metadata": metadata,
        "createdAt": created_at,
    })
}

/// The full correlation metadata a stamped sandbox carries, for the given session +
/// owner/repo/work-label, using [`config_hash`]'s split halves.
pub(crate) fn correlation_metadata(
    session_id: &str,
    owner: &str,
    repo: &str,
    work_label_hex: &str,
) -> Value {
    json!({
        "fkst-managed": "true",
        "fkst-session-id": session_id,
        "fkst-installation-id": "42",
        "fkst-trigger-issue": "7",
        "fkst-last-pending-at": "1000",
        "fkst-owner": owner,
        "fkst-repo": repo,
        "fkst-config-hash": "a".repeat(32),
        "fkst-config-hash-2": "a".repeat(32),
        "fkst-work-label": work_label_hex,
    })
}
