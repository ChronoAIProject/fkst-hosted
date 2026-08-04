//! Assembly of everything a session runtime is launched WITH: its branch
//! topology, its pod spec, and its complete credential bundle.
//!
//! Split out of [`super::execute`] because it is pure-ish argument construction —
//! no runtime is created here — and because it is shared by both create-side
//! verbs in [`super::execute_spawn`]: spawn and credential recovery must build
//! byte-identical bundles, or a restart would heal a session into a different
//! credential layout than the one it launched with.
//!
//! Secret hygiene: the minted installation token and every environment value pass
//! through here on their way into the credential bundle and are never logged.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroize;

use crate::access_policy::AccessPolicy;
use crate::config::Config;
use crate::disposable_environment::{DisposableEnvironmentLookup, DISPOSABLE_ENVIRONMENT_MARKER};
use crate::environment_profile::EnvironmentProfileStore;
use crate::github_app::{session_permissions, GithubAppError};
use crate::k8s::{session_github_token_json, SessionPodSpec};
use crate::reconcile::desired::{runtime_config_hash, SessionRegistration};
use crate::reconcile::execute::ReconcileCtx;
use crate::reconcile::execute_comments::{env_not_ready_comment, env_verify_failed_comment};
use crate::reconcile::execute_spawn::ResolvedBranchTopology;
use crate::reconcile::reachability;
use crate::reconcile::work_labels::{apply_work_label_namespace, WorkLabelError};
use crate::session_spec::creds::{credential_secret_data, StorageWriterCreds};

/// The `validation-status` annotation value a fully-written environment carries;
/// only a `ready` environment is injected into a session (mirrors Model A).
const ENV_STATUS_READY: &str = "ready";

pub(crate) enum CredentialResolutionError {
    EnvironmentBlocked { comment: String },
    TokenMintFailed(GithubAppError),
    WorkLabelsInvalid(WorkLabelError),
}

/// Resolve the current full session credential bundle from durable/control-plane
/// sources: the registration, environment store, static service configuration, and a
/// repository-scoped GitHub token. No bundle is persisted in GitHub or logs.
///
/// The GitHub token is FORCE-minted (#3410): see the call site for why a session may
/// never be handed a cached one.
pub(crate) async fn resolve_session_credentials(
    reg: &SessionRegistration,
    detected_work_labels: &[String],
    branches: &ResolvedBranchTopology,
    ctx: &ReconcileCtx,
) -> Result<(SessionPodSpec, BTreeMap<String, SecretString>), CredentialResolutionError> {
    let environment = match resolve_environment(reg, ctx).await {
        EnvResolution::Proceed(environment) => environment,
        EnvResolution::Blocked { comment } => {
            return Err(CredentialResolutionError::EnvironmentBlocked { comment });
        }
    };

    let owner_repo = format!("{}/{}", reg.repo.owner, reg.repo.name);
    // FORCED re-mint (#3410). A session is a LONG-LIVED consumer of this token, and the
    // control plane only revisits it every `pod_token_refresh_secs` (default 45 min).
    // The cached variant may legally serve a token with as little as the shared
    // `EXPIRY_BUFFER` (5 min) of life left — harmless for the reconciler's own
    // millisecond read calls, fatal here: the session's `gh`/`git` would start returning
    // `Bad credentials` minutes after spawn and stay broken until the next rotation tick.
    // Nor is the poisoning entry rare: the cache is keyed on `(repo, permissions)` and
    // `session_permissions()` is structurally equal to `default_permissions()`, so ANY
    // App call on the repo — the reachability probe a few lines up, a branch lookup, an
    // issue comment — leaves behind the entry this mint would otherwise read.
    // Forcing the mint makes the delivered life a full token TTL, which
    // [`ReconcileConfig`] already bounds strictly above `pod_token_refresh_secs`, so the
    // next rotation always lands before expiry. This is the same reasoning the rotation
    // sweep applies (`k8s::token_rotation::rotate_one`); delivery — spawn AND
    // crash-recovery, on both session backends — needs it just as much.
    let implementation_repos = ctx
        .config
        .delivery_grants
        .implementation_repos_for(&reg.repo);
    let mint_result = if implementation_repos.is_empty() {
        ctx.github
            .token_with_expiry_for_repo_forced(&owner_repo, Some(session_permissions()))
            .await
    } else {
        ctx.github
            .token_with_expiry_for_repositories_forced(
                &owner_repo,
                &implementation_repos,
                Some(session_permissions()),
            )
            .await
    };
    let (token, expires_at) = match mint_result {
        Ok(pair) => pair,
        Err(error) => return Err(CredentialResolutionError::TokenMintFailed(error)),
    };
    let github_token_json = session_github_token_json(&token, expires_at);
    let spec = session_pod_spec_from(
        reg,
        detected_work_labels,
        branches,
        ctx.config.reconcile.github_bot_login.clone(),
        &ctx.config.access,
        ctx.config.delivery_grants.session_json_for(&reg.repo),
        ctx.config.reconcile.work_label_namespace.as_deref(),
    )
    .map_err(CredentialResolutionError::WorkLabelsInvalid)?;
    let storage = storage_writer_creds(&ctx.config);
    let creds = credential_secret_data(
        &github_token_json,
        ctx.config.llm_api_key.expose_secret(),
        environment
            .user_env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str())),
        &environment.install,
        &environment.secret_keys,
        storage,
    )
    .into_iter()
    .map(|(key, value)| (key, SecretString::from(value)))
    .collect();

    Ok((spec, creds))
}

/// Build the launch spec from a registration + its effective work-label set (pure;
/// unit-tested). `package_roots` are the EFFECTIVE package refs (explicit ∪
/// manifest-expanded, I7) rendered back to `owner/repo@ref:path` — so the pod clones a
/// manifest's packages too (`FKST_SESSION_PACKAGE_ROOTS`). The pod's `work_label` is the
/// comma-joined `detected_work_labels` (the explicit `### Work Label` ∪ package-discovered
/// labels — the set that actually wakes the session), NOT just the explicit label, so a
/// `### Work Label`-less session runs on its packages' auto-declared labels (epic #594
/// I4). `bot_login` falls back to empty when unset.
pub(crate) fn session_pod_spec_from(
    reg: &SessionRegistration,
    detected_work_labels: &[String],
    branches: &ResolvedBranchTopology,
    bot_login: Option<String>,
    access: &AccessPolicy,
    delivery_grants_json: Option<String>,
    work_label_namespace: Option<&str>,
) -> Result<SessionPodSpec, WorkLabelError> {
    let labels = apply_work_label_namespace(detected_work_labels, work_label_namespace)?;
    Ok(SessionPodSpec {
        session_id: reg.session_id.clone(),
        installation_id: reg.installation_id,
        repo: reg.repo.clone(),
        trigger_issue_number: reg.trigger_issue,
        package_roots: reg
            .effective_packages
            .iter()
            .map(reachability::render_ref)
            .collect(),
        work_label: crate::k8s::work_label_wire::join_work_labels(&labels.effective),
        work_label_map_json: labels.map_json(),
        work_label_namespace: work_label_namespace.map(str::to_string),
        // Serialized only when the session configures at least one package, so an
        // unconfigured session renders no key. BTreeMap ordering makes the JSON
        // deterministic, which matters because this value feeds the runtime config
        // hash: a non-deterministic rendering would look like config drift and
        // respawn the pod on every pass.
        package_env_json: if reg.effective_package_env.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&reg.effective_package_env)
                    .expect("a BTreeMap of strings always serializes"),
            )
        },
        bot_login: bot_login.unwrap_or_default(),
        config_hash: runtime_config_hash(&reg.config_hash, work_label_namespace),
        output_lang: reg.def.output_lang.clone(),
        engine_config: reg.def.engine_config.clone(),
        creator_login: reg.creator_login.clone(),
        // Attribution the runtime is stamped with. Threaded verbatim from the
        // registration and deliberately EXCLUDED from `config_hash` /
        // `full_config_hash` (see `crate::reconcile::hashing`): re-attributing an
        // issue must never look like a configuration change and respawn a
        // running session.
        creator_id: reg.creator_id,
        trigger_author_id: reg.trigger_author_id,
        trigger_author_login: reg.trigger_author_login.clone(),
        contributors: session_contributors(reg, access),
        upstream_branch: branches.upstream.clone(),
        target_branch: branches.integration.clone(),
        delivery_grants_json,
    })
}

/// Package-side work authors: creator first, then Session Collaborators, then
/// login-shaped deployment admins. Log-access/FKST-Contributor entries are not
/// work authority and deliberately do not feed this environment contract.
pub(crate) fn session_contributors(
    reg: &SessionRegistration,
    access: &AccessPolicy,
) -> Vec<String> {
    let mut contributors: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let creator = reg.creator_login.trim();
    let admin_logins: Vec<&str> = access.global_admin_login_entries().collect();
    let excluded_admins = access
        .global_admin_count()
        .saturating_sub(admin_logins.len());
    if excluded_admins > 0 {
        tracing::debug!(
            excluded_global_admin_entries = excluded_admins,
            "session env: numeric global-admin entries omitted from login-only author policy"
        );
    }
    for token in std::iter::once(creator)
        .chain(reg.collaborators.iter().map(String::as_str))
        .chain(admin_logins)
    {
        if token.is_empty() {
            continue;
        }
        let folded = token.to_ascii_lowercase();
        if !seen.contains(&folded) {
            seen.push(folded);
            contributors.push(token.to_string());
        }
    }
    contributors
}

/// Resolve the chrono-storage SA creds to inject into a session Secret, or `None`
/// when the control plane has no storage config (the in-pod uploader then fails
/// closed — no bundle). The single configured NyxID SA serves both the control
/// plane's own storage calls and the in-pod uploader. Borrows the config, exposing
/// the client secret only to copy it into the Secret builder.
pub(crate) fn storage_writer_creds(config: &Config) -> Option<StorageWriterCreds<'_>> {
    let storage = config.storage.as_ref()?;
    Some(StorageWriterCreds {
        client_id: &storage.nyxid_client_id,
        client_secret: storage.nyxid_client_secret.expose_secret(),
        token_url: &storage.nyxid_token_url,
        base_url: &storage.base_url,
        bucket: &storage.bucket,
    })
}

// --- Environment resolution (mirrors the Model-A webhook pre-flight) ----------

/// The outcome of pre-flighting the issue's named environment.
pub(crate) struct ResolvedEnvironment {
    pub(crate) user_env: BTreeMap<String, String>,
    pub(crate) install: Vec<String>,
    pub(crate) secret_keys: Vec<String>,
}

impl Drop for ResolvedEnvironment {
    fn drop(&mut self) {
        for command in &mut self.install {
            command.zeroize();
        }
        for key in &mut self.secret_keys {
            key.zeroize();
        }
        for (mut key, mut value) in std::mem::take(&mut self.user_env) {
            key.zeroize();
            value.zeroize();
        }
    }
}

pub(crate) enum EnvResolution {
    /// Launch with the merged variables/secret VALUES to inject, the ordered install
    /// commands to run in the pod, and the NAMES of the env vars that are secrets
    /// (so the pod can keep them out of the codex config). All empty when the issue
    /// declared no environment.
    Proceed(ResolvedEnvironment),
    /// Do NOT launch; post `comment` on the trigger issue explaining why.
    Blocked { comment: String },
}

/// Resolve either the exact disposable marker through the private handoff, or a
/// normal saved profile through the durable environment store.
async fn resolve_environment(reg: &SessionRegistration, ctx: &ReconcileCtx) -> EnvResolution {
    if reg.def.environment.as_deref() != Some(DISPOSABLE_ENVIRONMENT_MARKER) {
        return resolve_named_environment(
            ctx.env_store.as_ref(),
            reg.creator_id,
            &reg.creator_login,
            reg.def.environment.as_deref(),
        )
        .await;
    }

    let Some(creator_id) = reg.creator_id else {
        tracing::warn!(
            session_id = %reg.session_id,
            "reconcile spawn: disposable environment has no numeric creator; blocking"
        );
        return EnvResolution::Blocked {
            comment: disposable_environment_unavailable_comment(),
        };
    };
    match ctx.disposable_environments.resolve(
        &reg.repo.owner,
        &reg.repo.name,
        reg.trigger_issue,
        creator_id,
    ) {
        DisposableEnvironmentLookup::Found(material) => {
            tracing::info!(
                session_id = %reg.session_id,
                install_commands = material.install.len(),
                env_vars = material.user_env.len(),
                secret_env_vars = material.secret_keys.len(),
                "reconcile spawn: disposable environment resolved"
            );
            EnvResolution::Proceed(ResolvedEnvironment {
                user_env: material.user_env,
                install: material.install,
                secret_keys: material.secret_keys,
            })
        }
        DisposableEnvironmentLookup::Missing => {
            tracing::warn!(
                session_id = %reg.session_id,
                "reconcile spawn: disposable environment handoff missing; blocking"
            );
            EnvResolution::Blocked {
                comment: disposable_environment_unavailable_comment(),
            }
        }
        DisposableEnvironmentLookup::CreatorMismatch => {
            tracing::warn!(
                session_id = %reg.session_id,
                "reconcile spawn: disposable environment creator mismatch; blocking"
            );
            EnvResolution::Blocked {
                comment: disposable_environment_unavailable_comment(),
            }
        }
    }
}

fn disposable_environment_unavailable_comment() -> String {
    "This session's disposable one-time environment is unavailable. Close this trigger and create a new session to submit the environment again."
        .to_string()
}

/// Pre-flight the issue's named environment against the CREATOR's store (keyed by
/// the signed numeric GitHub id). An assignee-derived creator has no id in issue
/// metadata, so environment resolution is unavailable and the session proceeds
/// without an environment (auto-seeded triggers never select one). `None` → an
/// empty session. A named selection for an id-bearing creator must exist and be
/// `ready`; otherwise the launch is blocked with feedback — fail closed.
pub(crate) async fn resolve_named_environment(
    env_store: &dyn EnvironmentProfileStore,
    creator_id: Option<i64>,
    creator_login: &str,
    environment: Option<&str>,
) -> EnvResolution {
    let Some(creator_id) = creator_id else {
        tracing::info!(
            creator = %creator_login,
            requested_environment = environment.unwrap_or_default(),
            "reconcile spawn: creator has no numeric id; resolving no environment"
        );
        return EnvResolution::Proceed(ResolvedEnvironment {
            user_env: BTreeMap::new(),
            install: Vec::new(),
            secret_keys: Vec::new(),
        });
    };

    let name = match environment {
        None => {
            return EnvResolution::Proceed(ResolvedEnvironment {
                user_env: BTreeMap::new(),
                install: Vec::new(),
                secret_keys: Vec::new(),
            })
        }
        Some(name) => name,
    };

    match env_store.get_environment(creator_id, name).await {
        Ok(Some(record)) if record.status == ENV_STATUS_READY => {
            match env_store
                .load_environment_for_session(creator_id, name)
                .await
            {
                Ok(Some((install, user_env, secret_keys))) => {
                    tracing::info!(
                        github_user_id = creator_id,
                        environment = %name,
                        install_commands = install.len(),
                        env_vars = user_env.len(),
                        secret_env_vars = secret_keys.len(),
                        "reconcile spawn: named environment resolved"
                    );
                    EnvResolution::Proceed(ResolvedEnvironment {
                        user_env,
                        install,
                        secret_keys,
                    })
                }
                Ok(None) => EnvResolution::Blocked {
                    comment: env_not_ready_comment(name),
                },
                Err(error) => {
                    tracing::error!(environment = %name, error = %error, "reconcile spawn: environment load failed");
                    EnvResolution::Blocked {
                        comment: env_verify_failed_comment(name),
                    }
                }
            }
        }
        Ok(_) => EnvResolution::Blocked {
            comment: env_not_ready_comment(name),
        },
        Err(error) => {
            tracing::error!(environment = %name, error = %error, "reconcile spawn: environment pre-flight read failed");
            EnvResolution::Blocked {
                comment: env_verify_failed_comment(name),
            }
        }
    }
}

#[cfg(test)]
#[path = "execute_launch_spec_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "execute_token_tests.rs"]
mod token_tests;
