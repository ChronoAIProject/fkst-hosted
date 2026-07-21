//! Backend-neutral composed recovery scenarios. Every pass starts at full resync;
//! no webhook, heartbeat, cluster, or external GitHub state is involved.

use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::recovery_chaos_support::*;
use crate::reconcile::desired::KillReason;
use crate::reconcile::{
    SUBSTRATE_ANNOUNCED_LABEL, SUBSTRATE_CONFIG_REJECTED_LABEL, SUBSTRATE_INVALID_LABEL,
    SUBSTRATE_RETIRED_LABEL, WORK_PICKED_UP_LABEL, WORK_UNAUTHORIZED_LABEL,
};
use crate::session_spec::derive_session_id;

const TRIGGER_LABEL: &str = "fkst-substrate-trigger";

async fn new_harness(
    profile: BackendProfile,
    allowed_users: Option<&str>,
    enforce_work_authz: bool,
) -> (MockServer, ChaosHarness) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/tools/contents/pkg/demo/fkst.toml"))
        .and(query_param("ref", "main"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("[github]\nwork_labels = [\"fkst-demo\"]\n"),
        )
        .mount(&server)
        .await;
    let harness = ChaosHarness::new(profile, &server.uri(), allowed_users, enforce_work_authz);
    (server, harness)
}

fn seed_valid(harness: &ChaosHarness, work_author: (&str, i64)) {
    harness.ledger.put(issue(
        TRIGGER,
        trigger_body("demo-session", WORK_LABEL),
        &[TRIGGER_LABEL],
        "alice",
        AUTHOR_ID,
    ));
    harness.ledger.put(issue(
        WORK,
        "work",
        &[WORK_LABEL],
        work_author.0,
        work_author.1,
    ));
}

fn session_id(trigger: i64) -> String {
    derive_session_id(INSTALLATION_ID, "acme", "site", trigger)
}

fn complete_credential_keys() -> Vec<String> {
    [
        "github-token",
        "install",
        "llm-api-key",
        "secret-keys",
        "storage-base-url",
        "storage-bucket",
        "storage-client-id",
        "storage-client-secret",
        "storage-token-url",
        "userenv.DEPLOY_KEY",
        "userenv.PUBLIC_VALUE",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[tokio::test]
async fn converges_after_controller_runtime_and_combined_loss_for_both_backends() {
    for profile in BackendProfile::ALL {
        let (_server, mut harness) = new_harness(profile, None, false).await;
        seed_valid(&harness, ("alice", AUTHOR_ID));
        let expected_id = session_id(TRIGGER);

        // Cold controller discovery creates one deterministic runtime without a webhook.
        harness.full_resync().await;
        assert_eq!(
            harness.runtime_ids(),
            vec![expected_id.clone()],
            "{profile:?}"
        );
        assert_eq!(harness.ensures().len(), 1, "{profile:?}");
        assert!(harness.ensures()[0].created);
        assert_eq!(
            harness.ensures()[0].credential_keys,
            complete_credential_keys(),
            "only the complete, key-name-only inventory is recorded"
        );
        let cold_effects = harness.ledger.effects();
        assert_eq!(cold_effects.comments, 2, "announcement plus work pickup");
        assert_eq!(cold_effects.label_adds, 2);

        // A fresh controller reconstructs every process-local cache. Kubernetes's
        // durable projection needs no ensure; OpenSandbox adopts the complete bundle.
        harness.restart_controller();
        harness.full_resync().await;
        assert_eq!(harness.runtime_ids(), vec![expected_id.clone()]);
        assert_eq!(
            harness.ledger.effects(),
            cold_effects,
            "durable GitHub latches"
        );
        match profile {
            BackendProfile::Kubernetes => assert_eq!(harness.ensures().len(), 1),
            BackendProfile::OpenSandbox => {
                let ensures = harness.ensures();
                assert_eq!(ensures.len(), 2);
                let adoption = ensures.last().unwrap();
                assert!(!adoption.created);
                assert_eq!(adoption.credential_keys, complete_credential_keys());
            }
        }

        // Runtime-only loss recreates the same identity exactly once.
        harness.delete_runtime(&expected_id);
        harness.full_resync().await;
        assert_eq!(harness.runtime_ids(), vec![expected_id.clone()]);

        // Simultaneous controller/runtime loss also converges in one resync.
        harness.delete_runtime(&expected_id);
        harness.restart_controller();
        harness.full_resync().await;
        assert_eq!(harness.runtime_ids(), vec![expected_id.clone()]);
        let ensures = harness.ensures();
        assert_eq!(
            ensures.iter().filter(|event| event.created).count(),
            3,
            "cold plus exactly one recreation per runtime-loss event ({profile:?})"
        );
        assert!(ensures.iter().all(|event| event.session_id == expected_id));
        assert_eq!(harness.ledger.effects(), cold_effects);
    }
}

#[tokio::test]
async fn open_trigger_without_work_stays_registered_and_idle_after_restart() {
    for profile in BackendProfile::ALL {
        let (_server, mut harness) = new_harness(profile, None, false).await;
        harness.ledger.put(issue(
            TRIGGER,
            trigger_body("idle-session", WORK_LABEL),
            &[TRIGGER_LABEL],
            "alice",
            AUTHOR_ID,
        ));

        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty(), "{profile:?}");
        assert!(harness
            .ledger
            .labels(TRIGGER)
            .contains(&SUBSTRATE_ANNOUNCED_LABEL.to_string()));
        let effects = harness.ledger.effects();

        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert!(harness.ensures().is_empty());
        assert_eq!(harness.ledger.effects(), effects);
    }
}

#[tokio::test]
async fn closed_work_and_closed_trigger_never_resurrect_a_runtime() {
    for profile in BackendProfile::ALL {
        let (_server, mut harness) = new_harness(profile, None, false).await;
        seed_valid(&harness, ("alice", AUTHOR_ID));
        let expected_id = session_id(TRIGGER);
        harness.full_resync().await;

        // Work disappears at the same time as the runtime. An open trigger alone is
        // registration, not demand, so reconstruction must remain idle.
        harness.ledger.set_state(WORK, "closed");
        harness.delete_runtime(&expected_id);
        harness.restart_controller();
        harness.full_resync().await;
        assert!(
            harness.runtime_ids().is_empty(),
            "closed work ({profile:?})"
        );

        // Re-establish demand, then close the trigger while a runtime is live. The
        // orphan is stopped and its still-open work issue is durably retired.
        harness.ledger.set_state(WORK, "open");
        harness.full_resync().await;
        assert_eq!(harness.runtime_ids(), vec![expected_id.clone()]);
        harness.ledger.set_state(TRIGGER, "closed");
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert_eq!(
            harness.stops(),
            vec![(expected_id.clone(), KillReason::TriggerClosed)]
        );
        assert!(harness
            .ledger
            .labels(WORK)
            .contains(&SUBSTRATE_RETIRED_LABEL.to_string()));
        assert!(!harness
            .ledger
            .labels(WORK)
            .contains(&WORK_PICKED_UP_LABEL.to_string()));

        let effects = harness.ledger.effects();
        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert_eq!(harness.ledger.effects(), effects);
    }
}

#[tokio::test]
async fn invalid_trigger_feedback_is_durable_and_never_spawns() {
    for profile in BackendProfile::ALL {
        let (_server, mut harness) = new_harness(profile, None, false).await;
        harness.ledger.put(issue(
            TRIGGER,
            "### Session Name\nmissing-everything-else\n",
            &[TRIGGER_LABEL],
            "alice",
            AUTHOR_ID,
        ));

        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert!(harness
            .ledger
            .labels(TRIGGER)
            .contains(&SUBSTRATE_INVALID_LABEL.to_string()));
        assert_eq!(harness.ledger.comments(TRIGGER).len(), 1);
        let effects = harness.ledger.effects();

        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert_eq!(harness.ledger.effects(), effects, "{profile:?}");
    }
}

#[tokio::test]
async fn unauthorized_trigger_and_work_are_blocked_across_reconstruction() {
    for profile in BackendProfile::ALL {
        // Trigger intake is gated by the deployment login allowlist.
        let (_server, mut harness) = new_harness(profile, Some("alice"), false).await;
        harness.ledger.put(issue(
            TRIGGER,
            trigger_body("denied-trigger", WORK_LABEL),
            &[TRIGGER_LABEL],
            "mallory",
            909,
        ));
        harness
            .ledger
            .put(issue(WORK, "work", &[WORK_LABEL], "alice", AUTHOR_ID));
        harness.full_resync().await;
        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert_eq!(
            harness.ledger.effects().comments,
            0,
            "silent trigger denial"
        );

        // An admitted trigger still rejects work raised outside author/collaborator/admin.
        let (_server, mut harness) = new_harness(profile, Some("alice"), true).await;
        seed_valid(&harness, ("mallory", 909));
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty(), "{profile:?}");
        assert!(harness
            .ledger
            .labels(WORK)
            .contains(&WORK_UNAUTHORIZED_LABEL.to_string()));
        assert_eq!(harness.ledger.comments(WORK).len(), 1);
        let effects = harness.ledger.effects();
        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert_eq!(harness.ledger.effects(), effects);
    }
}

#[tokio::test]
async fn colliding_registrations_create_only_the_canonical_runtime() {
    for profile in BackendProfile::ALL {
        let (_server, mut harness) = new_harness(profile, None, false).await;
        seed_valid(&harness, ("alice", AUTHOR_ID));
        harness.ledger.put(issue(
            TRIGGER + 1,
            trigger_body("collision-loser", WORK_LABEL),
            &[TRIGGER_LABEL],
            "alice",
            AUTHOR_ID,
        ));

        harness.full_resync().await;
        assert_eq!(
            harness.runtime_ids(),
            vec![session_id(TRIGGER)],
            "{profile:?}"
        );
        assert_eq!(harness.ensures().len(), 1);
        assert!(harness
            .ledger
            .labels(TRIGGER + 1)
            .contains(&SUBSTRATE_INVALID_LABEL.to_string()));
        let effects = harness.ledger.effects();

        harness.restart_controller();
        harness.full_resync().await;
        assert_eq!(harness.runtime_ids(), vec![session_id(TRIGGER)]);
        assert_eq!(harness.ledger.effects(), effects);
    }
}

#[tokio::test]
async fn announced_trigger_edit_cannot_reconfigure_a_recreated_runtime() {
    for profile in BackendProfile::ALL {
        let (_server, mut harness) = new_harness(profile, None, false).await;
        seed_valid(&harness, ("alice", AUTHOR_ID));
        let expected_id = session_id(TRIGGER);
        harness.full_resync().await;
        assert_eq!(harness.ensures().len(), 1);

        harness.delete_runtime(&expected_id);
        harness
            .ledger
            .set_body(TRIGGER, trigger_body("edited-after-announce", WORK_LABEL));
        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty(), "{profile:?}");
        assert_eq!(
            harness.ensures().len(),
            1,
            "edited config never reaches backend"
        );
        assert!(harness
            .ledger
            .labels(TRIGGER)
            .contains(&SUBSTRATE_CONFIG_REJECTED_LABEL.to_string()));
        assert_eq!(
            harness
                .ledger
                .comments(TRIGGER)
                .iter()
                .filter(|body| body.contains("Config changes are not allowed"))
                .count(),
            1
        );
        let effects = harness.ledger.effects();

        harness.restart_controller();
        harness.full_resync().await;
        assert!(harness.runtime_ids().is_empty());
        assert_eq!(harness.ledger.effects(), effects);
    }
}
