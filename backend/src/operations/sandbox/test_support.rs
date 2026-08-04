//! Shared, credential-free fixtures for the sandbox-inventory unit tests.
//!
//! A [`RuntimeInventoryItem`] has thirty-odd fields, almost all of which are
//! irrelevant to any one authorization, filter, or ordering assertion. The
//! builder here keeps each test about the ONE field it is arguing over, so a
//! future field addition does not force a rewrite of every fixture.

use k8s_openapi::chrono::{DateTime, TimeZone, Utc};

use crate::runtime_identity::{AttributionSource, RuntimeBackendKind};
use crate::session_backend::inventory::{
    InventoryWarningCode, RuntimeInventoryItem, RuntimeInventoryStatus, RuntimeMetadataState,
};

/// A fixed instant every fixture is anchored to.
pub(crate) fn instant(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 31, hour, minute, 0)
        .single()
        .expect("a valid fixed instant")
}

/// A complete, plausible runtime for `session`, running under Kubernetes.
pub(crate) fn item(runtime_id: &str, session: Option<&str>) -> RuntimeInventoryItem {
    RuntimeInventoryItem {
        backend: RuntimeBackendKind::Kubernetes,
        runtime_id: runtime_id.to_string(),
        runtime_name: Some(runtime_id.to_string()),
        runtime_uid: Some(format!("uid-{runtime_id}")),
        backend_location: Some("chronoai-fkst".to_string()),
        session_id: session.map(str::to_string),
        managed: true,
        metadata_state: RuntimeMetadataState::Complete,
        creator_id: Some(101),
        creator_login: Some("alice".to_string()),
        trigger_author_id: Some(101),
        trigger_author_login: Some("alice".to_string()),
        attribution_source: AttributionSource::LaunchMetadata,
        repo_full_name: Some("acme/site".to_string()),
        installation_id: Some(1),
        trigger_issue: Some(7),
        status: RuntimeInventoryStatus::Running,
        raw_status: "Running".to_string(),
        status_reason: None,
        status_message: None,
        created_at: Some(instant(12, 0)),
        age_seconds: Some(600),
        max_lifetime_seconds: None,
        expires_at: None,
        remaining_seconds: None,
        minimum_lifetime_seconds: 300,
        minimum_lifetime_remaining_seconds: None,
        idle_grace_seconds: 900,
        last_pending_at: None,
        idle_for_seconds: Some(600),
        restart_count: Some(0),
        last_transition_at: None,
        deletion_timestamp: None,
        warnings: Vec::new(),
    }
}

/// The same runtime carrying its own data-quality codes.
pub(crate) fn with_warnings(
    runtime_id: &str,
    session: Option<&str>,
    warnings: Vec<InventoryWarningCode>,
) -> RuntimeInventoryItem {
    RuntimeInventoryItem {
        warnings,
        ..item(runtime_id, session)
    }
}

/// The same runtime with an explicit normalized status.
pub(crate) fn with_status(
    runtime_id: &str,
    session: Option<&str>,
    status: RuntimeInventoryStatus,
) -> RuntimeInventoryItem {
    RuntimeInventoryItem {
        status,
        raw_status: status.as_str().to_string(),
        ..item(runtime_id, session)
    }
}

// ---------------------------------------------------------------------------
// The shared service-pipeline fixture.
//
// One session (Alice's), one ready projection, one policy. Every pipeline test
// varies exactly one thing against it — the caller, the scope, the filters, or
// what the backend does — so the file that holds the assertions stays about the
// claim rather than about the setup.
// ---------------------------------------------------------------------------

use std::time::Duration;

use crate::access_policy::AccessPolicy;
use crate::error::AppError;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::session_access::test_support::{context, policy_with_admins};
use crate::session_access::{
    AuthenticatedViewer, RequestedScope, ScopeRequest, SessionAccessRegistry, ViewerScope,
};
use crate::session_backend::inventory::RuntimeLifetimePolicy;
use crate::session_backend::test_support::FakeSessionBackend;

use super::filters::SandboxFilters;
use super::service::{run, AuthorizedInventory, SandboxInventoryRequest};

pub(crate) const ALICE: (i64, &str) = (101, "alice");
pub(crate) const ERIN: (i64, &str) = (105, "erin");
pub(crate) const GRACE: (i64, &str) = (900, "grace");
/// The session Alice created — the only one the fixture projection knows.
pub(crate) const MINE: &str = "sess-alice";
/// A session belonging to somebody outside the fixture entirely.
pub(crate) const THEIRS: &str = "sess-stranger";

pub(crate) fn access() -> AccessPolicy {
    policy_with_admins(GRACE.1)
}

pub(crate) fn viewer(who: (i64, &str), access: &AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(
        GithubUser {
            login: who.1.to_string(),
            id: who.0,
        },
        access,
    )
}

/// The scope is always stated EXPLICITLY: omitting it resolves to the caller's
/// natural default, which for an administrator is the global scope.
pub(crate) fn scope(viewer: &AuthenticatedViewer, global: bool) -> ViewerScope {
    let requested = if global {
        RequestedScope::Global
    } else {
        RequestedScope::Personal
    };
    viewer
        .resolve_scope(ScopeRequest::new(Some(requested)))
        .expect("the fixture resolves")
}

/// A ready projection knowing exactly one session: Alice's.
pub(crate) fn registry() -> SessionAccessRegistry {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        vec![(MINE.to_string(), context(Some(ALICE.0), ALICE.1, &[], &[]))],
    );
    registry
}

pub(crate) fn lifetime() -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: 0,
        minimum_lifetime_seconds: 300,
        idle_grace_seconds: 900,
        max_items: 5_000,
        max_warnings: 256,
    }
}

/// The owned half of a pipeline request, so the borrowed
/// [`SandboxInventoryRequest`] can be assembled per call.
pub(crate) struct Fixture {
    pub(crate) access: AccessPolicy,
    pub(crate) registry: SessionAccessRegistry,
    pub(crate) admins: Vec<String>,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self {
            access: access(),
            registry: registry(),
            admins: Vec::new(),
        }
    }

    pub(crate) fn request<'a>(
        &'a self,
        viewer: &'a AuthenticatedViewer,
        scope: &'a ViewerScope,
        filters: &'a SandboxFilters,
        max_result_items: usize,
    ) -> SandboxInventoryRequest<'a> {
        SandboxInventoryRequest {
            viewer,
            scope,
            access: &self.access,
            legacy_log_admins: &self.admins,
            registry: &self.registry,
            filters,
            lifetime: lifetime(),
            max_result_items,
            timeout: Duration::from_millis(500),
        }
    }
}

/// A mixed fleet: two runtimes Alice may see, three she may not.
pub(crate) fn mixed_fleet() -> Vec<RuntimeInventoryItem> {
    vec![
        with_status(
            "hidden-failed",
            Some(THEIRS),
            RuntimeInventoryStatus::Failed,
        ),
        with_status("mine-running", Some(MINE), RuntimeInventoryStatus::Running),
        with_status("hidden-orphan", None, RuntimeInventoryStatus::Failed),
        with_status("mine-failed", Some(MINE), RuntimeInventoryStatus::Failed),
        with_status(
            "hidden-unknown-ctx",
            Some("sess-nowhere"),
            RuntimeInventoryStatus::Running,
        ),
    ]
}

/// Run the pipeline for one caller against a scripted backend.
pub(crate) async fn run_for(
    who: (i64, &str),
    global: bool,
    filters: SandboxFilters,
    backend: FakeSessionBackend,
) -> Result<AuthorizedInventory, AppError> {
    let fixture = Fixture::new();
    let viewer = viewer(who, &fixture.access);
    let scope = scope(&viewer, global);
    run(&backend, &fixture.request(&viewer, &scope, &filters, 5_000)).await
}

/// The runtime ids of an authorized result, in order.
pub(crate) fn ids(inventory: &AuthorizedInventory) -> Vec<String> {
    inventory
        .items
        .iter()
        .map(|runtime| runtime.item.runtime_id.clone())
        .collect()
}
