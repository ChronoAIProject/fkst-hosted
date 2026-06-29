//! The Kubernetes API client wrapper.

/// Errors building or probing the Kubernetes client.
#[derive(Debug, thiserror::Error)]
pub enum KubeError {
    /// The client could not be built (no in-cluster ServiceAccount and no
    /// usable kubeconfig), or an API call failed.
    #[error("kubernetes client error: {0}")]
    Client(#[from] kube::Error),
}

/// A Kubernetes API client bound to the namespace per-session Jobs live in.
///
/// Built once at startup when pod dispatch is enabled. `kube::Client` is
/// internally reference-counted, so `KubeClient` is cheap to clone and hand to
/// the launcher + watcher.
#[derive(Clone)]
pub struct KubeClient {
    client: kube::Client,
    namespace: String,
}

impl KubeClient {
    /// Build from the ambient environment: the in-cluster ServiceAccount when
    /// running inside a pod, otherwise the local kubeconfig. `try_default`
    /// performs that inference.
    pub async fn from_inferred(namespace: impl Into<String>) -> Result<Self, KubeError> {
        let client = kube::Client::try_default().await?;
        Ok(Self {
            client,
            namespace: namespace.into(),
        })
    }

    /// Wrap an already-built client (constructor seam for tests/callers that
    /// build the `kube::Client` themselves).
    pub fn new(client: kube::Client, namespace: impl Into<String>) -> Self {
        Self {
            client,
            namespace: namespace.into(),
        }
    }

    /// The namespace per-session Jobs + Secrets are created in.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The underlying client (the launcher/watcher build typed `Api`s from it).
    pub fn client(&self) -> &kube::Client {
        &self.client
    }

    /// Startup readiness probe: confirm the API server is reachable and return
    /// its reported `major.minor` version.
    pub async fn check_reachable(&self) -> Result<String, KubeError> {
        let info = self.client.apiserver_version().await?;
        Ok(format!("{}.{}", info.major, info.minor))
    }
}
