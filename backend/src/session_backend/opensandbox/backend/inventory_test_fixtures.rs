//! Shared fixtures for the OpenSandbox inventory suites: the rendering policy,
//! the fully stamped sandbox list item, and the single-page mock. Kept separate
//! so the projection suite and the bounds/secrecy suite each stay small.

use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::runtime_identity::{stamp_pairs, RuntimeIdentityMetadata, OSB_IDENTITY_KEYS};
use crate::session_backend::inventory::RuntimeLifetimePolicy;

use super::super::backend_test_support::{
    correlation_metadata, list_page, sandbox_json, SESSION_ID,
};

pub(super) const WORK_LABEL_HEX: &str = "666b73742d776f726b";

/// Unlimited maximum lifetime, the shipped shield/grace/ceiling defaults.
pub(super) fn policy() -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: 0,
        minimum_lifetime_seconds: 120,
        idle_grace_seconds: 300,
        max_items: 5000,
        max_warnings: 256,
    }
}

/// The `metadata` filter value the fleet-wide inventory walk sends.
pub(super) fn managed_filter() -> String {
    "fkst-managed=true".to_string()
}

/// Correlation metadata plus a complete launch attribution stamp.
pub(super) fn stamped_metadata(session: &str) -> Value {
    let mut metadata = correlation_metadata(session, "acme", "site", WORK_LABEL_HEX);
    let identity = RuntimeIdentityMetadata::new(Some(11), "alice", 22, "carol");
    let object = metadata.as_object_mut().expect("object");
    for (key, value) in stamp_pairs(&OSB_IDENTITY_KEYS, &identity) {
        object.insert(key.to_string(), Value::String(value));
    }
    metadata
}

pub(super) fn sandbox(id: &str, state: &str) -> Value {
    sandbox_json(
        id,
        state,
        "2026-07-01T09:00:00Z",
        stamped_metadata(SESSION_ID),
    )
}

/// Mount a single-page managed list returning `items`, asserting the walk asks for
/// exactly one page.
pub(super) async fn mount_single_page(server: &MockServer, items: Value) {
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("metadata", managed_filter()))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(items)))
        .expect(1)
        .mount(server)
        .await;
}

/// A managed sandbox carrying nothing but the managed marker — an orphan whose
/// missing session id is exactly the drift the inventory must keep visible.
pub(super) fn orphan(id: &str) -> Value {
    json!({
        "id": id,
        "status": { "state": "Running" },
        "metadata": { "fkst-managed": "true" },
        "createdAt": "2026-07-01T09:00:00Z",
    })
}
