use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::{ConfigMap, Secret};

use super::api::{ApiFailure, EnvironmentKubeApi};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Operation {
    CreateSecret,
    ReplaceSecret,
    DeleteSecret,
    DeleteConfigMap,
}

#[derive(Default)]
struct State {
    next_version: u64,
    secrets: BTreeMap<(String, String), Secret>,
    config_maps: BTreeMap<(String, String), ConfigMap>,
    fail_once: Option<Operation>,
}

#[derive(Default)]
pub(super) struct FakeEnvironmentApi {
    state: Mutex<State>,
}

fn name(metadata: &kube::core::ObjectMeta) -> Result<String, ApiFailure> {
    metadata
        .name
        .clone()
        .ok_or_else(|| ApiFailure::Other("test object has no name".to_string()))
}

fn labels_match(metadata: &kube::core::ObjectMeta, selector: &str) -> bool {
    if selector.is_empty() {
        return true;
    }
    let Some(labels) = metadata.labels.as_ref() else {
        return false;
    };
    selector.split(',').all(|requirement| {
        let Some((key, value)) = requirement.split_once('=') else {
            return false;
        };
        labels.get(key).map(String::as_str) == Some(value)
    })
}

fn assign_version(state: &mut State, metadata: &mut kube::core::ObjectMeta) {
    state.next_version += 1;
    metadata.resource_version = Some(state.next_version.to_string());
}

fn maybe_fail(state: &mut State, operation: Operation) -> Result<(), ApiFailure> {
    if state.fail_once == Some(operation) {
        state.fail_once = None;
        return Err(ApiFailure::Other("injected API failure".to_string()));
    }
    Ok(())
}

impl FakeEnvironmentApi {
    pub(super) fn fail_once(&self, operation: Operation) {
        self.state.lock().expect("state").fail_once = Some(operation);
    }

    pub(super) fn insert_secret(&self, namespace: &str, mut secret: Secret) {
        let mut state = self.state.lock().expect("state");
        assign_version(&mut state, &mut secret.metadata);
        let object = name(&secret.metadata).expect("named secret");
        state
            .secrets
            .insert((namespace.to_string(), object), secret);
    }

    pub(super) fn insert_config_map(&self, namespace: &str, mut config_map: ConfigMap) {
        let mut state = self.state.lock().expect("state");
        assign_version(&mut state, &mut config_map.metadata);
        let object = name(&config_map.metadata).expect("named config map");
        state
            .config_maps
            .insert((namespace.to_string(), object), config_map);
    }

    pub(super) fn secret(&self, namespace: &str, object: &str) -> Option<Secret> {
        self.state
            .lock()
            .expect("state")
            .secrets
            .get(&(namespace.to_string(), object.to_string()))
            .cloned()
    }

    pub(super) fn config_map(&self, namespace: &str, object: &str) -> Option<ConfigMap> {
        self.state
            .lock()
            .expect("state")
            .config_maps
            .get(&(namespace.to_string(), object.to_string()))
            .cloned()
    }

    pub(super) fn mutate_secret(
        &self,
        namespace: &str,
        object: &str,
        mutate: impl FnOnce(&mut Secret),
    ) {
        let mut state = self.state.lock().expect("state");
        let secret = state
            .secrets
            .get_mut(&(namespace.to_string(), object.to_string()))
            .expect("secret exists");
        mutate(secret);
    }
}

#[async_trait]
impl EnvironmentKubeApi for FakeEnvironmentApi {
    async fn get_secret(
        &self,
        namespace: &str,
        object: &str,
    ) -> Result<Option<Secret>, ApiFailure> {
        Ok(self.secret(namespace, object))
    }

    async fn list_secrets(
        &self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<Vec<Secret>, ApiFailure> {
        Ok(self
            .state
            .lock()
            .expect("state")
            .secrets
            .iter()
            .filter(|((record_namespace, _), secret)| {
                record_namespace == namespace && labels_match(&secret.metadata, label_selector)
            })
            .map(|(_, secret)| secret.clone())
            .collect())
    }

    async fn create_secret(&self, namespace: &str, secret: &Secret) -> Result<(), ApiFailure> {
        let mut state = self.state.lock().expect("state");
        maybe_fail(&mut state, Operation::CreateSecret)?;
        let object = name(&secret.metadata)?;
        let key = (namespace.to_string(), object);
        if state.secrets.contains_key(&key) {
            return Err(ApiFailure::Conflict);
        }
        let mut secret = secret.clone();
        assign_version(&mut state, &mut secret.metadata);
        state.secrets.insert(key, secret);
        Ok(())
    }

    async fn replace_secret(
        &self,
        namespace: &str,
        object: &str,
        secret: &Secret,
    ) -> Result<(), ApiFailure> {
        let mut state = self.state.lock().expect("state");
        maybe_fail(&mut state, Operation::ReplaceSecret)?;
        let key = (namespace.to_string(), object.to_string());
        let current = state.secrets.get(&key).ok_or(ApiFailure::Conflict)?;
        if secret.metadata.resource_version != current.metadata.resource_version {
            return Err(ApiFailure::Conflict);
        }
        let mut secret = secret.clone();
        assign_version(&mut state, &mut secret.metadata);
        state.secrets.insert(key, secret);
        Ok(())
    }

    async fn delete_secret(
        &self,
        namespace: &str,
        object: &str,
        resource_version: Option<&str>,
    ) -> Result<bool, ApiFailure> {
        let mut state = self.state.lock().expect("state");
        maybe_fail(&mut state, Operation::DeleteSecret)?;
        let key = (namespace.to_string(), object.to_string());
        let Some(current) = state.secrets.get(&key) else {
            return Ok(false);
        };
        if resource_version.is_some()
            && current.metadata.resource_version.as_deref() != resource_version
        {
            return Err(ApiFailure::Conflict);
        }
        state.secrets.remove(&key);
        Ok(true)
    }

    async fn list_config_maps(
        &self,
        namespace: &str,
        label_selector: &str,
    ) -> Result<Vec<ConfigMap>, ApiFailure> {
        Ok(self
            .state
            .lock()
            .expect("state")
            .config_maps
            .iter()
            .filter(|((record_namespace, _), config_map)| {
                record_namespace == namespace && labels_match(&config_map.metadata, label_selector)
            })
            .map(|(_, config_map)| config_map.clone())
            .collect())
    }

    async fn delete_config_map(
        &self,
        namespace: &str,
        object: &str,
        resource_version: Option<&str>,
    ) -> Result<bool, ApiFailure> {
        let mut state = self.state.lock().expect("state");
        maybe_fail(&mut state, Operation::DeleteConfigMap)?;
        let key = (namespace.to_string(), object.to_string());
        let Some(current) = state.config_maps.get(&key) else {
            return Ok(false);
        };
        if resource_version.is_some()
            && current.metadata.resource_version.as_deref() != resource_version
        {
            return Err(ApiFailure::Conflict);
        }
        state.config_maps.remove(&key);
        Ok(true)
    }
}
