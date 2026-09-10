//! Durable local workspace consistency, independent of provider-writable markers.
//! Internally supplied source facts are not authenticated production lease authority.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::Journal;
use crate::source_workspace::{ImmutableRevision, WorkspaceResource};
use crate::RunError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceIntent {
    pub stable_key: String,
    pub run_id: String,
    pub generation: i64,
    pub source_lease_id: String,
    pub source_object_id: String,
    pub source_digest: String,
    pub immutable_revision: ImmutableRevision,
    pub source_provider_identity: String,
    pub deadline_utc: String,
    pub relative_location: String,
    pub roots_identity: String,
    pub provider_scope: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    Prepared,
    DirectoryAttempted,
    DirectoryReady,
    CreateAttempted,
    Bound,
    StopAttempted,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedWorkspace {
    pub intent: WorkspaceIntent,
    pub state: WorkspaceState,
    pub directory_identity: Option<String>,
    pub resource: Option<WorkspaceResource>,
    pub blocker: Option<String>,
}

impl Journal {
    pub(super) fn migrate_v8(&mut self) -> Result<(), RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "CREATE TABLE workspace_ownership (
                stable_key TEXT PRIMARY KEY NOT NULL,
                run_id TEXT NOT NULL,
                generation INTEGER NOT NULL CHECK (generation > 0),
                intent_json TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN (
                    'prepared', 'directory_attempted', 'directory_ready',
                    'create_attempted', 'bound', 'stop_attempted', 'stopped')),
                directory_identity TEXT,
                resource_json TEXT,
                provider_scope TEXT NOT NULL,
                provider_identity TEXT,
                blocker TEXT,
                UNIQUE (run_id, generation),
                UNIQUE (provider_scope, provider_identity),
                CHECK ((resource_json IS NULL) = (provider_identity IS NULL)),
                CHECK (state NOT IN ('directory_ready','create_attempted','bound','stop_attempted','stopped')
                    OR directory_identity IS NOT NULL),
                CHECK (state NOT IN ('bound','stop_attempted','stopped') OR resource_json IS NOT NULL)
            );
            PRAGMA user_version = 8;",
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn workspace(&self, stable_key: &str) -> Result<Option<OwnedWorkspace>, RunError> {
        load(&self.connection, stable_key)
    }

    /// Includes incomplete and stopped records: filesystem cleanup may still be pending.
    pub fn workspaces(&self) -> Result<Vec<OwnedWorkspace>, RunError> {
        let mut statement = self
            .connection
            .prepare("SELECT stable_key FROM workspace_ownership ORDER BY stable_key")?;
        let keys = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        keys.iter()
            .map(|key| {
                self.workspace(key)?
                    .ok_or(RunError::InvalidJournal("workspace disappeared"))
            })
            .collect()
    }

    pub(crate) fn prepare_workspace(
        &mut self,
        intent: &WorkspaceIntent,
    ) -> Result<OwnedWorkspace, RunError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = load(&transaction, &intent.stable_key)? {
            if existing.intent != *intent {
                return Err(RunError::Lifecycle("conflicting durable workspace intent"));
            }
            transaction.commit()?;
            return Ok(existing);
        }
        transaction.execute(
            "INSERT INTO workspace_ownership (stable_key, run_id, generation, intent_json, state, provider_scope)
             VALUES (?1, ?2, ?3, ?4, 'prepared', ?5)",
            params![intent.stable_key, intent.run_id, intent.generation, encode(intent)?, intent.provider_scope],
        )?;
        transaction.commit()?;
        Ok(OwnedWorkspace {
            intent: intent.clone(),
            state: WorkspaceState::Prepared,
            directory_identity: None,
            resource: None,
            blocker: None,
        })
    }

    // Exact-record compare-and-swap serializes claims without a transaction across callbacks.
    pub(crate) fn update_workspace(
        &mut self,
        before: &OwnedWorkspace,
        after: &OwnedWorkspace,
    ) -> Result<OwnedWorkspace, RunError> {
        if before.intent != after.intent
            || (before.directory_identity.is_some()
                && before.directory_identity != after.directory_identity)
            || (before.resource.is_some() && before.resource != after.resource)
        {
            return Err(RunError::InvalidJournal("workspace ownership is immutable"));
        }
        use WorkspaceState::*;
        if before.state != after.state
            && !matches!(
                (before.state, after.state),
                (Prepared, DirectoryAttempted)
                    | (DirectoryAttempted, DirectoryReady)
                    | (DirectoryReady, CreateAttempted)
                    | (CreateAttempted, Bound)
                    | (Bound, StopAttempted)
                    | (Bound | StopAttempted, Stopped)
            )
        {
            return Err(RunError::InvalidJournal("invalid workspace transition"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if load(&transaction, &before.intent.stable_key)?.as_ref() != Some(before) {
            return Err(RunError::Lifecycle(
                "workspace changed concurrently; reconcile required",
            ));
        }
        transaction.execute(
            "UPDATE workspace_ownership SET state = ?2, directory_identity = ?3,
             resource_json = ?4, provider_identity = ?5, blocker = ?6 WHERE stable_key = ?1",
            params![
                after.intent.stable_key,
                state_name(after.state),
                after.directory_identity,
                after.resource.as_ref().map(encode).transpose()?,
                after
                    .resource
                    .as_ref()
                    .map(|resource| &resource.provider_identity),
                after.blocker
            ],
        )?;
        transaction.commit()?;
        Ok(after.clone())
    }
}

fn state_name(state: WorkspaceState) -> &'static str {
    match state {
        WorkspaceState::Prepared => "prepared",
        WorkspaceState::DirectoryAttempted => "directory_attempted",
        WorkspaceState::DirectoryReady => "directory_ready",
        WorkspaceState::CreateAttempted => "create_attempted",
        WorkspaceState::Bound => "bound",
        WorkspaceState::StopAttempted => "stop_attempted",
        WorkspaceState::Stopped => "stopped",
    }
}

fn encode<T: Serialize>(value: &T) -> Result<String, RunError> {
    serde_json::to_string(value)
        .map_err(|_| RunError::InvalidJournal("workspace serialization failed"))
}

fn load(connection: &rusqlite::Connection, key: &str) -> Result<Option<OwnedWorkspace>, RunError> {
    let row = connection.query_row(
        "SELECT intent_json, state, directory_identity, resource_json, blocker FROM workspace_ownership WHERE stable_key = ?1",
        [key], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?)),
    ).optional()?;
    row.map(|(intent, state, directory_identity, resource, blocker)| {
        Ok(OwnedWorkspace {
            intent: serde_json::from_str(&intent)
                .map_err(|_| RunError::InvalidJournal("invalid workspace intent"))?,
            state: serde_json::from_value(serde_json::Value::String(state))
                .map_err(|_| RunError::InvalidJournal("invalid workspace state"))?,
            directory_identity,
            resource: resource
                .map(|json| serde_json::from_str(&json))
                .transpose()
                .map_err(|_| RunError::InvalidJournal("invalid workspace resource"))?,
            blocker,
        })
    })
    .transpose()
}
