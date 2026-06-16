//! Persistence seam for the journaler: a small trait over the two journal
//! collections so the `Journaler` core is unit-testable without a live
//! MongoDB, plus the production Mongo implementation.
//!
//! The duplicate-key path is part of the CONTRACT, not an error: the
//! `sp_run_idem_uniq` unique partial index firing (`E11000`) is the local
//! idempotency guarantee, surfaced as [`InsertOutcome::Duplicate`].

use std::future::Future;

use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::options::ReplaceOptions;
use mongodb::Collection;

use crate::journal::model::{
    RunJournalDoc, SessionProgressDoc, RUN_JOURNALS_COLLECTION, SESSION_PROGRESS_COLLECTION,
};
use crate::journal::JournalError;

/// Outcome of one progress-document insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    /// A new document was written (first observation of this signal).
    Inserted,
    /// The unique `run_key+idem_key` index fired: already journaled by this
    /// or a prior session — a benign no-op.
    Duplicate,
}

/// Storage operations the journaler needs. Implemented by
/// [`MongoProgressStore`] in production and by an in-memory store in tests.
pub trait ProgressStore: Send + Sync {
    /// Insert one progress document; a duplicate-key violation is the
    /// idempotency guarantee firing and maps to `Ok(Duplicate)`. Any other
    /// write failure propagates as [`JournalError::Mongo`].
    fn insert_progress(
        &self,
        doc: &SessionProgressDoc,
    ) -> impl Future<Output = Result<InsertOutcome, JournalError>> + Send;

    /// Fetch the journal head for a logical run.
    fn get_run_journal(
        &self,
        run_key: &str,
    ) -> impl Future<Output = Result<Option<RunJournalDoc>, JournalError>> + Send;

    /// Upsert (replace-or-insert) the journal head.
    fn upsert_run_journal(
        &self,
        doc: &RunJournalDoc,
    ) -> impl Future<Output = Result<(), JournalError>> + Send;
}

/// True for a MongoDB duplicate-key failure (`E11000`, code 11000) in either
/// the write-error or command-error shape.
pub fn is_duplicate_key(error: &mongodb::error::Error) -> bool {
    match &*error.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => write_error.code == 11000,
        ErrorKind::Command(command_error) => command_error.code == 11000,
        _ => false,
    }
}

/// Production store over the `session_progress` + `run_journals` collections.
#[derive(Debug, Clone)]
pub struct MongoProgressStore {
    progress: Collection<SessionProgressDoc>,
    journals: Collection<RunJournalDoc>,
}

impl MongoProgressStore {
    pub fn new(database: &mongodb::Database) -> Self {
        Self {
            progress: database.collection(SESSION_PROGRESS_COLLECTION),
            journals: database.collection(RUN_JOURNALS_COLLECTION),
        }
    }
}

impl ProgressStore for MongoProgressStore {
    async fn insert_progress(
        &self,
        doc: &SessionProgressDoc,
    ) -> Result<InsertOutcome, JournalError> {
        match self.progress.insert_one(doc).await {
            Ok(_) => Ok(InsertOutcome::Inserted),
            Err(error) if is_duplicate_key(&error) => {
                tracing::debug!(
                    session_id = %doc.session_id,
                    run_key = %doc.run_key,
                    idem_key = ?doc.idem_key,
                    "progress insert deduplicated (E11000, benign)"
                );
                Ok(InsertOutcome::Duplicate)
            }
            Err(error) => {
                tracing::error!(
                    session_id = %doc.session_id,
                    run_key = %doc.run_key,
                    idem_key = ?doc.idem_key,
                    error = %error,
                    "progress insert failed"
                );
                Err(JournalError::Mongo(error))
            }
        }
    }

    async fn get_run_journal(&self, run_key: &str) -> Result<Option<RunJournalDoc>, JournalError> {
        self.journals
            .find_one(bson::doc! { "_id": run_key })
            .await
            .map_err(|error| {
                tracing::error!(run_key = %run_key, error = %error, "run journal read failed");
                JournalError::Mongo(error)
            })
    }

    async fn upsert_run_journal(&self, doc: &RunJournalDoc) -> Result<(), JournalError> {
        self.journals
            .replace_one(bson::doc! { "_id": &doc.run_key }, doc)
            .with_options(ReplaceOptions::builder().upsert(true).build())
            .await
            .map_err(|error| {
                tracing::error!(run_key = %doc.run_key, error = %error, "run journal upsert failed");
                JournalError::Mongo(error)
            })?;
        Ok(())
    }
}
