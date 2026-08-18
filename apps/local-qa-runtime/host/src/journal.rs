use std::path::Path;
use std::time::Duration;

use fkst_qa_contracts::{
    validate_cancel_disposition, validate_event_sequence, validate_execution_outcome,
    validate_local_state, validate_scalar,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::executor::ExecutionOutcome;
use crate::RunError;

pub struct Journal {
    pub(crate) connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceIntent {
    pub intent_id: String,
    pub run_id: String,
    pub profile_id: String,
    pub environment_id: String,
    pub generation: i64,
    pub deadline_utc: String,
    pub stable_provider_key: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedHandle {
    pub intent_id: String,
    pub run_id: String,
    pub profile_id: String,
    pub environment_id: String,
    pub generation: i64,
    pub deadline_utc: String,
    pub stable_provider_key: String,
    pub provider_identity: String,
    pub state: String,
}

pub(crate) enum Admission {
    Created(Vec<u8>),
    Replay(Vec<u8>),
    DifferentKey,
}

pub(crate) struct RunSnapshot {
    pub(crate) state: String,
    pub(crate) execution_outcome: Option<String>,
    pub(crate) latest_event_sequence: i64,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum EventPayload {
    State(RunStateEvent),
    Completed(RunCompletedEvent),
}

pub(crate) struct StoredEvent {
    pub(crate) sequence: i64,
    pub(crate) event_type: String,
    pub(crate) event: EventPayload,
}

pub(crate) enum Cancellation {
    Accepted(i64),
    AlreadyAccepted(i64),
    Terminal(i64),
    NotFound,
}

pub(crate) struct ClaimedRun {
    pub(crate) run_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunStateEvent {
    run_id: String,
    state: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunCompletedEvent {
    run_id: String,
    state: String,
    execution_outcome: String,
}

impl Journal {
    pub fn open(path: &Path) -> Result<Self, RunError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        let foreign_keys: i64 =
            connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        if foreign_keys != 1 {
            return Err(RunError::InvalidJournal("SQLite foreign keys unavailable"));
        }

        let journal_mode: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            connection.pragma_update(None, "journal_mode", "WAL")?;
            let confirmed_mode: String =
                connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
            if !confirmed_mode.eq_ignore_ascii_case("wal") {
                return Err(RunError::JournalMode(confirmed_mode));
            }
        }

        let mut journal = Self { connection };
        journal.migrate()?;
        Ok(journal)
    }

    fn migrate(&mut self) -> Result<(), RunError> {
        let version: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                let transaction = self
                    .connection
                    .transaction_with_behavior(TransactionBehavior::Immediate)?;
                transaction.execute_batch(
                    "CREATE TABLE accepted_requests (
                        run_id TEXT PRIMARY KEY NOT NULL,
                        idempotency_key TEXT NOT NULL,
                        request_digest TEXT NOT NULL,
                        response_json BLOB NOT NULL,
                        UNIQUE (run_id, idempotency_key)
                    );
                    CREATE TABLE runs (
                        run_id TEXT PRIMARY KEY NOT NULL,
                        state TEXT NOT NULL
                    );
                    CREATE TABLE events (
                        run_id TEXT NOT NULL,
                        sequence INTEGER NOT NULL,
                        event_type TEXT NOT NULL,
                        event_json TEXT NOT NULL,
                        PRIMARY KEY (run_id, sequence),
                        FOREIGN KEY (run_id) REFERENCES runs(run_id)
                    );
                    PRAGMA user_version = 1;",
                )?;
                transaction.commit()?;
                self.migrate_v2()?;
                self.migrate_v3()
            }
            1 => {
                self.migrate_v2()?;
                self.migrate_v3()
            }
            2 => self.migrate_v3(),
            3 => self.migrate_v4(),
            other => Err(RunError::UnsupportedDatabaseVersion(other)),
        }
    }

    fn migrate_v2(&mut self) -> Result<(), RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE cancel_requests (
                run_id TEXT PRIMARY KEY NOT NULL,
                idempotency_key TEXT NOT NULL,
                event_sequence INTEGER NOT NULL,
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );
            PRAGMA user_version = 2;",
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn migrate_v3(&mut self) -> Result<(), RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "ALTER TABLE runs ADD COLUMN execution_outcome TEXT
                CHECK (
                    execution_outcome IS NULL OR execution_outcome IN
                    ('passed', 'failed', 'cancelled', 'timed_out', 'lost', 'blocked')
                );
            CREATE TABLE execution_attempts (
                run_id TEXT PRIMARY KEY NOT NULL,
                status TEXT NOT NULL CHECK (status IN ('claimed', 'completed')),
                execution_outcome TEXT
                    CHECK (
                        execution_outcome IS NULL OR execution_outcome IN
                        ('passed', 'failed', 'cancelled', 'timed_out', 'lost', 'blocked')
                    ),
                CHECK (
                    (status = 'claimed' AND execution_outcome IS NULL) OR
                    (status = 'completed' AND execution_outcome IS NOT NULL)
                ),
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );
            PRAGMA user_version = 3;",
        )?;
        transaction.commit()?;
        self.migrate_v4()
    }

    fn migrate_v4(&mut self) -> Result<(), RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE resource_intents (
                intent_id TEXT PRIMARY KEY NOT NULL,
                run_id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                environment_id TEXT NOT NULL,
                generation INTEGER NOT NULL CHECK (generation > 0),
                deadline_utc TEXT NOT NULL,
                stable_provider_key TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL CHECK (status IN ('prepared', 'bound')),
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );
            CREATE TABLE owned_handles (
                intent_id TEXT PRIMARY KEY NOT NULL,
                run_id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                environment_id TEXT NOT NULL,
                generation INTEGER NOT NULL CHECK (generation > 0),
                stable_provider_key TEXT NOT NULL UNIQUE,
                provider_identity TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state = 'active'),
                FOREIGN KEY (intent_id) REFERENCES resource_intents(intent_id),
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );
            PRAGMA user_version = 4;",
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn prepare_intent(
        &mut self,
        intent_id: &str,
        run_id: &str,
        profile_id: &str,
        environment_id: &str,
        generation: i64,
        deadline_utc: &str,
    ) -> Result<ResourceIntent, RunError> {
        validate_scalar("UUID", run_id)
            .map_err(|_| RunError::InvalidJournal("run_id must be a canonical UUID"))?;
        validate_scalar("ISO8601", deadline_utc)
            .map_err(|_| RunError::InvalidJournal("deadline_utc must be ISO8601"))?;
        if intent_id.is_empty()
            || profile_id.is_empty()
            || environment_id.is_empty()
            || generation <= 0
        {
            return Err(RunError::InvalidJournal("invalid resource intent"));
        }
        let stable_provider_key = format!("fkst-local-qa/environment/v1/{intent_id}");
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT intent_id, run_id, profile_id, environment_id, generation,
                        deadline_utc, stable_provider_key, status
                 FROM resource_intents WHERE intent_id = ?1",
                [intent_id],
                resource_intent_from_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing.run_id != run_id
                || existing.profile_id != profile_id
                || existing.environment_id != environment_id
                || existing.generation != generation
                || existing.deadline_utc != deadline_utc
                || existing.stable_provider_key != stable_provider_key
            {
                return Err(RunError::InvalidJournal("conflicting resource intent"));
            }
            transaction.commit()?;
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO resource_intents
             (intent_id, run_id, profile_id, environment_id, generation, deadline_utc,
              stable_provider_key, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'prepared')",
            params![
                intent_id,
                run_id,
                profile_id,
                environment_id,
                generation,
                deadline_utc,
                stable_provider_key
            ],
        )?;
        transaction.commit()?;
        Ok(ResourceIntent {
            intent_id: intent_id.to_owned(),
            run_id: run_id.to_owned(),
            profile_id: profile_id.to_owned(),
            environment_id: environment_id.to_owned(),
            generation,
            deadline_utc: deadline_utc.to_owned(),
            stable_provider_key,
            status: "prepared".to_owned(),
        })
    }

    pub fn owned_handle(&self, intent_id: &str) -> Result<Option<OwnedHandle>, RunError> {
        Ok(self
            .connection
            .query_row(
                "SELECT owned_handles.intent_id, owned_handles.run_id,
                        owned_handles.profile_id, owned_handles.environment_id,
                        owned_handles.generation, resource_intents.deadline_utc,
                        owned_handles.stable_provider_key, owned_handles.provider_identity,
                        owned_handles.state
                 FROM owned_handles
                 JOIN resource_intents ON resource_intents.intent_id = owned_handles.intent_id
                 WHERE owned_handles.intent_id = ?1",
                [intent_id],
                owned_handle_from_row,
            )
            .optional()?)
    }

    pub fn record_handle(&mut self, handle: &OwnedHandle) -> Result<OwnedHandle, RunError> {
        if handle.state != "active"
            || handle.intent_id.is_empty()
            || handle.provider_identity.is_empty()
            || handle.generation <= 0
        {
            return Err(RunError::InvalidJournal("invalid owned handle"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let intent = transaction
            .query_row(
                "SELECT intent_id, run_id, profile_id, environment_id, generation,
                        deadline_utc, stable_provider_key, status
                 FROM resource_intents WHERE intent_id = ?1",
                [&handle.intent_id],
                resource_intent_from_row,
            )
            .optional()?
            .ok_or(RunError::InvalidJournal("resource intent is missing"))?;
        if intent.run_id != handle.run_id
            || intent.profile_id != handle.profile_id
            || intent.environment_id != handle.environment_id
            || intent.generation != handle.generation
            || intent.deadline_utc != handle.deadline_utc
            || intent.stable_provider_key != handle.stable_provider_key
        {
            return Err(RunError::InvalidJournal("owned handle does not match intent"));
        }
        let existing = transaction
            .query_row(
                "SELECT owned_handles.intent_id, owned_handles.run_id,
                        owned_handles.profile_id, owned_handles.environment_id,
                        owned_handles.generation, resource_intents.deadline_utc,
                        owned_handles.stable_provider_key, owned_handles.provider_identity,
                        owned_handles.state
                 FROM owned_handles
                 JOIN resource_intents ON resource_intents.intent_id = owned_handles.intent_id
                 WHERE owned_handles.intent_id = ?1",
                [&handle.intent_id],
                owned_handle_from_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != *handle {
                return Err(RunError::InvalidJournal("conflicting owned handle"));
            }
            transaction.commit()?;
            return Ok(existing);
        }
        if intent.status != "prepared" {
            return Err(RunError::InvalidJournal("resource intent is already bound"));
        }
        transaction.execute(
            "INSERT INTO owned_handles
             (intent_id, run_id, profile_id, environment_id, generation,
              stable_provider_key, provider_identity, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active')",
            params![
                handle.intent_id,
                handle.run_id,
                handle.profile_id,
                handle.environment_id,
                handle.generation,
                handle.stable_provider_key,
                handle.provider_identity
            ],
        )?;
        transaction.execute(
            "UPDATE resource_intents SET status = 'bound' WHERE intent_id = ?1",
            [&handle.intent_id],
        )?;
        transaction.commit()?;
        Ok(handle.clone())
    }
    pub(crate) fn admit(
        &mut self,
        run_id: &str,
        idempotency_key: &str,
        request_digest: &str,
    ) -> Result<Admission, RunError> {
        validate_state("accepted")?;
        validate_sequence(1)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored = transaction
            .query_row(
                "SELECT idempotency_key, request_digest, response_json
                 FROM accepted_requests WHERE run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((stored_key, stored_digest, response_json)) = stored {
            let admission = if stored_key == idempotency_key && stored_digest == request_digest {
                Admission::Replay(response_json)
            } else {
                Admission::DifferentKey
            };
            transaction.commit()?;
            return Ok(admission);
        }

        let existing_run: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = ?1)",
            [run_id],
            |row| row.get(0),
        )?;
        if existing_run {
            transaction.commit()?;
            return Ok(Admission::DifferentKey);
        }

        let response_json =
            format!("{{\"run_id\":\"{run_id}\",\"state\":\"accepted\",\"event_sequence\":1}}\n")
                .into_bytes();
        let event_json = state_event_json(run_id, "accepted")?;
        transaction.execute(
            "INSERT INTO accepted_requests
             (run_id, idempotency_key, request_digest, response_json)
             VALUES (?1, ?2, ?3, ?4)",
            params![run_id, idempotency_key, request_digest, response_json],
        )?;
        transaction.execute(
            "INSERT INTO runs (run_id, state, execution_outcome)
             VALUES (?1, 'accepted', NULL)",
            [run_id],
        )?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, event_type, event_json)
             VALUES (?1, 1, 'run.accepted', ?2)",
            params![run_id, event_json],
        )?;
        transaction.commit()?;
        Ok(Admission::Created(response_json))
    }

    pub(crate) fn snapshot(&self, run_id: &str) -> Result<Option<RunSnapshot>, RunError> {
        let snapshot = self
            .connection
            .query_row(
                "SELECT runs.state, runs.execution_outcome, MAX(events.sequence)
                 FROM runs JOIN events ON events.run_id = runs.run_id
                 WHERE runs.run_id = ?1
                 GROUP BY runs.run_id, runs.state, runs.execution_outcome",
                [run_id],
                |row| {
                    Ok(RunSnapshot {
                        state: row.get(0)?,
                        execution_outcome: row.get(1)?,
                        latest_event_sequence: row.get(2)?,
                    })
                },
            )
            .optional()?;
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        validate_state(&snapshot.state)?;
        validate_sequence(snapshot.latest_event_sequence)?;
        match (
            snapshot.state.as_str(),
            snapshot.execution_outcome.as_deref(),
        ) {
            ("terminal", Some(outcome)) => validate_outcome(outcome)?,
            ("terminal", None) => {
                return Err(RunError::InvalidJournal(
                    "terminal Run is missing execution outcome",
                ))
            }
            (_, Some(_)) => {
                return Err(RunError::InvalidJournal(
                    "preterminal Run has an execution outcome",
                ))
            }
            (_, None) => {}
        }
        Ok(Some(snapshot))
    }

    pub(crate) fn events(
        &self,
        run_id: &str,
        after: i64,
        limit: i64,
    ) -> Result<Option<Vec<StoredEvent>>, RunError> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE run_id = ?1)",
            [run_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(None);
        }

        let mut statement = self.connection.prepare(
            "SELECT sequence, event_type, event_json
             FROM events
             WHERE run_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC
             LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![run_id, after, limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut events = Vec::with_capacity(rows.len());
        for (sequence, event_type, event_json) in rows {
            validate_sequence(sequence)?;
            let event = parse_event(run_id, &event_type, &event_json)?;
            events.push(StoredEvent {
                sequence,
                event_type,
                event,
            });
        }
        Ok(Some(events))
    }

    pub(crate) fn cancel(
        &mut self,
        run_id: &str,
        idempotency_key: &str,
    ) -> Result<Cancellation, RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = transaction
            .query_row(
                "SELECT runs.state, runs.execution_outcome, MAX(events.sequence)
                 FROM runs JOIN events ON events.run_id = runs.run_id
                 WHERE runs.run_id = ?1
                 GROUP BY runs.run_id, runs.state, runs.execution_outcome",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((state, execution_outcome, latest_sequence)) = run else {
            transaction.commit()?;
            return Ok(Cancellation::NotFound);
        };
        validate_state(&state)?;
        validate_sequence(latest_sequence)?;

        if state == "terminal" {
            let outcome = execution_outcome.ok_or(RunError::InvalidJournal(
                "terminal Run is missing execution outcome",
            ))?;
            validate_outcome(&outcome)?;
            validate_disposition("terminal")?;
            transaction.commit()?;
            return Ok(Cancellation::Terminal(latest_sequence));
        }
        if execution_outcome.is_some() {
            return Err(RunError::InvalidJournal(
                "preterminal Run has an execution outcome",
            ));
        }

        let existing_sequence = transaction
            .query_row(
                "SELECT event_sequence FROM cancel_requests WHERE run_id = ?1",
                [run_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(event_sequence) = existing_sequence {
            validate_sequence(event_sequence)?;
            validate_disposition("already_accepted")?;
            transaction.commit()?;
            return Ok(Cancellation::AlreadyAccepted(event_sequence));
        }

        let active_attempt: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_attempts WHERE run_id = ?1)",
            [run_id],
            |row| row.get(0),
        )?;
        if active_attempt {
            return Err(RunError::ActiveAttempt);
        }

        let event_sequence = latest_sequence
            .checked_add(1)
            .ok_or(RunError::InvalidJournal("Event sequence overflow"))?;
        validate_sequence(event_sequence)?;
        validate_disposition("accepted")?;
        let event_json = state_event_json(run_id, &state)?;
        transaction.execute(
            "INSERT INTO cancel_requests (run_id, idempotency_key, event_sequence)
             VALUES (?1, ?2, ?3)",
            params![run_id, idempotency_key, event_sequence],
        )?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, event_type, event_json)
             VALUES (?1, ?2, 'run.cancel_requested', ?3)",
            params![run_id, event_sequence, event_json],
        )?;
        transaction.commit()?;
        Ok(Cancellation::Accepted(event_sequence))
    }

    pub(crate) fn claim_next(&mut self) -> Result<Option<ClaimedRun>, RunError> {
        validate_state("accepted")?;
        validate_state("preparing")?;
        validate_sequence(2)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run_id = transaction
            .query_row(
                "SELECT runs.run_id
                 FROM runs
                 WHERE runs.state = 'accepted'
                   AND runs.execution_outcome IS NULL
                   AND NOT EXISTS (
                       SELECT 1 FROM execution_attempts
                       WHERE execution_attempts.run_id = runs.run_id
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM cancel_requests
                       WHERE cancel_requests.run_id = runs.run_id
                   )
                 ORDER BY runs.rowid
                 LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(run_id) = run_id else {
            transaction.commit()?;
            return Ok(None);
        };

        let latest_sequence: i64 = transaction.query_row(
            "SELECT MAX(sequence) FROM events WHERE run_id = ?1",
            [&run_id],
            |row| row.get(0),
        )?;
        if latest_sequence != 1 {
            return Err(RunError::InvalidJournal(
                "accepted Run has an unexpected Event sequence",
            ));
        }

        transaction.execute(
            "INSERT INTO execution_attempts (run_id, status, execution_outcome)
             VALUES (?1, 'claimed', NULL)",
            [&run_id],
        )?;
        let updated = transaction.execute(
            "UPDATE runs SET state = 'preparing'
             WHERE run_id = ?1 AND state = 'accepted' AND execution_outcome IS NULL",
            [&run_id],
        )?;
        if updated != 1 {
            return Err(RunError::InvalidJournal(
                "Run claim lost its state predicate",
            ));
        }
        let event_json = state_event_json(&run_id, "preparing")?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, event_type, event_json)
             VALUES (?1, 2, 'run.state_changed', ?2)",
            params![run_id, event_json],
        )?;
        transaction.commit()?;
        Ok(Some(ClaimedRun { run_id }))
    }

    pub(crate) fn transition(
        &mut self,
        run_id: &str,
        expected_state: &str,
        next_state: &str,
        event_sequence: i64,
    ) -> Result<(), RunError> {
        validate_state(expected_state)?;
        validate_state(next_state)?;
        validate_sequence(event_sequence)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, execution_outcome, latest_sequence, attempt_status, cancellation_exists) =
            transaction.query_row(
                "SELECT runs.state,
                        runs.execution_outcome,
                        (SELECT MAX(sequence) FROM events WHERE events.run_id = runs.run_id),
                        (SELECT status FROM execution_attempts
                         WHERE execution_attempts.run_id = runs.run_id),
                        EXISTS(SELECT 1 FROM cancel_requests
                               WHERE cancel_requests.run_id = runs.run_id)
                 FROM runs WHERE runs.run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )?;
        if state != expected_state
            || execution_outcome.is_some()
            || latest_sequence != event_sequence - 1
            || attempt_status.as_deref() != Some("claimed")
            || cancellation_exists
        {
            return Err(RunError::InvalidJournal(
                "Run transition predicate did not match",
            ));
        }

        let updated = transaction.execute(
            "UPDATE runs SET state = ?2
             WHERE run_id = ?1 AND state = ?3 AND execution_outcome IS NULL",
            params![run_id, next_state, expected_state],
        )?;
        if updated != 1 {
            return Err(RunError::InvalidJournal(
                "Run transition lost its state predicate",
            ));
        }
        let event_json = state_event_json(run_id, next_state)?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, event_type, event_json)
             VALUES (?1, ?2, 'run.state_changed', ?3)",
            params![run_id, event_sequence, event_json],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn complete(
        &mut self,
        run_id: &str,
        outcome: &ExecutionOutcome,
    ) -> Result<(), RunError> {
        validate_state("finalizing_local")?;
        validate_state("terminal")?;
        validate_outcome(outcome.as_str())?;
        validate_sequence(9)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, execution_outcome, latest_sequence, attempt_status, attempt_outcome) =
            transaction.query_row(
                "SELECT runs.state,
                        runs.execution_outcome,
                        (SELECT MAX(sequence) FROM events WHERE events.run_id = runs.run_id),
                        execution_attempts.status,
                        execution_attempts.execution_outcome
                 FROM runs JOIN execution_attempts
                   ON execution_attempts.run_id = runs.run_id
                 WHERE runs.run_id = ?1",
                [run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )?;
        if state != "finalizing_local"
            || execution_outcome.is_some()
            || latest_sequence != 8
            || attempt_status != "claimed"
            || attempt_outcome.is_some()
        {
            return Err(RunError::InvalidJournal(
                "Run completion predicate did not match",
            ));
        }

        let attempt_updated = transaction.execute(
            "UPDATE execution_attempts
             SET status = 'completed', execution_outcome = ?2
             WHERE run_id = ?1 AND status = 'claimed' AND execution_outcome IS NULL",
            params![run_id, outcome.as_str()],
        )?;
        let run_updated = transaction.execute(
            "UPDATE runs SET state = 'terminal', execution_outcome = ?2
             WHERE run_id = ?1 AND state = 'finalizing_local'
               AND execution_outcome IS NULL",
            params![run_id, outcome.as_str()],
        )?;
        if attempt_updated != 1 || run_updated != 1 {
            return Err(RunError::InvalidJournal(
                "Run completion lost its update predicate",
            ));
        }
        let event_json = completed_event_json(run_id, outcome.as_str())?;
        transaction.execute(
            "INSERT INTO events (run_id, sequence, event_type, event_json)
             VALUES (?1, 9, 'run.completed', ?2)",
            params![run_id, event_json],
        )?;
        transaction.commit()?;
        Ok(())
    }
}

fn state_event_json(run_id: &str, state: &str) -> Result<String, RunError> {
    validate_state(state)?;
    serde_json::to_string(&RunStateEvent {
        run_id: run_id.to_owned(),
        state: state.to_owned(),
    })
    .map_err(|_| RunError::InvalidJournal("state Event serialization failed"))
}

fn completed_event_json(run_id: &str, outcome: &str) -> Result<String, RunError> {
    validate_state("terminal")?;
    validate_outcome(outcome)?;
    serde_json::to_string(&RunCompletedEvent {
        run_id: run_id.to_owned(),
        state: "terminal".to_owned(),
        execution_outcome: outcome.to_owned(),
    })
    .map_err(|_| RunError::InvalidJournal("completed Event serialization failed"))
}

fn parse_event(run_id: &str, event_type: &str, event_json: &str) -> Result<EventPayload, RunError> {
    match event_type {
        "run.accepted" | "run.state_changed" | "run.cancel_requested" => {
            let event = serde_json::from_str::<RunStateEvent>(event_json)
                .map_err(|_| RunError::InvalidJournal("invalid state Event JSON"))?;
            if event.run_id != run_id {
                return Err(RunError::InvalidJournal("Event Run ID mismatch"));
            }
            validate_state(&event.state)?;
            if event_type == "run.accepted" && event.state != "accepted" {
                return Err(RunError::InvalidJournal("invalid accepted Event state"));
            }
            Ok(EventPayload::State(event))
        }
        "run.completed" => {
            let event = serde_json::from_str::<RunCompletedEvent>(event_json)
                .map_err(|_| RunError::InvalidJournal("invalid completed Event JSON"))?;
            if event.run_id != run_id || event.state != "terminal" {
                return Err(RunError::InvalidJournal("invalid completed Event identity"));
            }
            validate_state(&event.state)?;
            validate_outcome(&event.execution_outcome)?;
            Ok(EventPayload::Completed(event))
        }
        _ => Err(RunError::InvalidJournal("unknown Event type")),
    }
}

fn validate_state(value: &str) -> Result<(), RunError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|_| RunError::Contract("LocalState serialization failed"))?;
    validate_local_state(&encoded).map_err(|_| RunError::Contract("invalid LocalState"))?;
    Ok(())
}

fn validate_outcome(value: &str) -> Result<(), RunError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|_| RunError::Contract("ExecutionOutcome serialization failed"))?;
    validate_execution_outcome(&encoded)
        .map_err(|_| RunError::Contract("invalid ExecutionOutcome"))?;
    Ok(())
}

fn validate_disposition(value: &str) -> Result<(), RunError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|_| RunError::Contract("CancelDisposition serialization failed"))?;
    validate_cancel_disposition(&encoded)
        .map_err(|_| RunError::Contract("invalid CancelDisposition"))?;
    Ok(())
}

fn validate_sequence(value: i64) -> Result<(), RunError> {
    validate_event_sequence(value.to_string().as_bytes())
        .map_err(|_| RunError::Contract("invalid EventSequence"))?;
    Ok(())
}

fn resource_intent_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ResourceIntent> {
    Ok(ResourceIntent {
        intent_id: row.get(0)?,
        run_id: row.get(1)?,
        profile_id: row.get(2)?,
        environment_id: row.get(3)?,
        generation: row.get(4)?,
        deadline_utc: row.get(5)?,
        stable_provider_key: row.get(6)?,
        status: row.get(7)?,
    })
}

fn owned_handle_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<OwnedHandle> {
    Ok(OwnedHandle {
        intent_id: row.get(0)?,
        run_id: row.get(1)?,
        profile_id: row.get(2)?,
        environment_id: row.get(3)?,
        generation: row.get(4)?,
        deadline_utc: row.get(5)?,
        stable_provider_key: row.get(6)?,
        provider_identity: row.get(7)?,
        state: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{Admission, Journal};
    use crate::CANONICAL_REQUEST_DIGEST;

    #[test]
    fn same_key_with_a_different_digest_is_conflict_without_mutation() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "fkst-local-qa-host-digest-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("temporary directory must be created");
        let database_path = directory.join("journal.sqlite");

        {
            let mut journal = Journal::open(&database_path).expect("journal must open");
            assert!(matches!(
                journal.admit("run-001", "idem-001", CANONICAL_REQUEST_DIGEST),
                Ok(Admission::Created(_))
            ));
            assert!(matches!(
                journal.admit("run-001", "idem-001", "different-digest"),
                Ok(Admission::DifferentKey)
            ));
            assert_eq!(
                journal
                    .connection
                    .query_row("SELECT COUNT(*) FROM accepted_requests", [], |row| row
                        .get::<_, i64>(0))
                    .expect("accepted count must be readable"),
                1
            );
            assert_eq!(
                journal
                    .connection
                    .query_row("SELECT COUNT(*) FROM events", [], |row| row
                        .get::<_, i64>(0))
                    .expect("Event count must be readable"),
                1
            );
        }

        fs::remove_dir_all(directory).expect("temporary directory must be removed");
    }
}
