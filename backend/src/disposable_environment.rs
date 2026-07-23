//! Private, process-local handoff for one-time session environments.
//!
//! The create-session API receives commands, variables, and write-only secrets,
//! but the corresponding GitHub trigger contains only
//! [`DISPOSABLE_ENVIRONMENT_MARKER`]. Once GitHub assigns an issue number, the
//! payload is held in this in-memory registry just long enough for reconciliation
//! to deliver the complete credential bundle to the sandbox. It is never exposed
//! through a read API or written to a durable store.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use utoipa::ToSchema;
use zeroize::Zeroize;

use crate::config::Config;
use crate::environment_validation::{validate_entries, validate_install};
use crate::error::AppError;

/// The only disposable-environment text allowed to cross the GitHub boundary.
/// It contains no user-supplied data and is deliberately not a valid saved
/// environment name.
pub const DISPOSABLE_ENVIRONMENT_MARKER: &str = "Disposable one-time environment. Details are injected privately into the session sandbox and are not stored in this GitHub issue.";

/// Secret-bearing part of `POST .../sessions`. `Debug` and `Serialize` are
/// implemented by neither derive nor accident: the server only accepts this
/// shape and never echoes or logs it.
#[derive(Clone, Deserialize, ToSchema)]
pub struct DisposableEnvironmentRequest {
    /// Ordered shell commands run once in the real session sandbox before the
    /// engine starts.
    #[serde(default)]
    pub install: Vec<String>,
    /// Non-secret process environment variables.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    /// Write-only secret process environment variables. Values are never
    /// returned by any response schema.
    #[serde(default)]
    #[schema(write_only)]
    pub secrets: BTreeMap<String, String>,
}

impl fmt::Debug for DisposableEnvironmentRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DisposableEnvironmentRequest([REDACTED])")
    }
}

impl Drop for DisposableEnvironmentRequest {
    fn drop(&mut self) {
        for command in &mut self.install {
            command.zeroize();
        }
        zeroize_map(&mut self.variables);
        zeroize_map(&mut self.secrets);
    }
}

fn zeroize_map(map: &mut BTreeMap<String, String>) {
    for (mut key, mut value) in std::mem::take(map) {
        key.zeroize();
        value.zeroize();
    }
}

impl DisposableEnvironmentRequest {
    /// Validate the payload before any GitHub request is made.
    pub fn validate(&self, config: &Config) -> Result<(), AppError> {
        if self.install.is_empty() && self.variables.is_empty() && self.secrets.is_empty() {
            return Err(AppError::Unprocessable(
                "disposable_environment must contain at least one install command, variable, or secret"
                    .to_string(),
            ));
        }
        validate_install(&self.install, config, false)?;
        validate_entries(&self.variables, config)?;
        validate_entries(&self.secrets, config)?;
        if let Some(key) = self
            .variables
            .keys()
            .find(|key| self.secrets.contains_key(*key))
        {
            return Err(AppError::Unprocessable(format!(
                "env var {key:?} cannot be both a variable and a secret"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct RegistryKey {
    owner: String,
    repo: String,
    issue_number: i64,
}

struct RegistryEntry {
    creator_id: i64,
    install: Vec<SecretString>,
    variables: BTreeMap<String, SecretString>,
    secrets: BTreeMap<String, SecretString>,
}

impl Drop for RegistryEntry {
    fn drop(&mut self) {
        for (mut key, value) in std::mem::take(&mut self.variables) {
            key.zeroize();
            drop(value);
        }
        for (mut key, value) in std::mem::take(&mut self.secrets) {
            key.zeroize();
            drop(value);
        }
    }
}

/// Materialized launch inputs. This intentionally has no `Debug`, `Serialize`,
/// or response DTO implementation.
pub struct DisposableEnvironmentMaterial {
    pub install: Vec<String>,
    pub user_env: BTreeMap<String, String>,
    pub secret_keys: Vec<String>,
}

/// A lookup distinguishes an absent handoff from a creator mismatch without
/// exposing either case to GitHub in a data-bearing error.
pub enum DisposableEnvironmentLookup {
    Found(DisposableEnvironmentMaterial),
    Missing,
    CreatorMismatch,
}

/// Cheap-to-clone, process-local registry shared by the HTTP handler and the
/// active reconcile generation.
#[derive(Clone, Default)]
pub struct DisposableEnvironmentRegistry {
    inner: Arc<Mutex<HashMap<RegistryKey, RegistryEntry>>>,
}

impl DisposableEnvironmentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<RegistryKey, RegistryEntry>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Insert immediately after GitHub returns the new trigger issue number.
    pub fn insert(
        &self,
        owner: &str,
        repo: &str,
        issue_number: i64,
        creator_id: i64,
        request: &DisposableEnvironmentRequest,
    ) {
        let entry = RegistryEntry {
            creator_id,
            install: request
                .install
                .iter()
                .cloned()
                .map(SecretString::from)
                .collect(),
            variables: request
                .variables
                .iter()
                .map(|(key, value)| (key.clone(), SecretString::from(value.clone())))
                .collect(),
            secrets: request
                .secrets
                .iter()
                .map(|(key, value)| (key.clone(), SecretString::from(value.clone())))
                .collect(),
        };
        self.lock().insert(
            RegistryKey {
                owner: owner.to_string(),
                repo: repo.to_string(),
                issue_number,
            },
            entry,
        );
    }

    /// Resolve only when the effective GitHub creator id matches the verified
    /// caller that submitted the private payload.
    pub fn resolve(
        &self,
        owner: &str,
        repo: &str,
        issue_number: i64,
        creator_id: i64,
    ) -> DisposableEnvironmentLookup {
        let key = RegistryKey {
            owner: owner.to_string(),
            repo: repo.to_string(),
            issue_number,
        };
        let entries = self.lock();
        let Some(entry) = entries.get(&key) else {
            return DisposableEnvironmentLookup::Missing;
        };
        if entry.creator_id != creator_id {
            return DisposableEnvironmentLookup::CreatorMismatch;
        }

        let mut user_env: BTreeMap<String, String> = entry
            .variables
            .iter()
            .map(|(key, value)| (key.clone(), value.expose_secret().to_string()))
            .collect();
        let secret_keys: Vec<String> = entry.secrets.keys().cloned().collect();
        for (key, value) in &entry.secrets {
            user_env.insert(key.clone(), value.expose_secret().to_string());
        }
        DisposableEnvironmentLookup::Found(DisposableEnvironmentMaterial {
            install: entry
                .install
                .iter()
                .map(|value| value.expose_secret().to_string())
                .collect(),
            user_env,
            secret_keys,
        })
    }

    /// Delete a payload only after the backend acknowledges the complete bundle.
    /// A mismatched creator cannot consume or remove someone else's handoff.
    pub fn remove(&self, owner: &str, repo: &str, issue_number: i64, creator_id: i64) -> bool {
        let key = RegistryKey {
            owner: owner.to_string(),
            repo: repo.to_string(),
            issue_number,
        };
        let mut entries = self.lock();
        if entries.get(&key).map(|entry| entry.creator_id) != Some(creator_id) {
            return false;
        }
        entries.remove(&key).is_some()
    }

    /// Forget an unconsumed handoff when its trigger is closed. The key itself
    /// proves scope: this registry contains only session-create payloads.
    pub fn remove_issue(&self, owner: &str, repo: &str, issue_number: i64) -> bool {
        self.lock()
            .remove(&RegistryKey {
                owner: owner.to_string(),
                repo: repo.to_string(),
                issue_number,
            })
            .is_some()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> DisposableEnvironmentRequest {
        DisposableEnvironmentRequest {
            install: vec!["apt-get install -y jq".to_string()],
            variables: BTreeMap::from([("APP_MODE".to_string(), "test".to_string())]),
            secrets: BTreeMap::from([("DEPLOY_TOKEN".to_string(), "top-secret".to_string())]),
        }
    }

    #[test]
    fn validation_accepts_variables_without_install_commands() {
        let request = DisposableEnvironmentRequest {
            install: Vec::new(),
            variables: BTreeMap::from([("APP_MODE".to_string(), "test".to_string())]),
            secrets: BTreeMap::new(),
        };
        request.validate(&Config::default()).expect("valid");
    }

    #[test]
    fn validation_rejects_empty_and_overlapping_payloads() {
        let empty = DisposableEnvironmentRequest {
            install: Vec::new(),
            variables: BTreeMap::new(),
            secrets: BTreeMap::new(),
        };
        assert!(matches!(
            empty.validate(&Config::default()),
            Err(AppError::Unprocessable(_))
        ));

        let overlap = DisposableEnvironmentRequest {
            install: Vec::new(),
            variables: BTreeMap::from([("TOKEN".to_string(), "plain".to_string())]),
            secrets: BTreeMap::from([("TOKEN".to_string(), "secret".to_string())]),
        };
        assert!(matches!(
            overlap.validate(&Config::default()),
            Err(AppError::Unprocessable(_))
        ));
    }

    #[test]
    fn validation_reuses_reserved_key_and_size_limits() {
        let reserved = DisposableEnvironmentRequest {
            install: Vec::new(),
            variables: BTreeMap::from([("GITHUB_TOKEN".to_string(), "x".to_string())]),
            secrets: BTreeMap::new(),
        };
        assert!(matches!(
            reserved.validate(&Config::default()),
            Err(AppError::Unprocessable(_))
        ));

        let mut config = Config::default();
        config.env.install_max_commands = 1;
        let too_many_commands = DisposableEnvironmentRequest {
            install: vec!["one".to_string(), "two".to_string()],
            variables: BTreeMap::new(),
            secrets: BTreeMap::new(),
        };
        assert!(matches!(
            too_many_commands.validate(&config),
            Err(AppError::Unprocessable(_))
        ));

        config.vault_value_byte_cap = 3;
        let oversized_value = DisposableEnvironmentRequest {
            install: Vec::new(),
            variables: BTreeMap::from([("APP_MODE".to_string(), "large".to_string())]),
            secrets: BTreeMap::new(),
        };
        assert!(matches!(
            oversized_value.validate(&config),
            Err(AppError::Unprocessable(_))
        ));
    }

    #[test]
    fn registry_enforces_creator_and_removes_only_for_the_owner() {
        let registry = DisposableEnvironmentRegistry::new();
        registry.insert("acme", "site", 7, 42, &request());

        assert!(matches!(
            registry.resolve("acme", "site", 7, 99),
            DisposableEnvironmentLookup::CreatorMismatch
        ));
        assert!(!registry.remove("acme", "site", 7, 99));
        assert_eq!(registry.len(), 1);

        let DisposableEnvironmentLookup::Found(material) = registry.resolve("acme", "site", 7, 42)
        else {
            panic!("creator should resolve the payload")
        };
        assert_eq!(material.install.len(), 1);
        assert_eq!(material.user_env.len(), 2);
        assert_eq!(material.secret_keys, ["DEPLOY_TOKEN"]);
        assert!(registry.remove("acme", "site", 7, 42));
        assert!(matches!(
            registry.resolve("acme", "site", 7, 42),
            DisposableEnvironmentLookup::Missing
        ));
    }

    #[test]
    fn debug_output_is_fully_redacted() {
        let rendered = format!("{:?}", request());
        assert_eq!(rendered, "DisposableEnvironmentRequest([REDACTED])");
        assert!(!rendered.contains("top-secret"));
        assert!(!rendered.contains("DEPLOY_TOKEN"));
    }
}
