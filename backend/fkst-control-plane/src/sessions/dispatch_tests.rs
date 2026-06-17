//! Unit tests for the controller-side dispatch resolver (#151).
//!
//! Each test builds an in-memory `Inner` (no Mongo, no network) wired with the
//! exact same enablers the production service uses — a fake GitHub-App
//! transport, an in-memory goal store, an in-memory vault with an inline secret,
//! optional NyxID (wiremock) / codex / ornn fakes — then asserts the resolved
//! dispatch carries values BEHAVIOURALLY IDENTICAL to what `drive_inner`
//! resolves in-process, and that no secret ever appears in its `Debug`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use fkst_shared::protocol::OrnnSource;

use super::*;
use crate::engine::EngineConfig;
use crate::github_app::api::{
    GithubApi, InstallationId, InstallationToken, InstallationTokenRequest,
};
use crate::github_app::{GithubAppConfig, GithubAppError, GithubAppTokens};
use crate::goals::{GoalDoc, GoalIssueStore, GoalStatus};
use crate::models::{RepoRef, SessionDoc, SessionStatus};
use crate::nyxid::NyxIdClient;
use crate::nyxid::ProxyResponse;
use crate::ornn::{OrnnClient, OrnnPinKind, OrnnSkillPin, OrnnTransport};
use crate::sessions::service::SessionService;
use crate::sessions::SessionRepo;
use crate::vault::{EnvKind, EnvScopeRef, VaultLimits, VaultService};

const OWNER: &str = "acme";
const REPO: &str = "site";
const OWNER_USER: &str = "user-1";
const WORKER: &str = "worker-7";

// ---- fakes ----------------------------------------------------------------

/// Minimal fake GitHub API: every mint returns a distinct `ghs_…` token.
#[derive(Default)]
struct FakeGithubApi {
    mint_count: AtomicUsize,
}

#[async_trait]
impl GithubApi for FakeGithubApi {
    async fn installation_for_repo(
        &self,
        _app_jwt: &SecretString,
        _owner: &str,
        _repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        Ok(InstallationId(99))
    }

    async fn create_installation_token(
        &self,
        _app_jwt: &SecretString,
        _id: InstallationId,
        _req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        let n = self.mint_count.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(InstallationToken {
            token: SecretString::from(format!("ghs_dispatch_token_{n}")),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        })
    }
}

/// In-memory ornn transport: canned proxy replies (matched by path substring)
/// plus a fixed zip for the (unused-here) direct download.
struct FakeOrnnTransport {
    proxy: StdMutex<Vec<(String, u16, serde_json::Value)>>,
}

impl FakeOrnnTransport {
    fn new() -> Self {
        Self {
            proxy: StdMutex::new(Vec::new()),
        }
    }
    fn push(&self, needle: &str, status: u16, body: serde_json::Value) {
        self.proxy
            .lock()
            .unwrap()
            .push((needle.to_string(), status, body));
    }
}

#[async_trait]
impl OrnnTransport for FakeOrnnTransport {
    async fn proxy_get(
        &self,
        path: &str,
        _query: &[(&str, &str)],
        _user_token: &SecretString,
    ) -> Result<ProxyResponse, crate::error::AppError> {
        let mut queue = self.proxy.lock().unwrap();
        let idx = queue
            .iter()
            .position(|(needle, _, _)| path.contains(needle.as_str()))
            .unwrap_or_else(|| panic!("no fake ornn reply for {path}"));
        let (_, status, body) = queue.remove(idx);
        Ok(ProxyResponse {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers: reqwest::header::HeaderMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }
    async fn download_direct(&self, _url: &str) -> Result<Vec<u8>, crate::error::AppError> {
        // resolve_plan never downloads (hop 2 is the worker's job).
        Ok(Vec::new())
    }
}

// ---- builders -------------------------------------------------------------

/// A goal session doc targeting `acme/site`, owned by `user-1`, scoped to the
/// repo (so the vault's repo overlay resolves), with `ornn_skills` set.
fn goal_session(goal_id: bson::Uuid, with_pins: bool) -> SessionDoc {
    SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        status: SessionStatus::Pending,
        pod_id: None,
        fencing_token: Some(11),
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: Some(OWNER_USER.to_string()),
        org_id: None,
        package_names: vec!["pkg-a".to_string(), "pkg-b".to_string()],
        goal_id: Some(goal_id),
        repo: Some(RepoRef {
            owner: OWNER.to_string(),
            name: REPO.to_string(),
        }),
        env_scope: Some(EnvScopeRef::repo(OWNER, REPO)),
        triggered_by: Some("goal-trigger".to_string()),
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: with_pins.then(|| {
            vec![
                OrnnSkillPin {
                    kind: OrnnPinKind::Skillset,
                    name: "research".to_string(),
                    version: "3.0".to_string(),
                },
                OrnnSkillPin {
                    kind: OrnnPinKind::Skill,
                    name: "fmt".to_string(),
                    version: "2.0".to_string(),
                },
            ]
        }),
        terminal_cause: None,
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    }
}

fn goal_doc(goal_id: bson::Uuid) -> GoalDoc {
    GoalDoc {
        id: goal_id,
        title: "Ship the thing".to_string(),
        description: "SECRET-ENGINE-PROMPT-BODY".to_string(),
        package_names: vec!["pkg-a".to_string(), "pkg-b".to_string()],
        repo: None,
        status: GoalStatus::Triggered,
        owner_user_id: OWNER_USER.to_string(),
        org_id: None,
        active_session_id: None,
        created_at: bson::DateTime::now(),
        updated_at: bson::DateTime::now(),
    }
}

/// Build an RSA-backed test GitHub-App config (the encoding key must parse).
fn github_config() -> GithubAppConfig {
    use rand::rngs::OsRng;
    use rsa::pkcs8::{EncodePrivateKey, LineEnding};
    use rsa::RsaPrivateKey;
    let private = RsaPrivateKey::new(&mut OsRng, 2048).expect("rsa key");
    let pem = private.to_pkcs8_pem(LineEnding::LF).expect("pem");
    GithubAppConfig {
        app_id: 42,
        private_key_pem: SecretString::from(pem.to_string()),
        app_slug: Some("fkst".to_string()),
        webhook_secret: None,
        api_base: "https://api.github.test".to_string(),
    }
}

/// A `SessionService` carrying the wired `Inner` for resolve_dispatch tests; the
/// in-memory session store needs no datastore.
async fn base_service() -> SessionService {
    SessionService::new(SessionRepo::new(), EngineConfig::default())
}

/// Enable goal support with the fake GitHub API and seed `goal` into the store.
async fn enable_goal(service: &SessionService, goal: &GoalDoc) {
    let github_app =
        GithubAppTokens::with_api(&github_config(), Arc::new(FakeGithubApi::default()))
            .expect("github app");
    let goals = GoalIssueStore::new(None);
    goals.insert(goal).await.expect("seed goal");
    service.enable_goal_support(goals, github_app);
}

/// Enable a vault holding one inline secret at the session's repo scope.
fn enable_vault_with_inline(service: &SessionService) {
    let vault = VaultService::new(VaultLimits {
        value_byte_cap: 64 * 1024,
        entries_per_scope_cap: 100,
    });
    vault
        .set_inline(
            OWNER_USER,
            &EnvScopeRef::repo(OWNER, REPO),
            vec![(
                "OPENAI_API_KEY".to_string(),
                EnvKind::Secret,
                SecretString::from("sk-INLINE-SECRET-VALUE"),
            )],
        )
        .expect("set inline");
    service.enable_vault(vault);
}

// ---- tests ----------------------------------------------------------------

/// The minimal path: goal + github only. The dispatch carries the first token,
/// the goal prompt as a SecretString, the clone spec, and the mint nonce; with
/// no vault/codex/ornn wired those resolve empty/None — byte-identical to a
/// drive_inner with the same (absent) wiring.
#[tokio::test]
async fn resolve_dispatch_minimal_carries_token_goal_and_nonce() {
    let goal_id = bson::Uuid::new();
    let service = base_service().await;
    enable_goal(&service, &goal_doc(goal_id)).await;
    let session = goal_session(goal_id, false);

    let dispatch = service
        .resolve_dispatch(&session, None, WORKER, 11)
        .await
        .expect("resolve");

    assert_eq!(dispatch.worker_id, WORKER);
    assert_eq!(dispatch.fencing_id, 11);
    assert_eq!(dispatch.session_id, session.id.to_string());
    assert_eq!(dispatch.goal.goal_id, goal_id.to_string());
    assert_eq!(dispatch.goal.title, "Ship the thing");
    // The prompt is the SecretString description (exposed only here, in the test).
    assert_eq!(
        dispatch.goal.description.expose_secret(),
        "SECRET-ENGINE-PROMPT-BODY"
    );
    assert_eq!(dispatch.goal.repo.owner, OWNER);
    // The first token was minted.
    assert!(dispatch.github_token.expose_secret().starts_with("ghs_"));
    assert!(dispatch.github_token_expires_at_unix_ms > 0);
    // Clone spec: default-branch HEAD + the effective package roots.
    assert_eq!(dispatch.clone_spec.git_ref, "HEAD");
    assert_eq!(dispatch.clone_spec.package_roots, vec!["pkg-a", "pkg-b"]);
    // No vault/codex/ornn wired => empty env, no codex, no plan.
    assert!(dispatch.env_profile.is_empty());
    assert!(dispatch.codex_config_toml.is_none());
    assert!(dispatch.ornn.is_none());
    // The nonce is a 32-hex-char value (the engine's scheme).
    assert_eq!(dispatch.mint_nonce.expose_secret().len(), 32);
    assert!(dispatch
        .mint_nonce
        .expose_secret()
        .chars()
        .all(|c| c.is_ascii_hexdigit()));
}

/// The fully-wired path: vault inline secret + NyxID + codex + ornn pins. The
/// dispatch's env carries the inline secret AND the merged NyxID entries, the
/// codex toml is present, and the ornn plan carries the pinned skills'
/// presigned URLs — each via the SAME helper the in-process driver uses.
#[tokio::test]
async fn resolve_dispatch_full_merges_env_codex_and_ornn() {
    let goal_id = bson::Uuid::new();
    let service = base_service().await;
    enable_goal(&service, &goal_doc(goal_id)).await;
    enable_vault_with_inline(&service);

    // NyxID: a wiremock that mints a per-session agent key.
    let nyxid_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/api-keys"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "key-xyz",
            "full_key": "nyxid_ag_SESSION_KEY_SECRET"
        })))
        .mount(&nyxid_server)
        .await;
    let nyxid_client = NyxIdClient::new(
        &nyxid_server.uri(),
        "api-github",
        "sa_client".to_string(),
        SecretString::from("sa_secret".to_string()),
        Duration::from_secs(30),
    )
    .expect("nyxid client");
    service.enable_nyxid_token(
        nyxid_client,
        "https://nyxid.test".to_string(),
        Duration::from_secs(3600),
    );

    // Codex: operator-pinned defaults; with the vault wired this renders.
    service.enable_codex(
        "gpt-test".to_string(),
        "https://chrono-llm.test".to_string(),
    );

    // Ornn: a skillset closure (one member `web`) + a direct pin `fmt`.
    let transport = Arc::new(FakeOrnnTransport::new());
    transport.push(
        "/skillsets/research/closure",
        200,
        serde_json::json!({
            "data": { "instructions": "Master prompt.",
                      "items": [ { "name": "web", "version": "1.0" } ] }
        }),
    );
    transport.push(
        "/skills/web",
        200,
        serde_json::json!({ "data": { "name": "web", "version": "1.0",
            "presignedPackageUrl": "https://storage/web.zip?sig=w" } }),
    );
    transport.push(
        "/skills/fmt",
        200,
        serde_json::json!({ "data": { "name": "fmt", "version": "2.0",
            "presignedPackageUrl": "https://storage/fmt.zip?sig=f" } }),
    );
    service.enable_ornn(OrnnClient::new(transport));

    let session = goal_session(goal_id, true);
    let raw = SecretString::from("user-raw-token");
    let dispatch = service
        .resolve_dispatch(&session, Some(&raw), WORKER, 11)
        .await
        .expect("resolve");

    // Env: the inline secret survives the reserved-key filter...
    assert_eq!(
        dispatch
            .env_profile
            .get("OPENAI_API_KEY")
            .map(|v| v.expose_secret()),
        Some("sk-INLINE-SECRET-VALUE")
    );
    // ...and the NyxID entries were merged (B4).
    assert_eq!(
        dispatch
            .env_profile
            .get("NYXID_ACCESS_TOKEN")
            .map(|v| v.expose_secret()),
        Some("nyxid_ag_SESSION_KEY_SECRET")
    );
    assert_eq!(
        dispatch
            .env_profile
            .get("NYXID_URL")
            .map(|v| v.expose_secret()),
        Some("https://nyxid.test")
    );

    // Codex rendered (the default chrono-llm layer for a vault with no override).
    let toml = dispatch.codex_config_toml.expect("codex toml present");
    assert!(toml.contains("chrono-llm"));
    assert!(toml.contains("gpt-test"));

    // Ornn plan: both leaf skills, sourced from their presigned URLs, plus the
    // skillset's rendered AGENTS.md marker block.
    let plan = dispatch.ornn.expect("ornn plan present");
    let by_name: std::collections::HashMap<&str, &OrnnSource> = plan
        .skills
        .iter()
        .map(|s| (s.name.as_str(), &s.source))
        .collect();
    assert_eq!(plan.skills.len(), 2);
    assert_eq!(
        by_name.get("web"),
        Some(&&OrnnSource::PresignedUrl(
            "https://storage/web.zip?sig=w".to_string()
        ))
    );
    assert_eq!(
        by_name.get("fmt"),
        Some(&&OrnnSource::PresignedUrl(
            "https://storage/fmt.zip?sig=f".to_string()
        ))
    );
    assert_eq!(plan.agents_md_appends.len(), 1);
    assert!(plan.agents_md_appends[0].contains("Master prompt."));
}

/// The redaction guarantee: NO secret — the inline value, the token, the goal
/// prompt, the NyxID key, or the nonce — appears in the dispatch's `Debug`.
#[tokio::test]
async fn resolve_dispatch_never_leaks_a_secret_in_debug() {
    let goal_id = bson::Uuid::new();
    let service = base_service().await;
    enable_goal(&service, &goal_doc(goal_id)).await;
    enable_vault_with_inline(&service);

    let nyxid_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/api-keys"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "id": "key-xyz",
            "full_key": "nyxid_ag_NEVER_LEAK_THIS"
        })))
        .mount(&nyxid_server)
        .await;
    let nyxid_client = NyxIdClient::new(
        &nyxid_server.uri(),
        "api-github",
        "sa_client".to_string(),
        SecretString::from("sa_secret".to_string()),
        Duration::from_secs(30),
    )
    .expect("nyxid client");
    service.enable_nyxid_token(
        nyxid_client,
        "https://nyxid.test".to_string(),
        Duration::from_secs(3600),
    );

    let session = goal_session(goal_id, false);
    let raw = SecretString::from("user-raw-token");
    let dispatch = service
        .resolve_dispatch(&session, Some(&raw), WORKER, 11)
        .await
        .expect("resolve");

    let rendered = format!("{dispatch:?}");
    for leak in [
        "sk-INLINE-SECRET-VALUE",
        "nyxid_ag_NEVER_LEAK_THIS",
        "SECRET-ENGINE-PROMPT-BODY",
        dispatch.github_token.expose_secret(),
        dispatch.mint_nonce.expose_secret(),
    ] {
        assert!(
            !rendered.contains(leak),
            "secret leaked in dispatch Debug: {leak}"
        );
    }
}

/// A goal-less session is unsupported since #115; the resolver rejects it
/// loudly (mirroring drive_inner) rather than dispatching a session with no
/// clone/mint target.
#[tokio::test]
async fn resolve_dispatch_rejects_a_goal_less_session() {
    let goal_id = bson::Uuid::new();
    let service = base_service().await;
    enable_goal(&service, &goal_doc(goal_id)).await;
    let mut session = goal_session(goal_id, false);
    session.repo = None;

    let err = service
        .resolve_dispatch(&session, None, WORKER, 11)
        .await
        .expect_err("must reject");
    assert!(
        matches!(err, DispatchError::NotAGoalSession(_)),
        "got {err:?}"
    );
}
