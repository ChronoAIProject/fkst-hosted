//! Storage-handle tests: permissions, restart persistence, concurrent writers,
//! and the fail-closed readiness latch.

use tempfile::TempDir;

use super::*;
use crate::audit_relay::test_support::{completion, now, settings, start, wire_instant};

#[tokio::test]
async fn a_committed_record_survives_a_restart() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    let event_id = "11111111-1111-4111-8111-111111111111";

    {
        let database = Database::open(&path, settings()).expect("opens");
        let body = start(event_id);
        let identity = body.to_identity().expect("valid start");
        database
            .write(move |transaction| ingest::register_start(transaction, &body, &identity, now()))
            .await
            .expect("start commits");
    }

    // A whole new process-equivalent handle over the same file.
    let reopened = Database::open(&path, settings()).expect("reopens");
    let found = reopened.read(ingest::record_count).await.expect("counts");
    assert_eq!(found, 1, "a committed record must survive a restart");
}

#[tokio::test]
async fn concurrent_writers_never_lose_or_corrupt_a_record() {
    // Two "replicas" writing unique ids plus a deliberately duplicated one,
    // through ONE relay. The unique ones must all land; the duplicate must land
    // exactly once and never as two rows.
    let (_dir, database) = crate::audit_relay::test_support::open_database();
    let duplicate = "99999999-9999-4999-8999-999999999999";
    let mut tasks = Vec::new();
    for replica in 0..2u8 {
        for index in 0..8u8 {
            let event_id = format!("{replica}{index}111111-1111-4111-8111-111111111111");
            let database = database.clone();
            tasks.push(tokio::spawn(async move {
                let body = start(&event_id);
                let identity = body.to_identity().expect("valid start");
                database
                    .write(move |tx| ingest::register_start(tx, &body, &identity, now()))
                    .await
            }));
        }
        let database = database.clone();
        let duplicate = duplicate.to_string();
        tasks.push(tokio::spawn(async move {
            let body = start(&duplicate);
            let identity = body.to_identity().expect("valid start");
            database
                .write(move |tx| ingest::register_start(tx, &body, &identity, now()))
                .await
        }));
    }
    let mut created = 0usize;
    let mut replayed = 0usize;
    for task in tasks {
        match task.await.expect("task joins").expect("write succeeds") {
            ingest::Ingested::Created(_) => created += 1,
            ingest::Ingested::Replayed(_) => replayed += 1,
        }
    }
    assert_eq!(
        created, 17,
        "16 unique ids plus one first write of the duplicate"
    );
    assert_eq!(
        replayed, 1,
        "the duplicate is acknowledged, never re-created"
    );

    let stored = database.read(ingest::record_count).await.expect("counts");
    assert_eq!(stored, 17);
}

#[cfg(unix)]
#[tokio::test]
async fn a_read_only_volume_cannot_be_opened_for_ingress() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    // A pre-existing, unwritable database file: the relay must refuse to start
    // rather than accept records it could never keep.
    std::fs::write(&path, b"").expect("creates the file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400))
        .expect("makes it read-only");
    let error = Database::open(&path, settings()).expect_err("a read-only file is refused");
    assert!(
        error.is_fatal_storage(),
        "an unwritable volume must be a fatal storage failure, got {error:?}"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("restores the file so the temp dir can be cleaned up");
}

#[tokio::test]
async fn a_fatal_storage_failure_latches_ingress_readiness_off() {
    // A failpoint injected INSIDE the transaction, which is exactly the shape a
    // read-only volume, a full disk, or a corrupt page takes at the call site.
    // Latching is permanent for the process: a relay that flapped back to
    // "ready" would tell `required` mode it could promise durability again.
    let (_dir, database) = super::super::test_support::open_database();
    assert!(database.ingress_ready());
    for injected in [
        DbError::Unavailable("disk_full"),
        DbError::Unavailable("corrupt"),
    ] {
        let outcome = database
            .write(move |_transaction| Err::<(), _>(injected))
            .await;
        assert_eq!(outcome.expect_err("the failpoint fires"), injected);
        assert!(!database.ingress_ready());
    }

    // A NON-fatal failure must not latch anything: a busy database is a retry.
    let (_dir, healthy) = super::super::test_support::open_database();
    let outcome = healthy
        .write(|_transaction| Err::<(), _>(DbError::Busy))
        .await;
    assert_eq!(outcome.expect_err("the failpoint fires"), DbError::Busy);
    assert!(healthy.ingress_ready());
}

#[tokio::test]
async fn a_corrupt_database_cannot_be_opened() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    std::fs::write(
        &path,
        b"this is not a sqlite database at all, not even close",
    )
    .expect("writes garbage");
    let error = Database::open(&path, settings()).expect_err("a corrupt file is refused");
    assert!(
        error.is_fatal_storage(),
        "a corrupt database must be a fatal storage failure, got {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn the_directory_is_0700_and_the_file_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("nested").join("audit.sqlite3");
    let _database = Database::open(&path, settings()).expect("opens");
    let dir_mode = std::fs::metadata(path.parent().expect("parent"))
        .expect("dir metadata")
        .permissions()
        .mode()
        & 0o777;
    let file_mode = std::fs::metadata(&path)
        .expect("file metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700, "the data directory must be 0700");
    assert_eq!(file_mode, 0o600, "the database file must be 0600");
}

#[tokio::test]
async fn the_stored_bytes_contain_no_credential_canary() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("audit.sqlite3");
    let database = Database::open(&path, settings()).expect("opens");
    let event_id = "11111111-1111-4111-8111-111111111111";
    let body = start(event_id);
    let identity = body.to_identity().expect("valid start");
    database
        .write(move |tx| ingest::register_start(tx, &body, &identity, now()))
        .await
        .expect("start commits");
    let terminal = completion(event_id, Some(101));
    database
        .write(move |tx| {
            ingest::commit_completion(tx, &terminal, wire_instant(&terminal.completed_at), now())
        })
        .await
        .expect("completion commits");

    // Force a checkpoint so everything is in the main file, then scan it.
    drop(database);
    let bytes = std::fs::read(&path).expect("reads the database file");
    let wal = std::fs::read(path.with_extension("sqlite3-wal")).unwrap_or_default();
    let haystack: Vec<u8> = bytes.into_iter().chain(wal).collect();
    for canary in [
        crate::audit_relay::test_support::WRITE_TOKEN,
        crate::audit_relay::test_support::READ_TOKEN,
    ] {
        assert!(
            !contains(&haystack, canary.as_bytes()),
            "the credential canary `{canary}` must never reach the database file"
        );
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
