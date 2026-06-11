//! `LeaseStore`: atomic lease lifecycle primitives over the `leases`
//! collection.
//!
//! Every mutation is a single conditional MongoDB operation (the document
//! `_id` is the package name, so the implicit unique `_id` index structurally
//! enforces one lease document per package). The authoritative
//! mutual-exclusion gate is the atomic `acquire` / `renew` result —
//! [`LeaseStore::holds_current`] is a fast-path assertion / diagnostic, never
//! the source of truth.
//!
//! Boundary convention (locked, so `acquire` and `holds_current` are exact
//! complements): a lease is dead/acquirable when `expires_at <= now`
//! (`$lte`), live/held when `expires_at > now` (`$gt`).

use std::time::Duration;

use bson::doc;
use mongodb::error::{ErrorKind, WriteFailure};
use mongodb::options::{IndexOptions, ReturnDocument};
use mongodb::{Collection, IndexModel};

use super::config::PoolConfig;
use super::error::PoolError;
use crate::db::{Db, IDX_LEASES_EXPIRES_AT, LEASES};
use crate::models::LeaseDoc;

/// Stable name of the non-unique `{holder_pod: 1}` index, so a pod can
/// enumerate the leases it believes it holds (e.g. shutdown release).
///
/// Ensured ONLY by [`LeaseStore::ensure_indexes`] — deliberately NOT part of
/// [`crate::db::Db::ensure_indexes`]; wiring this into the startup path is a
/// downstream issue.
pub const IDX_LEASES_HOLDER_POD: &str = "leases_holder_pod";

/// [`LeaseStore::reap_expired`] deletes only documents expired for at least
/// `lease_ttl * REAP_EXPIRY_MARGIN_FACTOR`. See the method doc for the
/// same-pod ABA hazard this margin closes.
pub const REAP_EXPIRY_MARGIN_FACTOR: u32 = 2;

/// Result of [`LeaseStore::acquire`]. There are exactly two variants: the
/// contended case is `NotAcquired` — any current-holder detail belongs only
/// in the diagnostic log line, never the return type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireOutcome {
    /// We are now the holder; `LeaseDoc::fencing_token` is the token to
    /// fence with (it bumps by 1 on EVERY acquire, including self-reacquire).
    Acquired(LeaseDoc),
    /// Another pod holds a live lease; we did not take it.
    NotAcquired,
}

/// Result of [`LeaseStore::renew`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewOutcome {
    /// The lease was extended; the token is unchanged, `expires_at` and
    /// `renewed_at` advanced (post-image carried for the heartbeat loop).
    Renewed(LeaseDoc),
    /// We no longer hold the lease at this token (expired-and-taken, fenced,
    /// or deleted). The caller MUST stop acting on this package.
    Lost,
}

/// Result of [`LeaseStore::release`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// Our lease document was deleted.
    Released,
    /// We did not own the lease at that token (already fenced /
    /// expired-and-taken / already released). NOT an error: release is
    /// idempotent.
    NotHeld,
}

/// Atomic lease store bound to the `leases` collection for one pod identity.
///
/// Cheap to clone (the driver `Collection` shares the underlying connection
/// pool).
#[derive(Clone)]
pub struct LeaseStore {
    coll: Collection<LeaseDoc>,
    pod_id: String,
    lease_ttl: Duration,
}

/// One clock read per operation: `(now, now + lease_ttl)` as
/// millisecond-precision BSON datetimes. Integer millis arithmetic only — no
/// `time`/`chrono` conversion ambiguity. `now` is the application clock;
/// bounded cross-pod skew is accepted (the TTL is chosen generously).
fn now_and_expiry(lease_ttl: Duration) -> (bson::DateTime, bson::DateTime) {
    let now = bson::DateTime::now();
    let expires =
        bson::DateTime::from_millis(now.timestamp_millis() + lease_ttl.as_millis() as i64);
    (now, expires)
}

/// True when `code` is MongoDB's duplicate-key error code (E11000).
fn code_is_dup(code: i32) -> bool {
    code == 11000
}

/// True when the driver error is a duplicate-key (`_id` collision) failure.
/// Mirrors `crate::packages::error::is_duplicate_key`: the single-write path
/// plus defensive `BulkWrite` / `Command` arms (driver versions differ in how
/// they report the failure).
fn is_dup_key(err: &mongodb::error::Error) -> bool {
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => code_is_dup(write_error.code),
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .values()
            .any(|write_error| code_is_dup(write_error.code)),
        ErrorKind::Command(command_error) => code_is_dup(command_error.code),
        _ => false,
    }
}

/// Build a non-unique ascending index with a stable name (local mirror of
/// the private helper in `crate::db`; deliberately NO TTL option — expiry is
/// interpreted logically in queries, never by the Mongo TTL monitor).
fn index_model(keys: bson::Document, name: &str) -> IndexModel {
    IndexModel::builder()
        .keys(keys)
        .options(IndexOptions::builder().name(name.to_string()).build())
        .build()
}

impl LeaseStore {
    /// Bind to the `leases` collection using `cfg`'s pod identity and TTL.
    pub fn new(db: &Db, cfg: &PoolConfig) -> Self {
        Self {
            coll: db.leases(),
            pod_id: cfg.pod_id.clone(),
            lease_ttl: cfg.lease_ttl,
        }
    }

    /// Idempotently ensure the non-unique `holder_pod` and `expires_at`
    /// indexes (stable names; safe to call repeatedly and concurrently).
    ///
    /// `expires_at` reuses [`IDX_LEASES_EXPIRES_AT`] with the IDENTICAL
    /// `{expires_at: 1}` key spec the startup path
    /// ([`crate::db::Db::ensure_indexes`]) declares, so re-ensuring it here
    /// is a no-op against an already-bootstrapped database. Neither index is
    /// a TTL index: the TTL monitor's unbounded deletion latency would race
    /// with `acquire`'s expired-doc takeover.
    pub async fn ensure_indexes(&self) -> Result<(), PoolError> {
        let lease_indexes = [
            (doc! { "holder_pod": 1 }, IDX_LEASES_HOLDER_POD),
            (doc! { "expires_at": 1 }, IDX_LEASES_EXPIRES_AT),
        ];
        self.coll
            .create_indexes(
                lease_indexes
                    .iter()
                    .map(|(keys, name)| index_model(keys.clone(), name)),
            )
            .await
            .map_err(|error| self.log_mongo_error("ensure_indexes", LEASES, error))?;
        for (_, name) in &lease_indexes {
            tracing::debug!(collection = LEASES, index = name, "lease index ensured");
        }
        Ok(())
    }

    /// Become the holder of `package_name`'s lease iff there is currently no
    /// *live* holder, or the live holder is already us (self-reacquire).
    ///
    /// One atomic `find_one_and_update` (upsert, post-image): the filter
    /// matches only acquirable states (`holder_pod == us` OR
    /// `expires_at <= now`), and an aggregation-pipeline update derives
    /// `fencing_token = old + 1` (`$add`/`$ifNull`) inside the same atomic
    /// operation — the token bumps by 1 on EVERY successful acquire, so
    /// callers must re-read the returned token (the token-preserving path is
    /// [`Self::renew`]). A self-reacquire rebinds `session_id` to the new
    /// session; callers must not assume it is immutable.
    ///
    /// Duplicate-key handling (E11000): when the document exists but is not
    /// acquirable by us, the upsert's insert attempt collides with the
    /// existing `_id` and the server surfaces E11000 — this is the NORMAL
    /// contended signal, not an error (it also covers two pods racing the
    /// first insert: the loser gets E11000). On E11000 the same call is
    /// retried EXACTLY ONCE so the filter re-evaluates against the winner's
    /// document (it may have expired or been released in between); a second
    /// E11000 means the holder is still live -> `NotAcquired`. No further
    /// retries (avoids livelock).
    pub async fn acquire(
        &self,
        package_name: &str,
        session_id: bson::Uuid,
    ) -> Result<AcquireOutcome, PoolError> {
        let result = match self.acquire_attempt(package_name, session_id).await {
            Err(error) if is_dup_key(&error) => {
                tracing::debug!(
                    package = package_name,
                    pod = %self.pod_id,
                    "lease acquire hit duplicate _id; retrying once"
                );
                match self.acquire_attempt(package_name, session_id).await {
                    Err(retry_error) if is_dup_key(&retry_error) => Ok(None),
                    other => other,
                }
            }
            other => other,
        };
        match result {
            Ok(Some(lease)) => {
                tracing::info!(
                    package = package_name,
                    pod = %self.pod_id,
                    token = lease.fencing_token,
                    session = %lease.session_id,
                    expires_at = %lease.expires_at,
                    "lease acquired"
                );
                Ok(AcquireOutcome::Acquired(lease))
            }
            Ok(None) => {
                tracing::info!(
                    package = package_name,
                    wanted_by = %self.pod_id,
                    "lease contended"
                );
                Ok(AcquireOutcome::NotAcquired)
            }
            Err(error) => Err(self.log_mongo_error("acquire", package_name, error)),
        }
    }

    /// The single raw acquire operation (no retry, no outcome mapping).
    async fn acquire_attempt(
        &self,
        package_name: &str,
        session_id: bson::Uuid,
    ) -> Result<Option<LeaseDoc>, mongodb::error::Error> {
        let (now, expires) = now_and_expiry(self.lease_ttl);
        let filter = doc! {
            "_id": package_name,
            "$or": [
                { "holder_pod": &self.pod_id },
                { "expires_at": { "$lte": now } },
            ],
        };
        // Aggregation pipeline so `$fencing_token` is readable in the same
        // atomic op. `holder_pod` goes through `$literal`: in a pipeline
        // `$set`, a plain string is an *expression*, and an operator-shaped
        // pod id (e.g. `$$REMOVE`) must stay an opaque literal value.
        // `session_id` (Binary) and the datetimes are literals by type.
        let update = vec![doc! {
            "$set": {
                "session_id": session_id,
                "holder_pod": { "$literal": &self.pod_id },
                "fencing_token": { "$add": [{ "$ifNull": ["$fencing_token", 0] }, 1] },
                "expires_at": expires,
                "renewed_at": now,
            },
        }];
        self.coll
            .find_one_and_update(filter, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await
    }

    /// Extend the lease we hold at `fencing_token` (token unchanged;
    /// `expires_at` / `renewed_at` advance). One atomic conditional update —
    /// the filter pins ownership AND the token by EQUALITY (defends against
    /// a stale heartbeat firing after we were fenced or after a release
    /// reset the counter) AND requires the lease to still be live
    /// (`expires_at > now`): renew never resurrects a dead lease. No upsert —
    /// renew never creates a lease.
    ///
    /// Implemented as `find_one_and_update` (the atomic single-document
    /// update plus the post-image in one op) so `Renewed` can carry the
    /// fresh `expires_at` for the heartbeat loop; no-match (`None`) is
    /// `update_one`'s matched-0, i.e. `Lost`.
    pub async fn renew(
        &self,
        package_name: &str,
        fencing_token: i64,
    ) -> Result<RenewOutcome, PoolError> {
        let (now, expires) = now_and_expiry(self.lease_ttl);
        let filter = doc! {
            "_id": package_name,
            "holder_pod": &self.pod_id,
            "fencing_token": fencing_token,
            "expires_at": { "$gt": now },
        };
        let update = doc! { "$set": { "expires_at": expires, "renewed_at": now } };
        let renewed = self
            .coll
            .find_one_and_update(filter, update)
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| self.log_mongo_error("renew", package_name, error))?;
        match renewed {
            Some(lease) => {
                tracing::debug!(
                    package = package_name,
                    token = lease.fencing_token,
                    expires_at = %lease.expires_at,
                    "lease renewed"
                );
                Ok(RenewOutcome::Renewed(lease))
            }
            None => {
                tracing::warn!(
                    package = package_name,
                    token = fencing_token,
                    pod = %self.pod_id,
                    "lease lost on renew"
                );
                Ok(RenewOutcome::Lost)
            }
        }
    }

    /// Voluntarily give up the lease we hold at `fencing_token` (graceful
    /// shutdown / session stop). One conditional `delete_one` pinning
    /// ownership and the token by EQUALITY. Idempotent: an already-gone (or
    /// never-ours) lease is `NotHeld`, not an error.
    ///
    /// Deleting the document (rather than back-dating `expires_at`) resets
    /// the fencing counter for the NEXT continuous lease — see the module
    /// doc's reset boundary and equality-only comparison rule.
    pub async fn release(
        &self,
        package_name: &str,
        fencing_token: i64,
    ) -> Result<ReleaseOutcome, PoolError> {
        let filter = doc! {
            "_id": package_name,
            "holder_pod": &self.pod_id,
            "fencing_token": fencing_token,
        };
        let result = self
            .coll
            .delete_one(filter)
            .await
            .map_err(|error| self.log_mongo_error("release", package_name, error))?;
        if result.deleted_count == 1 {
            tracing::info!(
                package = package_name,
                pod = %self.pod_id,
                token = fencing_token,
                "lease released"
            );
            Ok(ReleaseOutcome::Released)
        } else {
            tracing::info!(
                package = package_name,
                pod = %self.pod_id,
                token = fencing_token,
                "lease release found nothing held at this token"
            );
            Ok(ReleaseOutcome::NotHeld)
        }
    }

    /// Spawn guard: do we hold the current, LIVE lease at exactly this
    /// token? Pure read (`find_one` pinning `_id`, ownership, token equality,
    /// and `expires_at > now`); `false` for missing doc, wrong pod, wrong
    /// token, or expired.
    ///
    /// This is a fast-path assertion / diagnostic ONLY — a read can race a
    /// concurrent takeover. The authoritative gate is a successful
    /// `acquire`/`renew`; spawn only after one, keep running only while
    /// `renew` returns `Renewed`.
    pub async fn holds_current(
        &self,
        package_name: &str,
        fencing_token: i64,
    ) -> Result<bool, PoolError> {
        let filter = doc! {
            "_id": package_name,
            "holder_pod": &self.pod_id,
            "fencing_token": fencing_token,
            "expires_at": { "$gt": bson::DateTime::now() },
        };
        let held = self
            .coll
            .find_one(filter)
            .await
            .map_err(|error| self.log_mongo_error("holds_current", package_name, error))?
            .is_some();
        tracing::debug!(
            package = package_name,
            pod = %self.pod_id,
            token = fencing_token,
            held,
            "lease holds_current check"
        );
        Ok(held)
    }

    /// Housekeeping: physically delete LONG-expired lease documents — only
    /// those with `expires_at <= now - margin`, where
    /// `margin = lease_ttl * `[`REAP_EXPIRY_MARGIN_FACTOR`] — and return the
    /// deleted count. Correctness never depends on this — `acquire` already
    /// treats expired docs as acquirable. Never invoked automatically by
    /// this module (scheduling is downstream).
    ///
    /// # Why the margin (same-pod ABA hazard)
    ///
    /// Reap is garbage collection, NOT takeover (takeover happens via
    /// `acquire`, which preserves the counter by updating the surviving
    /// document). Deleting a *barely* expired document would reset the
    /// fencing counter: the next `acquire` starts a fresh doc at token `1`,
    /// and a stale holder's delayed `renew(pkg, 1)` from the previous lease
    /// generation could then match again — same pod, same token — wrongly
    /// extending a lease its session no longer legitimately holds (ABA).
    /// A generous margin of `2 x lease_ttl` past expiry guarantees any
    /// in-flight heartbeat from the old generation has long since fired and
    /// returned `Lost` before the document (and with it the counter) can be
    /// destroyed.
    pub async fn reap_expired(&self) -> Result<u64, PoolError> {
        let margin = self.lease_ttl * REAP_EXPIRY_MARGIN_FACTOR;
        let cutoff = bson::DateTime::from_millis(
            bson::DateTime::now().timestamp_millis() - margin.as_millis() as i64,
        );
        let result = self
            .coll
            .delete_many(doc! { "expires_at": { "$lte": cutoff } })
            .await
            .map_err(|error| self.log_mongo_error("reap_expired", LEASES, error))?;
        if result.deleted_count > 0 {
            tracing::info!(reaped = result.deleted_count, "expired leases reaped");
        } else {
            tracing::debug!("no expired leases to reap");
        }
        Ok(result.deleted_count)
    }

    /// Log an unexpected driver failure with full context (server-side only;
    /// driver text may carry connection detail and never reaches a client)
    /// and wrap it.
    fn log_mongo_error(
        &self,
        op: &'static str,
        package: &str,
        error: mongodb::error::Error,
    ) -> PoolError {
        tracing::error!(
            op,
            package,
            pod = %self.pod_id,
            error = %error,
            "lease mongodb operation failed"
        );
        PoolError::Mongo(error)
    }
}

#[cfg(test)]
mod tests {
    use mongodb::error::{BulkWriteError, CommandError, WriteError};

    use super::*;

    /// Build a driver `WriteError` through its public `Deserialize` impl
    /// (the struct is `#[non_exhaustive]`).
    fn write_error(code: i32) -> WriteError {
        bson::from_document(doc! { "code": code, "errmsg": "boom" }).expect("WriteError shape")
    }

    fn write_failure_error(code: i32) -> mongodb::error::Error {
        mongodb::error::Error::from(ErrorKind::Write(WriteFailure::WriteError(write_error(
            code,
        ))))
    }

    #[test]
    fn now_and_expiry_is_exactly_ttl_millis_apart() {
        let ttl = Duration::from_secs(30);
        let (now, expires) = now_and_expiry(ttl);
        assert_eq!(
            expires.timestamp_millis() - now.timestamp_millis(),
            30_000,
            "expiry must be now + ttl in integer millis"
        );
        assert!(expires > now);
    }

    #[test]
    fn code_is_dup_only_for_11000() {
        assert!(code_is_dup(11000));
        assert!(!code_is_dup(0));
        assert!(!code_is_dup(121));
        assert!(!code_is_dup(11001));
    }

    #[test]
    fn is_dup_key_detects_write_error_11000() {
        assert!(is_dup_key(&write_failure_error(11000)));
        assert!(!is_dup_key(&write_failure_error(121)));
    }

    #[test]
    fn is_dup_key_detects_command_error_11000() {
        let command: CommandError = bson::from_document(
            doc! { "code": 11000, "codeName": "DuplicateKey", "errmsg": "dup" },
        )
        .expect("CommandError shape");
        let err = mongodb::error::Error::from(ErrorKind::Command(command));
        assert!(is_dup_key(&err));
    }

    #[test]
    fn is_dup_key_detects_bulk_write_error_11000() {
        let mut bulk = BulkWriteError::default();
        bulk.write_errors.insert(0, write_error(11000));
        let err = mongodb::error::Error::from(ErrorKind::BulkWrite(bulk));
        assert!(is_dup_key(&err));
    }

    #[test]
    fn is_dup_key_is_false_for_non_write_kinds() {
        let io = std::io::Error::other("connection refused");
        assert!(!is_dup_key(&mongodb::error::Error::from(io)));
    }

    #[test]
    fn index_model_sets_stable_name_and_no_ttl() {
        let model = index_model(doc! { "holder_pod": 1 }, IDX_LEASES_HOLDER_POD);
        let options = model.options.expect("options present");
        assert_eq!(options.name.as_deref(), Some("leases_holder_pod"));
        assert!(
            options.expire_after.is_none(),
            "lease indexes must never be TTL indexes"
        );
        assert!(options.unique.is_none());
    }
}
