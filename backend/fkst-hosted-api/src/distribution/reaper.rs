//! The takeover reaper: the failover (redo) half of the pool-manager.
//!
//! Every pod runs one reaper loop. A pass does three independent scans:
//!
//! 1. **Unplaced pending sessions** (`pod_id: null`) are (re)placed — this
//!    is the retry path for `NoCapacity` and for transient placement
//!    failures at create time.
//! 2. **Orphaned leases** (`expires_at <= now - grace`) are joined to their
//!    sessions: terminal/missing sessions get their lease released; an
//!    active session whose package was deleted is failed (lease released);
//!    an active session whose holder is unhealthy is TAKEN OVER — the lease
//!    is re-acquired by this pod (strictly greater fencing token, the
//!    document survives so the counter continues) and the session is
//!    normalized back to `pending` with `pid`/`runtime_dir` cleared so the
//!    local driver redoes it from scratch.
//! 3. **Self-assigned pickup**: `pending` sessions owned by this pod whose
//!    lease this pod holds get a local driver task ensured (the seam to the
//!    sessions service; the driver's claim CAS dedupes).
//!
//! Per-item failures are logged and never abort the pass; a whole-pass
//! failure is logged by the loop and retried on the next tick.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use bson::doc;
use tokio_util::sync::CancellationToken;

use super::distributor::{Distributor, Placement, PlacementError};
use super::health::active_status_bson;
use crate::leases::AcquireOutcome;
use crate::models::{LeaseDoc, SessionDoc, SessionStatus};
use crate::packages::PACKAGES_COLLECTION;
use crate::sessions::repo::{status_bson, ORPHANED_ERROR};

/// The seam through which the reaper asks the local session service to run
/// a driver task for a session this pod owns. Implementations MUST be
/// idempotent (no-op when a driver for the session is already live); the
/// driver's own claim CAS dedupes across restarts.
#[async_trait]
pub trait DriverHost: Send + Sync {
    async fn ensure_driver(&self, session: &SessionDoc);
}

impl Distributor {
    /// One reaper pass (see the module docs). Returns the takeovers this
    /// pod won. Iterates items independently: one item's transient error is
    /// logged (`ERROR`, with package/session/pod context) and does not
    /// abort the pass.
    pub async fn reap_and_takeover(
        &self,
        host: &dyn DriverHost,
    ) -> Result<Vec<Placement>, PlacementError> {
        let healthy: HashSet<String> = self
            .health
            .healthy_pods_and_loads()
            .await?
            .into_iter()
            .map(|pod| pod.pod_id)
            .collect();

        self.place_unassigned_pending().await?;

        let orphans = self.orphaned_leases().await?;
        tracing::debug!(
            orphaned_count = orphans.len(),
            pod = %self.pod_id(),
            "reaper.tick"
        );
        let mut won = Vec::new();
        for lease in orphans {
            match self.try_takeover(&lease, &healthy).await {
                Ok(Some(placement)) => won.push(placement),
                Ok(None) => {}
                Err(error) => tracing::error!(
                    package_name = %lease.package_name,
                    session_id = %lease.session_id,
                    pod_id = %self.pod_id(),
                    error = %error,
                    "takeover failed for one lease; continuing the pass"
                ),
            }
        }

        self.pickup_self_assigned(host).await?;
        Ok(won)
    }

    /// Long-running loop: ticks [`Self::reap_and_takeover`] every
    /// `scan_interval`. Logs and continues on errors; exits cleanly when
    /// `shutdown` is cancelled.
    pub async fn run_reaper(
        self: Arc<Self>,
        host: Arc<dyn DriverHost>,
        shutdown: CancellationToken,
    ) {
        let mut tick = tokio::time::interval(self.cfg.scan_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tracing::info!(
            pod = %self.pod_id(),
            scan_interval_secs = self.cfg.scan_interval.as_secs(),
            "takeover reaper started"
        );
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!(pod = %self.pod_id(), "takeover reaper stopped");
                    return;
                }
                _ = tick.tick() => {
                    if let Err(error) = self.reap_and_takeover(host.as_ref()).await {
                        tracing::error!(
                            pod_id = %self.pod_id(),
                            error = %error,
                            "reaper pass failed; retrying on the next tick"
                        );
                    }
                }
            }
        }
    }

    /// Boot recovery (replaces the bare v1 orphan sweep): fail every
    /// pre-terminal session **owned by this pod** AND release each one's
    /// lease at its stored holder + token (equality pins; `NotHeld` is
    /// ignored). Run BEFORE the listener binds.
    ///
    /// Scoped to `pod_id == self` so it stays correct beyond replicas=1
    /// (#27): a restarting pod's stale identity can only refer to engine
    /// processes that died with its previous incarnation, while sessions
    /// owned by OTHER (possibly healthy) pods — and unassigned pending
    /// sessions — are untouched here; dead foreign holders are recovered by
    /// the reaper's lease-expiry takeover instead. With replicas=1 the
    /// behavior is identical to the unscoped sweep.
    pub async fn fail_orphans_at_boot(&self) -> Result<u64, PlacementError> {
        let coll = self.db.sessions();
        let filter = doc! {
            "status": { "$in": active_status_bson() },
            "pod_id": self.pod_id(),
        };
        let mut cursor = coll.find(filter.clone()).await?;
        let mut orphans = Vec::new();
        while cursor.advance().await? {
            orphans.push(cursor.deserialize_current()?);
        }

        let result = coll
            .update_many(
                filter,
                doc! { "$set": {
                    "status": status_bson(SessionStatus::Failed),
                    "error": ORPHANED_ERROR,
                    "stopped_at": bson::DateTime::now(),
                } },
            )
            .await?;
        if result.modified_count > 0 {
            tracing::warn!(
                count = result.modified_count,
                "orphaned sessions failed by startup sweep"
            );
        } else {
            tracing::info!("orphan sweep found no pre-terminal sessions");
        }

        for session in &orphans {
            let (Some(pod_id), Some(token)) = (&session.pod_id, session.fencing_token) else {
                continue;
            };
            let lease_key = session.lease_key();
            if let Err(error) = self.store_for(pod_id).release(&lease_key, token).await {
                tracing::error!(
                    lease_key = %lease_key,
                    session_id = %session.id,
                    pod_id = %pod_id,
                    error = %error,
                    "failed to release an orphaned session's lease; \
                     it will lapse and be reaped"
                );
            }
        }
        Ok(result.modified_count)
    }

    /// Scan 1 selection: leases dead for at least the grace window
    /// (`expires_at <= now - grace`). The grace applies only to the SCAN —
    /// the takeover acquire re-asserts plain expiry atomically inside
    /// [`crate::leases::LeaseStore::acquire`].
    async fn orphaned_leases(&self) -> Result<Vec<LeaseDoc>, PlacementError> {
        let cutoff = bson::DateTime::from_millis(
            bson::DateTime::now().timestamp_millis() - self.cfg.grace.as_millis() as i64,
        );
        let mut cursor = self
            .db
            .leases()
            .find(doc! { "expires_at": { "$lte": cutoff } })
            .await?;
        let mut orphans = Vec::new();
        while cursor.advance().await? {
            orphans.push(cursor.deserialize_current()?);
        }
        Ok(orphans)
    }

    /// Handle one orphaned lease. `Ok(Some)` is a won takeover.
    async fn try_takeover(
        &self,
        lease: &LeaseDoc,
        healthy: &HashSet<String>,
    ) -> Result<Option<Placement>, PlacementError> {
        let package_name = lease.package_name.as_str();

        // Join the session. Terminal or missing: release, never redo.
        let session = self
            .db
            .sessions()
            .find_one(doc! { "_id": lease.session_id })
            .await?;
        let session = match session {
            None => {
                tracing::warn!(
                    package_name,
                    session_id = %lease.session_id,
                    reason = "missing_session",
                    "takeover.skipped: releasing the lease"
                );
                self.release_as_holder(lease).await;
                return Ok(None);
            }
            Some(session)
                if matches!(
                    session.status,
                    SessionStatus::Stopped | SessionStatus::Failed
                ) =>
            {
                tracing::debug!(
                    package_name,
                    session_id = %lease.session_id,
                    reason = "terminal",
                    "takeover.skipped: releasing the lease"
                );
                self.release_as_holder(lease).await;
                return Ok(None);
            }
            Some(session) => session,
        };

        // Package-deleted check: for classic sessions, verify the single
        // package; for goal sessions, verify ALL package names in the
        // effective set. A deleted package means the session must fail
        // rather than redo indefinitely.
        let names_to_check = session.effective_package_names();
        for name in &names_to_check {
            let exists = self
                .db
                .collection::<bson::Document>(PACKAGES_COLLECTION)
                .find_one(doc! { "_id": name })
                .await?
                .is_some();
            if !exists {
                tracing::warn!(
                    package_name = %name,
                    session_id = %session.id,
                    "package deleted while session active; failing the session"
                );
                let _ = self
                    .db
                    .sessions()
                    .update_one(
                        doc! {
                            "_id": session.id,
                            "status": { "$in": active_status_bson() },
                        },
                        doc! { "$set": {
                            "status": status_bson(SessionStatus::Failed),
                            "error": format!(
                                "package {name} deleted while session active"
                            ),
                            "stopped_at": bson::DateTime::now(),
                        } },
                    )
                    .await?;
                self.release_as_holder(lease).await;
                return Ok(None);
            }
        }

        // A healthy holder (other than us) with a lapsed lease is a GC
        // pause / clock-skew smell: fail closed, skip the tick. Our own
        // lapsed lease is always reclaimable (the old process is us).
        if lease.holder_pod != self.pod_id() && healthy.contains(&lease.holder_pod) {
            tracing::warn!(
                package_name,
                session_id = %session.id,
                holder = %lease.holder_pod,
                reason = "holder_healthy",
                "takeover.skipped: holder is healthy but its lease lapsed"
            );
            return Ok(None);
        }

        // Takeover acquire using the session's lease key (goal-<uuid> for
        // goal sessions, package_name for classic). Wins only while the
        // lease is still expired at write time (atomic inside the lease
        // store); token bumps by 1 on the SURVIVING document, so it is
        // strictly greater than the old holder's token.
        let lease_key = session.lease_key();
        let new_lease = match self.leases.acquire(&lease_key, lease.session_id).await? {
            AcquireOutcome::Acquired(new_lease) => new_lease,
            AcquireOutcome::NotAcquired => {
                tracing::debug!(
                    lease_key = %lease_key,
                    session_id = %session.id,
                    reason = "lost_race",
                    "takeover.skipped: another survivor won"
                );
                return Ok(None);
            }
        };

        // Normalize the session for the redo, guarded so a concurrently
        // terminal session voids the win (release, no orphaned lease).
        let updated = self
            .db
            .sessions()
            .update_one(
                doc! {
                    "_id": session.id,
                    "status": { "$in": active_status_bson() },
                },
                doc! { "$set": {
                    "status": status_bson(SessionStatus::Pending),
                    "pod_id": self.pod_id(),
                    "fencing_token": new_lease.fencing_token,
                    "pid": bson::Bson::Null,
                    "runtime_dir": bson::Bson::Null,
                } },
            )
            .await?;
        if updated.matched_count == 0 {
            tracing::debug!(
                lease_key = %lease_key,
                session_id = %session.id,
                reason = "terminal",
                "takeover.skipped: session went terminal during takeover; \
                 releasing the won lease"
            );
            let _ = self
                .leases
                .release(&lease_key, new_lease.fencing_token)
                .await;
            return Ok(None);
        }

        tracing::info!(
            session_id = %session.id,
            lease_key = %lease_key,
            from_pod = %lease.holder_pod,
            to_pod = %self.pod_id(),
            old_token = lease.fencing_token,
            new_token = new_lease.fencing_token,
            "takeover.won"
        );
        Ok(Some(Placement {
            session_id: session.id,
            package_name: lease_key,
            pod_id: self.pod_id().to_string(),
            fencing_token: new_lease.fencing_token,
        }))
    }

    /// Scan 0: place `pending` sessions nobody owns yet (`pod_id: null`) —
    /// the retry path after `NoCapacity` or a transient placement failure.
    /// `AlreadyRunning` permanently fails the session (its package's live
    /// lease belongs to another session); other errors are logged per item.
    async fn place_unassigned_pending(&self) -> Result<(), PlacementError> {
        let mut cursor = self
            .db
            .sessions()
            .find(doc! {
                "status": status_bson(SessionStatus::Pending),
                "pod_id": bson::Bson::Null,
            })
            .await?;
        let mut unplaced: Vec<SessionDoc> = Vec::new();
        while cursor.advance().await? {
            unplaced.push(cursor.deserialize_current()?);
        }
        for session in unplaced {
            let lease_key = session.lease_key();
            match self.place(&lease_key, session.id).await {
                Ok(placement) => tracing::info!(
                    session_id = %session.id,
                    package_name = %session.package_name,
                    lease_key = %lease_key,
                    pod_id = %placement.pod_id,
                    "reaper placed an unassigned pending session"
                ),
                Err(PlacementError::AlreadyRunning(_)) => {
                    let _ = self
                        .db
                        .sessions()
                        .update_one(
                            doc! {
                                "_id": session.id,
                                "status": status_bson(SessionStatus::Pending),
                                "pod_id": bson::Bson::Null,
                            },
                            doc! { "$set": {
                                "status": status_bson(SessionStatus::Failed),
                                "error": "package already has a live session",
                                "stopped_at": bson::DateTime::now(),
                            } },
                        )
                        .await?;
                    tracing::warn!(
                        session_id = %session.id,
                        package_name = %session.package_name,
                        lease_key = %lease_key,
                        "unassigned pending session failed: live lease for the package"
                    );
                }
                Err(PlacementError::NoCapacity) => {
                    // Stays pending; retried next tick.
                    tracing::warn!(
                        session_id = %session.id,
                        package_name = %session.package_name,
                        lease_key = %lease_key,
                        "no capacity for an unassigned pending session; will retry"
                    );
                }
                Err(error) => tracing::error!(
                    session_id = %session.id,
                    package_name = %session.package_name,
                    lease_key = %lease_key,
                    pod_id = %self.pod_id(),
                    error = %error,
                    "placement of an unassigned pending session failed; continuing"
                ),
            }
        }
        Ok(())
    }

    /// Scan 2: ensure a local driver for every `pending` session assigned
    /// to this pod (placement or takeover wrote `pod_id = self`,
    /// `pid = null`) whose lease this pod currently holds. The
    /// `holds_current` read is a fast-path filter; the driver's claim CAS
    /// and lease renewal stay authoritative.
    async fn pickup_self_assigned(&self, host: &dyn DriverHost) -> Result<(), PlacementError> {
        let mut cursor = self
            .db
            .sessions()
            .find(doc! {
                "pod_id": self.pod_id(),
                "status": status_bson(SessionStatus::Pending),
                "pid": bson::Bson::Null,
            })
            .await?;
        let mut owned: Vec<SessionDoc> = Vec::new();
        while cursor.advance().await? {
            owned.push(cursor.deserialize_current()?);
        }
        for session in owned {
            let Some(token) = session.fencing_token else {
                continue;
            };
            let lease_key = session.lease_key();
            match self.leases.holds_current(&lease_key, token).await {
                Ok(true) => {
                    tracing::debug!(
                        session_id = %session.id,
                        package_name = %session.package_name,
                        lease_key = %lease_key,
                        "ensuring a local driver for an owned pending session"
                    );
                    host.ensure_driver(&session).await;
                }
                Ok(false) => {}
                Err(error) => tracing::error!(
                    session_id = %session.id,
                    package_name = %session.package_name,
                    lease_key = %lease_key,
                    pod_id = %self.pod_id(),
                    error = %error,
                    "lease check failed for an owned pending session; continuing"
                ),
            }
        }
        Ok(())
    }

    /// Release `lease` impersonating its stored holder (equality-pinned on
    /// holder + token, so this can never destroy a successor's lease).
    /// Failures are logged only — the lease will lapse and be reaped.
    async fn release_as_holder(&self, lease: &LeaseDoc) {
        if let Err(error) = self
            .store_for(&lease.holder_pod)
            .release(&lease.package_name, lease.fencing_token)
            .await
        {
            tracing::error!(
                package_name = %lease.package_name,
                session_id = %lease.session_id,
                pod_id = %lease.holder_pod,
                error = %error,
                "failed to release a dead lease; it will be reaped later"
            );
        }
    }
}
