//! The wire bodies and the seeded cross-user dataset every relay suite shares.
//!
//! Split from the harness next door so each file has one job: `mod.rs` runs a
//! relay PROCESS (start, stop, restart, sweep, read), and this file describes the
//! DATA that goes into it. The split also keeps both under this repository's
//! five-hundred-line ceiling.
//!
//! The methods stay on `Relay` — a second inherent `impl` block in a sibling
//! module — so every call site reads `Relay::start_body(..)` exactly as before.

use k8s_openapi::chrono::Duration;

use fkst_control_plane::audit_relay::protocol::{
    format_instant, ActorV1, CorrelationV1, LifecycleEventV1, PrincipalV1, RequestCompletionV1,
    RequestStartV1, PROTOCOL_SCHEMA_VERSION,
};

use super::{anchor, Relay, ALICE, BOB};

impl Relay {
    /// Two of Alice's calls (one inside `sess-1`), one of Bob's inside the same
    /// session, one unattributed call, and one system lifecycle row.
    pub async fn seed_cross_user_fixture(&self) {
        let client = self.client();
        let fixture: [(&str, Option<i64>, Option<&str>); 4] = [
            ("a1111111-1111-4111-8111-111111111111", Some(ALICE), None),
            (
                "a2222222-2222-4222-8222-222222222222",
                Some(ALICE),
                Some("sess-1"),
            ),
            (
                "b1111111-1111-4111-8111-111111111111",
                Some(BOB),
                Some("sess-1"),
            ),
            ("c1111111-1111-4111-8111-111111111111", None, None),
        ];
        for (index, (event_id, actor_id, session_id)) in fixture.into_iter().enumerate() {
            client
                .register_start(&Self::start_body(event_id))
                .await
                .expect("the start is acknowledged");
            let mut completion = Self::completion_body(event_id, actor_id);
            // Distinct completion instants so pagination has a total order.
            let completed_at = anchor() + Duration::seconds(index as i64);
            completion.completed_at = format_instant(completed_at);
            completion.duration_ms =
                u64::try_from((completed_at - anchor()).num_milliseconds()).unwrap_or(0);
            if let Some(session_id) = session_id {
                completion.session_id = Some(session_id.to_string());
                completion.correlation.session_id = Some(session_id.to_string());
            }
            client
                .complete(&completion)
                .await
                .expect("the completion is acknowledged");
        }
        client
            .submit_lifecycle(&Self::lifecycle_body(
                "d1111111-1111-4111-8111-111111111111",
                "sess-1",
            ))
            .await
            .expect("the lifecycle event is acknowledged");
    }

    /// A start body for `event_id`.
    pub fn start_body(event_id: &str) -> RequestStartV1 {
        RequestStartV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event_id.to_string(),
            request_id: format!("req-{event_id}"),
            started_at: format_instant(anchor()),
            method: "GET".to_string(),
            route_template: "/api/v1/overview".to_string(),
            operation_id: "canvas_overview".to_string(),
            service_version: "0.2.3".to_string(),
            deployment_environment: "test".to_string(),
            completion_deadline_at: format_instant(anchor() + Duration::seconds(60)),
        }
    }

    /// The terminal body matching [`Relay::start_body`].
    pub fn completion_body(event_id: &str, actor_id: Option<i64>) -> RequestCompletionV1 {
        let start = Self::start_body(event_id);
        RequestCompletionV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: start.event_id.clone(),
            request_id: start.request_id.clone(),
            started_at: start.started_at.clone(),
            completed_at: format_instant(anchor()),
            method: start.method.clone(),
            route_template: start.route_template.clone(),
            operation_id: start.operation_id.clone(),
            arguments: serde_json::Map::new(),
            arguments_parse_status: "parsed".to_string(),
            actor_id,
            actor: match actor_id {
                Some(id) => ActorV1 {
                    kind: "github_user".to_string(),
                    id: Some(id),
                    login: Some(format!("user-{id}")),
                    authentication: "bearer".to_string(),
                },
                None => ActorV1 {
                    kind: "anonymous".to_string(),
                    id: None,
                    login: None,
                    authentication: "none".to_string(),
                },
            },
            principal: PrincipalV1 {
                kind: "github_user_token".to_string(),
                id: None,
            },
            status_code: Some(200),
            outcome: "success".to_string(),
            error_code: None,
            duration_ms: 0,
            session_id: None,
            correlation: CorrelationV1::default(),
            service_version: "0.2.3".to_string(),
            deployment_environment: "test".to_string(),
        }
    }

    /// A system lifecycle transition for `session_id`.
    pub fn lifecycle_body(event_id: &str, session_id: &str) -> LifecycleEventV1 {
        LifecycleEventV1 {
            schema_version: PROTOCOL_SCHEMA_VERSION,
            event_id: event_id.to_string(),
            occurred_at: format_instant(anchor()),
            lifecycle_action: "created".to_string(),
            actor: ActorV1 {
                kind: "system".to_string(),
                id: None,
                login: None,
                authentication: "internal".to_string(),
            },
            principal: PrincipalV1 {
                kind: "reconciler".to_string(),
                id: Some("reconciler".to_string()),
            },
            session_id: session_id.to_string(),
            backend: "opensandbox".to_string(),
            runtime_id: Some("sbx-1".to_string()),
            runtime_created_at: Some(format_instant(anchor())),
            incarnation_hint: None,
            creator_id: Some(ALICE),
            creator_login: Some("alice".to_string()),
            trigger_author_id: Some(ALICE),
            trigger_author_login: Some("alice".to_string()),
            correlation: CorrelationV1 {
                session_id: Some(session_id.to_string()),
                repo_full_name: Some("acme/site".to_string()),
                installation_id: Some(4242),
                trigger_issue: Some(7),
                webhook_delivery_id: None,
                request_id: None,
            },
            reason_code: None,
            service_version: "0.2.3".to_string(),
            deployment_environment: "test".to_string(),
        }
    }
}
