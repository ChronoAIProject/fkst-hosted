//! Durable environment-store unit, concurrency, and migration tests.

use std::collections::BTreeMap;
use std::sync::Arc;

use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::ByteString;
use secrecy::SecretString;

use super::record::{
    ProfileEnvelope, CIPHERTEXT_DATA_KEY, NONCE_DATA_KEY, REVISION_ANNOTATION,
    SECRET_KEYS_ANNOTATION,
};
use super::test_support::{FakeEnvironmentApi, Operation};
use super::*;
use crate::k8s::env_store::meta::{
    content_hash, env_annotations, env_labels, env_object_name, INSTALL_KEY, VARIABLES_KEY,
};

const DURABLE_NAMESPACE: &str = "durable-environments";
const LEGACY_NAMESPACE: &str = "chronoai-fkst";
const USER_ID: i64 = 250_120_269;
const LOGIN: &str = "chronoai-shining";
const NAME: &str = "video-studio";
const VALIDATED_AT: &str = "2026-07-21T00:00:00Z";
const IMAGE: &str = "fkst-control-plane:test";
const KEY_B64: &str = "ERERERERERERERERERERERERERERERERERERERERERE=";
const OTHER_KEY_B64: &str = "IiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiIiI=";

fn variables(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("PUBLIC_VALUE".to_string(), value.to_string())])
}

fn secrets(value: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("DEPLOY_KEY".to_string(), value.to_string())])
}

fn install(command: &str) -> Vec<String> {
    vec![command.to_string()]
}

fn build_store(api: Arc<FakeEnvironmentApi>) -> DurableEnvStore {
    DurableEnvStore::from_api(api, DURABLE_NAMESPACE, &SecretString::from(KEY_B64))
        .expect("valid store")
}

#[allow(clippy::too_many_arguments)]
async fn put(
    store: &DurableEnvStore,
    install: &[String],
    variables: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, String>,
    expected_version: Option<&str>,
) -> Result<(), AppError> {
    let keys: Vec<String> = secrets.keys().cloned().collect();
    store
        .put_environment(
            USER_ID,
            LOGIN,
            NAME,
            install,
            variables,
            secrets,
            VALIDATED_AT,
            &content_hash(install, variables, &keys),
            IMAGE,
            expected_version,
        )
        .await
}

fn legacy_pair(
    install: &[String],
    variables: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, String>,
) -> (ConfigMap, Secret) {
    let object = env_object_name(USER_ID, NAME);
    let keys: Vec<String> = secrets.keys().cloned().collect();
    let hash = content_hash(install, variables, &keys);
    let metadata = ObjectMeta {
        name: Some(object),
        labels: Some(env_labels(USER_ID, LOGIN)),
        annotations: Some(env_annotations(NAME, VALIDATED_AT, &hash, IMAGE)),
        ..ObjectMeta::default()
    };
    let config_map = ConfigMap {
        metadata: metadata.clone(),
        data: Some(BTreeMap::from([
            (
                INSTALL_KEY.to_string(),
                serde_json::to_string(install).expect("install json"),
            ),
            (
                VARIABLES_KEY.to_string(),
                serde_json::to_string(variables).expect("variables json"),
            ),
        ])),
        ..ConfigMap::default()
    };
    let secret = Secret {
        metadata,
        data: Some(
            secrets
                .iter()
                .map(|(key, value)| (key.clone(), ByteString(value.as_bytes().to_vec())))
                .collect(),
        ),
        type_: Some("Opaque".to_string()),
        ..Secret::default()
    };
    (config_map, secret)
}

#[tokio::test]
async fn round_trip_uses_one_encrypted_secret_and_preserves_public_shapes() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    let install = install("tool install --channel stable");
    let variables = variables("public-value");
    let secrets = secrets("top-secret-value");
    put(&store, &install, &variables, &secrets, None)
        .await
        .expect("create");

    let object = env_object_name(USER_ID, NAME);
    let stored = api
        .secret(DURABLE_NAMESPACE, &object)
        .expect("single durable secret");
    let data = stored.data.as_ref().expect("encrypted data");
    assert_eq!(
        data.keys().cloned().collect::<Vec<_>>(),
        vec![CIPHERTEXT_DATA_KEY.to_string(), NONCE_DATA_KEY.to_string()]
    );
    let persisted_json = serde_json::to_string(&stored).expect("secret json");
    for plaintext in [
        "tool install --channel stable",
        "public-value",
        "top-secret-value",
    ] {
        assert!(!persisted_json.contains(plaintext));
    }
    assert_eq!(
        stored
            .metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get(SECRET_KEYS_ANNOTATION))
            .map(String::as_str),
        Some("[\"DEPLOY_KEY\"]")
    );

    let public = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .expect("record");
    assert_eq!(public.install, install);
    assert_eq!(public.variables, variables);
    assert_eq!(public.secret_keys, vec!["DEPLOY_KEY"]);
    assert!(public.store_version.is_some());
    let public_json = serde_json::to_value(&public).expect("public record json");
    assert!(public_json.get("store_version").is_none());
    assert!(public_json.get("private_content_hash").is_none());
    assert_eq!(store.count_environments(USER_ID).await.expect("count"), 1);
    assert_eq!(
        store.list_environments(USER_ID).await.expect("list")[0].secret_count,
        1
    );

    let (_, materialized, secret_keys) = store
        .load_environment_for_session(USER_ID, NAME)
        .await
        .expect("load")
        .expect("record");
    assert_eq!(materialized["PUBLIC_VALUE"], "public-value");
    assert_eq!(materialized["DEPLOY_KEY"], "top-secret-value");
    assert_eq!(secret_keys, vec!["DEPLOY_KEY"]);

    assert!(store
        .delete_environment(USER_ID, NAME)
        .await
        .expect("delete"));
    assert!(store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get after delete")
        .is_none());
    assert!(!store
        .delete_environment(USER_ID, NAME)
        .await
        .expect("idempotent delete"));
}

#[test]
fn every_encryption_uses_a_fresh_nonce_and_debug_is_redacted() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api);
    let install = install("echo ready");
    let variables = variables("same");
    let secrets = secrets("same-secret");
    let hash = content_hash(&install, &variables, &["DEPLOY_KEY".to_string()]);
    let record = ProfileEnvelope::first_revision(
        USER_ID,
        LOGIN,
        NAME,
        &install,
        &variables,
        &secrets,
        VALIDATED_AT,
        &hash,
        IMAGE,
        VALIDATED_AT,
        VALIDATED_AT,
    )
    .expect("record");
    let first = store.cipher.seal(&record).expect("seal");
    let second = store.cipher.seal(&record).expect("seal");
    assert_ne!(first.nonce, second.nonce);
    assert_ne!(first.ciphertext, second.ciphertext);
    let debug = format!("{store:?}");
    assert!(!debug.contains(KEY_B64));
    assert!(debug.contains("REDACTED"));
}

#[tokio::test]
async fn tamper_wrong_key_and_identity_changes_fail_without_plaintext_errors() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    let install = install("echo integrity");
    let variables = variables("visible-but-encrypted");
    let secrets = secrets("never-in-error");
    put(&store, &install, &variables, &secrets, None)
        .await
        .expect("create");
    let object = env_object_name(USER_ID, NAME);

    let wrong_key = DurableEnvStore::from_api(
        api.clone(),
        DURABLE_NAMESPACE,
        &SecretString::from(OTHER_KEY_B64),
    )
    .expect("other key store");
    let error = wrong_key
        .get_environment(USER_ID, NAME)
        .await
        .expect_err("wrong key must fail");
    assert!(!error.to_string().contains("never-in-error"));

    let mut rebound = api
        .secret(DURABLE_NAMESPACE, &object)
        .expect("stored secret");
    let rebound_id = USER_ID + 1;
    let rebound_name = "video-studio-copy";
    rebound.metadata.name = Some(env_object_name(rebound_id, rebound_name));
    rebound.metadata.labels = Some(env_labels(rebound_id, LOGIN));
    rebound
        .metadata
        .annotations
        .as_mut()
        .expect("annotations")
        .insert(
            crate::k8s::env_store::meta::ENV_NAME_ANNOTATION.to_string(),
            rebound_name.to_string(),
        );
    api.insert_secret(DURABLE_NAMESPACE, rebound);
    assert!(store
        .get_environment(rebound_id, rebound_name)
        .await
        .is_err());

    api.mutate_secret(DURABLE_NAMESPACE, &object, |secret| {
        secret
            .data
            .as_mut()
            .expect("data")
            .get_mut(CIPHERTEXT_DATA_KEY)
            .expect("ciphertext")
            .0[0] ^= 0x80;
    });
    let error = store
        .get_environment(USER_ID, NAME)
        .await
        .expect_err("tamper must fail");
    let rendered = error.to_string();
    assert!(!rendered.contains("visible-but-encrypted"));
    assert!(!rendered.contains("never-in-error"));
}

#[tokio::test]
async fn concurrent_replaces_have_one_winner_and_no_mixed_record() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    put(
        &store,
        &install("echo initial"),
        &variables("initial"),
        &secrets("secret-initial"),
        None,
    )
    .await
    .expect("create");
    let observed = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .expect("record")
        .store_version
        .expect("version");
    let install_a = install("echo A");
    let variables_a = variables("A");
    let secrets_a = secrets("secret-A");
    let install_b = install("echo B");
    let variables_b = variables("B");
    let secrets_b = secrets("secret-B");
    let (a, b) = tokio::join!(
        put(
            &store,
            &install_a,
            &variables_a,
            &secrets_a,
            Some(&observed)
        ),
        put(
            &store,
            &install_b,
            &variables_b,
            &secrets_b,
            Some(&observed)
        )
    );
    assert_ne!(a.is_ok(), b.is_ok(), "exactly one replace wins");
    let loser = if a.is_err() { a } else { b };
    assert!(matches!(loser, Err(AppError::Conflict(_))));

    let public = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get winner")
        .expect("winner");
    let (_, materialized, _) = store
        .load_environment_for_session(USER_ID, NAME)
        .await
        .expect("load winner")
        .expect("winner");
    let winner = public.variables.get("PUBLIC_VALUE").expect("winner value");
    assert_eq!(materialized["DEPLOY_KEY"], format!("secret-{winner}"));
    let stored = api
        .secret(DURABLE_NAMESPACE, &env_object_name(USER_ID, NAME))
        .expect("stored");
    assert_eq!(
        stored
            .metadata
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.get(REVISION_ANNOTATION))
            .map(String::as_str),
        Some("2")
    );
}

#[tokio::test]
async fn failed_create_or_replace_never_exposes_a_partial_update() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    api.fail_once(Operation::CreateSecret);
    assert!(put(
        &store,
        &install("echo new"),
        &variables("new"),
        &secrets("secret-new"),
        None
    )
    .await
    .is_err());
    assert!(api
        .secret(DURABLE_NAMESPACE, &env_object_name(USER_ID, NAME))
        .is_none());

    put(
        &store,
        &install("echo old"),
        &variables("old"),
        &secrets("secret-old"),
        None,
    )
    .await
    .expect("create old");
    let version = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .expect("record")
        .store_version
        .expect("version");
    api.fail_once(Operation::ReplaceSecret);
    assert!(put(
        &store,
        &install("echo new"),
        &variables("new"),
        &secrets("secret-new"),
        Some(&version)
    )
    .await
    .is_err());
    let public = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get old")
        .expect("old");
    let (_, materialized, _) = store
        .load_environment_for_session(USER_ID, NAME)
        .await
        .expect("load old")
        .expect("old");
    assert_eq!(public.variables["PUBLIC_VALUE"], "old");
    assert_eq!(materialized["DEPLOY_KEY"], "secret-old");
}

mod migration;
