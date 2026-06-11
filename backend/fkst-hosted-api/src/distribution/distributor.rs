//! Placement: deciding which healthy pod runs a new session and writing the
//! ownership (lease + `sessions.pod_id` / `sessions.fencing_token`) so that
//! pod's driver takes over. There is no RPC between pods; MongoDB is the
//! coordination substrate.

use std::sync::Arc;

use bson::doc;

use super::config::DistributionConfig;
use super::health::{active_status_bson, HealthView, PodLoad};
use crate::db::Db;
use crate::leases::{AcquireOutcome, LeaseStore, PoolConfig, PoolError};
use crate::models::LeaseDoc;
use crate::packages::is_valid_name;

/// Errors surfaced by placement and takeover. Contended placement
/// (`AlreadyRunning`) and exhausted capacity (`NoCapacity`) are expected
/// operational outcomes the API edge maps to client responses; the rest are
/// validation or infrastructure failures.
#[derive(Debug, thiserror::Error)]
pub enum PlacementError {
    /// A live lease already exists for the package, bound to a different
    /// session. The Sessions API maps this to `409 Conflict`.
    #[error("package {0} already has a live lease")]
    AlreadyRunning(String),
    /// No healthy pod has capacity (retriable; the session stays `pending`).
    #[error("no healthy pod has capacity")]
    NoCapacity,
    /// The package name failed re-validation (defense in depth).
    #[error("invalid package name")]
    InvalidPackageName,
    /// The session document vanished or went terminal while placing; the
    /// just-won lease has been released (no half-applied state).
    #[error("session {0} not found or no longer active")]
    SessionNotFound(bson::Uuid),
    /// Lease-layer failure (driver error underneath).
    #[error(transparent)]
    Lease(#[from] PoolError),
    /// Raw MongoDB driver failure outside the lease layer.
    #[error(transparent)]
    Mongo(#[from] mongodb::error::Error),
}

/// Outcome of placing (or taking over) a session: the pod and fencing token
/// that now own the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub session_id: bson::Uuid,
    pub package_name: String,
    pub pod_id: String,
    pub fencing_token: i64,
}

impl Placement {
    fn from_lease(lease: &LeaseDoc) -> Self {
        Placement {
            session_id: lease.session_id,
            package_name: lease.package_name.clone(),
            pod_id: lease.holder_pod.clone(),
            fencing_token: lease.fencing_token,
        }
    }
}

/// The distribution brain: least-loaded placement of new sessions and (via
/// the reaper, `super::reaper`) lease-fenced takeover of sessions whose
/// holder pod died. Cheap to clone (handles are `Arc`-backed).
#[derive(Clone)]
pub struct Distributor {
    pub(super) db: Db,
    pub(super) leases: LeaseStore,
    pub(super) health: Arc<dyn HealthView>,
    pub(super) cfg: DistributionConfig,
}

impl Distributor {
    pub fn new(
        db: Db,
        leases: LeaseStore,
        health: Arc<dyn HealthView>,
        cfg: DistributionConfig,
    ) -> Self {
        Self {
            db,
            leases,
            health,
            cfg,
        }
    }

    /// This pod's identity (the one the reaper claims takeovers for).
    pub fn pod_id(&self) -> &str {
        &self.cfg.pool.pod_id
    }

    /// The lease store bound to this pod's identity (session drivers renew
    /// and release through it).
    pub fn leases(&self) -> &LeaseStore {
        &self.leases
    }

    /// The distribution configuration (renew/scan cadences, grace, cap).
    pub fn config(&self) -> &DistributionConfig {
        &self.cfg
    }

    /// A lease store impersonating `holder_pod`'s identity, for releasing a
    /// lease held by another (typically dead) pod. Safe because release and
    /// renew pin the holder AND the fencing token by equality.
    pub(super) fn store_for(&self, holder_pod: &str) -> LeaseStore {
        LeaseStore::new(
            &self.db,
            &PoolConfig {
                pod_id: holder_pod.to_string(),
                lease_ttl: self.cfg.pool.lease_ttl,
            },
        )
    }

    /// Place a `pending` session on the least-loaded healthy pod: acquire
    /// the package lease for the chosen pod (token bumps by 1) and write
    /// `sessions.pod_id` / `sessions.fencing_token` (status stays
    /// `pending`; the owning pod's driver advances it). Idempotent:
    /// replaying for the session that already holds the live lease returns
    /// the existing `Placement` without bumping the token. Does NOT spawn
    /// the engine (the driver does).
    pub async fn place(
        &self,
        package_name: &str,
        session_id: bson::Uuid,
    ) -> Result<Placement, PlacementError> {
        // 1. Defense in depth: re-assert the package name before any write.
        if !is_valid_name(package_name) {
            tracing::warn!(
                package_name_bytes = package_name.len(),
                "placement rejected: invalid package name"
            );
            return Err(PlacementError::InvalidPackageName);
        }

        // 2. Live-lease lookup: idempotent replay or conflict.
        if let Some(early) = self.check_live_lease(package_name, session_id).await? {
            return early;
        }

        // 3. Pick the least-loaded healthy pod.
        let pods = self.health.healthy_pods_and_loads().await?;
        let Some(chosen) = select_pod(&pods, self.cfg.max_load) else {
            tracing::warn!(
                package = package_name,
                session = %session_id,
                healthy_pods = pods.len(),
                max_load = self.cfg.max_load,
                "placement found no healthy pod with capacity"
            );
            return Err(PlacementError::NoCapacity);
        };
        let chosen_pod = chosen.pod_id.clone();
        let chosen_load = chosen.active_sessions;

        // 4. Acquire the lease for the chosen pod; on a lost race re-read
        //    and resolve idempotently or as a conflict.
        let lease = match self
            .leases
            .acquire_for(package_name, session_id, &chosen_pod)
            .await?
        {
            AcquireOutcome::Acquired(lease) => lease,
            AcquireOutcome::NotAcquired => {
                if let Some(early) = self.check_live_lease(package_name, session_id).await? {
                    return early;
                }
                tracing::info!(
                    package = package_name,
                    session = %session_id,
                    "placement lost the acquire race"
                );
                return Err(PlacementError::AlreadyRunning(package_name.to_string()));
            }
        };

        // 5. Write ownership onto the session, guarded so a terminal or
        //    deleted session voids the win (release, no orphaned lease).
        let updated = self
            .db
            .sessions()
            .update_one(
                doc! {
                    "_id": session_id,
                    "status": { "$in": active_status_bson() },
                },
                doc! { "$set": {
                    "pod_id": &lease.holder_pod,
                    "fencing_token": lease.fencing_token,
                } },
            )
            .await?;
        if updated.matched_count == 0 {
            tracing::warn!(
                package = package_name,
                session = %session_id,
                "session went terminal during placement; releasing the won lease"
            );
            let _ = self
                .store_for(&lease.holder_pod)
                .release(package_name, lease.fencing_token)
                .await;
            return Err(PlacementError::SessionNotFound(session_id));
        }

        tracing::info!(
            session_id = %session_id,
            package_name = package_name,
            pod_id = %lease.holder_pod,
            fencing_token = lease.fencing_token,
            chosen_load,
            "placement.assigned"
        );
        Ok(Placement::from_lease(&lease))
    }

    /// Shared live-lease resolution for `place`: `Some(Ok)` is the
    /// idempotent replay (the live lease is bound to this very session);
    /// `Some(Err(AlreadyRunning))` is a conflict with another session's live
    /// lease; `None` means no live lease (placement proceeds).
    async fn check_live_lease(
        &self,
        package_name: &str,
        session_id: bson::Uuid,
    ) -> Result<Option<Result<Placement, PlacementError>>, PlacementError> {
        let live = self
            .db
            .leases()
            .find_one(doc! {
                "_id": package_name,
                "expires_at": { "$gt": bson::DateTime::now() },
            })
            .await?;
        Ok(live.map(|lease| {
            if lease.session_id == session_id {
                tracing::info!(
                    session_id = %session_id,
                    package_name = package_name,
                    fencing_token = lease.fencing_token,
                    "placement.idempotent"
                );
                Ok(Placement::from_lease(&lease))
            } else {
                tracing::info!(
                    session_id = %session_id,
                    package_name = package_name,
                    holder = %lease.holder_pod,
                    "placement conflicts with a live lease"
                );
                Err(PlacementError::AlreadyRunning(package_name.to_string()))
            }
        }))
    }
}

/// Pure, deterministic pod selection: the pod with the **minimum load**,
/// ties broken by **lowest `pod_id`** (avoids flapping). When `max_load > 0`
/// pods with `active_sessions >= max_load` are discarded; `max_load == 0`
/// means uncapped (never reject on load). Returns `None` when the input is
/// empty or every pod is at cap.
pub fn select_pod(pods: &[PodLoad], max_load: u64) -> Option<&PodLoad> {
    pods.iter()
        .filter(|pod| max_load == 0 || pod.active_sessions < max_load)
        .min_by(|a, b| {
            (a.active_sessions, a.pod_id.as_str()).cmp(&(b.active_sessions, b.pod_id.as_str()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(pod_id: &str, active_sessions: u64) -> PodLoad {
        PodLoad {
            pod_id: pod_id.to_string(),
            active_sessions,
        }
    }

    #[test]
    fn empty_input_selects_nothing() {
        assert_eq!(select_pod(&[], 0), None);
        assert_eq!(select_pod(&[], 5), None);
    }

    #[test]
    fn single_pod_is_selected() {
        let pods = [pod("pod-a", 7)];
        assert_eq!(select_pod(&pods, 0), Some(&pods[0]));
    }

    #[test]
    fn least_loaded_pod_wins() {
        let pods = [pod("pod-a", 2), pod("pod-b", 0), pod("pod-c", 1)];
        assert_eq!(
            select_pod(&pods, 0).map(|p| p.pod_id.as_str()),
            Some("pod-b")
        );
    }

    #[test]
    fn load_tie_breaks_by_lowest_pod_id() {
        // Input order must not matter: the decision is on (load, pod_id).
        let pods = [pod("pod-z", 1), pod("pod-b", 1), pod("pod-m", 1)];
        assert_eq!(
            select_pod(&pods, 0).map(|p| p.pod_id.as_str()),
            Some("pod-b")
        );
    }

    #[test]
    fn pods_at_cap_are_discarded() {
        let pods = [pod("pod-a", 3), pod("pod-b", 2), pod("pod-c", 3)];
        // Cap 3: a and c are at cap; b (load 2 < 3) wins.
        assert_eq!(
            select_pod(&pods, 3).map(|p| p.pod_id.as_str()),
            Some("pod-b")
        );
    }

    #[test]
    fn all_at_cap_selects_nothing() {
        let pods = [pod("pod-a", 3), pod("pod-b", 4)];
        assert_eq!(select_pod(&pods, 3), None);
    }

    #[test]
    fn cap_zero_means_uncapped() {
        // Even absurd loads are never rejected when the cap is 0.
        let pods = [pod("pod-a", u64::MAX), pod("pod-b", u64::MAX - 1)];
        assert_eq!(
            select_pod(&pods, 0).map(|p| p.pod_id.as_str()),
            Some("pod-b")
        );
    }
}
