use super::*;

#[tokio::test]
async fn migration_copies_exact_contents_then_cleans_up_idempotently() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    let install = install("tool install legacy");
    let variables = variables("legacy-public");
    let secrets = secrets("legacy-private");
    let (config_map, secret) = legacy_pair(&install, &variables, &secrets);
    let object = env_object_name(USER_ID, NAME);
    api.insert_config_map(LEGACY_NAMESPACE, config_map);
    api.insert_secret(LEGACY_NAMESPACE, secret);

    store
        .initialize(Some(LEGACY_NAMESPACE))
        .await
        .expect("migration");
    assert!(api.config_map(LEGACY_NAMESPACE, &object).is_none());
    assert!(api.secret(LEGACY_NAMESPACE, &object).is_none());
    let public = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .expect("migrated");
    assert_eq!(public.install, install);
    assert_eq!(public.variables, variables);
    let (_, materialized, _) = store
        .load_environment_for_session(USER_ID, NAME)
        .await
        .expect("load")
        .expect("migrated");
    assert_eq!(materialized["DEPLOY_KEY"], "legacy-private");
    store
        .initialize(Some(LEGACY_NAMESPACE))
        .await
        .expect("repeat migration is idempotent");
}

#[tokio::test]
async fn preexisting_durable_record_wins_over_legacy_data() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    put(
        &store,
        &install("echo durable"),
        &variables("durable"),
        &secrets("secret-durable"),
        None,
    )
    .await
    .expect("durable create");
    let (config_map, secret) = legacy_pair(
        &install("echo stale"),
        &variables("stale"),
        &secrets("secret-stale"),
    );
    let object = env_object_name(USER_ID, NAME);
    api.insert_config_map(LEGACY_NAMESPACE, config_map);
    api.insert_secret(LEGACY_NAMESPACE, secret);
    store
        .initialize(Some(LEGACY_NAMESPACE))
        .await
        .expect("existing durable wins");
    let public = store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .expect("durable");
    assert_eq!(public.variables["PUBLIC_VALUE"], "durable");
    assert!(api.config_map(LEGACY_NAMESPACE, &object).is_none());
    assert!(api.secret(LEGACY_NAMESPACE, &object).is_none());
}

#[tokio::test]
async fn incomplete_or_corrupt_legacy_pairs_fail_closed() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    let (config_map, _) = legacy_pair(
        &install("echo legacy"),
        &variables("legacy"),
        &secrets("secret-legacy"),
    );
    api.insert_config_map(LEGACY_NAMESPACE, config_map);
    assert!(store.initialize(Some(LEGACY_NAMESPACE)).await.is_err());
    assert!(store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .is_none());

    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    let (mut config_map, mut secret) = legacy_pair(
        &install("echo legacy"),
        &variables("legacy"),
        &secrets("secret-legacy"),
    );
    for metadata in [&mut config_map.metadata, &mut secret.metadata] {
        metadata.annotations.as_mut().expect("annotations").insert(
            crate::k8s::env_store::meta::CONTENT_HASH_ANNOTATION.to_string(),
            "invalid".to_string(),
        );
    }
    api.insert_config_map(LEGACY_NAMESPACE, config_map);
    api.insert_secret(LEGACY_NAMESPACE, secret);
    assert!(store.initialize(Some(LEGACY_NAMESPACE)).await.is_err());
    assert!(store
        .get_environment(USER_ID, NAME)
        .await
        .expect("get")
        .is_none());
}

#[tokio::test]
async fn interrupted_legacy_cleanup_converges_on_retry() {
    let api = Arc::new(FakeEnvironmentApi::default());
    let store = build_store(api.clone());
    let (config_map, secret) = legacy_pair(
        &install("echo legacy"),
        &variables("legacy"),
        &secrets("secret-legacy"),
    );
    let object = env_object_name(USER_ID, NAME);
    api.insert_config_map(LEGACY_NAMESPACE, config_map);
    api.insert_secret(LEGACY_NAMESPACE, secret);
    api.fail_once(Operation::DeleteSecret);
    assert!(store.initialize(Some(LEGACY_NAMESPACE)).await.is_err());
    assert!(api.secret(DURABLE_NAMESPACE, &object).is_some());
    assert!(api.config_map(LEGACY_NAMESPACE, &object).is_none());
    assert!(api.secret(LEGACY_NAMESPACE, &object).is_some());

    store
        .initialize(Some(LEGACY_NAMESPACE))
        .await
        .expect("retry completes cleanup");
    assert!(api.secret(LEGACY_NAMESPACE, &object).is_none());
    let (_, materialized, _) = store
        .load_environment_for_session(USER_ID, NAME)
        .await
        .expect("load")
        .expect("durable");
    assert_eq!(materialized["DEPLOY_KEY"], "secret-legacy");
}
