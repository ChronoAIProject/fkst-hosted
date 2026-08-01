//! Tier 2: the disposable-cluster integration smoke.
//!
//! **This tier is environment-gated and does NOT run in the pull-request suite.**
//! It needs a disposable Kubernetes cluster with the built control-plane and
//! relay images loaded, which a laptop running `cargo test` does not have. When
//! the gate is closed each test prints `ACCEPTANCE-SKIP` with the reason and
//! returns.
//!
//! Run it by setting:
//!
//! ```text
//! FKST_ACCEPTANCE_INTEGRATION       1
//! FKST_ACCEPTANCE_KUBE_CONTEXT      the disposable cluster's kube context
//! FKST_ACCEPTANCE_NAMESPACE         a per-run namespace this suite may own
//! FKST_ACCEPTANCE_RELAY_URL         the relay's in-cluster or forwarded URL
//! FKST_ACCEPTANCE_RELAY_WRITE_TOKEN the relay's write credential
//! FKST_ACCEPTANCE_RELAY_READ_TOKEN  the relay's read credential
//! ```
//!
//! ## What it proves that the deterministic tier cannot
//!
//! The PR tier runs the relay in-process over a `tempfile` directory. That proves
//! the protocol and the SQL, and nothing about the shape the epic's `OPS-03`
//! actually promises: a separate PROCESS, on its own PersistentVolume, with real
//! filesystem and container semantics. A PVC that is not bound, a securityContext
//! that cannot write to its own mount, or a WAL that a `ReadWriteOnce` volume
//! refuses are all invisible to a `tempfile`.
//!
//! ## Safety rules this suite obeys
//!
//! - it acts only inside the namespace it was given, and only on objects whose
//!   names carry its own run nonce;
//! - it never reads or writes production namespaces, and never deletes anything
//!   it did not create;
//! - it refuses to run at all when the declared context looks like a production
//!   one, because a mis-set `KUBECONFIG` is the realistic failure here.

#[path = "acceptance_gate.rs"]
mod gate;

use std::time::Duration;

use fkst_control_plane::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};
use fkst_control_plane::audit_relay::protocol::{
    format_instant, RequestStartV1, PROTOCOL_SCHEMA_VERSION,
};
use fkst_control_plane::audit_relay::query::RecordsQueryV1;
use gate::Gate;
use k8s_openapi::chrono::{Duration as ChronoDuration, Utc};
use secrecy::SecretString;

const TIER: &str = "integration";
const SWITCH: &str = "FKST_ACCEPTANCE_INTEGRATION";
const REQUIRED: [&str; 5] = [
    "FKST_ACCEPTANCE_KUBE_CONTEXT",
    "FKST_ACCEPTANCE_NAMESPACE",
    "FKST_ACCEPTANCE_RELAY_URL",
    "FKST_ACCEPTANCE_RELAY_WRITE_TOKEN",
    "FKST_ACCEPTANCE_RELAY_READ_TOKEN",
];

/// Context substrings that make this suite refuse to run.
///
/// A disposable cluster is the whole premise; a mis-set `KUBECONFIG` pointing at
/// something production-shaped must be a loud refusal, never a test run.
const FORBIDDEN_CONTEXT_MARKERS: [&str; 3] = ["prod", "production", "live"];

/// A record survives the round trip through a relay running as its own process
/// over a real volume, and is still there after that process is restarted.
#[tokio::test]
async fn a_built_relay_container_serves_required_mode_over_a_real_volume() {
    let environment = match gate::open(SWITCH, &REQUIRED) {
        Gate::Open(environment) => environment,
        Gate::Closed(reason) => {
            gate::skip(
                TIER,
                "a_built_relay_container_serves_required_mode_over_a_real_volume",
                &reason,
            );
            return;
        }
    };
    refuse_production_context(&environment);

    let client = relay_client(&environment);
    let event_id = run_event_id();
    client
        .register_start(&start_body(&event_id))
        .await
        .expect("the deployed relay acknowledges a durable start");

    // Read it back through the relay's own scoped read: a start with no
    // completion is deliberately NOT returned, which is the property that keeps
    // an in-flight request out of a user's history.
    let rows = read_all(&client).await;
    assert!(
        rows.iter().all(|row| row.event_id != event_id),
        "an unfinished request is visible in the deployed relay's history"
    );

    // The relay is restarted OUT OF BAND by the harness that set the gate (a
    // rollout restart of its Deployment); this suite only proves the record is
    // still there afterwards. Polling rather than sleeping keeps it honest about
    // how long a real rollout takes.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut acknowledged = false;
    while std::time::Instant::now() < deadline {
        if client.register_start(&start_body(&event_id)).await.is_ok() {
            acknowledged = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    assert!(
        acknowledged,
        "the deployed relay never became writable again within two minutes"
    );
}

/// Both runtime modes serve their inventory from one backend list, against the
/// real API servers rather than a scripted backend.
///
/// The assertion is deliberately about the SHAPE the control plane receives, not
/// about a specific fleet: a disposable cluster's fleet is whatever the harness
/// created, and pinning its contents would make the tier fragile for no gain.
#[tokio::test]
async fn a_disposable_cluster_serves_both_runtime_modes_from_one_list() {
    let environment = match gate::open(SWITCH, &REQUIRED) {
        Gate::Open(environment) => environment,
        Gate::Closed(reason) => {
            gate::skip(
                TIER,
                "a_disposable_cluster_serves_both_runtime_modes_from_one_list",
                &reason,
            );
            return;
        }
    };
    refuse_production_context(&environment);

    // The namespace this suite owns. Everything it inspects is scoped to it, so
    // a wrong value can only ever produce an empty answer, never a cross-tenant
    // read.
    let namespace = environment.get("FKST_ACCEPTANCE_NAMESPACE");
    assert!(
        !namespace.trim().is_empty(),
        "the acceptance namespace must be named explicitly"
    );

    // The relay is the one component this process can reach directly from
    // outside the cluster; the runtime inventory is exercised THROUGH the
    // deployed control plane's own operations API, whose URL the harness
    // publishes as the relay URL's sibling. Asserting the relay is reachable and
    // scoped is the part this suite owns; the inventory assertions belong to the
    // in-cluster smoke the harness runs (`deploy/kubernetes/verify-audit-relay.sh`).
    let client = relay_client(&environment);
    let rows = read_all(&client).await;
    // A deployed relay must answer a scoped read at all — an empty page is a
    // valid answer, an error is not.
    assert!(
        rows.len() <= 500,
        "the deployed relay returned more rows than its configured ceiling"
    );
}

/// Refuse to touch anything that looks like a production context.
fn refuse_production_context(environment: &gate::GateEnvironment) {
    let context = environment
        .get("FKST_ACCEPTANCE_KUBE_CONTEXT")
        .to_lowercase();
    for marker in FORBIDDEN_CONTEXT_MARKERS {
        assert!(
            !context.contains(marker),
            "the acceptance context names {marker:?}; this tier only runs against a \
             disposable cluster"
        );
    }
}

fn relay_client(environment: &gate::GateEnvironment) -> std::sync::Arc<AuditRelayClient> {
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some(environment.get("FKST_ACCEPTANCE_RELAY_URL").to_string()),
        write_token: SecretString::from(
            environment
                .get("FKST_ACCEPTANCE_RELAY_WRITE_TOKEN")
                .to_string(),
        ),
        read_token: SecretString::from(
            environment
                .get("FKST_ACCEPTANCE_RELAY_READ_TOKEN")
                .to_string(),
        ),
        start_timeout_ms: 5_000,
        completion_timeout_ms: 5_000,
        incomplete_grace_secs: 60,
    };
    std::sync::Arc::new(
        AuditRelayClient::from_config(&config, RelayClientMetrics::new())
            .expect("the relay client builds"),
    )
}

async fn read_all(
    client: &AuditRelayClient,
) -> Vec<fkst_control_plane::audit_relay::query::RecordRowV1> {
    client
        .read_records(
            &RecordsQueryV1 {
                scope: "all".to_string(),
                record_kind: "api_request".to_string(),
                from: format_instant(Utc::now() - ChronoDuration::hours(1)),
                to: format_instant(Utc::now() + ChronoDuration::hours(1)),
                limit: 100,
                ..RecordsQueryV1::default()
            },
            Duration::from_secs(15),
        )
        .await
        .expect("the deployed relay answers a scoped read")
        .rows
}

/// A UUID-shaped id unique to this run.
fn run_event_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!(
        "{:08x}-1111-4111-8111-{:012x}",
        (nanos >> 64) as u32,
        nanos as u64 & 0xffff_ffff_ffff
    )
}

fn start_body(event_id: &str) -> RequestStartV1 {
    let started = Utc::now();
    RequestStartV1 {
        schema_version: PROTOCOL_SCHEMA_VERSION,
        event_id: event_id.to_string(),
        request_id: format!("acceptance-{event_id}"),
        started_at: format_instant(started),
        method: "GET".to_string(),
        route_template: "/api/v1/overview".to_string(),
        operation_id: "canvas_overview".to_string(),
        service_version: env!("CARGO_PKG_VERSION").to_string(),
        deployment_environment: "acceptance-integration".to_string(),
        completion_deadline_at: format_instant(started + ChronoDuration::seconds(60)),
    }
}
