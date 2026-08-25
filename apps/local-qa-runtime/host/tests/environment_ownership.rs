use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fkst_local_qa_host::{
    ownership::{ENVIRONMENT_ID_LABEL, PROFILE_ID_LABEL, RUN_ID_LABEL},
    reconcile_environment, CreateRequest, EnvironmentProvider, EnvironmentRequest, FixedClock,
    Journal, OwnedHandle, ProviderResource, RunError,
};
use rusqlite::{params, Connection};

const RUN_ID: &str = "00000000-0000-4000-8000-000000000001";
const DEADLINE: &str = "2026-08-14T12:00:00Z";
const NOW: &str = "2026-08-14T11:59:00Z";
const EXPIRED_NOW: &str = "2026-08-14T12:00:01Z";
static DATABASE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct FakeProvider {
    database_path: PathBuf,
    discover_calls: Arc<Mutex<Vec<String>>>,
    create_calls: Arc<Mutex<Vec<CreateRequest>>>,
    provider_identity: String,
    expected_intent_id: String,
    mismatch: bool,
}

struct DiscoveringProvider {
    database_path: PathBuf,
    discover_calls: Arc<Mutex<Vec<String>>>,
    create_calls: Arc<Mutex<Vec<CreateRequest>>>,
    expected_intent_id: String,
    resource: ProviderResource,
}

impl EnvironmentProvider for DiscoveringProvider {
    fn discover(
        &mut self,
        stable_provider_key: &str,
    ) -> Result<Option<ProviderResource>, RunError> {
        let connection =
            Connection::open(&self.database_path).expect("visibility connection must open");
        let prepared: String = connection
            .query_row(
                "SELECT status FROM resource_intents WHERE intent_id = ?1",
                [&self.expected_intent_id],
                |row| row.get(0),
            )
            .expect("prepared intent must be visible before discover");
        assert_eq!(prepared, "prepared");
        self.discover_calls
            .lock()
            .expect("discover calls lock must be available")
            .push(stable_provider_key.to_owned());
        Ok(Some(self.resource.clone()))
    }

    fn create(&mut self, request: CreateRequest) -> Result<ProviderResource, RunError> {
        self.create_calls
            .lock()
            .expect("create calls lock must be available")
            .push(request);
        Err(RunError::InvalidJournal(
            "create must not run for exact discovery",
        ))
    }
}

impl EnvironmentProvider for FakeProvider {
    fn discover(
        &mut self,
        stable_provider_key: &str,
    ) -> Result<Option<ProviderResource>, RunError> {
        self.discover_calls
            .lock()
            .expect("discover calls lock must be available")
            .push(stable_provider_key.to_owned());
        Ok(None)
    }

    fn create(&mut self, request: CreateRequest) -> Result<ProviderResource, RunError> {
        let connection =
            Connection::open(&self.database_path).expect("visibility connection must open");
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
    let sequence = DATABASE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "fkst-local-qa-host-{label}-{}-{sequence}.sqlite",
        std::process::id()
    ))
}

fn insert_run(path: &Path) {
    let connection = Connection::open(path).expect("database must open");
    connection
        .execute(
            "INSERT INTO runs (run_id, executor_run_id, state) VALUES (?1, ?1, 'accepted')",
            [RUN_ID],
        )
        .expect("canonical run must exist before intent insertion");
}

#[test]
fn exact_environment_discovery_binds_without_create() {
    let path = database_path("environment-exact-discovery");
    let mut journal = Journal::open(&path).expect("journal must open");
    insert_run(&path);
    let connection = Connection::open(&path).expect("inspection connection must open");
    let run_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE run_id = ?1 AND state = 'accepted'",
            [RUN_ID],
            |row| row.get(0),
        )
        .expect("canonical run must be queryable before reconciliation");
    assert_eq!(run_count, 1);
    drop(connection);

    let expected_labels = BTreeMap::from([
        (RUN_ID_LABEL.to_owned(), RUN_ID.to_owned()),
        (PROFILE_ID_LABEL.to_owned(), "profile-001".to_owned()),
        (
            ENVIRONMENT_ID_LABEL.to_owned(),
            "environment-001".to_owned(),
        ),
    ]);
    let discover_calls = Arc::new(Mutex::new(Vec::new()));
    let create_calls = Arc::new(Mutex::new(Vec::new()));
    let mut provider = DiscoveringProvider {
        database_path: path.clone(),
        discover_calls: Arc::clone(&discover_calls),
        create_calls: Arc::clone(&create_calls),
        expected_intent_id: "intent-env-001".to_owned(),
        resource: ProviderResource {
            stable_provider_key: "fkst-local-qa/environment/v1/intent-env-001".to_owned(),
            labels: expected_labels,
            provider_identity: "provider-env-001".to_owned(),
        },
    };
    let clock = FixedClock::new(NOW).expect("fixture clock must be valid");
    let expected = OwnedHandle {
        intent_id: "intent-env-001".to_owned(),
        run_id: RUN_ID.to_owned(),
        profile_id: "profile-001".to_owned(),
        environment_id: "environment-001".to_owned(),
        generation: 1,
        deadline_utc: DEADLINE.to_owned(),
        stable_provider_key: "fkst-local-qa/environment/v1/intent-env-001".to_owned(),
        provider_identity: "provider-env-001".to_owned(),
        state: "active".to_owned(),
    };

    let recorded = reconcile_environment(
        &mut journal,
        &mut provider,
        &request("intent-env-001", DEADLINE),
        &clock,
    )
    .expect("exact discovered environment must bind");

    assert_eq!(recorded, expected);
    assert_eq!(
        discover_calls.lock().unwrap().as_slice(),
        ["fkst-local-qa/environment/v1/intent-env-001".to_owned()].as_slice()
    );
    assert!(create_calls.lock().unwrap().is_empty());
    let persisted = journal
        .owned_handle("intent-env-001")
        .expect("owned handle lookup must succeed")
        .expect("owned handle must be recorded");
    assert_eq!(persisted, recorded);

    let connection = Connection::open(&path).expect("inspection connection must open");
    let bound_intents: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM resource_intents
             WHERE intent_id = ?1 AND run_id = ?2 AND profile_id = ?3
               AND environment_id = ?4 AND generation = ?5 AND deadline_utc = ?6
               AND stable_provider_key = ?7 AND status = 'bound'",
            params![
                recorded.intent_id,
                recorded.run_id,
                recorded.profile_id,
                recorded.environment_id,
                recorded.generation,
                recorded.deadline_utc,
                recorded.stable_provider_key,
            ],
            |row| row.get(0),
        )
        .expect("bound intent must be queryable");
    assert_eq!(bound_intents, 1);
    let active_handles: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM owned_handles
             WHERE intent_id = ?1 AND run_id = ?2 AND profile_id = ?3
               AND environment_id = ?4 AND generation = ?5
               AND stable_provider_key = ?6 AND provider_identity = ?7
               AND state = 'active'",
            params![
                recorded.intent_id,
                recorded.run_id,
                recorded.profile_id,
                recorded.environment_id,
                recorded.generation,
                recorded.stable_provider_key,
                recorded.provider_identity,
            ],
            |row| row.get(0),
        )
        .expect("active handle must be queryable");
    assert_eq!(active_handles, 1);
    drop(connection);
    std::fs::remove_file(path).expect("temporary database must be removed");
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
    let clock = FixedClock::new(NOW).expect("fixture clock must be valid");

    let first = reconcile_environment(
        &mut journal,
        &mut provider,
        &request("intent-env-001", DEADLINE),
        &clock,
    )
    .expect("environment bind must succeed");
    assert_eq!(first.intent_id, "intent-env-001");
    assert_eq!(first.run_id, RUN_ID);
    assert_eq!(first.profile_id, "profile-001");
    assert_eq!(first.environment_id, "environment-001");
    assert_eq!(first.generation, 1);
    assert_eq!(first.deadline_utc, DEADLINE);
    assert_eq!(
        first.stable_provider_key,
        "fkst-local-qa/environment/v1/intent-env-001"
    );
    assert_eq!(first.provider_identity, "provider-env-001");
    assert_eq!(first.state, "active");
    assert_eq!(
        discover_calls.lock().unwrap().as_slice(),
        ["fkst-local-qa/environment/v1/intent-env-001".to_owned()].as_slice()
    );
    assert_eq!(create_calls.lock().unwrap().len(), 1);
    let create_requests = create_calls.lock().unwrap().clone();
    let create_request = &create_requests[0];
    assert_eq!(
        create_request.stable_provider_key,
        first.stable_provider_key
    );
    assert_eq!(
        create_request.labels,
        BTreeMap::from([
            (RUN_ID_LABEL.to_owned(), RUN_ID.to_owned()),
            (PROFILE_ID_LABEL.to_owned(), "profile-001".to_owned()),
            (
                ENVIRONMENT_ID_LABEL.to_owned(),
                "environment-001".to_owned()
            ),
        ])
    );

    let expired_clock = FixedClock::new(EXPIRED_NOW).expect("fixture clock must be valid");
    let replay = reconcile_environment(
        &mut journal,
        &mut provider,
        &request("intent-env-001", DEADLINE),
        &expired_clock,
    )
    .expect("identical replay must return the durable handle after expiry");
    assert_eq!(replay, first);
    assert_eq!(create_calls.lock().unwrap().len(), 1);
    assert_eq!(discover_calls.lock().unwrap().len(), 1);

    let connection = Connection::open(&path).expect("inspection connection must open");
    assert_eq!(
        connection
            .query_row("SELECT status FROM resource_intents", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "bound"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM owned_handles", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        6
    );
    let foreign_key_errors: Vec<(String, i64, String, i64)> = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
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
    let clock = FixedClock::new(NOW).expect("fixture clock must be valid");
    assert!(reconcile_environment(
        &mut journal,
        &mut mismatch_provider,
        &request("intent-env-mismatch", DEADLINE),
        &clock,
    )
    .is_err());
    assert_eq!(create_calls.lock().unwrap().len(), 1);
    assert_eq!(journal.owned_handle("intent-env-mismatch").unwrap(), None);
    let status: String = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT status FROM resource_intents WHERE intent_id = 'intent-env-mismatch'",
            [],
            |row| row.get(0),
        )
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
    let expired_clock = FixedClock::new(EXPIRED_NOW).expect("fixture clock must be valid");
    assert!(reconcile_environment(
        &mut journal,
        &mut expired_provider,
        &request("intent-env-expired", DEADLINE),
        &expired_clock,
    )
    .is_err());
    assert_eq!(
        expired_provider.discover_calls.lock().unwrap().len(),
        before_discover
    );
    assert_eq!(
        expired_provider.create_calls.lock().unwrap().len(),
        before_create
    );
    assert_eq!(journal.owned_handle("intent-env-expired").unwrap(), None);
    let expired_intents: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM resource_intents WHERE intent_id = 'intent-env-expired'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(expired_intents, 0);
    std::fs::remove_file(path).expect("temporary database must be removed");
}

#[test]
fn version_three_migration_preserves_lifecycle_rows() {
    let path = database_path("environment-v3-migration");
    {
        let journal = Journal::open(&path).expect("journal must open");
        drop(journal);
        let connection = Connection::open(&path).expect("fixture database must open");
        connection
            .execute_batch(
                "DROP TABLE owned_handles;
                 DROP TABLE resource_intents;
                 DROP INDEX runs_executor_run_id_unique;
                 DROP TRIGGER runs_executor_run_id_insert;
                 DROP TRIGGER runs_executor_run_id_update;
                 ALTER TABLE runs DROP COLUMN executor_run_id;
                 INSERT INTO runs (run_id, state)
                 VALUES ('00000000-0000-4000-8000-000000000001', 'accepted');
                 PRAGMA user_version = 3;",
            )
            .expect("version three fixture must be prepared");
    }

    let journal = Journal::open(&path).expect("version three journal must migrate");
    let connection = Connection::open(&path).expect("inspection database must open");
    let state: String = connection
        .query_row(
            "SELECT state FROM runs WHERE run_id = ?1",
            [RUN_ID],
            |row| row.get(0),
        )
        .expect("lifecycle row must survive migration");
    assert_eq!(state, "accepted");
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("schema version must be readable");
    assert_eq!(version, 5);
    let foreign_key_errors: Vec<(String, i64, String, i64)> = connection
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(foreign_key_errors.is_empty());
    drop(connection);
    drop(journal);
    std::fs::remove_file(path).expect("temporary database must be removed");
}

#[test]
fn discovered_resource_mismatch_fails_closed_without_handle() {
    let path = database_path("environment-discovered-mismatch");
    let mut journal = Journal::open(&path).expect("journal must open");
    insert_run(&path);
    let discover_calls = Arc::new(Mutex::new(Vec::new()));
    let create_calls = Arc::new(Mutex::new(Vec::new()));
    let mut provider = DiscoveringProvider {
        database_path: path.clone(),
        discover_calls: Arc::clone(&discover_calls),
        create_calls: Arc::clone(&create_calls),
        expected_intent_id: "intent-env-discovered-mismatch".to_owned(),
        resource: ProviderResource {
            stable_provider_key: "fkst-local-qa/environment/v1/intent-env-discovered-mismatch"
                .to_owned(),
            labels: BTreeMap::from([
                (RUN_ID_LABEL.to_owned(), RUN_ID.to_owned()),
                (PROFILE_ID_LABEL.to_owned(), "profile-other".to_owned()),
                (
                    ENVIRONMENT_ID_LABEL.to_owned(),
                    "environment-001".to_owned(),
                ),
            ]),
            provider_identity: "provider-env-001".to_owned(),
        },
    };
    let clock = FixedClock::new(NOW).expect("fixture clock must be valid");

    assert!(reconcile_environment(
        &mut journal,
        &mut provider,
        &request("intent-env-discovered-mismatch", DEADLINE),
        &clock,
    )
    .is_err());
    assert_eq!(
        discover_calls.lock().unwrap().as_slice(),
        ["fkst-local-qa/environment/v1/intent-env-discovered-mismatch".to_owned()].as_slice()
    );
    assert!(create_calls.lock().unwrap().is_empty());
    assert_eq!(
        journal
            .owned_handle("intent-env-discovered-mismatch")
            .unwrap(),
        None
    );
    let status: String = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT status FROM resource_intents WHERE intent_id = 'intent-env-discovered-mismatch'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "prepared");
    std::fs::remove_file(path).expect("temporary database must be removed");
}

#[test]
fn empty_provider_identity_is_rejected_before_provider_effects() {
    let path = database_path("environment-empty-provider-identity");
    let mut journal = Journal::open(&path).expect("journal must open");
    insert_run(&path);
    let discover_calls = Arc::new(Mutex::new(Vec::new()));
    let create_calls = Arc::new(Mutex::new(Vec::new()));
    let mut provider = FakeProvider {
        database_path: path.clone(),
        discover_calls: Arc::clone(&discover_calls),
        create_calls: Arc::clone(&create_calls),
        provider_identity: "provider-env-001".to_owned(),
        expected_intent_id: "intent-env-empty-provider-identity".to_owned(),
        mismatch: false,
    };
    let mut empty_request = request("intent-env-empty-provider-identity", DEADLINE);
    empty_request.provider_identity.clear();
    let clock = FixedClock::new(NOW).expect("fixture clock must be valid");

    assert!(reconcile_environment(&mut journal, &mut provider, &empty_request, &clock,).is_err());
    assert!(discover_calls.lock().unwrap().is_empty());
    assert!(create_calls.lock().unwrap().is_empty());
    let intent_count: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM resource_intents WHERE intent_id = 'intent-env-empty-provider-identity'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(intent_count, 0);
    std::fs::remove_file(path).expect("temporary database must be removed");
}
