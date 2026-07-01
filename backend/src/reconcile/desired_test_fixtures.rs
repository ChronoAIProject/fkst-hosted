//! Shared test fixtures for the `desired` planner tests, split out so the plan +
//! hash test files each stay under the 500-line limit. `pub(super)` so the sibling
//! test modules (`desired_plan_tests`, `desired_hash_tests`) can reuse them.

use std::collections::{HashMap, HashSet};

use k8s_openapi::chrono::{DateTime, Utc};

use super::{LivePod, PodLiveness, SessionDef, SessionRegistration};
use crate::goals::trigger_parse::PackageRef;
use crate::models::RepoRef;
use crate::reconcile_config::ReconcileConfig;

/// A fixed wall-clock instant to anchor the relative pod timestamps.
pub(super) fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_000_000, 0).expect("valid fixed timestamp")
}

/// `now` shifted back by `secs` seconds.
pub(super) fn ago(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_000_000 - secs, 0).expect("valid shifted timestamp")
}

/// A config with explicit idle-grace + min-lifetime, everything else default.
pub(super) fn cfg(idle_grace: u64, min_lifetime: u64) -> ReconcileConfig {
    ReconcileConfig {
        session_idle_grace_secs: idle_grace,
        pod_min_lifetime_secs: min_lifetime,
        ..ReconcileConfig::default()
    }
}

pub(super) fn reg(session_id: &str, trigger_issue: i64, config_hash: &str) -> SessionRegistration {
    SessionRegistration {
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue,
        trigger_author_id: 7,
        def: SessionDef {
            name: "demo".to_string(),
            packages: vec![],
            work_label: "wl".to_string(),
            environment: None,
        },
        session_id: session_id.to_string(),
        config_hash: config_hash.to_string(),
    }
}

pub(super) fn pod(
    session_id: &str,
    trigger_issue: i64,
    liveness: PodLiveness,
    created_at: DateTime<Utc>,
    last_pending_at: Option<DateTime<Utc>>,
    config_hash: Option<&str>,
) -> LivePod {
    LivePod {
        session_id: session_id.to_string(),
        trigger_issue,
        liveness,
        created_at,
        last_pending_at,
        config_hash: config_hash.map(str::to_string),
    }
}

pub(super) fn pending(entries: &[(&str, bool)]) -> HashMap<String, bool> {
    entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

pub(super) fn latched(issues: &[i64]) -> HashSet<i64> {
    issues.iter().copied().collect()
}

pub(super) fn pkg(owner: &str, repo: &str, git_ref: &str, path: &str) -> PackageRef {
    PackageRef {
        owner: owner.to_string(),
        repo: repo.to_string(),
        git_ref: git_ref.to_string(),
        path: path.to_string(),
    }
}
