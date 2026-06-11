//! Startup index creation for the journal collections (idempotent
//! `create_index` with stable, explicit names — CANON: indexes ensured at
//! startup, safe across restarts and concurrent pod starts).

use bson::doc;
use mongodb::options::IndexOptions;
use mongodb::IndexModel;

use crate::journal::model::{
    RunJournalDoc, SessionProgressDoc, RUN_JOURNALS_COLLECTION, SESSION_PROGRESS_COLLECTION,
};

/// `{ session_id: 1, seq: 1 }` — per-session ordered replay.
pub const IDX_SP_SESSION_SEQ: &str = "sp_session_seq";
/// `{ run_key: 1, idem_key: 1 }`, UNIQUE, PARTIAL on `idem_key $exists`:
/// the local enforcement of cross-pod idempotency. Lifecycle docs OMIT
/// `idem_key` entirely and are excluded (a stored `null` would still satisfy
/// `$exists` and wrongly collide them all).
pub const IDX_SP_RUN_IDEM_UNIQ: &str = "sp_run_idem_uniq";
/// `{ package_name: 1 }` on `session_progress`.
pub const IDX_SP_PACKAGE: &str = "sp_package";
/// `{ recorded_at: 1 }` on `session_progress`.
pub const IDX_SP_RECORDED_AT: &str = "sp_recorded_at";
/// `{ package_name: 1 }` on `run_journals`.
pub const IDX_RJ_PACKAGE: &str = "rj_package";
/// `{ "github.repo": 1, "github.journal_path": 1 }` on `run_journals`.
pub const IDX_RJ_GITHUB_PATH: &str = "rj_github_path";

/// Build a plain ascending index with a stable name.
fn index_model(keys: bson::Document, name: &str) -> IndexModel {
    IndexModel::builder()
        .keys(keys)
        .options(IndexOptions::builder().name(name.to_string()).build())
        .build()
}

/// Idempotently ensure every journal index. Safe to re-run on an
/// already-indexed database (MongoDB de-duplicates by name + spec); never
/// drops or alters existing indexes.
pub async fn ensure_journal_indexes(
    database: &mongodb::Database,
) -> Result<(), mongodb::error::Error> {
    let progress = database.collection::<SessionProgressDoc>(SESSION_PROGRESS_COLLECTION);
    let unique_partial = IndexModel::builder()
        .keys(doc! { "run_key": 1, "idem_key": 1 })
        .options(
            IndexOptions::builder()
                .name(IDX_SP_RUN_IDEM_UNIQ.to_string())
                .unique(true)
                .partial_filter_expression(doc! { "idem_key": { "$exists": true } })
                .build(),
        )
        .build();
    progress
        .create_indexes([
            index_model(doc! { "session_id": 1, "seq": 1 }, IDX_SP_SESSION_SEQ),
            unique_partial,
            index_model(doc! { "package_name": 1 }, IDX_SP_PACKAGE),
            index_model(doc! { "recorded_at": 1 }, IDX_SP_RECORDED_AT),
        ])
        .await?;
    for name in [
        IDX_SP_SESSION_SEQ,
        IDX_SP_RUN_IDEM_UNIQ,
        IDX_SP_PACKAGE,
        IDX_SP_RECORDED_AT,
    ] {
        tracing::debug!(
            collection = SESSION_PROGRESS_COLLECTION,
            index = name,
            "index ensured"
        );
    }

    let journals = database.collection::<RunJournalDoc>(RUN_JOURNALS_COLLECTION);
    journals
        .create_indexes([
            index_model(doc! { "package_name": 1 }, IDX_RJ_PACKAGE),
            index_model(
                doc! { "github.repo": 1, "github.journal_path": 1 },
                IDX_RJ_GITHUB_PATH,
            ),
        ])
        .await?;
    for name in [IDX_RJ_PACKAGE, IDX_RJ_GITHUB_PATH] {
        tracing::debug!(
            collection = RUN_JOURNALS_COLLECTION,
            index = name,
            "index ensured"
        );
    }

    // No INFO here: the single "journal indexes ensured" lifecycle line is
    // emitted by the caller (main.rs) so it appears exactly once.
    Ok(())
}
