//! Tests for [`super`] (the session Job + Secret launcher). Split into a sibling
//! file to keep `launcher.rs` under the 500-line limit; included via
//! `#[cfg(test)] #[path = "launcher_tests.rs"] mod tests;`.

use super::*;
use crate::models::RepoRef;
use crate::session_spec::{derive_session_id, SessionGoal};

fn spec() -> SessionSpec {
    SessionSpec {
        session_id: derive_session_id(42, "acme", "site", 7),
        run_key: "rk1".to_string(),
        installation_id: 42,
        repo: RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        owner_login: "acme".to_string(),
        issue_number: 7,
        goal: SessionGoal {
            title: "Add dark mode".to_string(),
            prompt: "do it".to_string(),
        },
        package_names: vec!["web".to_string()],
        log_branch: "fkst/session-x".to_string(),
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
    }
}

#[test]
fn build_job_sets_the_run_session_shape() {
    let spec = spec();
    let job = build_job(&spec, &config()).expect("job");
    let meta = &job.metadata;
    assert_eq!(meta.name.as_deref(), Some(&*object_name(&spec.session_id)));
    assert_eq!(meta.namespace.as_deref(), Some("fkst-sessions"));

    let jobspec = job.spec.unwrap();
    assert_eq!(jobspec.backoff_limit, Some(0));
    assert_eq!(jobspec.active_deadline_seconds, Some(3600));
    assert_eq!(jobspec.ttl_seconds_after_finished, Some(600));

    let pod = jobspec.template.spec.unwrap();
    assert_eq!(pod.restart_policy.as_deref(), Some("Never"));
    assert_eq!(
        pod.service_account_name.as_deref(),
        Some("fkst-session-runner")
    );

    let c = &pod.containers[0];
    assert_eq!(c.image.as_deref(), Some("registry/fkst-control-plane:1.0"));
    assert_eq!(c.args.as_deref(), Some(&["run-session".to_string()][..]));
    let env = c.env.as_ref().unwrap();
    assert!(env
        .iter()
        .any(|e| e.name == "FKST_SESSION_CREDS_DIR"
            && e.value.as_deref() == Some("/var/run/fkst/creds")));
    assert!(env.iter().any(|e| e.name == "FKST_SESSION_SPEC_PATH"
        && e.value.as_deref() == Some("/var/run/fkst/creds/session-spec.json")));
    // The LLM provider config is injected explicitly (pods don't inherit the
    // control-plane ConfigMap) so the runner reads the operator's values, not
    // its hard-coded fallbacks.
    assert!(env
        .iter()
        .any(|e| e.name == "FKST_LLM_BASE_URL"
            && e.value.as_deref() == Some("https://llm.example/p")));
    assert!(env
        .iter()
        .any(|e| e.name == "FKST_LLM_MODEL" && e.value.as_deref() == Some("gpt-5-codex")));
    assert!(env
        .iter()
        .any(|e| e.name == "FKST_LLM_WIRE_API" && e.value.as_deref() == Some("chat")));

    let mount = &c.volume_mounts.as_ref().unwrap()[0];
    assert_eq!(mount.mount_path, "/var/run/fkst/creds");
    assert_eq!(mount.read_only, Some(true));
    let vol = &pod.volumes.as_ref().unwrap()[0];
    let secret = vol.secret.as_ref().unwrap();
    assert_eq!(
        secret.secret_name.as_deref(),
        Some(&*object_name(&spec.session_id))
    );
    assert_eq!(secret.default_mode, Some(0o400));

    // The pod is hard-isolated (#338 R3): no ServiceAccount token, no service
    // links, external DNS only, and it runs as boxed root.
    assert_eq!(pod.automount_service_account_token, Some(false));
    assert_eq!(pod.enable_service_links, Some(false));
    assert_eq!(pod.dns_policy.as_deref(), Some("None"));
    let dns = pod.dns_config.as_ref().expect("dns config");
    assert_eq!(
        dns.nameservers.as_deref(),
        Some(&config().dns_nameservers[..])
    );

    let sc = pod.security_context.as_ref().expect("pod security context");
    assert_eq!(sc.run_as_user, Some(0));
    assert_eq!(sc.run_as_non_root, Some(false));

    // The container drops ALL capabilities (adding back only dpkg's few).
    let csc = pod.containers[0]
        .security_context
        .as_ref()
        .expect("container security context");
    assert_eq!(
        csc.capabilities.as_ref().and_then(|c| c.drop.as_deref()),
        Some(&["ALL".to_string()][..])
    );

    let ann = job.metadata.annotations.unwrap();
    assert_eq!(ann["fkst.chrono-ai.fun/owner"], "acme");
    assert_eq!(ann["fkst.chrono-ai.fun/repo"], "site");
    assert_eq!(ann["fkst.chrono-ai.fun/issue-number"], "7");
}

#[test]
fn build_job_requires_an_image() {
    let mut cfg = config();
    cfg.image = None;
    assert!(matches!(
        build_job(&spec(), &cfg),
        Err(LaunchError::NoImage)
    ));
}

#[test]
fn build_secret_carries_spec_and_creds_with_owner() {
    let spec = spec();
    let owner = OwnerReference {
        api_version: "batch/v1".to_string(),
        kind: "Job".to_string(),
        name: object_name(&spec.session_id),
        uid: "job-uid-123".to_string(),
        controller: Some(true),
        block_owner_deletion: Some(true),
    };
    let secrets = SessionSecrets {
        github_token: SecretString::from("ghs_xyz"),
        llm_api_key: SecretString::from("sk-test"),
        user_env: BTreeMap::new(),
    };
    let secret = build_secret(&spec, "fkst-sessions", &secrets, Some(owner)).expect("secret");
    let data = secret.string_data.unwrap();
    // The spec round-trips out of the mounted JSON.
    let back: SessionSpec = serde_json::from_str(&data["session-spec.json"]).unwrap();
    assert_eq!(back, spec);
    assert_eq!(data["github-token"], "ghs_xyz");
    assert_eq!(data["llm-api-key"], "sk-test");
    // With no user env, the Secret carries ONLY the spec + github token + LLM
    // key — no other files are written.
    assert_eq!(data.len(), 3);
    let owners = secret.metadata.owner_references.unwrap();
    assert_eq!(owners[0].kind, "Job");
    assert_eq!(owners[0].uid, "job-uid-123");
}

#[test]
fn build_secret_always_writes_the_llm_api_key() {
    let secrets = SessionSecrets {
        github_token: SecretString::from("ghs_xyz"),
        llm_api_key: SecretString::from("sk-always"),
        user_env: BTreeMap::new(),
    };
    let secret = build_secret(&spec(), "ns", &secrets, None).expect("secret");
    let data = secret.string_data.unwrap();
    assert!(data.contains_key("github-token"));
    assert!(data.contains_key("session-spec.json"));
    assert_eq!(data["llm-api-key"], "sk-always");
    assert!(secret.metadata.owner_references.is_none());
}

#[test]
fn build_secret_writes_user_env_under_the_userenv_prefix() {
    let mut user_env = BTreeMap::new();
    user_env.insert("FOO".to_string(), SecretString::from("foo-val"));
    user_env.insert("API_TOKEN".to_string(), SecretString::from("tok-val"));
    let secrets = SessionSecrets {
        github_token: SecretString::from("ghs_xyz"),
        llm_api_key: SecretString::from("sk-test"),
        user_env,
    };
    let secret = build_secret(&spec(), "ns", &secrets, None).expect("secret");
    let data = secret.string_data.unwrap();
    // Each user env var rides a `userenv.<KEY>` data key carrying its value.
    assert_eq!(data["userenv.FOO"], "foo-val");
    assert_eq!(data["userenv.API_TOKEN"], "tok-val");
    // The base credential keys remain; the two user env keys are additive.
    assert!(data.contains_key("github-token"));
    assert!(data.contains_key("llm-api-key"));
    assert!(data.contains_key("session-spec.json"));
    assert_eq!(data.len(), 5);
}

#[test]
fn mount_dir_matches_the_creds_layout_default() {
    // Drift guard: the Job mounts where the runner's CredsLayout reads.
    assert_eq!(CREDS_MOUNT_DIR, DEFAULT_CREDS_DIR);
}
