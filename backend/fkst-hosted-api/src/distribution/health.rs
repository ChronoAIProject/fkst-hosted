//! Healthy-pod view: which pods may receive placements, and each pod's
//! current active-session load.
//!
//! The [`HealthView`] trait is the seam toward the real pod registry /
//! heartbeat source of truth (`pm-health`, downstream work). The production
//! implementation today is [`SelfOnlyHealth`]: the deployment is
//! single-replica, so the healthy set is exactly this pod, with its load
//! computed from the `sessions` collection.

use async_trait::async_trait;
use bson::{doc, Bson};

use crate::db::{Db, SESSIONS};
use crate::leases::PoolError;
use crate::models::SessionStatus;
use crate::sessions::repo::status_bson;

/// A healthy pod and its current active-session load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodLoad {
    pub pod_id: String,
    pub active_sessions: u64,
}

/// The statuses that count toward a pod's load and keep a session eligible
/// for takeover. Terminal sessions (`stopped` / `failed`) are ignored.
pub(crate) const ACTIVE_STATUSES: [SessionStatus; 4] = [
    SessionStatus::Pending,
    SessionStatus::Validating,
    SessionStatus::Running,
    SessionStatus::Stopping,
];

/// The active statuses as their BSON wire strings, for `$in` filters.
pub(crate) fn active_status_bson() -> Vec<Bson> {
    ACTIVE_STATUSES.iter().map(|s| status_bson(*s)).collect()
}

/// Source of the healthy-pod set and per-pod loads for placement and
/// takeover decisions. Pods with zero active sessions still appear (load 0).
#[async_trait]
pub trait HealthView: Send + Sync {
    async fn healthy_pods_and_loads(&self) -> Result<Vec<PodLoad>, PoolError>;
}

/// v1 production health view: the healthy set is exactly this pod, with its
/// load joined from the `sessions` load aggregation. With a single healthy
/// pod every placement lands on self and every dead-holder lease (holder !=
/// self) is eligible for takeover, which is exactly the single-replica
/// recovery posture.
pub struct SelfOnlyHealth {
    db: Db,
    pod_id: String,
}

impl SelfOnlyHealth {
    pub fn new(db: Db, pod_id: impl Into<String>) -> Self {
        Self {
            db,
            pod_id: pod_id.into(),
        }
    }
}

#[async_trait]
impl HealthView for SelfOnlyHealth {
    async fn healthy_pods_and_loads(&self) -> Result<Vec<PodLoad>, PoolError> {
        let loads = pod_loads(&self.db).await?;
        let active_sessions = loads
            .iter()
            .find(|load| load.pod_id == self.pod_id)
            .map(|load| load.active_sessions)
            .unwrap_or(0);
        tracing::debug!(
            pod = %self.pod_id,
            active_sessions,
            "self-only health view computed"
        );
        Ok(vec![PodLoad {
            pod_id: self.pod_id.clone(),
            active_sessions,
        }])
    }
}

/// Load aggregation over `sessions`: count active sessions grouped by
/// `pod_id` (unassigned sessions, `pod_id: null`, are excluded — they are
/// nobody's load until placed). Pods absent from the result have load 0 and
/// are supplied by the healthy-pod set join in the caller.
pub(crate) async fn pod_loads(db: &Db) -> Result<Vec<PodLoad>, PoolError> {
    let pipeline = [
        doc! { "$match": {
            "status": { "$in": active_status_bson() },
            "pod_id": { "$ne": null },
        } },
        doc! { "$group": { "_id": "$pod_id", "active_sessions": { "$sum": 1 } } },
    ];
    let mut cursor = db
        .collection::<bson::Document>(SESSIONS)
        .aggregate(pipeline)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "session load aggregation failed");
            PoolError::Mongo(error)
        })?;
    let mut loads = Vec::new();
    while cursor.advance().await.map_err(PoolError::Mongo)? {
        let row = cursor.deserialize_current().map_err(PoolError::Mongo)?;
        let Some(pod_id) = row.get_str("_id").ok() else {
            continue;
        };
        let active_sessions = row
            .get_i64("active_sessions")
            .or_else(|_| row.get_i32("active_sessions").map(i64::from))
            .unwrap_or(0)
            .max(0) as u64;
        loads.push(PodLoad {
            pod_id: pod_id.to_string(),
            active_sessions,
        });
    }
    Ok(loads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_statuses_exclude_terminal_states() {
        assert!(!ACTIVE_STATUSES.contains(&SessionStatus::Stopped));
        assert!(!ACTIVE_STATUSES.contains(&SessionStatus::Failed));
        assert_eq!(ACTIVE_STATUSES.len(), 4);
    }

    #[test]
    fn active_status_bson_serializes_to_lowercase_strings() {
        assert_eq!(
            active_status_bson(),
            vec![
                Bson::String("pending".to_string()),
                Bson::String("validating".to_string()),
                Bson::String("running".to_string()),
                Bson::String("stopping".to_string()),
            ]
        );
    }
}
