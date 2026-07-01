//! Tests for [`super`] (the Model-B substrate-session Pod + Secret builders).
//! Split into a sibling file to keep `session_launcher.rs` under the 500-line
//! limit; included via `#[cfg(test)] #[path = "session_launcher_tests.rs"]`.

use super::*;

fn spec() -> SessionPodSpec {
    SessionPodSpec {
        session_id: "abc123".to_string(),
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        trigger_issue_number: 7,
        package_roots: vec!["web".to_string(), "api".to_string()],
        work_label: "fkst".to_string(),
        bot_login: "fkst-bot[bot]".to_string(),
        config_hash: "cfg-deadbeef".to_string(),
    }
}

fn config() -> PodConfig {
    PodConfig {
        dispatch: true,
        namespace: "fkst-sessions".to_string(),
        image: Some("registry/fkst-control-plane:1.0".to_string()),
        service_account: "fkst-session-runner".to_string(),
        run_ttl_secs: 600,
        active_deadline_secs: 3600,
        llm_base_url: "https://llm.example/p".to_string(),
        llm_model: "gpt-5-codex".to_string(),
        llm_wire_api: "chat".to_string(),
        dns_nameservers: vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()],
        runtime_class: None,
    }
}

/// Resolve an env var's value from a container env list.
fn env_value<'a>(env: &'a [EnvVar], name: &str) -> Option<&'a str> {
    env.iter()
        .find(|e| e.name == name)
        .and_then(|e| e.value.as_deref())
}

#[test]
fn build_session_pod_has_the_deterministic_run_substrate_shape() {
    let spec = spec();
    let pod = build_session_pod(&spec, &config()).expect("pod builds");
    let meta = &pod.metadata;
    // Deterministic name = fkst-sess-<id> (at-most-one / 409-idempotent).
    assert_eq!(meta.name.as_deref(), Some("fkst-sess-abc123"));
    assert_eq!(meta.namespace.as_deref(), Some("fkst-sessions"));

    let pod_spec = pod.spec.as_ref().expect("spec");
    // Long-lived self-healing daemon.
    assert_eq!(pod_spec.restart_policy.as_deref(), Some("Always"));
    assert_eq!(
        pod_spec.service_account_name.as_deref(),
        Some("fkst-session-runner")
    );

    let c = &pod_spec.containers[0];
    assert_eq!(c.name, "runner");
    assert_eq!(c.image.as_deref(), Some("registry/fkst-control-plane:1.0"));
    assert_eq!(c.args.as_deref(), Some(&["run-substrate".to_string()][..]));
}

#[test]
fn build_session_pod_requires_an_image() {
    let mut cfg = config();
    cfg.image = None;
    assert!(matches!(
        build_session_pod(&spec(), &cfg),
        Err(LaunchError::NoImage)
    ));
}

#[test]
fn build_session_pod_mounts_creds_whole_volume_with_no_sub_path() {
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let pod_spec = pod.spec.as_ref().expect("spec");
    let c = &pod_spec.containers[0];

    let mount = &c.volume_mounts.as_ref().expect("mounts")[0];
    assert_eq!(mount.mount_path, "/var/run/fkst/creds");
    assert_eq!(mount.read_only, Some(true));
    // Load-bearing: a subPath mount is NOT refreshed on Secret rewrite, which
    // would freeze the rotating github-token. Guard it stays a whole-volume mount.
    assert!(
        mount.sub_path.is_none(),
        "creds mount must NOT use subPath (breaks token rotation)"
    );

    let vol = &pod_spec.volumes.as_ref().expect("volumes")[0];
    let secret = vol.secret.as_ref().expect("secret volume source");
    assert_eq!(secret.secret_name.as_deref(), Some("fkst-sess-abc123"));
    // Mounted 0400 (owner-only read), matching the Model-A Job pod.
    assert_eq!(secret.default_mode, Some(0o400));
}

#[test]
fn build_session_pod_injects_the_section_5_2_env() {
    let spec = spec();
    let pod = build_session_pod(&spec, &config()).expect("pod builds");
    let pod_spec = pod.spec.as_ref().expect("spec");
    let env = pod_spec.containers[0].env.as_ref().expect("env");

    assert_eq!(env_value(env, "FKST_GITHUB_REPO"), Some("acme/site"));
    assert_eq!(
        env_value(env, "FKST_GITHUB_BOT_LOGIN"),
        Some("fkst-bot[bot]")
    );
    assert_eq!(env_value(env, "FKST_GITHUB_WRITE"), Some("1"));
    // Load-bearing: a GitHub App cannot be an assignee, so claiming is label-mode.
    assert_eq!(env_value(env, "FKST_GITHUB_CLAIM_MODE"), Some("label"));
    assert_eq!(
        env_value(env, "FKST_GITHUB_PROXY_POLL_LABEL_PREFIX"),
        Some("fkst")
    );
    // LLM provider config injected explicitly (pods don't inherit the ConfigMap).
    assert_eq!(env_value(env, "FKST_LLM_MODEL"), Some("gpt-5-codex"));
    assert_eq!(
        env_value(env, "FKST_LLM_BASE_URL"),
        Some("https://llm.example/p")
    );
    assert_eq!(env_value(env, "FKST_LLM_WIRE_API"), Some("chat"));
    // Durable/runtime/creds/codex roots.
    assert_eq!(
        env_value(env, "FKST_DURABLE_ROOT"),
        Some("/var/run/fkst/durable")
    );
    assert_eq!(
        env_value(env, "FKST_RUNTIME_ROOT"),
        Some("/var/run/fkst/runtime")
    );
    assert_eq!(
        env_value(env, "FKST_SESSION_CREDS_DIR"),
        Some("/var/run/fkst/creds")
    );
    assert_eq!(env_value(env, "CODEX_HOME"), Some("/var/run/fkst/codex"));
    // Git identity = the bot login.
    assert_eq!(env_value(env, "GIT_AUTHOR_NAME"), Some("fkst-bot[bot]"));
    assert_eq!(env_value(env, "GIT_COMMITTER_NAME"), Some("fkst-bot[bot]"));
    // Package roots space-joined; work label carried for the PR4 entrypoint.
    assert_eq!(
        env_value(env, "FKST_SESSION_PACKAGE_ROOTS"),
        Some("web api")
    );
    assert_eq!(env_value(env, "FKST_SESSION_WORK_LABEL"), Some("fkst"));
}

#[test]
fn build_session_pod_joins_empty_package_roots_to_a_blank_string() {
    let mut spec = spec();
    spec.package_roots = Vec::new();
    let pod = build_session_pod(&spec, &config()).expect("pod builds");
    let env = pod.spec.unwrap().containers.remove(0).env.unwrap();
    assert_eq!(env_value(&env, "FKST_SESSION_PACKAGE_ROOTS"), Some(""));
}

#[test]
fn build_session_pod_is_hard_isolated_like_a_job_pod() {
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let pod_spec = pod.spec.as_ref().expect("spec");

    // #338 R3 box, applied identically to a Model-A Job pod.
    assert_eq!(pod_spec.automount_service_account_token, Some(false));
    assert_eq!(pod_spec.enable_service_links, Some(false));
    assert_eq!(pod_spec.dns_policy.as_deref(), Some("None"));
    let dns = pod_spec.dns_config.as_ref().expect("dns config");
    assert_eq!(
        dns.nameservers.as_deref(),
        Some(&config().dns_nameservers[..])
    );
    // Test config leaves runtime_class unset => cluster default runtime (runc).
    assert_eq!(pod_spec.runtime_class_name, None);

    let sc = pod_spec
        .security_context
        .as_ref()
        .expect("pod security context");
    assert_eq!(sc.run_as_user, Some(0));
    assert_eq!(sc.run_as_non_root, Some(false));

    let csc = pod_spec.containers[0]
        .security_context
        .as_ref()
        .expect("container security context");
    assert_eq!(
        csc.capabilities.as_ref().and_then(|c| c.drop.as_deref()),
        Some(&["ALL".to_string()][..])
    );
}

#[test]
fn build_session_pod_threads_the_runtime_class_through() {
    let mut cfg = config();
    cfg.runtime_class = Some("kata".to_string());
    let pod = build_session_pod(&spec(), &cfg).expect("pod builds");
    assert_eq!(
        pod.spec.unwrap().runtime_class_name.as_deref(),
        Some("kata")
    );
}

#[test]
fn build_session_pod_labels_the_substrate_session_component() {
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let labels = pod.metadata.labels.as_ref().expect("labels");
    assert_eq!(labels["app.kubernetes.io/part-of"], "fkst-hosted");
    // The NetworkPolicy + reconciler select on this component value.
    assert_eq!(labels["app.kubernetes.io/component"], "substrate-session");
    assert_eq!(labels["fkst.chrono-ai.fun/session-id"], "abc123");
}

#[test]
fn build_session_pod_carries_the_reconciler_annotations() {
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let ann = pod.metadata.annotations.as_ref().expect("annotations");
    assert_eq!(ann["fkst.chrono-ai.fun/owner"], "acme");
    assert_eq!(ann["fkst.chrono-ai.fun/repo"], "site");
    assert_eq!(ann["fkst.chrono-ai.fun/trigger-issue-number"], "7");
    assert_eq!(ann["fkst.chrono-ai.fun/work-label"], "fkst");
    assert_eq!(ann["fkst.chrono-ai.fun/config-hash"], "cfg-deadbeef");
    // last-pending-at is seeded (RFC3339) and settable; assert it is present.
    assert!(ann.contains_key("fkst.chrono-ai.fun/last-pending-at"));
}

#[test]
fn build_session_secret_carries_creds_with_the_userenv_prefix_and_owner() {
    let mut user_env = BTreeMap::new();
    user_env.insert("FOO".to_string(), "foo-val".to_string());
    user_env.insert("API_TOKEN".to_string(), "tok-val".to_string());
    let owner = OwnerReference {
        api_version: "v1".to_string(),
        kind: "Pod".to_string(),
        name: "fkst-sess-abc123".to_string(),
        uid: "pod-uid-1".to_string(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    };
    let secret = build_session_secret(
        &spec(),
        r#"{"token":"ghs_xyz","expires_at":"2026-01-01T00:00:00Z"}"#,
        &SecretString::from("sk-test"),
        &user_env,
        Some(owner),
    );

    assert_eq!(secret.metadata.name.as_deref(), Some("fkst-sess-abc123"));
    let data = secret.string_data.as_ref().expect("string data");
    // The github-token rides as the rotating {token, expires_at} JSON verbatim.
    assert_eq!(
        data["github-token"],
        r#"{"token":"ghs_xyz","expires_at":"2026-01-01T00:00:00Z"}"#
    );
    assert_eq!(data["llm-api-key"], "sk-test");
    assert_eq!(data["userenv.FOO"], "foo-val");
    assert_eq!(data["userenv.API_TOKEN"], "tok-val");
    assert_eq!(data.len(), 4);
    assert_eq!(secret.type_.as_deref(), Some("Opaque"));

    let owners = secret.metadata.owner_references.as_ref().expect("owners");
    assert_eq!(owners[0].kind, "Pod");
    assert_eq!(owners[0].uid, "pod-uid-1");
}

#[test]
fn build_session_secret_without_user_env_carries_only_the_base_creds() {
    let secret = build_session_secret(
        &spec(),
        "ghs_json",
        &SecretString::from("sk-test"),
        &BTreeMap::new(),
        None,
    );
    let data = secret.string_data.as_ref().expect("string data");
    assert!(data.contains_key("github-token"));
    assert!(data.contains_key("llm-api-key"));
    assert_eq!(data.len(), 2);
    assert!(secret.metadata.owner_references.is_none());
}

#[test]
fn pod_owner_reference_is_none_without_a_uid() {
    let pod = Pod {
        metadata: ObjectMeta {
            name: Some("fkst-sess-abc123".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(pod_owner_reference(&pod).is_none());
}
