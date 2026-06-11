//! Placement policy: deciding which healthy pod runs a new session.

use super::health::PodLoad;

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
