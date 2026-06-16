//! Wire types for the internal controller<->worker protocol.
//!
//! Every request is authenticated by the shared secret in the
//! [`INTERNAL_AUTH_HEADER`] and carries an inline `protocol_version` checked
//! against [`PROTOCOL_VERSION`]. The lifecycle state machine is
//! `Active -> Draining -> Terminated`; the drain message vocabulary
//! ([`Draining`], [`Released`], [`ControlMessage::StopSession`], and the
//! `lifecycle_state` on [`Heartbeat`]) is defined here so the elasticity flow
//! (#140) only adds behaviour, never new wire types.
//!
//! Input validation: every request struct is `#[serde(deny_unknown_fields)]`
//! so malformed/extra-field input is rejected at the trust boundary.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::models::RepoRef;

/// Header carrying the shared internal-auth secret on every internal request.
pub const INTERNAL_AUTH_HEADER: &str = "x-fkst-internal-auth";

/// Current internal protocol version. Bumped on any breaking wire change.
pub const PROTOCOL_VERSION: u32 = 1;

/// Worker lifecycle state reported on every heartbeat. `Terminated` is not a
/// wire-reported state (a terminated worker simply stops heartbeating and is
/// expired by the controller), so only the two reportable states exist here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// Accepting and running work.
    #[default]
    Active,
    /// Draining: no longer pulling new work, flushing/handing off live work.
    Draining,
}

/// Worker registration request (worker -> controller). `worker_id` defaults to
/// the k8s pod name (`FKST_POD_ID`) and must be non-empty. `capacity` is the
/// max concurrent engine sessions the worker accepts (0 = derive later).
/// `engine_temp_root` is the worker's engine temp dir (for later re-adopt
/// reasoning; unused by this issue).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub worker_id: String,
    pub protocol_version: u32,
    pub capacity: u32,
    pub engine_temp_root: String,
}

/// Controller's answer to a registration. The controller is authoritative for
/// the heartbeat cadence the worker must use.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegisterResponse {
    pub accepted: bool,
    pub heartbeat_interval_secs: u64,
    pub controller_protocol_version: u32,
}

/// Periodic worker liveness + state report (worker -> controller).
/// `running_sessions` is the set of session ids the worker currently runs
/// (empty until later issues populate it). `lifecycle_state` is required by the
/// elasticity flow (#140).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Heartbeat {
    pub worker_id: String,
    pub protocol_version: u32,
    pub lifecycle_state: LifecycleState,
    pub running_sessions: Vec<String>,
    pub timestamp_unix_ms: i64,
}

/// Controller's heartbeat answer. The controller piggybacks control messages on
/// the heartbeat response (pull model: the worker asks, the controller answers).
/// Not `Eq`: [`ControlMessage::ResolvedDispatch`] carries `SecretString`s, which
/// are intentionally not comparable (use serde round-trips in tests).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatResponse {
    pub acknowledged: bool,
    pub control: Vec<ControlMessage>,
}

/// Controller -> worker control message, piggybacked on a heartbeat response
/// (point-to-point per requesting worker, over the internal-auth + TLS channel).
/// Extensible: future variants add here. Tagged by `type` (snake_case). Not
/// `Eq`: the `ResolvedDispatch` variant carries `SecretString`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    /// Stop a running session. The worker stops the engine and replies with a
    /// [`Released`] so the controller can safely reassign without a double-run.
    StopSession { session_id: String, reason: String },
    /// Dispatch a fully-resolved session to this worker to spawn + supervise
    /// (#151). The controller has already minted the first GitHub-App token and
    /// merged the env/secret profile; the worker clones, writes the runtime-dir
    /// files, and starts the engine. Boxed to keep the enum small (the payload
    /// carries the env profile + token bytes). Dormant until the worker grows a
    /// handler arm and the controller emits it behind `FKST_DISPATCH_MODE`.
    ResolvedDispatch(Box<ResolvedDispatch>),
}

/// Worker -> controller, sent when the worker begins draining. The controller
/// receives and logs it; no reassignment logic lands until #140.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Draining {
    pub worker_id: String,
    pub sessions: Vec<String>,
    pub checkpoint_done: bool,
}

/// Worker -> controller drain/handoff acknowledgement confirming an engine is
/// actually stopped, so the controller can later reassign without a double-run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Released {
    pub worker_id: String,
    pub session_id: String,
}

/// Worker -> controller work-pull request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullRequest {
    pub worker_id: String,
    pub protocol_version: u32,
    pub available_capacity: u32,
}

/// Controller's answer to a pull. Empty until claim authority lands (#135).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullResponse {
    pub assignments: Vec<WorkAssignment>,
}

/// A single unit of work the controller assigns to a worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkAssignment {
    pub session_id: String,
    pub goal_ref: String,
}

// ---------------------------------------------------------------------------
// Engine dispatch (#151): the controller resolves credentials/config and hands
// the worker a self-contained dispatch; the worker spawns + supervises. Secrets
// cross only this internal-auth + TLS channel and are `SecretString`s (redacting
// `Debug`, zeroizing) serialized via the helpers below — serialization is the
// only exposure; a `{:?}` never leaks them.
// ---------------------------------------------------------------------------

/// serde for a single `SecretString`: expose on the wire, re-wrap on read.
mod secret_string {
    use super::{ExposeSecret, SecretString};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &SecretString, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(value.expose_secret())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<SecretString, D::Error> {
        Ok(SecretString::from(String::deserialize(de)?))
    }
}

/// serde for a `BTreeMap<String, SecretString>` (the resolved env profile).
mod secret_string_map {
    use super::{BTreeMap, ExposeSecret, SecretString};
    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<String, SecretString>,
        ser: S,
    ) -> Result<S::Ok, S::Error> {
        let mut out = ser.serialize_map(Some(map.len()))?;
        for (key, value) in map {
            out.serialize_entry(key, value.expose_secret())?;
        }
        out.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        de: D,
    ) -> Result<BTreeMap<String, SecretString>, D::Error> {
        let raw = BTreeMap::<String, String>::deserialize(de)?;
        Ok(raw
            .into_iter()
            .map(|(k, v)| (k, SecretString::from(v)))
            .collect())
    }
}

/// The goal a dispatched session runs. `description` is the engine prompt — a
/// `SecretString` so it never renders in `Debug`/logs (the hosting discipline).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchGoal {
    pub goal_id: String,
    pub title: String,
    #[serde(with = "secret_string")]
    pub description: SecretString,
    pub repo: RepoRef,
}

/// What the worker must clone before it can spawn the engine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CloneSpec {
    pub repo: RepoRef,
    pub git_ref: String,
    pub package_roots: Vec<String>,
}

/// Where a resolved Ornn skill's bytes come from: inlined base64 ZIP (small
/// skillsets) or a presigned URL the worker fetches directly (egress-free
/// escape hatch for large ones).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrnnSource {
    ZipB64(String),
    PresignedUrl(String),
}

/// One resolved Ornn skill the worker installs into the engine's codex home.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OrnnSkillRef {
    pub name: String,
    pub source: OrnnSource,
}

/// Resolved Ornn injection plan (#114): the AGENTS.md appends + the skills.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OrnnPlan {
    pub agents_md_appends: Vec<String>,
    pub skills: Vec<OrnnSkillRef>,
}

/// The controller-resolved journaling plan a [`ResolvedDispatch`] carries (#151).
/// These are the PROCESS-level `fkst_journal::JournalConfig` fields (identical
/// for every session) the worker reconstructs a `JournalConfig` from; the
/// per-session identity (`SessionCtx`) the worker derives locally from the
/// dispatch + the cloned package. A `None` plan on the dispatch means journaling
/// is disabled for the run and the worker writes no progress record — so the
/// controller ships a plan ONLY when GitHub journaling is fully configured
/// (enabled + a repo + a token).
///
/// `github_token` is the PROCESS journal-repo token (the hosting app's own
/// `GITHUB_TOKEN` against the fixed journal repo) — NOT the per-session goal or
/// installation token, and NOT involved in the goal-token reactive refresh. It
/// is a `SecretString`: serialization is its only exposure, never `Debug`/logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalPlan {
    /// Max debounce before a flush, in milliseconds (reconstructs
    /// `JournalConfig::flush_interval`).
    pub flush_interval_ms: u64,
    /// Flush early once this many new completions are buffered.
    pub flush_max_batch: usize,
    /// Mirror completions to a GitHub issue comment (dormant by default).
    pub issue_comments: bool,
    /// Roll a single activity comment on the flush cadence (#139).
    pub activity_comment_enabled: bool,
    /// Max optimistic-concurrency retries per flush.
    pub cas_max_retries: u32,
    /// Bootstrap eventual-consistency re-reads after a 404 on `load_skip_set`.
    pub bootstrap_read_retries: u32,
    /// Branch the journal file lives on.
    pub github_branch: String,
    /// `owner/name` of the journal repo (always present in a shipped plan).
    pub github_repo: String,
    /// GitHub REST API base (tests point this at a mock server).
    pub github_api_base: String,
    /// JSON pointers forming event identity.
    pub identity_pointers: Vec<String>,
    /// Max stdout line length parsed; longer lines are malformed.
    pub max_line_bytes: usize,
    /// The PROCESS journal-repo token (never the goal/install token).
    #[serde(with = "secret_string")]
    pub github_token: SecretString,
}

/// A fully-resolved session dispatch (#151). The controller has already minted
/// the first installation token, merged the vault + NyxID env into
/// `env_profile`, rendered the codex config, and resolved the Ornn plan — the
/// worker needs no controller-only secret to start the engine (only a later
/// token *refresh* round-trips). Not `Eq` (carries `SecretString`s).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDispatch {
    pub session_id: String,
    pub worker_id: String,
    /// The claim's current fencing id; the worker echoes it on every
    /// controller mutation so a superseded worker is fenced off.
    pub fencing_id: i64,
    pub goal: DispatchGoal,
    pub clone_spec: CloneSpec,
    /// The first GitHub-App installation token (`ghs_…`), already minted.
    #[serde(with = "secret_string")]
    pub github_token: SecretString,
    pub github_token_expires_at_unix_ms: i64,
    /// The merged, reserved-key-filtered env the engine starts with (vault
    /// secrets + NyxID token + codex env), each value a `SecretString`.
    #[serde(with = "secret_string_map")]
    pub env_profile: BTreeMap<String, SecretString>,
    /// Rendered codex `config.toml` bytes, or `None` when codex is unconfigured.
    pub codex_config_toml: Option<String>,
    pub ornn: Option<OrnnPlan>,
    /// The resolved journaling plan, or `None` when journaling is disabled for
    /// this run (the worker then writes no progress record).
    pub journal: Option<JournalPlan>,
    /// The mint-request nonce the engine's credential helper presents; the
    /// worker writes it to `<runtime_dir>/.mint-nonce`.
    #[serde(with = "secret_string")]
    pub mint_nonce: SecretString,
}

/// Why the worker is asking the controller to mint a fresh installation token.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RefreshReason {
    /// The engine's credential helper wrote a `.request` file (JIT, blocking).
    Jit,
    /// The worker's proactive ~55-min pre-expiry timer fired.
    Periodic,
    /// A GitHub 401 was observed (engine stdout or the journal client).
    Reactive,
}

/// Worker -> controller: mint a fresh installation token for a running session
/// (#151). Fence-guarded: the controller refuses a stale `fencing_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CredentialRefreshRequest {
    pub worker_id: String,
    pub protocol_version: u32,
    pub session_id: String,
    pub fencing_id: i64,
    pub repo_ref: String,
    pub reason: RefreshReason,
}

/// A freshly-minted installation token + its expiry (controller -> worker).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshedToken {
    #[serde(with = "secret_string")]
    pub token: SecretString,
    pub expires_at_unix_ms: i64,
}

/// Controller's answer to a refresh. `credentials: None` means refused (stale
/// fence); `gone: true` means the App installation is gone (the controller will
/// also push a `StopSession`). Not `Eq` (carries a `SecretString`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialRefreshResponse {
    pub credentials: Option<RefreshedToken>,
    pub gone: bool,
}

/// A session's lifecycle status, reported worker -> controller.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Validating,
    Running,
    Stopped,
    Failed,
}

/// How a terminal engine exited (for the controller's claim bookkeeping).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TerminalExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
}

/// Worker -> controller session status report (#151). Fence-guarded so a
/// superseded worker cannot overwrite the claim's status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StatusReport {
    pub worker_id: String,
    pub protocol_version: u32,
    pub session_id: String,
    pub fencing_id: i64,
    pub status: SessionStatus,
    pub terminal: Option<TerminalExit>,
    pub timestamp_unix_ms: i64,
}

/// Errors at the protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    /// The peer's `protocol_version` does not match ours.
    #[error("protocol version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },
    /// The internal-auth secret was missing or wrong.
    #[error("unauthorized internal request")]
    Unauthorized,
}

/// Reject a request whose inline `protocol_version` does not match ours.
pub fn check_protocol_version(theirs: u32) -> Result<(), ProtocolError> {
    if theirs == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::VersionMismatch {
            expected: PROTOCOL_VERSION,
            got: theirs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_state_defaults_to_active_and_serializes_snake_case() {
        assert_eq!(LifecycleState::default(), LifecycleState::Active);
        assert_eq!(
            serde_json::to_string(&LifecycleState::Draining).unwrap(),
            "\"draining\""
        );
    }

    #[test]
    fn check_protocol_version_accepts_current_and_rejects_other() {
        assert!(check_protocol_version(PROTOCOL_VERSION).is_ok());
        assert_eq!(
            check_protocol_version(999),
            Err(ProtocolError::VersionMismatch {
                expected: PROTOCOL_VERSION,
                got: 999,
            })
        );
    }

    #[test]
    fn control_message_is_tag_typed_snake_case() {
        let msg = ControlMessage::StopSession {
            session_id: "s1".into(),
            reason: "drain".into(),
        };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "stop_session");
        assert_eq!(json["session_id"], "s1");
    }

    #[test]
    fn register_request_rejects_unknown_fields() {
        let json = r#"{"worker_id":"w1","protocol_version":1,"capacity":4,"engine_temp_root":"/tmp/e","extra":true}"#;
        assert!(serde_json::from_str::<RegisterRequest>(json).is_err());
    }

    #[test]
    fn every_protocol_type_round_trips_through_serde() {
        macro_rules! round_trip {
            ($val:expr) => {{
                let v = $val;
                let s = serde_json::to_string(&v).unwrap();
                let back = serde_json::from_str(&s).unwrap();
                assert_eq!(v, back);
            }};
        }
        round_trip!(RegisterRequest {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            capacity: 4,
            engine_temp_root: "/tmp/e".into(),
        });
        round_trip!(RegisterResponse {
            accepted: true,
            heartbeat_interval_secs: 10,
            controller_protocol_version: PROTOCOL_VERSION,
        });
        round_trip!(Heartbeat {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            lifecycle_state: LifecycleState::Active,
            running_sessions: vec!["s1".into()],
            timestamp_unix_ms: 1_700_000_000_000,
        });
        round_trip!(Draining {
            worker_id: "w1".into(),
            sessions: vec!["s1".into()],
            checkpoint_done: false,
        });
        round_trip!(Released {
            worker_id: "w1".into(),
            session_id: "s1".into(),
        });
        round_trip!(PullRequest {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            available_capacity: 2,
        });
        round_trip!(PullResponse {
            assignments: vec![WorkAssignment {
                session_id: "s1".into(),
                goal_ref: "owner/repo#1".into(),
            }],
        });
    }

    /// Build a representative `ResolvedDispatch` for the secret-bearing tests.
    fn sample_dispatch() -> ResolvedDispatch {
        let mut env = BTreeMap::new();
        env.insert(
            "OPENAI_API_KEY".to_string(),
            SecretString::from("sk-secret-val"),
        );
        env.insert("FOO".to_string(), SecretString::from("bar"));
        ResolvedDispatch {
            session_id: "s1".into(),
            worker_id: "w1".into(),
            fencing_id: 7,
            goal: DispatchGoal {
                goal_id: "g1".into(),
                title: "Build it".into(),
                description: SecretString::from("SECRET-PROMPT-BODY"),
                repo: RepoRef {
                    owner: "acme".into(),
                    name: "site".into(),
                },
            },
            clone_spec: CloneSpec {
                repo: RepoRef {
                    owner: "acme".into(),
                    name: "site".into(),
                },
                git_ref: "main".into(),
                package_roots: vec!["pkg-a".into()],
            },
            github_token: SecretString::from("ghs_installation_token_xyz"),
            github_token_expires_at_unix_ms: 1_700_000_000_000,
            env_profile: env,
            codex_config_toml: Some("[provider]\n".into()),
            ornn: Some(OrnnPlan {
                agents_md_appends: vec!["use skill X".into()],
                skills: vec![OrnnSkillRef {
                    name: "x".into(),
                    source: OrnnSource::PresignedUrl("https://store/x.zip".into()),
                }],
            }),
            journal: Some(JournalPlan {
                flush_interval_ms: 2000,
                flush_max_batch: 50,
                issue_comments: false,
                activity_comment_enabled: true,
                cas_max_retries: 5,
                bootstrap_read_retries: 3,
                github_branch: "main".into(),
                github_repo: "acme/journal".into(),
                github_api_base: "https://api.github.com".into(),
                identity_pointers: vec!["/department".into(), "/source".into()],
                max_line_bytes: 1_048_576,
                github_token: SecretString::from("ghp_journal_repo_token"),
            }),
            mint_nonce: SecretString::from("nonce-abc"),
        }
    }

    /// Secret-bearing types are not `Eq`; prove the round-trip by re-serializing
    /// the deserialized value and comparing the canonical JSON.
    #[test]
    fn secret_bearing_types_round_trip_through_serde() {
        macro_rules! json_round_trip {
            ($ty:ty, $val:expr) => {{
                let v: $ty = $val;
                let json1 = serde_json::to_string(&v).unwrap();
                let back: $ty = serde_json::from_str(&json1).unwrap();
                let json2 = serde_json::to_string(&back).unwrap();
                assert_eq!(json1, json2, "round-trip must be stable");
            }};
        }
        json_round_trip!(ResolvedDispatch, sample_dispatch());
        json_round_trip!(
            HeartbeatResponse,
            HeartbeatResponse {
                acknowledged: true,
                control: vec![ControlMessage::ResolvedDispatch(
                    Box::new(sample_dispatch())
                )],
            }
        );
        json_round_trip!(
            RefreshedToken,
            RefreshedToken {
                token: SecretString::from("ghs_fresh"),
                expires_at_unix_ms: 1_700_000_000_000,
            }
        );
        json_round_trip!(
            CredentialRefreshResponse,
            CredentialRefreshResponse {
                credentials: Some(RefreshedToken {
                    token: SecretString::from("ghs_fresh"),
                    expires_at_unix_ms: 1,
                }),
                gone: false,
            }
        );
    }

    /// The non-secret request/report types are `Eq` and round-trip directly.
    #[test]
    fn dispatch_request_types_round_trip() {
        macro_rules! round_trip {
            ($val:expr) => {{
                let v = $val;
                let s = serde_json::to_string(&v).unwrap();
                let back = serde_json::from_str(&s).unwrap();
                assert_eq!(v, back);
            }};
        }
        round_trip!(CredentialRefreshRequest {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            session_id: "s1".into(),
            fencing_id: 7,
            repo_ref: "acme/site".into(),
            reason: RefreshReason::Jit,
        });
        round_trip!(StatusReport {
            worker_id: "w1".into(),
            protocol_version: PROTOCOL_VERSION,
            session_id: "s1".into(),
            fencing_id: 7,
            status: SessionStatus::Running,
            terminal: Some(TerminalExit {
                code: Some(0),
                signal: None,
            }),
            timestamp_unix_ms: 1,
        });
    }

    /// A secret value is exposed ONLY through serialization — never through
    /// `Debug`. This is the load-bearing redaction guarantee for the wire types.
    #[test]
    fn secret_fields_redact_in_debug() {
        let dispatch = sample_dispatch();
        let rendered = format!("{dispatch:?}");
        for leak in [
            "ghs_installation_token_xyz",
            "sk-secret-val",
            "SECRET-PROMPT-BODY",
            "nonce-abc",
            "ghp_journal_repo_token",
        ] {
            assert!(!rendered.contains(leak), "secret leaked in Debug: {leak}");
        }
        // The serialized form DOES carry them (that is the only exposure).
        let json = serde_json::to_string(&dispatch).unwrap();
        assert!(json.contains("ghs_installation_token_xyz"));
        assert!(json.contains("sk-secret-val"));
        assert!(json.contains("ghp_journal_repo_token"));
        // And the deserialized value recovers them intact.
        let back: ResolvedDispatch = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.github_token.expose_secret(),
            "ghs_installation_token_xyz"
        );
        assert_eq!(
            back.env_profile
                .get("OPENAI_API_KEY")
                .unwrap()
                .expose_secret(),
            "sk-secret-val"
        );
        assert_eq!(back.goal.description.expose_secret(), "SECRET-PROMPT-BODY");
        assert_eq!(
            back.journal.unwrap().github_token.expose_secret(),
            "ghp_journal_repo_token"
        );
    }

    /// Unknown fields are rejected at the trust boundary on the new types too.
    #[test]
    fn new_types_reject_unknown_fields() {
        let bad = r#"{"worker_id":"w1","protocol_version":1,"session_id":"s1","fencing_id":7,"repo_ref":"acme/site","reason":"jit","extra":true}"#;
        assert!(serde_json::from_str::<CredentialRefreshRequest>(bad).is_err());
    }
}
