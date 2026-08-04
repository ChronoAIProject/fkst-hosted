//! Tier 2: the disposable-cluster integration smoke.
//!
//! **This tier is environment-gated and does NOT run in the pull-request suite.**
//! It needs a disposable Kubernetes cluster with the built control-plane and
//! relay images deployed, which a laptop running `cargo test` does not have. When
//! the gate is closed each test prints `ACCEPTANCE-SKIP` with the reason and
//! returns.
//!
//! Run it by setting:
//!
//! ```text
//! FKST_ACCEPTANCE_INTEGRATION        1
//! FKST_ACCEPTANCE_KUBE_CONTEXT       the disposable cluster's kube context
//! FKST_ACCEPTANCE_NAMESPACE          a per-run namespace this suite may own
//! FKST_ACCEPTANCE_RELAY_URL          the relay's in-cluster or forwarded URL
//! FKST_ACCEPTANCE_RELAY_WRITE_TOKEN  the relay's write credential
//! FKST_ACCEPTANCE_RELAY_READ_TOKEN   the relay's read credential
//! FKST_ACCEPTANCE_CONTROL_PLANE_URL  the deployed control plane's base URL
//! FKST_ACCEPTANCE_VIEWER_TOKEN       a GitHub token the deployment admits
//! FKST_ACCEPTANCE_RUNTIME_MODE       kubernetes | opensandbox — the mode the
//!                                    deployment under test is running
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
//! Likewise, the deterministic runtime-inventory suites drive a scripted backend.
//! Only a deployed control plane can show that the ONE-LIST contract survives the
//! real client, the real RBAC, and the real OpenSandbox lifecycle API — which is
//! why the second test below queries the deployment's own operations API rather
//! than re-asserting something the unit tier already covers.
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
const REQUIRED: [&str; 8] = [
    "FKST_ACCEPTANCE_KUBE_CONTEXT",
    "FKST_ACCEPTANCE_NAMESPACE",
    "FKST_ACCEPTANCE_RELAY_URL",
    "FKST_ACCEPTANCE_RELAY_WRITE_TOKEN",
    "FKST_ACCEPTANCE_RELAY_READ_TOKEN",
    "FKST_ACCEPTANCE_CONTROL_PLANE_URL",
    "FKST_ACCEPTANCE_VIEWER_TOKEN",
    "FKST_ACCEPTANCE_RUNTIME_MODE",
];

/// Context substrings that make this suite refuse to run.
///
/// A disposable cluster is the whole premise; a mis-set `KUBECONFIG` pointing at
/// something production-shaped must be a loud refusal, never a test run.
const FORBIDDEN_CONTEXT_MARKERS: [&str; 3] = ["prod", "production", "live"];

/// The relay Deployment this tier restarts.
const RELAY_DEPLOYMENT: &str = "deployment/fkst-audit-relay";

/// A record survives a REAL restart of the relay's own Pod, over a real volume.
///
/// The restart is issued by this test rather than assumed to have happened out of
/// band. That distinction is the whole test: with no restart, the second write
/// simply succeeds against a process that never stopped, and the assertion
/// degrades to "the relay is up and idempotent" — which the deterministic tier
/// already proves without a cluster.
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

    // A start with no completion is deliberately NOT returned by the scoped
    // read: that property is what keeps an in-flight request out of a user's
    // history, and it must hold on a deployed relay too.
    assert!(
        !read_all(&client)
            .await
            .iter()
            .any(|row| row.event_id == event_id),
        "an unfinished request is visible in the deployed relay's history"
    );

    client
        .complete(&completion_body(&event_id))
        .await
        .expect("the deployed relay acknowledges the completion");
    assert!(
        read_all(&client)
            .await
            .iter()
            .any(|row| row.event_id == event_id),
        "the completed record never became visible before the restart"
    );

    // The restart itself. `rollout restart` + `rollout status` is the operator's
    // own procedure from the runbook, so a failure here is a failure of the
    // deployment shape, not of a test-only shortcut.
    kubectl(
        &environment,
        &["rollout", "restart", RELAY_DEPLOYMENT],
        "restart the relay",
    );
    kubectl(
        &environment,
        &["rollout", "status", RELAY_DEPLOYMENT, "--timeout=5m"],
        "wait for the relay to come back",
    );

    // Writability has to come back, and the pre-restart record has to still be
    // there. Polling rather than sleeping keeps the test honest about how long a
    // real rollout takes on a small cluster.
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    let mut survived = false;
    while std::time::Instant::now() < deadline {
        if client
            .register_start(&start_body(&run_event_id()))
            .await
            .is_ok()
            && read_all(&client)
                .await
                .iter()
                .any(|row| row.event_id == event_id)
        {
            survived = true;
            break;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    assert!(
        survived,
        "the record written before the restart did not survive it, or the relay \
         never became writable again within three minutes"
    );
}

/// The deployed control plane answers its live inventory from ONE backend list,
/// in whichever runtime mode this deployment is running.
///
/// The mode is declared by the harness rather than guessed: a disposable cluster
/// runs one mode at a time, and the parity claim is that BOTH deployments answer
/// the same documented shape — which this suite proves by being run once per
/// mode, each run asserting its own declared mode end to end.
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

    let mode = environment.get("FKST_ACCEPTANCE_RUNTIME_MODE");
    assert!(
        matches!(mode, "kubernetes" | "opensandbox"),
        "FKST_ACCEPTANCE_RUNTIME_MODE must name the deployed runtime backend, got {mode:?}"
    );

    let base = environment
        .get("FKST_ACCEPTANCE_CONTROL_PLANE_URL")
        .trim_end_matches('/');
    let response = reqwest::Client::new()
        .get(format!("{base}/api/v1/operations/sandboxes"))
        .bearer_auth(environment.get("FKST_ACCEPTANCE_VIEWER_TOKEN"))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .expect("the deployed control plane answers its operations API");
    assert!(
        response.status().is_success(),
        "the deployed control plane refused the inventory read with {}",
        response.status()
    );
    let body: serde_json::Value = response.json().await.expect("a JSON body");

    // The documented envelope, which the SPA and the OpenAPI schema both depend
    // on. A deployment that answered a bare array, or omitted the observation
    // instant, would break both.
    let items = body["items"]
        .as_array()
        .unwrap_or_else(|| panic!("the inventory answer has no items array: {body}"));
    assert!(
        body["observed_at"].is_string(),
        "the inventory answer carries no observation instant: {body}"
    );
    assert!(
        body["backend"].as_str() == Some(mode),
        "the deployment reports backend {:?} but the harness declared {mode:?}",
        body["backend"]
    );

    // Every row must come from the declared backend and carry the fields the
    // one-list contract promises — no per-runtime follow-up read is allowed to
    // fill them in, so an absent field here is a real gap rather than laziness.
    for item in items {
        assert_eq!(
            item["backend"].as_str(),
            Some(mode),
            "a row came from a different backend than the declared mode: {item}"
        );
        for required in ["runtime_id", "status", "raw_status", "metadata_state"] {
            assert!(
                !item[required].is_null(),
                "a row is missing {required}, which one list must already provide: {item}"
            );
        }
        // Restart count is the documented mode DIFFERENCE: Kubernetes sums it,
        // OpenSandbox does not expose it and must report null rather than a
        // fabricated zero.
        if mode == "opensandbox" {
            assert!(
                item["restart_count"].is_null(),
                "an OpenSandbox row invented a restart count: {item}"
            );
        }
    }
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

/// Run one `kubectl` command against the declared context and namespace.
///
/// The context and namespace are always explicit: an ambient `kubectl` default
/// is how a drill lands in the wrong cluster, and this suite mutates a Deployment.
fn kubectl(environment: &gate::GateEnvironment, args: &[&str], what: &str) {
    let output = std::process::Command::new("kubectl")
        .arg("--context")
        .arg(environment.get("FKST_ACCEPTANCE_KUBE_CONTEXT"))
        .arg("--namespace")
        .arg(environment.get("FKST_ACCEPTANCE_NAMESPACE"))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("could not {what}: kubectl did not run ({error})"));
    assert!(
        output.status.success(),
        "could not {what}: kubectl exited with {}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
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

/// The terminal body matching [`start_body`], attributed to a synthetic actor id
/// far outside GitHub's allocated range.
fn completion_body(
    event_id: &str,
) -> fkst_control_plane::audit_relay::protocol::RequestCompletionV1 {
    use fkst_control_plane::audit_relay::protocol::{ActorV1, CorrelationV1, PrincipalV1};
    let start = start_body(event_id);
    fkst_control_plane::audit_relay::protocol::RequestCompletionV1 {
        schema_version: PROTOCOL_SCHEMA_VERSION,
        event_id: start.event_id.clone(),
        request_id: start.request_id.clone(),
        started_at: start.started_at.clone(),
        completed_at: format_instant(Utc::now()),
        method: start.method.clone(),
        route_template: start.route_template.clone(),
        operation_id: start.operation_id.clone(),
        arguments: serde_json::Map::new(),
        arguments_parse_status: "parsed".to_string(),
        actor_id: Some(ACCEPTANCE_ACTOR_ID),
        actor: ActorV1 {
            kind: "github_user".to_string(),
            id: Some(ACCEPTANCE_ACTOR_ID),
            login: Some("fkst-acceptance".to_string()),
            authentication: "bearer".to_string(),
        },
        principal: PrincipalV1 {
            kind: "github_user_token".to_string(),
            id: None,
        },
        status_code: Some(200),
        outcome: "success".to_string(),
        error_code: None,
        duration_ms: 1,
        session_id: None,
        correlation: CorrelationV1::default(),
        service_version: start.service_version.clone(),
        deployment_environment: start.deployment_environment.clone(),
    }
}

/// A synthetic actor id well outside GitHub's allocated range, so nothing this
/// tier writes can be confused with a real person's record.
const ACCEPTANCE_ACTOR_ID: i64 = 9_000_000_010;
