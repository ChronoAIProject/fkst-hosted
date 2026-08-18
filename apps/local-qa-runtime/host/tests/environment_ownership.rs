use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::{
    ownership::{ENVIRONMENT_ID_LABEL, PROFILE_ID_LABEL, RUN_ID_LABEL},
    reconcile_environment, CreateRequest, EnvironmentProvider, EnvironmentRequest, Journal,
    ProviderResource, RunError,
};
use rusqlite::Connection;

const RUN_ID: &str = "00000000-0000-4000-8000-000000000001";
const DEADLINE: &str = "2026-08-14T12:00:00Z";
const NOW: &str = "2026-08-14T11:59:00Z";

struct FakeProvider {
    database_path: PathBuf,
    discover_calls: Arc<Mutex<Vec<String>>>,
    create_calls: Arc<Mutex<Vec<CreateRequest>>>,
    provider_identity: String,
    expected_intent_id: String,
    mismatch: bool,
}

impl EnvironmentProvider for FakeProvider {
    fn discover(&mut self, stable_provider_key: &str) -> Result<Option<ProviderResource>, RunError> {
        self.discover_calls
            .lock()
            .expect("discover calls lock must be available")
            .push(stable_provider_key.to_owned());
        Ok(None)
    }

    fn create(&mut self, request: CreateRequest) -> Result<ProviderResource, RunError> {
        let connection = Connection::open(&self.database_path).expect("visibility connection must open");
        let prepared: String = connection
            .query_row(
                "SELECT status FROM resource_intents WHERE intent_id = ?1",
                [&self.expected_intent_id],
                |row| row.get(0),
            )
            .expect("prepared intent must be visible before create");
        assert_eq!(prepared, "prepared");
        self.create_calls
            .lock()
            .expect("create calls lock must be available")
            .push(request.clone());
        let mut labels = request.labels.clone();
        if self.mismatch {
            labels.insert(PROFILE_ID_LABEL.to_owned(), "profile-other".to_owned());
        }
        Ok(ProviderResource {
            stable_provider_key: request.stable_provider_key,
            labels,
            provider_identity: self.provider_identity.clone(),
        })
    }
}

fn request(intent_id: &str, deadline_utc: &str) -> EnvironmentRequest {
    EnvironmentRequest {
        intent_id: intent_id.to_owned(),
        run_id: RUN_ID.to_owned(),
        profile_id: "profile-001".to_owned(),
        environment_id: "environment-001".to_owned(),
        generation: 1,
        deadline_utc: deadline_utc.to_owned(),
        provider_identity: "provider-env-001".to_owned(),
    }
}

fn database_path(label: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("fkst-local-qa-host-{label}-{timestamp}.sqlite"))
}

fn insert_run(path: &Path) {
    let connection = Connection::open(path).expect("database must open");
    connection
        .execute(
            "INSERT INTO runs (run_id, state) VALUES (?1, 'accepted')",
            [RUN_ID],
        )
        .expect("canonical run must exist before intent insertion");
}

#[test]
fn environment_bind_walks_the_host_and_replays_without_provider_effect() {
    let path = database_path("environment-bind");
    let mut journal = Journal::open(&path).expect("journal must open");
    insert_run(&path);
    let discover_calls = Arc::new(Mutex::new(Vec::new()));
    let create_calls = Arc::new(Mutex::new(Vec::new()));
    let mut provider = FakeProvider {
        database_path: path.clone(),
        discover_calls: Arc::clone(&discover_calls),
        create_calls: Arc::clone(&create_calls),
        provider_identity: "provider-env-001".to_owned(),
        expected_intent_id: "intent-env-001".to_owned(),
        mismatch: false,
    };

    let first = reconcile_environment(&mut journal, &mut provider, &request("intent-env-001", DEADLINE), NOW)
        .expect("environment bind must succeed");
    assert_eq!(first.intent_id, "intent-env-001");
    assert_eq!(first.run_id, RUN_ID);
    assert_eq!(first.profile_id, "profile-001");
    assert_eq!(first.environment_id, "environment-001");
    assert_eq!(first.generation, 1);
    assert_eq!(first.stable_provider_key, "fkst-local-qa/environment/v1/intent-env-001");
    assert_eq!(first.provider_identity, "provider-env-001");
    assert_eq!(first.state, "active");
    assert_eq!(
        discover_calls.lock().unwrap().as_slice(),
        ["fkst-local-qa/environment/v1/intent-env-001".to_owned()].as_slice()
    );
    assert_eq!(create_calls.lock().unwrap().len(), 1);
    let create_requests = create_calls.lock().unwrap().clone();
    let create_request = &create_requests[0];
    assert_eq!(create_request.stable_provider_key, first.stable_provider_key);
    assert_eq!(create_request.labels, BTreeMap::from([
        (RUN_ID_LABEL.to_owned(), RUN_ID.to_owned()),
        (PROFILE_ID_LABEL.to_owned(), "profile-001".to_owned()),
        (ENVIRONMENT_ID_LABEL.to_owned(), "environment-001".to_owned()),
    ]));

    let replay = reconcile_environment(&mut journal, &mut provider, &request("intent-env-001", DEADLINE), NOW)
        .expect("identical replay must return the durable handle");
    assert_eq!(replay, first);
    assert_eq!(create_calls.lock().unwrap().len(), 1);

    let connection = Connection::open(&path).expect("inspection connection must open");
    assert_eq!(connection.query_row("SELECT status FROM resource_intents", [], |row| row.get::<_, String>(0)).unwrap(), "bound");
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM owned_handles", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(connection.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0)).unwrap(), 4);
    let foreign_key_errors: Vec<(String, i64, String, i64)> = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(foreign_key_errors.is_empty());
    std::fs::remove_file(path).expect("temporary database must be removed");
}

#[test]
fn provider_mismatch_and_expiry_fail_closed_without_effects() {
    let path = database_path("environment-fail-closed");
    let mut journal = Journal::open(&path).expect("journal must open");
    insert_run(&path);
    let discover_calls = Arc::new(Mutex::new(Vec::new()));
    let create_calls = Arc::new(Mutex::new(Vec::new()));
    let mut mismatch_provider = FakeProvider {
        database_path: path.clone(),
        discover_calls: Arc::clone(&discover_calls),
        create_calls: Arc::clone(&create_calls),
        provider_identity: "provider-env-001".to_owned(),
        expected_intent_id: "intent-env-mismatch".to_owned(),
        mismatch: true,
    };
    assert!(reconcile_environment(
        &mut journal,
        &mut mismatch_provider,
        &request("intent-env-mismatch", DEADLINE),
        NOW,
    )
    .is_err());
    assert_eq!(create_calls.lock().unwrap().len(), 1);
    assert_eq!(journal.owned_handle("intent-env-mismatch").unwrap(), None);
    let status: String = Connection::open(&path)
        .unwrap()
        .query_row("SELECT status FROM resource_intents WHERE intent_id = 'intent-env-mismatch'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(status, "prepared");

    let before_discover = discover_calls.lock().unwrap().len();
    let before_create = create_calls.lock().unwrap().len();
    let mut expired_provider = FakeProvider {
        database_path: path.clone(),
        discover_calls,
        create_calls,
        provider_identity: "provider-env-001".to_owned(),
        expected_intent_id: "intent-env-expired".to_owned(),
        mismatch: false,
    };
    assert!(reconcile_environment(
        &mut journal,
        &mut expired_provider,
        &request("intent-env-expired", "2026-08-14T10:00:00Z"),
        NOW,
    )
    .is_err());
    assert_eq!(expired_provider.discover_calls.lock().unwrap().len(), before_discover);
    assert_eq!(expired_provider.create_calls.lock().unwrap().len(), before_create);
    assert_eq!(journal.owned_handle("intent-env-expired").unwrap(), None);
    std::fs::remove_file(path).expect("temporary database must be removed");
}
