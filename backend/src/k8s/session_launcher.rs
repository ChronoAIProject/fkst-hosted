//! The long-lived substrate-session Pod + its creds Secret builders (Model B,
//! issue #359 §5.2/§9, PR3).
//!
//! Model B replaces the run-to-completion per-session Job ([`super::launcher`])
//! with ONE long-lived, self-healing Pod per substrate session. The Pod runs the
//! control-plane image in `run-substrate` mode as a claim-mode="label" GitHub
//! daemon; a reconciler (later PR) DELETEs it for idle-kill / deregister. Its
//! name is the deterministic `fkst-sess-<session_id>`, so a re-trigger is an
//! at-most-one-Pod, 409-idempotent no-op.
//!
//! These are pure builders + an idempotent create — nothing calls them yet (PR3
//! is additive). The Model-A Job launcher stays the source of truth for the
//! credential-Secret layout: this module reuses
//! [`crate::session_spec::creds::credential_secret_data`] so the two never
//! diverge, and reuses [`crate::k8s::isolation::apply_isolation`] so the session
//! Pod is the SAME #338 R3 hard-isolation box as a Job pod.

use std::collections::BTreeMap;
use std::time::SystemTime;

use k8s_openapi::api::core::v1::{
    Container, EnvVar, EnvVarSource, ObjectFieldSelector, Pod, PodSpec, Secret, SecretVolumeSource,
    Volume, VolumeMount,
};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use k8s_openapi::chrono::{DateTime, Utc};
use kube::api::{Api, PostParams};
use secrecy::{ExposeSecret, SecretString};

use crate::config::PodConfig;
use crate::models::RepoRef;
use crate::session_pod::log_stream::{
    ENV_CONFIG_HASH, ENV_POD_NAME, ENV_POD_UID, ENV_SESSION_ID, ENV_TRIGGER_ISSUE,
};
use crate::session_spec::creds::{CREDS_COMPLETE_SENTINEL, DEFAULT_CREDS_DIR};

/// Errors launching a substrate-session Pod (relocated from the deleted Model-A
/// Job launcher, trimmed to the variants the Pod path actually raises).
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// Pod dispatch is enabled but no image is configured (should be caught at
    /// config load; guarded here too).
    #[error("FKST_POD_IMAGE is not configured")]
    NoImage,
    /// A Kubernetes API call failed (non-conflict).
    #[error("kubernetes api: {0}")]
    Kube(#[from] kube::Error),
}

/// Where the per-session credential Secret is mounted in the pod (must match the
/// runner's `FKST_SESSION_CREDS_DIR`, [`DEFAULT_CREDS_DIR`]).
const CREDS_MOUNT_DIR: &str = DEFAULT_CREDS_DIR;
/// The Secret-volume name used by both the volume and its mount.
const CREDS_VOLUME: &str = "creds";
/// File mode for the mounted Secret (octal; k8s wants the decimal value). 0400 =
/// owner-only read. The isolated session pod runs as root, so root reads the
/// owner-only files directly — no `fsGroup`/group-read relaxation is needed.
const SECRET_FILE_MODE: i32 = 0o400;
/// The container name inside the session pod. `pub(crate)` so the engine-observe
/// exec targets the same container the builder stamps.
pub(crate) const RUNNER_CONTAINER: &str = "runner";
/// The single container arg: the PR4 `run-substrate` entrypoint.
const RUN_SUBSTRATE_ARG: &str = "run-substrate";

/// The durable delivery-state root the run-substrate entrypoint initializes
/// idempotently so a container restart resumes rather than wiping it.
/// `pub(crate)`: the engine-observe exec MUST pass this exact string — the
/// engine derives its live-observe socket path from an FNV hash of the
/// durable-root string AS GIVEN (no canonicalization), so a second literal
/// with e.g. a trailing slash would silently degrade every observe call into
/// a redb-lock error.
pub(crate) const DURABLE_ROOT_DIR: &str = "/var/run/fkst/durable";
/// The per-restart scratch/runtime root.
const RUNTIME_ROOT_DIR: &str = "/var/run/fkst/runtime";
/// The codex config/home dir inside the pod.
const CODEX_HOME_DIR: &str = "/var/run/fkst/codex";

// --- §5.2 non-secret env keys ------------------------------------------------
// The isolated pod does NOT inherit the control-plane ConfigMap (apply_isolation
// strips service-link env), so every value the run-substrate entrypoint needs is
// injected as an explicit EnvVar. Keys are `const`s so the pod and the (PR4)
// reader can never disagree on a name.
const GITHUB_REPO_ENV: &str = "FKST_GITHUB_REPO";
const GITHUB_BOT_LOGIN_ENV: &str = "FKST_GITHUB_BOT_LOGIN";
const GITHUB_WRITE_ENV: &str = "FKST_GITHUB_WRITE";
/// Fixed: the session daemon always has write access to the repo.
const GITHUB_WRITE_VALUE: &str = "1";
const GITHUB_CLAIM_MODE_ENV: &str = "FKST_GITHUB_CLAIM_MODE";
/// Load-bearing: a GitHub App cannot be an issue ASSIGNEE, so the session claims
/// work by LABEL, never by assignment. Must be `label`.
const GITHUB_CLAIM_MODE_VALUE: &str = "label";
const GITHUB_PROXY_POLL_LABEL_PREFIX_ENV: &str = "FKST_GITHUB_PROXY_POLL_LABEL_PREFIX";
/// Package lifecycle labels suppressed from github-proxy's level replay. These are
/// prefixes, not the session's positive work-item allowlist: notably `fkst-dev:`
/// suppresses `fkst-dev:claimed` while leaving the exact work label `fkst-dev`
/// eligible for replay and exact admission through [`SESSION_WORK_LABEL_ENV`].
const GITHUB_PROXY_POLL_LABEL_PREFIX_VALUE: &str =
    "fkst-dev:,fkst-class:,fkst-security:,fkst-workflow:,fkst-dashboard";
/// Comma-separated GitHub logins the packages' github author policy trusts
/// (issues/comments from anyone else are ignored by the session). The bot's own
/// login is implicitly authorized package-side via `FKST_GITHUB_BOT_LOGIN`.
const GITHUB_AUTHORIZED_LOGINS_ENV: &str = "FKST_GITHUB_AUTHORIZED_LOGINS";
const LLM_MODEL_ENV: &str = "FKST_LLM_MODEL";
const LLM_BASE_URL_ENV: &str = "FKST_LLM_BASE_URL";
const LLM_WIRE_API_ENV: &str = "FKST_LLM_WIRE_API";
const DURABLE_ROOT_ENV: &str = "FKST_DURABLE_ROOT";
const RUNTIME_ROOT_ENV: &str = "FKST_RUNTIME_ROOT";
const SESSION_CREDS_DIR_ENV: &str = "FKST_SESSION_CREDS_DIR";
const CODEX_HOME_ENV: &str = "CODEX_HOME";
const GIT_AUTHOR_NAME_ENV: &str = "GIT_AUTHOR_NAME";
const GIT_COMMITTER_NAME_ENV: &str = "GIT_COMMITTER_NAME";
/// Space-joined declared platform-package roots (consumed by the PR4
/// `run-substrate` entrypoint to build the supervise command).
const SESSION_PACKAGE_ROOTS_ENV: &str = "FKST_SESSION_PACKAGE_ROOTS";
/// The claim/poll work label (also carried in the env so the entrypoint can
/// build the supervise command without re-reading the pod annotation).
const SESSION_WORK_LABEL_ENV: &str = "FKST_SESSION_WORK_LABEL";
/// The engine's required HostFact pair (its config registry declares them with
/// NO default and fails closed), consumed by the `setup_worktree()` Lua SDK to
/// compose candidate branch names as `{prefix}-{date}-{slug}-{ulid}{sep}{parent}`.
/// Platform-owned constants, not knobs: process env beats a workspace repo's own
/// `fkst.env` in the engine's resolver, so hosted candidate branch naming is
/// uniform across sessions. Both values satisfy the engine's branch-fragment
/// validation (no `..`/`//`, no leading/trailing `/`, no `~^:?*[\` or whitespace).
const CANDIDATE_PREFIX_ENV: &str = "FKST_CANDIDATE_PREFIX";
const CANDIDATE_PREFIX_VALUE: &str = "fkst-cand";
const CANDIDATE_FROM_SEP_ENV: &str = "FKST_CANDIDATE_FROM_SEP";
const CANDIDATE_FROM_SEP_VALUE: &str = "--from--";

// --- labels + annotations ----------------------------------------------------
// These keys are the single source of truth for the Model B session pod's
// selector + metadata: the builder here STAMPS them and the reconciler (PR5b)
// SELECTS + READS them, so both must agree. They are `pub` for that reason.
/// `app.kubernetes.io/component` value the NetworkPolicy + reconciler select on.
pub const COMPONENT_LABEL_VALUE: &str = "substrate-session";
/// The `app.kubernetes.io/component` label KEY the reconciler builds its pod-LIST
/// selector from (`<key>=<COMPONENT_LABEL_VALUE>`).
pub const COMPONENT_LABEL_KEY: &str = "app.kubernetes.io/component";
pub const ANNOTATION_OWNER: &str = "fkst.chrono-ai.fun/owner";
pub const ANNOTATION_REPO: &str = "fkst.chrono-ai.fun/repo";
/// The GitHub App installation id the pod's token is minted from. Stamped so the
/// reconciler's pod-sweep can recover the `(installation, repo)` key a live pod
/// belongs to without re-resolving it from GitHub.
pub const ANNOTATION_INSTALLATION: &str = "fkst.chrono-ai.fun/installation-id";
pub const ANNOTATION_TRIGGER_ISSUE: &str = "fkst.chrono-ai.fun/trigger-issue-number";
pub const ANNOTATION_WORK_LABEL: &str = "fkst.chrono-ai.fun/work-label";
pub const ANNOTATION_CONFIG_HASH: &str = "fkst.chrono-ai.fun/config-hash";
pub const ANNOTATION_LAST_PENDING_AT: &str = "fkst.chrono-ai.fun/last-pending-at";
/// The label KEY carrying the deterministic session id (the reconciler reads it
/// back off a listed pod to map it to its registration).
pub const SESSION_ID_LABEL: &str = "fkst.chrono-ai.fun/session-id";

/// What the control plane needs to launch (and later reconcile) one long-lived
/// substrate-session Pod. Non-secret: a `{:?}` of it can never leak a token.
pub struct SessionPodSpec {
    /// Session id; `fkst-sess-<session_id>` is the deterministic Pod (and Secret)
    /// name, so a re-trigger is an at-most-one / 409-idempotent no-op.
    pub session_id: String,
    /// GitHub App installation id used to mint the Pod's token (by the caller).
    pub installation_id: i64,
    /// The `owner/name` repository the session works.
    pub repo: RepoRef,
    /// The issue that triggered the session (recorded as an annotation).
    pub trigger_issue_number: i64,
    /// Declared platform-package names / repo paths (consumed by the PR4
    /// `run-substrate` entrypoint to build the supervise command).
    pub package_roots: Vec<String>,
    /// The session's effective work-label SET, comma-joined into one value (epic #594
    /// I4) - the explicit `### Work Label` plus its packages' auto-declared labels.
    /// Feeds the exact-admission `FKST_SESSION_WORK_LABEL` and the
    /// `fkst.chrono-ai.fun/work-label` runtime correlation annotation. A single-label
    /// session renders as the bare label (no comma), byte-identical to the pre-multi-
    /// label value. Built via [`crate::k8s::work_label_wire::join_work_labels`];
    /// recovered on observe via [`crate::k8s::work_label_wire::split_work_labels`].
    pub work_label: String,
    /// The bot login (`FKST_GITHUB_BOT_LOGIN` + git author/committer name).
    pub bot_login: String,
    /// Config-hash annotation used by the reconciler for drift detection.
    pub config_hash: String,
    /// Optional session output locale (`### Output Language` → `FKST_OUTPUT_LANG`).
    /// `None` renders no env key at all (the engine defaults to `en`).
    pub output_lang: Option<String>,
    /// The validated `### Engine Config` map (allowlisted engine tunables).
    /// Tighten-merged with the operator rate-pool defaults at render time —
    /// see [`crate::k8s::engine_env::engine_tunables_env`].
    pub engine_config: BTreeMap<String, String>,
    /// The session's trusted users (trigger author first, then the
    /// `### FKST Contributors` list, case-insensitively deduped) — rendered as
    /// `FKST_GITHUB_AUTHORIZED_LOGINS` for the packages' github author policy.
    pub contributors: Vec<String>,
}

/// The deterministic Pod/Secret name for a session (`fkst-sess-<session_id>`).
/// `pub` so the reconciler (PR5b) can address a session's Pod/Secret by id for
/// its patch (last-pending), delete (idle/config/deregister kill), and token
/// rotation without duplicating the naming convention.
pub fn session_object_name(session_id: &str) -> String {
    format!("fkst-sess-{session_id}")
}

/// Serialize the rotating `github-token` Secret value: the
/// `{"token": "ghs_…", "expires_at": "<RFC3339>"}` JSON the in-pod git credential
/// helper + `gh` PATH shim read on every op (§5.2/§5.4). `expires_at` is RFC3339
/// (a reader compares it against `now`); the token is exposed only to serialize it
/// here and is NEVER logged. One control-plane Secret rewrite through this shape
/// refreshes both `git` and `gh` with no in-pod refresh loop.
pub fn session_github_token_json(token: &SecretString, expires_at: SystemTime) -> String {
    let expires_rfc3339 = DateTime::<Utc>::from(expires_at).to_rfc3339();
    serde_json::json!({
        "token": token.expose_secret(),
        "expires_at": expires_rfc3339,
    })
    .to_string()
}

/// Shorthand for a plain `name=value` [`EnvVar`].
fn env_var(name: &str, value: impl Into<String>) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value: Some(value.into()),
        ..Default::default()
    }
}

/// A downward-API [`EnvVar`] whose value the kubelet fills from `field_path` (e.g.
/// `metadata.uid`). No API token is involved — the field rides on the pod object —
/// so this is safe inside the #338 hard-isolation box.
fn downward_env_var(name: &str, field_path: &str) -> EnvVar {
    EnvVar {
        name: name.to_string(),
        value_from: Some(EnvVarSource {
            field_ref: Some(ObjectFieldSelector {
                field_path: field_path.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The plain (non-downward-API) env var name/value pairs injected into a session
/// runtime, factored out so BOTH session backends share ONE env source and can
/// never drift. This is the §5.2 non-secret env (the ~16 core vars), the engine's
/// required HostFact pair ([`CANDIDATE_PREFIX_ENV`] / [`CANDIDATE_FROM_SEP_ENV`],
/// which reach the supervise child through the driver's base-env layer), plus the three
/// PLAIN log-streaming vars ([`ENV_SESSION_ID`] / [`ENV_TRIGGER_ISSUE`] /
/// [`ENV_CONFIG_HASH`], the collector's bundle key + `meta.json` inputs) — but NOT
/// the two downward-API vars ([`ENV_POD_UID`] / [`ENV_POD_NAME`]): those use a pod
/// fieldRef, so the Kubernetes backend appends them in [`session_env`] while an
/// OpenSandbox backend (which has no downward API) supplies them as plain values.
/// It adds NO storage credential — the storage SA creds ride the per-session
/// Secret (see [`build_session_secret`]), never env. Order is stable so the rendered
/// runtime env is deterministic (aids tests + drift detection). Keys are owned
/// `String`s because the operator rate pools render dynamic
/// `FKST_RATE_POOL_<NAME>` names.
pub(crate) fn session_env_pairs(
    spec: &SessionPodSpec,
    config: &PodConfig,
) -> Vec<(String, String)> {
    let mut pairs: Vec<(&'static str, String)> = vec![
        (
            GITHUB_REPO_ENV,
            format!("{}/{}", spec.repo.owner, spec.repo.name),
        ),
        (GITHUB_BOT_LOGIN_ENV, spec.bot_login.clone()),
        (GITHUB_WRITE_ENV, GITHUB_WRITE_VALUE.to_string()),
        (GITHUB_CLAIM_MODE_ENV, GITHUB_CLAIM_MODE_VALUE.to_string()),
        (
            GITHUB_PROXY_POLL_LABEL_PREFIX_ENV,
            GITHUB_PROXY_POLL_LABEL_PREFIX_VALUE.to_string(),
        ),
        (LLM_MODEL_ENV, config.llm_model.clone()),
        (LLM_BASE_URL_ENV, config.llm_base_url.clone()),
        (LLM_WIRE_API_ENV, config.llm_wire_api.clone()),
        (DURABLE_ROOT_ENV, DURABLE_ROOT_DIR.to_string()),
        (RUNTIME_ROOT_ENV, RUNTIME_ROOT_DIR.to_string()),
        (SESSION_CREDS_DIR_ENV, CREDS_MOUNT_DIR.to_string()),
        (CODEX_HOME_ENV, CODEX_HOME_DIR.to_string()),
        (GIT_AUTHOR_NAME_ENV, spec.bot_login.clone()),
        (GIT_COMMITTER_NAME_ENV, spec.bot_login.clone()),
        (SESSION_PACKAGE_ROOTS_ENV, spec.package_roots.join(" ")),
        (SESSION_WORK_LABEL_ENV, spec.work_label.clone()),
        // The engine's required HostFacts — without them any package calling
        // `setup_worktree()` fails at runtime with a HostFact-missing error.
        (CANDIDATE_PREFIX_ENV, CANDIDATE_PREFIX_VALUE.to_string()),
        (CANDIDATE_FROM_SEP_ENV, CANDIDATE_FROM_SEP_VALUE.to_string()),
        // The three PLAIN log-streaming vars; the two downward-API vars are appended
        // per-backend (pod fieldRef here, plain value in the OpenSandbox backend).
        (ENV_SESSION_ID, spec.session_id.clone()),
        (ENV_TRIGGER_ISSUE, spec.trigger_issue_number.to_string()),
        (ENV_CONFIG_HASH, spec.config_hash.clone()),
    ];
    let mut env: Vec<(String, String)> = pairs
        .drain(..)
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    // Trusted users — ABSENT (not empty) when there are none, so the packages'
    // author-policy default is untouched rather than fed an empty list.
    if !spec.contributors.is_empty() {
        env.push((
            GITHUB_AUTHORIZED_LOGINS_ENV.to_string(),
            spec.contributors.join(","),
        ));
    }
    // The engine-tunable tail: output locale + the tighten-merged engine config
    // (operator rate-pool defaults vs the trigger's `### Engine Config`). All
    // map-level merged, so duplicate EnvVar names are impossible; nothing set ⇒
    // nothing rendered (byte-identical to the pre-feature layout).
    env.extend(crate::k8s::engine_env::engine_tunables_env(
        spec.output_lang.as_deref(),
        &spec.engine_config,
        &config.rate_pools,
    ));
    env
}

/// The §5.2 non-secret env injected into the session Pod: the shared
/// [`session_env_pairs`] rendered as [`EnvVar`]s, then the two downward-API vars
/// ([`ENV_POD_UID`] / [`ENV_POD_NAME`]) the kubelet fills from the pod's own
/// fieldRefs (never a literal value — safe inside the #338 hard-isolation box). The
/// rendered order is IDENTICAL to the pre-refactor env.
fn session_env(spec: &SessionPodSpec, config: &PodConfig) -> Vec<EnvVar> {
    let mut env: Vec<EnvVar> = session_env_pairs(spec, config)
        .into_iter()
        .map(|(name, value)| env_var(&name, value))
        .collect();
    // Log streaming is unconditional: every session carries the log env; the pod
    // UID/name ride the downward API (a fieldRef), never a literal value.
    env.push(downward_env_var(ENV_POD_UID, "metadata.uid"));
    env.push(downward_env_var(ENV_POD_NAME, "metadata.name"));
    env
}

/// Labels the NetworkPolicy + reconciler select on. `substrate-session` is the
/// Model-B component (adding it to the isolation NetworkPolicy selector is a
/// manifest follow-up; here we just stamp the label).
fn session_labels(spec: &SessionPodSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "app.kubernetes.io/part-of".to_string(),
            "fkst-hosted".to_string(),
        ),
        (
            COMPONENT_LABEL_KEY.to_string(),
            COMPONENT_LABEL_VALUE.to_string(),
        ),
        (SESSION_ID_LABEL.to_string(), spec.session_id.clone()),
    ])
}

/// Annotations the reconciler reads (owner/repo can exceed the label charset, so
/// these are annotations). `last-pending-at` is seeded to now (RFC3339) and stays
/// settable — the caller/reconciler overwrites it as the session goes pending.
fn session_annotations(spec: &SessionPodSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), spec.repo.owner.clone()),
        (ANNOTATION_REPO.to_string(), spec.repo.name.clone()),
        (
            ANNOTATION_INSTALLATION.to_string(),
            spec.installation_id.to_string(),
        ),
        (
            ANNOTATION_TRIGGER_ISSUE.to_string(),
            spec.trigger_issue_number.to_string(),
        ),
        (ANNOTATION_WORK_LABEL.to_string(), spec.work_label.clone()),
        (ANNOTATION_CONFIG_HASH.to_string(), spec.config_hash.clone()),
        (
            ANNOTATION_LAST_PENDING_AT.to_string(),
            Utc::now().to_rfc3339(),
        ),
    ])
}

/// Build the long-lived substrate-session Pod (pure; no API calls). A bare
/// `core/v1` Pod (NOT a Job) running `config.image` with `["run-substrate"]`,
/// mounting the per-session creds Secret 0400 as a WHOLE volume, then the shared
/// #338 R3 hard-isolation box.
pub fn build_session_pod(spec: &SessionPodSpec, config: &PodConfig) -> Result<Pod, LaunchError> {
    let image = config.image.clone().ok_or(LaunchError::NoImage)?;
    let name = session_object_name(&spec.session_id);

    let container = Container {
        name: RUNNER_CONTAINER.to_string(),
        image: Some(image),
        args: Some(vec![RUN_SUBSTRATE_ARG.to_string()]),
        env: Some(session_env(spec, config)),
        volume_mounts: Some(vec![VolumeMount {
            name: CREDS_VOLUME.to_string(),
            mount_path: CREDS_MOUNT_DIR.to_string(),
            read_only: Some(true),
            // NEVER use sub_path: a subPath mount is NOT refreshed when the
            // Secret is rewritten, so the rotating `github-token` would freeze at
            // its first value. Whole-volume projection propagates rotations, so
            // this stays explicitly None.
            sub_path: None,
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

    let mut pod_spec = PodSpec {
        // "Always": the session is a long-lived self-healing daemon. The
        // reconciler DELETEs the pod for idle-kill / deregister, so this policy
        // never fights a deliberate kill. NOTE for PR4: the run-substrate
        // entrypoint MUST initialize FKST_DURABLE_ROOT idempotently so a
        // container restart RESUMES delivery state rather than wiping it.
        restart_policy: Some("Always".to_string()),
        service_account_name: Some(config.service_account.clone()),
        containers: vec![container],
        volumes: Some(vec![volume]),
        ..Default::default()
    };
    // The SAME #338 R3 hard-isolation box as a Model-A Job pod (no API token, no
    // service discovery, external DNS only, host namespaces off, root boxed).
    crate::k8s::isolation::apply_isolation(
        &mut pod_spec,
        &config.dns_nameservers,
        config.runtime_class.as_deref(),
    );

    Ok(Pod {
        metadata: ObjectMeta {
            name: Some(name),
            namespace: Some(config.namespace.clone()),
            labels: Some(session_labels(spec)),
            annotations: Some(session_annotations(spec)),
            ..Default::default()
        },
        spec: Some(pod_spec),
        ..Default::default()
    })
}

/// Build the per-session creds Secret (pure; no API calls) from an ALREADY-ASSEMBLED
/// `creds` map — the caller (the reconciler's executor) owns the credential layout
/// now, assembling it through the shared
/// [`credential_secret_data`](crate::session_spec::creds::credential_secret_data)
/// helper so the layout never diverges from the Model-A Job Secret. Each value is a
/// [`SecretString`] and is exposed only here to write it into `string_data`. The
/// non-secret credentials-complete sentinel ([`CREDS_COMPLETE_SENTINEL`]) is added so
/// it rides the atomic Secret mount and the in-pod driver's engine-start gate passes
/// instantly. Owner-referenced to the Pod (when `owner` is provided) so K8s
/// cascade-deletes it on Pod GC.
pub fn build_session_secret(
    spec: &SessionPodSpec,
    creds: BTreeMap<String, SecretString>,
    owner: Option<OwnerReference>,
) -> Secret {
    let mut string_data: BTreeMap<String, String> = creds
        .into_iter()
        .map(|(key, value)| (key, value.expose_secret().to_string()))
        .collect();
    // The credential writer's completeness signal: present at the atomic Secret mount
    // so the in-pod driver's engine-start gate passes instantly. Non-secret (a bare
    // marker), so it rides string_data directly. Deliberately NOT part of the shared
    // credential_secret_data map — it is a per-backend writer signal, not a credential.
    string_data.insert(CREDS_COMPLETE_SENTINEL.to_string(), "1".to_string());

    Secret {
        metadata: ObjectMeta {
            name: Some(session_object_name(&spec.session_id)),
            labels: Some(session_labels(spec)),
            owner_references: owner.map(|o| vec![o]),
            ..Default::default()
        },
        string_data: Some(string_data),
        type_: Some("Opaque".to_string()),
        ..Default::default()
    }
}

/// The OwnerReference a created session Pod presents to its Secret, so the Secret
/// cascade-deletes when the Pod is removed. `None` if the created Pod is missing
/// a name or UID (it never is post-create).
fn pod_owner_reference(pod: &Pod) -> Option<OwnerReference> {
    let name = pod.metadata.name.clone()?;
    let uid = pod.metadata.uid.clone()?;
    Some(OwnerReference {
        api_version: "v1".to_string(),
        kind: "Pod".to_string(),
        name,
        uid,
        controller: Some(true),
        block_owner_deletion: Some(true),
    })
}

/// What a create did: a freshly created Pod, or an idempotent no-op because the
/// deterministically-named Pod already existed (the session is already live).
#[derive(Debug, PartialEq, Eq)]
pub enum SessionPodOutcome {
    Created,
    AlreadyLive,
}

/// Create the session's Pod, then its owner-referenced Secret (idempotent). The
/// Pod is created FIRST so the Secret can carry its UID as an ownerReference (the
/// pod waits for the Secret to mount) — the Secret is BUILT here, after the create,
/// from the already-assembled `creds` so it can be owner-ref'd to the live Pod. A
/// `409 AlreadyExists` on the Pod (same deterministic name) means the session is
/// already live → [`AlreadyLive`], mirroring the Model-A launcher's idempotent create.
///
/// [`AlreadyLive`]: SessionPodOutcome::AlreadyLive
pub async fn create_session_pod(
    client: &kube::Client,
    spec: &SessionPodSpec,
    pod: Pod,
    creds: BTreeMap<String, SecretString>,
) -> Result<SessionPodOutcome, LaunchError> {
    // Namespace rides on the Pod (build_session_pod set it to config.namespace);
    // the Secret inherits it just before creation.
    let namespace = pod
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let pod_name = pod.metadata.name.clone().unwrap_or_default();
    let pods: Api<Pod> = Api::namespaced(client.clone(), &namespace);
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &namespace);

    let created = match pods.create(&PostParams::default(), &pod).await {
        Ok(created) => created,
        Err(kube::Error::Api(err)) if err.code == 409 => {
            tracing::info!(
                pod = %pod_name,
                "session pod create: pod already exists; already-live no-op"
            );
            return Ok(SessionPodOutcome::AlreadyLive);
        }
        Err(err) => return Err(LaunchError::Kube(err)),
    };

    // Build the Secret owner-ref'd to the created Pod so cascade-GC removes it when
    // the Pod is deleted, and pin it to the Pod's namespace.
    let mut secret = build_session_secret(spec, creds, pod_owner_reference(&created));
    secret.metadata.namespace = Some(namespace.clone());
    match secrets.create(&PostParams::default(), &secret).await {
        Ok(_) => {}
        Err(kube::Error::Api(err)) if err.code == 409 => {
            tracing::info!(
                pod = %pod_name,
                "session pod create: creds secret already exists; reusing"
            );
        }
        Err(err) => return Err(LaunchError::Kube(err)),
    }

    tracing::info!(
        pod = %pod_name,
        namespace = %namespace,
        "session pod create: pod created"
    );
    Ok(SessionPodOutcome::Created)
}

#[cfg(test)]
#[path = "session_launcher_tests.rs"]
mod tests;
