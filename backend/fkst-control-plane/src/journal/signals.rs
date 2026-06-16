//! Engine progress signal types: lifecycle transitions, the decoded
//! progress-signal envelope, and the redo skip-set.

use std::collections::HashSet;

use crate::journal::model::LogRef;

/// A session lifecycle transition (mirrors `sessions.status` plus the
/// journal-only `malformed_raised` / `log_watermark` anomalies).
#[derive(Debug, Clone, PartialEq)]
pub enum Transition {
    Spawned {
        pid: i32,
    },
    Validating,
    Running,
    Stopping,
    Stopped {
        exit_code: Option<i32>,
    },
    Failed {
        exit_code: Option<i32>,
        error: String,
    },
    LogWatermark(LogRef),
    MalformedRaised {
        detail: String,
    },
}

impl Transition {
    /// Stable wire name of the transition.
    pub fn name(&self) -> &'static str {
        match self {
            Transition::Spawned { .. } => "spawned",
            Transition::Validating => "validating",
            Transition::Running => "running",
            Transition::Stopping => "stopping",
            Transition::Stopped { .. } => "stopped",
            Transition::Failed { .. } => "failed",
            Transition::LogWatermark(_) => "log_watermark",
            Transition::MalformedRaised { .. } => "malformed_raised",
        }
    }
}

/// One lifecycle observation.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleEvent {
    pub transition: Transition,
    pub at: bson::DateTime,
}

impl LifecycleEvent {
    /// Lifecycle event timestamped "now".
    pub fn now(transition: Transition) -> Self {
        Self {
            transition,
            at: bson::DateTime::now(),
        }
    }
}

/// One decoded engine progress signal. The journaler assigns `seq` itself
/// (a per-session monotonic total order over BOTH kinds).
#[derive(Debug, Clone, PartialEq)]
pub enum ProgressSignal {
    /// A parsed `RAISED: <b64-json>` line (the decoded envelope, verbatim).
    Raised { event_json: serde_json::Value },
    /// A session lifecycle transition.
    Lifecycle(LifecycleEvent),
}

/// The redo skip-set: the `idem_key`s GitHub says are already
/// completed-and-durable for this logical run.
#[derive(Debug, Clone, Default)]
pub struct SkipSet(HashSet<String>);

impl SkipSet {
    pub fn contains(&self, idem_key: &str) -> bool {
        self.0.contains(idem_key)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<String> for SkipSet {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_set_membership_size_and_emptiness() {
        let empty = SkipSet::default();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert!(!empty.contains("k1"));

        let set: SkipSet = ["k1".to_string(), "k2".to_string(), "k1".to_string()]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 2, "duplicates collapse");
        assert!(set.contains("k1"));
        assert!(set.contains("k2"));
        assert!(!set.contains("k3"));
        assert!(!set.is_empty());
    }
}
