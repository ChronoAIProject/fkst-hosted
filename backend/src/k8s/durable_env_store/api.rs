//! Narrow Kubernetes API seam used by the durable store and its migration.

use async_trait::async_trait;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use kube::api::{Api, DeleteParams, ListParams, PostParams, Preconditions};

#[derive(Debug, thiserror::Error)]
pub(super) enum ApiFailure {
    #[error("kubernetes resource version conflict")]
    Conflict,
    #[error("kubernetes API request failed: {0}")]
    Other(String),
}

fn classify(error: kube::Error) -> ApiFailure {
    match error {
        kube::Error::Api(response) if response.code == 409 => ApiFailure::Conflict,
        other => ApiFailure::Other(other.to_string()),
    }
}

#[async_trait]
pub(super) trait EnvironmentKubeApi: Send + Sync {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Option<Secret>, ApiFailure>;

    async fn list_secrets(
        &self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<Vec<Secret>, ApiFailure>;

    async fn create_secret(&self, namespace: &str, secret: &Secret) -> Result<(), ApiFailure>;

    async fn replace_secret(
        &self,
        namespace: &str,
        name: &str,
        secret: &Secret,
    ) -> Result<(), ApiFailure>;

    async fn delete_secret(
        &self,
        namespace: &str,
        name: &str,
        resource_version: Option<&str>,
    ) -> Result<bool, ApiFailure>;

    async fn list_config_maps(
        &self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<Vec<ConfigMap>, ApiFailure>;

    async fn delete_config_map(
        &self,
        namespace: &str,
        name: &str,
        resource_version: Option<&str>,
    ) -> Result<bool, ApiFailure>;
}

#[derive(Clone)]
pub(super) struct KubernetesEnvironmentApi {
    client: kube::Client,
}

impl KubernetesEnvironmentApi {
    pub(super) fn new(client: kube::Client) -> Self {
        Self { client }
    }

    fn secrets(&self, namespace: &str) -> Api<Secret> {
        Api::namespaced(self.client.clone(), namespace)
    }

    fn config_maps(&self, namespace: &str) -> Api<ConfigMap> {
        Api::namespaced(self.client.clone(), namespace)
    }
}

fn delete_params(resource_version: Option<&str>) -> DeleteParams {
    DeleteParams {
        preconditions: resource_version.map(|version| Preconditions {
            resource_version: Some(version.to_string()),
            uid: None,
        }),
        ..DeleteParams::default()
    }
}

#[async_trait]
impl EnvironmentKubeApi for KubernetesEnvironmentApi {
    async fn get_secret(&self, namespace: &str, name: &str) -> Result<Option<Secret>, ApiFailure> {
        self.secrets(namespace)
            .get_opt(name)
            .await
            .map_err(classify)
    }

    async fn list_secrets(
        &self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<Vec<Secret>, ApiFailure> {
        let params = ListParams::default().labels(label_selector);
        self.secrets(namespace)
            .list(&params)
            .await
            .map(|list| list.items)
            .map_err(classify)
    }

    async fn create_secret(&self, namespace: &str, secret: &Secret) -> Result<(), ApiFailure> {
        self.secrets(namespace)
            .create(&PostParams::default(), secret)
            .await
            .map(|_| ())
            .map_err(classify)
    }

    async fn replace_secret(
        &self,
        namespace: &str,
        name: &str,
        secret: &Secret,
    ) -> Result<(), ApiFailure> {
        self.secrets(namespace)
            .replace(name, &PostParams::default(), secret)
            .await
            .map(|_| ())
            .map_err(classify)
    }

    async fn delete_secret(
        &self,
        namespace: &str,
        name: &str,
        resource_version: Option<&str>,
    ) -> Result<bool, ApiFailure> {
        match self
            .secrets(namespace)
            .delete(name, &delete_params(resource_version))
            .await
        {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(response)) if response.code == 404 => Ok(false),
            Err(error) => Err(classify(error)),
        }
    }

    async fn list_config_maps(
        &self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<Vec<ConfigMap>, ApiFailure> {
        let params = ListParams::default().labels(label_selector);
        self.config_maps(namespace)
            .list(&params)
            .await
            .map(|list| list.items)
            .map_err(classify)
    }

    async fn delete_config_map(
        &self,
        namespace: &str,
        name: &str,
        resource_version: Option<&str>,
    ) -> Result<bool, ApiFailure> {
        match self
            .config_maps(namespace)
            .delete(name, &delete_params(resource_version))
            .await
        {
            Ok(_) => Ok(true),
            Err(kube::Error::Api(response)) if response.code == 404 => Ok(false),
            Err(error) => Err(classify(error)),
        }
    }
}
