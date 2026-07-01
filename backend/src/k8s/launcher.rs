//! The per-session Job + Secret launcher (issue #293).
//!
//! Turns a [`SessionSpec`] + the session's credentials into a running Kubernetes
//! Job: one per-session Secret (mounted 0440, group-readable by the non-root
//! session user) carrying the spec + tokens, and one
//! Job running the control-plane image in `run-session` mode. The Job name is the
//! deterministic session id, so a webhook redelivery is an at-most-one-Job no-op;
//! the Secret is owner-referenced to the Job, so K8s cascade-deletes it on GC.

use std::collections::BTreeMap;

use k8s_openapi::api::batch::v1::{Job, JobSpec};
use k8s_openapi::api::core::v1::{
    Container, EnvVar, PodSecurityContext, PodSpec, PodTemplateSpec, Secret, SecretVolumeSource,
    Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::api::{Api, PostParams};
use kube::ResourceExt;
use secrecy::{ExposeSecret, SecretString};

use crate::config::PodConfig;
use crate::session_spec::creds::{
    DEFAULT_CREDS_DIR, GITHUB_TOKEN_FILE, LLM_API_KEY_FILE, USER_ENV_PREFIX,
};
use crate::session_spec::SessionSpec;

/// Where the per-session credential Secret is mounted in the pod (must match the
/// runner's `FKST_SESSION_CREDS_DIR` default, [`DEFAULT_CREDS_DIR`]).
const CREDS_MOUNT_DIR: &str = DEFAULT_CREDS_DIR;
/// The Secret key (and mounted filename) holding the serialized SessionSpec. The
/// prompt lives in the spec, so it rides the 0400 Secret, never a ConfigMap.
const SPEC_FILE_KEY: &str = "session-spec.json";
/// The Secret-volume name used by both the volume and its mount.
const CREDS_VOLUME: &str = "creds";
/// File mode for the mounted Secret (octal; k8s wants the decimal value). 0440 =
/// owner + GROUP read, no world. The runtime image runs as the non-root `fkst`
/// user (uid/gid [`RUNNER_UID`]); with `fsGroup = RUNNER_UID` the mounted files
/// are `root:RUNNER_UID`, so the session process reads them via its group. 0400
/// (owner-only) would be root-owned and UNREADABLE by the non-root process.
const SECRET_FILE_MODE: i32 = 0o440;
/// The uid/gid the runtime image runs as (`USER fkst`, `groupadd/useradd 10001`
/// in the Dockerfile). The pod's `fsGroup` matches so the 0440 creds are
/// group-readable by the session process.
const RUNNER_UID: i64 = 10001;
/// The container name inside the session pod.
const RUNNER_CONTAINER: &str = "runner";

/// Env var the runner reads for the creds dir (mirrors `runner::CREDS_DIR_ENV`).
const CREDS_DIR_ENV: &str = "FKST_SESSION_CREDS_DIR";
/// Env var the runner reads for the spec path (mirrors `runner::SPEC_PATH_ENV`).
const SPEC_PATH_ENV: &str = "FKST_SESSION_SPEC_PATH";
/// LLM provider env vars injected into the session pod. Session pods do NOT
/// inherit the control-plane ConfigMap (build_job sets only explicit env), so
/// these are injected as plain EnvVars sourced from [`PodConfig`]; without them
/// the runner would fall back to its hard-coded defaults.
const LLM_BASE_URL_ENV: &str = "FKST_LLM_BASE_URL";
const LLM_MODEL_ENV: &str = "FKST_LLM_MODEL";
const LLM_WIRE_API_ENV: &str = "FKST_LLM_WIRE_API";

/// The credentials minted for one session, written into its Secret.
pub struct SessionSecrets {
    /// The GitHub App installation token (clone + git ops + log push).
    pub github_token: SecretString,
    /// The static LLM API key the engine's codex provider authenticates with
    /// (sourced from the control plane's `FKST_LLM_API_KEY` config). Always
    /// written — an engine with no LLM credential 401s.
    pub llm_api_key: SecretString,
    /// Per-user env to inject into the session (PR4b), resolved from the issue
    /// author's `fkst-user-<id>` store by the trigger. Each `(KEY, value)` is
    /// written as a `userenv.<KEY>` Secret data key; the runner globs those back
    /// into the engine `env_profile`. Empty when the issue declared no
    /// `### Environment` keys (the common case).
    pub user_env: BTreeMap<String, SecretString>,
}

/// Errors launching a session Job.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// Pod dispatch is enabled but no image is configured (should be caught at
    /// config load; guarded here too).
    #[error("FKST_POD_IMAGE is not configured")]
    NoImage,
    /// Serializing the SessionSpec for the Secret failed.
    #[error("serialize session spec: {0}")]
    Serialize(#[from] serde_json::Error),
    /// A Kubernetes API call failed (non-conflict).
    #[error("kubernetes api: {0}")]
    Kube(#[from] kube::Error),
}

/// The deterministic Job/Secret name for a session.
fn object_name(session_id: &str) -> String {
    format!("fkst-sess-{session_id}")
}

/// Common labels stamped on the Job + Secret (and the pod template) so a watcher
/// can find a session's objects by selector.
fn labels(spec: &SessionSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "app.kubernetes.io/part-of".to_string(),
            "fkst-hosted".to_string(),
        ),
        (
            "app.kubernetes.io/component".to_string(),
            "session".to_string(),
        ),
        (
            "fkst.chrono-ai.fun/session-id".to_string(),
            spec.session_id.clone(),
        ),
        (
            "fkst.chrono-ai.fun/run-key".to_string(),
            spec.run_key.clone(),
        ),
    ])
}

/// Job annotations the watcher reads to resolve the goal issue (owner/repo can
/// exceed the label charset/length, so these are annotations, not labels).
fn annotations(spec: &SessionSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "fkst.chrono-ai.fun/owner".to_string(),
            spec.repo.owner.clone(),
        ),
        (
            "fkst.chrono-ai.fun/repo".to_string(),
            spec.repo.name.clone(),
        ),
        (
            "fkst.chrono-ai.fun/issue-number".to_string(),
            spec.issue_number.to_string(),
        ),
        (
            "fkst.chrono-ai.fun/log-branch".to_string(),
            spec.log_branch.clone(),
        ),
    ])
}

/// Build the per-session Job (pure; no API calls). Runs `config.image` with
/// `["run-session"]`, mounts the per-session Secret 0400, and is bounded by
/// `backoffLimit:0` + `activeDeadlineSeconds` + `ttlSecondsAfterFinished`.
pub fn build_job(spec: &SessionSpec, config: &PodConfig) -> Result<Job, LaunchError> {
    let image = config.image.clone().ok_or(LaunchError::NoImage)?;
    let name = object_name(&spec.session_id);
    let labels = labels(spec);

    let container = Container {
        name: RUNNER_CONTAINER.to_string(),
        image: Some(image),
        args: Some(vec!["run-session".to_string()]),
        env: Some(vec![
            EnvVar {
                name: CREDS_DIR_ENV.to_string(),
                value: Some(CREDS_MOUNT_DIR.to_string()),
                ..Default::default()
            },
            EnvVar {
                name: SPEC_PATH_ENV.to_string(),
                value: Some(format!("{CREDS_MOUNT_DIR}/{SPEC_FILE_KEY}")),
                ..Default::default()
            },
            EnvVar {
                name: LLM_BASE_URL_ENV.to_string(),
                value: Some(config.llm_base_url.clone()),
                ..Default::default()
            },
            EnvVar {
                name: LLM_MODEL_ENV.to_string(),
                value: Some(config.llm_model.clone()),
                ..Default::default()
            },
            EnvVar {
                name: LLM_WIRE_API_ENV.to_string(),
                value: Some(config.llm_wire_api.clone()),
                ..Default::default()
            },
        ]),
        volume_mounts: Some(vec![VolumeMount {
            name: CREDS_VOLUME.to_string(),
            mount_path: CREDS_MOUNT_DIR.to_string(),
            read_only: Some(true),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let volume = Volume {
        name: CREDS_VOLUME.to_string(),
        secret: Some(SecretVolumeSource {
            secret_name: Some(name.clone()),
            default_mode: Some(SECRET_FILE_MODE),
            ..Default::default()
        }),
        ..Default::default()
    };

    let pod_spec = PodSpec {
        restart_policy: Some("Never".to_string()),
        service_account_name: Some(config.service_account.clone()),
        // Run as the image's non-root `fkst` user and set `fsGroup` to its gid so
        // the 0440 creds Secret (mounted root:fsGroup) is group-readable by the
        // session process. Without this, the pod runs non-root but the 0400/root
        // creds are unreadable -> `run-session` fails reading its own spec.
        security_context: Some(PodSecurityContext {
            run_as_non_root: Some(true),
            run_as_user: Some(RUNNER_UID),
            run_as_group: Some(RUNNER_UID),
            fs_group: Some(RUNNER_UID),
            ..Default::default()
        }),
        containers: vec![container],
        volumes: Some(vec![volume]),
        ..Default::default()
    };

    let job = Job {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(config.namespace.clone()),
            labels: Some(labels.clone()),
            annotations: Some(annotations(spec)),
            ..Default::default()
        },
        spec: Some(JobSpec {
            backoff_limit: Some(0),
            active_deadline_seconds: Some(config.active_deadline_secs),
            ttl_seconds_after_finished: Some(config.run_ttl_secs),
            template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(pod_spec),
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    Ok(job)
}

/// Build the per-session Secret (pure; no API calls). Carries the serialized
/// SessionSpec + the credential files; owner-referenced to the Job when `owner`
/// is provided so K8s cascade-deletes it on Job GC.
pub fn build_secret(
    spec: &SessionSpec,
    namespace: &str,
    secrets: &SessionSecrets,
    owner: Option<OwnerReference>,
) -> Result<Secret, LaunchError> {
    let mut data: BTreeMap<String, String> = BTreeMap::new();
    data.insert(SPEC_FILE_KEY.to_string(), serde_json::to_string(spec)?);
    data.insert(
        GITHUB_TOKEN_FILE.to_string(),
        secrets.github_token.expose_secret().to_string(),
    );
    // The static LLM API key is always written (no longer an optional token).
    data.insert(
        LLM_API_KEY_FILE.to_string(),
        secrets.llm_api_key.expose_secret().to_string(),
    );
    // Per-user env (PR4b): each entry rides a `userenv.<KEY>` data key. KEY is
    // env-var-shaped, so the composite is a valid Secret data key; the runner
    // strips the prefix to recover KEY and folds it into the engine env_profile.
    for (key, value) in &secrets.user_env {
        data.insert(
            format!("{USER_ENV_PREFIX}{key}"),
            value.expose_secret().to_string(),
        );
    }

    Ok(Secret {
        metadata: ObjectMeta {
            name: Some(object_name(&spec.session_id)),
            namespace: Some(namespace.to_string()),
            labels: Some(labels(spec)),
            owner_references: owner.map(|o| vec![o]),
            ..Default::default()
        },
        string_data: Some(data),
        type_: Some("Opaque".to_string()),
        ..Default::default()
    })
}

/// Build the OwnerReference a created Job presents to its Secret.
fn owner_reference(job: &Job) -> Option<OwnerReference> {
    let name = job.metadata.name.clone()?;
    let uid = job.uid()?;
    Some(OwnerReference {
        api_version: "batch/v1".to_string(),
        kind: "Job".to_string(),
        name,
        uid,
        controller: Some(true),
        block_owner_deletion: Some(true),
    })
}

/// Creates per-session Jobs + Secrets against the cluster.
#[derive(Clone)]
pub struct PodSessionLauncher {
    client: kube::Client,
    namespace: String,
    config: PodConfig,
}

/// What a launch did: a freshly created Job, or an idempotent no-op because the
/// Job already existed (a webhook redelivery for the same session).
#[derive(Debug, PartialEq, Eq)]
pub enum LaunchOutcome {
    Created,
    AlreadyRunning,
}

impl PodSessionLauncher {
    /// Build a launcher from a live client + the pod config.
    pub fn new(client: kube::Client, namespace: impl Into<String>, config: PodConfig) -> Self {
        Self {
            client,
            namespace: namespace.into(),
            config,
        }
    }

    /// Create the session's Job, then its owner-referenced Secret. Idempotent: a
    /// `409 AlreadyExists` on the Job (same deterministic name) is a no-op
    /// success. The Job is created first so the Secret can carry its UID as an
    /// ownerReference (the pod waits for the Secret to mount, within the deadline).
    pub async fn launch(
        &self,
        spec: &SessionSpec,
        secrets: SessionSecrets,
    ) -> Result<LaunchOutcome, LaunchError> {
        let jobs: Api<Job> = Api::namespaced(self.client.clone(), &self.namespace);
        let secrets_api: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);

        let job = build_job(spec, &self.config)?;
        let created = match jobs.create(&PostParams::default(), &job).await {
            Ok(created) => created,
            Err(kube::Error::Api(err)) if err.code == 409 => {
                tracing::info!(
                    session_id = %spec.session_id,
                    "pod launch: job already exists; idempotent no-op"
                );
                return Ok(LaunchOutcome::AlreadyRunning);
            }
            Err(err) => return Err(LaunchError::Kube(err)),
        };

        let owner = owner_reference(&created);
        let secret = build_secret(spec, &self.namespace, &secrets, owner)?;
        match secrets_api.create(&PostParams::default(), &secret).await {
            Ok(_) => {}
            Err(kube::Error::Api(err)) if err.code == 409 => {
                tracing::info!(
                    session_id = %spec.session_id,
                    "pod launch: secret already exists; reusing"
                );
            }
            Err(err) => return Err(LaunchError::Kube(err)),
        }

        tracing::info!(
            session_id = %spec.session_id,
            namespace = %self.namespace,
            "pod launch: session job created"
        );
        Ok(LaunchOutcome::Created)
    }
}

#[cfg(test)]
mod tests {
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
        assert!(env.iter().any(|e| e.name == "FKST_SESSION_CREDS_DIR"
            && e.value.as_deref() == Some("/var/run/fkst/creds")));
        assert!(env.iter().any(|e| e.name == "FKST_SESSION_SPEC_PATH"
            && e.value.as_deref() == Some("/var/run/fkst/creds/session-spec.json")));
        // The LLM provider config is injected explicitly (pods don't inherit the
        // control-plane ConfigMap) so the runner reads the operator's values, not
        // its hard-coded fallbacks.
        assert!(env.iter().any(|e| e.name == "FKST_LLM_BASE_URL"
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
        assert_eq!(secret.default_mode, Some(0o440));

        // The pod runs as the non-root fkst user with fsGroup = its gid so the
        // 0440 creds are group-readable (else run-session can't read its spec).
        let sc = pod.security_context.as_ref().expect("pod security context");
        assert_eq!(sc.run_as_non_root, Some(true));
        assert_eq!(sc.run_as_user, Some(10001));
        assert_eq!(sc.fs_group, Some(10001));

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
}
