//! Tests for [`super`] (the Model-B substrate-session Pod + Secret builders).
//! Split into a sibling file to keep `session_launcher.rs` under the 500-line
//! limit; included via `#[cfg(test)] #[path = "session_launcher_tests.rs"]`.

use super::*;
use crate::session_spec::creds::{
    credential_secret_data, StorageWriterCreds, CREDS_COMPLETE_SENTINEL,
};

/// Assemble a `creds` map the way the executor does: through the shared
/// [`credential_secret_data`] helper, then wrap each value as a [`SecretString`] for
/// [`build_session_secret`].
fn creds_map<'a>(
    github_token_json: &str,
    llm_api_key: &str,
    user_env: impl IntoIterator<Item = (&'a str, &'a str)>,
    storage: Option<StorageWriterCreds<'_>>,
) -> BTreeMap<String, SecretString> {
    credential_secret_data(github_token_json, llm_api_key, user_env, &[], &[], storage)
        .into_iter()
        .map(|(k, v)| (k, SecretString::from(v)))
        .collect()
}

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
        output_lang: None,
        engine_config: BTreeMap::new(),
        contributors: vec!["author-login".to_string()],
    }
}

fn config() -> PodConfig {
    PodConfig {
        dispatch: true,
        mode: crate::config::PodMode::K8sCustomized,
        namespace: "fkst-sessions".to_string(),
        image: Some("registry/fkst-control-plane:1.0".to_string()),
        service_account: "fkst-session-runner".to_string(),
        llm_base_url: "https://llm.example/p".to_string(),
        llm_model: "gpt-5-codex".to_string(),
        llm_wire_api: "chat".to_string(),
        dns_nameservers: vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()],
        runtime_class: None,
        rate_pools: BTreeMap::new(),
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
    // The engine's required HostFact pair (no engine default — a session without
    // them fails any `setup_worktree()` call). Platform constants, not knobs.
    assert_eq!(env_value(env, "FKST_CANDIDATE_PREFIX"), Some("fkst-cand"));
    assert_eq!(env_value(env, "FKST_CANDIDATE_FROM_SEP"), Some("--from--"));
}

#[test]
fn session_env_pairs_supply_the_engine_hostfacts_exactly_once() {
    // Exactly once: a duplicate EnvVar name would resolve last-wins by kubelet
    // accident; the shared pairs are the single source both backends render from.
    let pairs = session_env_pairs(&spec(), &config());
    for (key, value) in [
        ("FKST_CANDIDATE_PREFIX", "fkst-cand"),
        ("FKST_CANDIDATE_FROM_SEP", "--from--"),
    ] {
        let hits: Vec<_> = pairs.iter().filter(|(k, _)| *k == key).collect();
        assert_eq!(hits.len(), 1, "{key} must appear exactly once");
        assert_eq!(hits[0].1, value, "{key} must be the platform constant");
    }
}

#[test]
fn session_env_pairs_render_operator_rate_pools_with_a_pinned_ledger_root() {
    let mut cfg = config();
    cfg.rate_pools = BTreeMap::from([
        (
            "GH".to_string(),
            crate::config::RatePool {
                burst: 50,
                refill_per_minute: 50,
            },
        ),
        (
            "GIT".to_string(),
            crate::config::RatePool {
                burst: 120,
                refill_per_minute: 1,
            },
        ),
    ]);
    let pairs = session_env_pairs(&spec(), &cfg);
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("FKST_RATE_POOL_GH"), Some("50,50"));
    assert_eq!(get("FKST_RATE_POOL_GIT"), Some("120,1"));
    // The ledger root MUST ride along: the engine's default is `~/.fkst/rate-pools`
    // and its `~`-expansion fails when HOME is unset in the session container.
    assert_eq!(get("FKST_RATE_POOL_ROOT"), Some("/var/run/fkst/rate-pools"));
    // No duplicate names: both backends render these pairs as-is.
    let mut names: Vec<_> = pairs.iter().map(|(k, _)| k.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), pairs.len(), "env names must be unique");
}

#[test]
fn session_env_pairs_render_the_contributors_only_when_present() {
    // The fixture spec carries the author login → the env pair renders.
    let pairs = session_env_pairs(&spec(), &config());
    assert_eq!(
        pairs
            .iter()
            .find(|(k, _)| k == "FKST_GITHUB_AUTHORIZED_LOGINS")
            .map(|(_, v)| v.as_str()),
        Some("author-login")
    );
    // Multiple contributors render comma-joined, author first.
    let mut with_more = spec();
    with_more.contributors = vec![
        "author-login".to_string(),
        "alice".to_string(),
        "bob".to_string(),
    ];
    let pairs = session_env_pairs(&with_more, &config());
    assert_eq!(
        pairs
            .iter()
            .find(|(k, _)| k == "FKST_GITHUB_AUTHORIZED_LOGINS")
            .map(|(_, v)| v.as_str()),
        Some("author-login,alice,bob")
    );
    // Empty ⇒ ABSENT (not an empty value), preserving the packages' default.
    let mut none = spec();
    none.contributors = Vec::new();
    let pairs = session_env_pairs(&none, &config());
    assert!(pairs
        .iter()
        .all(|(k, _)| k != "FKST_GITHUB_AUTHORIZED_LOGINS"));
}

#[test]
fn session_env_pairs_render_the_output_language_only_when_set() {
    // None ⇒ NO key at all (the engine's own `en` default applies), so a
    // session without the section renders the pre-feature env exactly.
    let pairs = session_env_pairs(&spec(), &config());
    assert!(pairs.iter().all(|(k, _)| k != "FKST_OUTPUT_LANG"));

    let mut with_lang = spec();
    with_lang.output_lang = Some("zh-CN".to_string());
    let pairs = session_env_pairs(&with_lang, &config());
    assert_eq!(
        pairs
            .iter()
            .find(|(k, _)| k == "FKST_OUTPUT_LANG")
            .map(|(_, v)| v.as_str()),
        Some("zh-CN")
    );
}

#[test]
fn session_env_pairs_render_the_tighten_merged_engine_config() {
    // Operator default GH=50,50; the user narrows GH and adds permit slots.
    let mut cfg = config();
    cfg.rate_pools = BTreeMap::from([(
        "GH".to_string(),
        crate::config::RatePool {
            burst: 50,
            refill_per_minute: 50,
        },
    )]);
    let mut spec = spec();
    spec.engine_config = BTreeMap::from([
        ("FKST_CODEX_PERMIT_SLOTS".to_string(), "8".to_string()),
        ("FKST_RATE_POOL_GH".to_string(), "999,10".to_string()),
    ]);
    let pairs = session_env_pairs(&spec, &cfg);
    let get = |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("FKST_CODEX_PERMIT_SLOTS"), Some("8"));
    // Tighten-only: the widened burst clamps to the operator's 50; the
    // narrower refill (10) survives.
    assert_eq!(get("FKST_RATE_POOL_GH"), Some("50,10"));
    assert_eq!(get("FKST_RATE_POOL_ROOT"), Some("/var/run/fkst/rate-pools"));
    // Still no duplicate names after the merge.
    let mut names: Vec<_> = pairs.iter().map(|(k, _)| k.clone()).collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), pairs.len(), "env names must be unique");
}

#[test]
fn session_env_pairs_render_no_rate_pool_keys_when_unconfigured() {
    let pairs = session_env_pairs(&spec(), &config());
    assert!(
        pairs.iter().all(|(k, _)| !k.starts_with("FKST_RATE_POOL_")),
        "an unconfigured deploy must render the pre-knob env exactly"
    );
}

#[test]
fn build_session_pod_injects_the_log_streaming_env_unconditionally() {
    // Streaming is always on: every session carries the log env + downward refs.
    let pod = build_session_pod(&spec(), &config()).expect("pod builds");
    let env = pod.spec.unwrap().containers.remove(0).env.unwrap();

    // The session id is the collector's bundle key; the retired FKST_LOG_BRANCH /
    // FKST_LOG_STREAMING enable-flag are gone.
    assert_eq!(env_value(&env, "FKST_SESSION_ID"), Some("abc123"));
    assert!(env.iter().all(|e| e.name != "FKST_LOG_BRANCH"));
    assert!(env.iter().all(|e| e.name != "FKST_LOG_STREAMING"));
    assert_eq!(env_value(&env, "FKST_TRIGGER_ISSUE"), Some("7"));
    assert_eq!(env_value(&env, "FKST_CONFIG_HASH"), Some("cfg-deadbeef"));

    // The pod UID/name ride the downward API (a fieldRef), never a literal value —
    // and NO storage credential is added to the env (it rides the Secret).
    let uid = env
        .iter()
        .find(|e| e.name == "FKST_POD_UID")
        .expect("uid env");
    assert_eq!(
        uid.value, None,
        "uid must come from the downward API, not a literal"
    );
    assert_eq!(
        uid.value_from
            .as_ref()
            .and_then(|s| s.field_ref.as_ref())
            .map(|f| f.field_path.as_str()),
        Some("metadata.uid")
    );
    let name = env
        .iter()
        .find(|e| e.name == "FKST_POD_NAME")
        .expect("name env");
    assert_eq!(
        name.value_from
            .as_ref()
            .and_then(|s| s.field_ref.as_ref())
            .map(|f| f.field_path.as_str()),
        Some("metadata.name")
    );
}

#[test]
fn session_env_pairs_are_the_plain_env_minus_the_downward_api_vars() {
    let spec = spec();
    let cfg = config();
    let pairs = session_env_pairs(&spec, &cfg);

    // The two downward-API vars are NOT in the shared pairs (the pod path appends
    // them as fieldRefs; the OpenSandbox backend supplies them as plain values).
    assert!(
        pairs
            .iter()
            .all(|(k, _)| *k != "FKST_POD_UID" && *k != "FKST_POD_NAME"),
        "downward-API vars must not appear in the shared plain pairs"
    );
    // The three PLAIN log-streaming vars ARE part of the shared pairs.
    assert!(pairs.iter().any(|(k, _)| *k == "FKST_SESSION_ID"));
    assert!(pairs.iter().any(|(k, _)| *k == "FKST_TRIGGER_ISSUE"));
    assert!(pairs.iter().any(|(k, _)| *k == "FKST_CONFIG_HASH"));

    // Behaviour-preserving proof: the rendered pod env is EXACTLY the shared pairs
    // (as EnvVars) followed by the two downward-API vars — so extracting
    // `session_env_pairs` changed nothing about what the pod runs.
    let pod = build_session_pod(&spec, &cfg).expect("pod builds");
    let env = pod.spec.unwrap().containers.remove(0).env.unwrap();
    let mut expected: Vec<EnvVar> = pairs
        .iter()
        .map(|(name, value)| env_var(name, value.clone()))
        .collect();
    expected.push(downward_env_var("FKST_POD_UID", "metadata.uid"));
    expected.push(downward_env_var("FKST_POD_NAME", "metadata.name"));
    assert_eq!(env, expected);
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
    assert_eq!(ann["fkst.chrono-ai.fun/installation-id"], "42");
    assert_eq!(ann["fkst.chrono-ai.fun/trigger-issue-number"], "7");
    assert_eq!(ann["fkst.chrono-ai.fun/work-label"], "fkst");
    assert_eq!(ann["fkst.chrono-ai.fun/config-hash"], "cfg-deadbeef");
    // last-pending-at is seeded (RFC3339) and settable; assert it is present.
    assert!(ann.contains_key("fkst.chrono-ai.fun/last-pending-at"));
}

#[test]
fn build_session_secret_carries_creds_with_the_userenv_prefix_and_owner() {
    let user_env = [("FOO", "foo-val"), ("API_TOKEN", "tok-val")];
    let owner = OwnerReference {
        api_version: "v1".to_string(),
        kind: "Pod".to_string(),
        name: "fkst-sess-abc123".to_string(),
        uid: "pod-uid-1".to_string(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    };
    let creds = creds_map(
        r#"{"token":"ghs_xyz","expires_at":"2026-01-01T00:00:00Z"}"#,
        "sk-test",
        user_env,
        None,
    );
    let secret = build_session_secret(&spec(), creds, Some(owner));

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
    // The writer stamps the completeness sentinel so the in-pod gate passes at mount.
    assert_eq!(data[CREDS_COMPLETE_SENTINEL], "1");
    // Two base creds + two user-env keys + the sentinel.
    assert_eq!(data.len(), 5);
    assert_eq!(secret.type_.as_deref(), Some("Opaque"));

    let owners = secret.metadata.owner_references.as_ref().expect("owners");
    assert_eq!(owners[0].kind, "Pod");
    assert_eq!(owners[0].uid, "pod-uid-1");
}

#[test]
fn build_session_secret_without_user_env_carries_only_the_base_creds() {
    let creds = creds_map("ghs_json", "sk-test", std::iter::empty(), None);
    let secret = build_session_secret(&spec(), creds, None);
    let data = secret.string_data.as_ref().expect("string data");
    assert!(data.contains_key("github-token"));
    assert!(data.contains_key("llm-api-key"));
    // The writer always stamps the completeness sentinel alongside the base creds.
    assert_eq!(data[CREDS_COMPLETE_SENTINEL], "1");
    assert_eq!(data.len(), 3);
    assert!(secret.metadata.owner_references.is_none());
}

#[test]
fn build_session_secret_carries_the_storage_sa_when_configured() {
    let storage = StorageWriterCreds {
        client_id: "writer-client",
        client_secret: "writer-secret",
        token_url: "https://nyx.example/oauth/token",
        base_url: "https://storage.example/proxy",
        bucket: "fkst-logs",
    };
    let creds = creds_map("ghs_json", "sk-test", std::iter::empty(), Some(storage));
    let secret = build_session_secret(&spec(), creds, None);
    let data = secret.string_data.as_ref().expect("string data");
    // Base creds + the five storage-* files, nothing else.
    assert_eq!(data["storage-client-id"], "writer-client");
    assert_eq!(data["storage-client-secret"], "writer-secret");
    assert_eq!(data["storage-token-url"], "https://nyx.example/oauth/token");
    assert_eq!(data["storage-base-url"], "https://storage.example/proxy");
    assert_eq!(data["storage-bucket"], "fkst-logs");
    // The writer stamps the completeness sentinel on top of the credential set.
    assert_eq!(data[CREDS_COMPLETE_SENTINEL], "1");
    assert_eq!(data.len(), 8);
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
