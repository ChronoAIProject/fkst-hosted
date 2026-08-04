//! The one-pass OpenSandbox live inventory verb (issue #5674).
//!
//! ONE logical list operation: the lifecycle client's paginated
//! `GET /v1/sandboxes` walk filtered to `fkst-managed=true`. Walking the API's
//! pages is part of that one operation (the server chose the page size, not us);
//! a `GET /v1/sandboxes/{id}` per sandbox is NOT, and nothing here issues one.
//!
//! Everything the inventory needs is already in a list item — id, full raw state,
//! reason/message, `createdAt`, `lastTransitionAt`, and the complete correlation +
//! attribution metadata — which is precisely why the per-sandbox GET is
//! unnecessary rather than merely discouraged.
//!
//! Two asymmetries with Kubernetes are reported honestly rather than papered over:
//! the API exposes no restart count (so `restart_count` is `None`, never a
//! zero-as-guess), and a delete 404s immediately with no drain window (so
//! `deletion_timestamp` is always `None` — `Stopping` is the state that carries
//! that meaning here).

use k8s_openapi::chrono::{DateTime, Utc};

use crate::runtime_identity::RuntimeBackendKind;
use crate::session_backend::inventory::build::{build_item, RawRuntimeFacts};
use crate::session_backend::inventory::status::RuntimeInventoryStatus;
use crate::session_backend::inventory::warning::{InventoryWarningCode, WarningSink};
use crate::session_backend::inventory::{RuntimeInventorySnapshot, RuntimeLifetimePolicy};
use crate::session_backend::opensandbox::dto::SandboxView;
use crate::session_backend::BackendError;

use super::{correlate, OsbBackend};

impl OsbBackend {
    pub(super) async fn list_runtime_inventory_impl(
        &self,
        policy: &RuntimeLifetimePolicy,
    ) -> Result<RuntimeInventorySnapshot, BackendError> {
        let filter = vec![(correlate::KEY_MANAGED.to_string(), "true".to_string())];
        let (views, page_walk_truncated) = self.lifecycle.list_sandboxes_paged(&filter).await?;
        if views.len() > policy.max_items {
            tracing::error!(
                listed = views.len(),
                limit = policy.max_items,
                "opensandbox runtime inventory: fleet exceeds the configured ceiling; refusing to \
                 return a partial snapshot"
            );
            return Err(BackendError::InventoryTooLarge {
                limit: policy.max_items,
            });
        }

        // ONE clock for the whole snapshot, taken after the walk so a slow page
        // traversal cannot make the first page's runtimes look negatively aged.
        let observed_at = Utc::now();
        let mut warnings = WarningSink::new(policy.max_warnings);
        if page_walk_truncated {
            // The walk stopped short: say so rather than let a clipped fleet read
            // as the complete one.
            warnings.push_snapshot(InventoryWarningCode::SourceTruncated);
        }
        let location = self.lifecycle.server_label().map(str::to_string);
        let items = views
            .iter()
            .map(|view| {
                build_item(
                    facts_from_view(view, location.clone()),
                    RuntimeBackendKind::OpenSandbox,
                    observed_at,
                    policy,
                    &mut warnings,
                )
            })
            .collect();

        Ok(RuntimeInventorySnapshot {
            observed_at,
            backend: RuntimeBackendKind::OpenSandbox,
            items,
            warnings: warnings.into_warnings(),
        })
    }
}

/// Project one listed sandbox into the backend-neutral raw facts.
///
/// Never returns `None`: a sandbox that matched `fkst-managed=true` is ours, and a
/// missing session-id stamp makes it an orphan to SHOW, not one to hide.
fn facts_from_view(view: &SandboxView, backend_location: Option<String>) -> RawRuntimeFacts {
    let metadata = &view.metadata;
    let created_at_raw = view.created_at.as_deref();
    let created_at = created_at_raw.and_then(parse_rfc3339);
    let last_pending_raw = metadata.get(correlate::KEY_LAST_PENDING);
    // The last-pending marker is stamped as decimal epoch SECONDS (an RFC3339
    // string is not a valid K8s label value, which sandbox metadata must be).
    let last_pending_at = last_pending_raw
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .and_then(|secs| DateTime::from_timestamp(secs, 0));

    RawRuntimeFacts {
        runtime_id: view.id.clone(),
        // OpenSandbox assigns an id and no separate name or uid; claiming
        // otherwise would invent structure the backend does not have.
        runtime_name: None,
        runtime_uid: None,
        backend_location,

        session_id: metadata.get(correlate::KEY_SESSION_ID).cloned(),
        // Normally true (the list filter pins it); read back so a sandbox whose
        // marker drifted stays visible as such rather than silently trusted.
        managed: metadata.get(correlate::KEY_MANAGED).map(String::as_str) == Some("true"),
        identity: correlate::recover_identity(view),

        owner: metadata.get(correlate::KEY_OWNER).cloned(),
        repo: metadata.get(correlate::KEY_REPO).cloned(),
        installation_id_raw: metadata.get(correlate::KEY_INSTALLATION).cloned(),
        trigger_issue_raw: metadata.get(correlate::KEY_TRIGGER_ISSUE).cloned(),

        status: RuntimeInventoryStatus::from_opensandbox(view.state.as_str()),
        raw_status: view.state.as_str().to_string(),
        status_reason: view.reason.clone(),
        status_message: view.message.clone(),

        created_at,
        created_at_malformed: created_at_raw.is_some() && created_at.is_none(),
        last_pending_at,
        last_pending_malformed: last_pending_raw.is_some() && last_pending_at.is_none(),

        // The lifecycle API reports no restart count. `None` keeps "never
        // restarted" and "not knowable" distinguishable; a zero would assert the
        // first without evidence.
        restart_count: None,
        last_transition_at: view.last_transition_at.as_deref().and_then(parse_rfc3339),
        // A sandbox delete takes effect immediately (the id 404s at once), so
        // there is no pending-deletion instant to report.
        deletion_timestamp: None,
    }
}

fn parse_rfc3339(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
#[path = "inventory_safety_tests.rs"]
mod inventory_safety_tests;
#[cfg(test)]
#[path = "inventory_test_fixtures.rs"]
mod inventory_test_fixtures;
#[cfg(test)]
#[path = "inventory_tests.rs"]
mod inventory_tests;
