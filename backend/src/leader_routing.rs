//! Service routing publication for the elected leader.
//!
//! All replicas use `/health` for Kubernetes Pod readiness so a two-replica
//! Deployment can complete normally. The public Service additionally selects
//! [`LEADER_SERVING_LABEL=true`]. Only a Lease holder whose acquisition resync
//! succeeded publishes that label; publication clears any stale holder first.

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, ListParams, Patch, PatchParams};

pub const LEADER_SERVING_LABEL: &str = "fkst.chronoai.io/leader-serving";
pub const CONTROL_PLANE_SELECTOR: &str = "app.kubernetes.io/name=fkst-control-plane";

#[derive(Clone)]
pub struct LeaderServiceRouter {
    pods: Api<Pod>,
    pod_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LeaderRoutingError {
    #[error("leader routing pod was not found")]
    PodNotFound,
    #[error("leader routing API access denied")]
    Forbidden,
    #[error("leader routing API request failed")]
    Api,
    #[error("leader routing transport failed")]
    Transport,
    #[error("leader routing API returned an invalid pod")]
    InvalidResponse,
}

impl LeaderServiceRouter {
    pub fn new(client: kube::Client, namespace: &str, pod_name: String) -> Self {
        Self {
            pods: Api::namespaced(client, namespace),
            pod_name,
        }
    }

    /// Converge the control-plane Pod labels. Publication clears stale selected
    /// replicas before selecting this holder, so the Service never intentionally
    /// routes to two control planes. Withdrawal touches only this process's Pod.
    pub async fn reconcile(&self, publish: bool) -> Result<(), LeaderRoutingError> {
        let pods = self
            .pods
            .list(&ListParams::default().labels(CONTROL_PLANE_SELECTOR))
            .await
            .map_err(map_kube_error)?;
        let plan = routing_plan(&pods.items, &self.pod_name, publish)?;
        for (pod, selected) in plan {
            let value = if selected { "true" } else { "false" };
            let patch = serde_json::json!({
                "metadata": {
                    "labels": {
                        (LEADER_SERVING_LABEL): value
                    }
                }
            });
            let updated = self
                .pods
                .patch(&pod, &PatchParams::default(), &Patch::Merge(&patch))
                .await
                .map_err(map_kube_error)?;
            let actual = updated
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(LEADER_SERVING_LABEL))
                .map(String::as_str);
            if actual != Some(value) {
                return Err(LeaderRoutingError::InvalidResponse);
            }
        }
        Ok(())
    }
}

fn map_kube_error(error: kube::Error) -> LeaderRoutingError {
    match error {
        kube::Error::Api(response) => match response.code {
            403 => LeaderRoutingError::Forbidden,
            404 => LeaderRoutingError::PodNotFound,
            _ => LeaderRoutingError::Api,
        },
        _ => LeaderRoutingError::Transport,
    }
}

/// Return patches in safety order: stale selected pods first, current holder last.
fn routing_plan(
    pods: &[Pod],
    own_name: &str,
    publish: bool,
) -> Result<Vec<(String, bool)>, LeaderRoutingError> {
    let mut own = None;
    let mut stale = Vec::new();
    for pod in pods {
        let Some(name) = pod.metadata.name.as_deref() else {
            continue;
        };
        let selected = pod
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(LEADER_SERVING_LABEL))
            .is_some_and(|value| value == "true");
        if name == own_name {
            own = Some(selected);
        } else if publish && selected {
            stale.push((name.to_string(), false));
        }
    }

    match (publish, own) {
        (true, None) => Err(LeaderRoutingError::PodNotFound),
        (true, Some(false)) => {
            stale.push((own_name.to_string(), true));
            Ok(stale)
        }
        (true, Some(true)) => Ok(stale),
        (false, Some(true)) => Ok(vec![(own_name.to_string(), false)]),
        (false, Some(false) | None) => Ok(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn pod(name: &str, selected: bool) -> Pod {
        Pod {
            metadata: kube::api::ObjectMeta {
                name: Some(name.to_string()),
                labels: Some(BTreeMap::from([
                    (
                        "app.kubernetes.io/name".to_string(),
                        "fkst-control-plane".to_string(),
                    ),
                    (LEADER_SERVING_LABEL.to_string(), selected.to_string()),
                ])),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn publication_clears_stale_holder_before_selecting_self() {
        let plan =
            routing_plan(&[pod("pod-a", true), pod("pod-b", false)], "pod-b", true).expect("plan");
        assert_eq!(
            plan,
            vec![("pod-a".to_string(), false), ("pod-b".to_string(), true)]
        );
    }

    #[test]
    fn publication_is_idempotent_and_requires_own_pod() {
        assert!(routing_plan(&[pod("pod-a", true)], "pod-a", true)
            .expect("plan")
            .is_empty());
        assert_eq!(
            routing_plan(&[pod("pod-a", true)], "pod-b", true),
            Err(LeaderRoutingError::PodNotFound)
        );
    }

    #[test]
    fn withdrawal_only_clears_self_when_selected() {
        assert_eq!(
            routing_plan(&[pod("pod-a", true), pod("pod-b", true)], "pod-b", false).expect("plan"),
            vec![("pod-b".to_string(), false)]
        );
        assert!(routing_plan(&[pod("pod-a", true)], "pod-b", false)
            .expect("terminating pod is already withdrawn")
            .is_empty());
    }
}
