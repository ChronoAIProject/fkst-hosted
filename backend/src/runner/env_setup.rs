//! Per-session environment setup for the `run-session` pod.
//!
//! Two closely-related pieces, split out of [`super`] (runner::mod) to keep each
//! file under the 500-line limit:
//!   - [`inject_user_env`] / [`collect_user_env`] — the injected user-env reader
//!     (PR4b), and
//!   - [`run_install_step`] — the pre-agent, run-as-ROOT named-environment install
//!     step (#338 §5).
//!
//! They belong together because they read the SAME mounted `userenv.*` source and
//! MUST agree on exactly which keys count as user env: the engine env-profile
//! re-wraps those values, while the install step hands the same values to a
//! subprocess as ordinary process env.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};

use super::creds;
use crate::engine::config::is_reserved_env_key;
use crate::install::{run_ordered, Verdict};
use crate::session_spec::{CredsLayout, SessionSpec};
use crate::sessions::codex_provider::LLM_ENV_KEY;

/// Optional env var bounding the pre-agent install step's whole-sequence
/// wall-clock. Reuses the validation-path name so an operator sets ONE knob for
/// both install call sites; the pod need not set it (the default then applies).
const INSTALL_DEADLINE_ENV: &str = "FKST_ENV_VALIDATE_DEADLINE_SECS";
/// Fallback install deadline when the pod sets no [`INSTALL_DEADLINE_ENV`].
const DEFAULT_INSTALL_DEADLINE_SECS: u64 = 600;
/// Trailing stderr bytes surfaced (to the error log) from a failed install
/// command — a small, non-secret bound mirroring the validation path.
const INSTALL_STDERR_TAIL_BYTES: usize = 4096;

/// Read the mounted `userenv.*` files into a plain `{ KEY: value }` map (PR4b).
///
/// Each file's name (minus the `userenv.` prefix) is the env var KEY recovered by
/// [`CredsLayout::user_env_files`]; its contents are the value. A KEY that is
/// platform-reserved ([`is_reserved_env_key`] — which covers the whole `FKST_*`
/// family the engine strips, plus the git-credential keys and host allow-list) or
/// that collides with the LLM credential key ([`LLM_ENV_KEY`]) is skipped with a
/// warning, so a user entry can never shadow a platform var. A missing/empty or
/// unreadable value is also skipped: optional user env must never abort the
/// session. Values are NEVER logged — only the KEY (non-secret) appears.
///
/// The values are plain `String`s (not `SecretString`): both consumers need them
/// unwrapped — the engine env-profile re-wraps each, and the pre-agent install
/// step hands them to a subprocess as ordinary process env. Sharing this ONE
/// reader guarantees the two paths agree on exactly which keys are user env.
pub(super) fn collect_user_env(creds: &CredsLayout, session_id: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let files = match creds.user_env_files() {
        Ok(files) => files,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "run-session: could not list user env files; none injected"
            );
            return out;
        }
    };
    for (key, path) in files {
        if is_reserved_env_key(&key) || key == LLM_ENV_KEY {
            tracing::warn!(
                session_id = %session_id,
                key = %key,
                "run-session: skipping reserved/forbidden user env key"
            );
            continue;
        }
        match creds::read_required_secret(&path) {
            Ok(value) => {
                out.insert(key, value.expose_secret().to_string());
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    key = %key,
                    error = %error,
                    "run-session: skipping unreadable/empty user env value"
                );
            }
        }
    }
    out
}

/// Fold the issue author's injected env into `env_profile` (PR4b), wrapping each
/// value back into a [`SecretString`]. See [`collect_user_env`] for the filtering
/// rules; this is a thin adapter so the engine profile and the install step share
/// one reader.
pub(super) fn inject_user_env(
    creds: &CredsLayout,
    session_id: &str,
    env_profile: &mut BTreeMap<String, SecretString>,
) {
    for (key, value) in collect_user_env(creds, session_id) {
        env_profile.insert(key, SecretString::from(value));
    }
}

/// The whole-sequence deadline for the install step: the pod-injected
/// [`INSTALL_DEADLINE_ENV`] when present and positive, else
/// [`DEFAULT_INSTALL_DEADLINE_SECS`].
fn install_deadline_from_env() -> Duration {
    let secs = std::env::var(INSTALL_DEADLINE_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_INSTALL_DEADLINE_SECS);
    Duration::from_secs(secs)
}

/// Run the resolved environment's install commands as ROOT before the agent
/// starts (#338 §5). Returns `None` on success (proceed to the agent) or
/// `Some(ExitCode::FAILURE)` when a command fails / times out — the caller then
/// returns that code so the Job exits non-zero and the watcher stamps the failure,
/// WITHOUT ever starting the agent. A no-environment session (`spec.install`
/// empty) is a no-op success. Secret VALUES are never logged; only the failing
/// command's index / exit code / captured stderr tail are.
pub(super) async fn run_install_step(
    spec: &SessionSpec,
    creds: &CredsLayout,
    session_id: &str,
) -> Option<ExitCode> {
    if spec.install.is_empty() {
        return None;
    }
    // The install commands need the environment's variables + secret VALUES as
    // their process env (e.g. a token for a private package index).
    let install_env = collect_user_env(creds, session_id);
    let deadline = install_deadline_from_env();
    tracing::info!(
        session_id = %session_id,
        commands = spec.install.len(),
        "run-session: running environment install commands as root before the agent"
    );
    match run_ordered(
        &spec.install,
        &install_env,
        deadline,
        INSTALL_STDERR_TAIL_BYTES,
    )
    .await
    {
        Verdict::Ok { commands } => {
            tracing::info!(
                session_id = %session_id,
                commands,
                "run-session: install commands completed"
            );
            None
        }
        Verdict::Failed {
            index,
            exit_code,
            timed_out,
            stderr_tail,
            ..
        } => {
            tracing::error!(
                session_id = %session_id,
                index,
                exit_code,
                timed_out,
                stderr_tail = %stderr_tail,
                "session setup: install command failed; failing the session before the agent starts"
            );
            Some(ExitCode::FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn user_env_files_are_injected_into_env_profile_with_reserved_keys_skipped() {
        let dir = tempfile::tempdir().expect("creds dir");
        std::fs::write(dir.path().join("userenv.FOO"), "foo-val\n").expect("write FOO");
        std::fs::write(dir.path().join("userenv.BAR"), "bar-val\n").expect("write BAR");
        // An `FKST_`-prefixed key the engine would strip: it must be skipped here.
        std::fs::write(dir.path().join("userenv.FKST_X"), "nope\n").expect("write FKST_X");
        // A collision with the LLM credential key must be skipped too (never shadow).
        std::fs::write(
            dir.path().join(format!("userenv.{LLM_ENV_KEY}")),
            "stolen\n",
        )
        .expect("write llm collision");
        // A non-user credential file must be ignored entirely (not injected).
        std::fs::write(dir.path().join("github-token"), "ghs_x\n").expect("write token");
        let creds = CredsLayout::new(dir.path());

        let mut env_profile: BTreeMap<String, SecretString> = BTreeMap::new();
        env_profile.insert(LLM_ENV_KEY.to_string(), SecretString::from("real-llm-key"));
        inject_user_env(&creds, "sess-1", &mut env_profile);

        assert_eq!(env_profile.get("FOO").unwrap().expose_secret(), "foo-val");
        assert_eq!(env_profile.get("BAR").unwrap().expose_secret(), "bar-val");
        // The reserved `FKST_X` is dropped, and the real LLM key is never overwritten.
        assert!(!env_profile.contains_key("FKST_X"));
        assert_eq!(
            env_profile.get(LLM_ENV_KEY).unwrap().expose_secret(),
            "real-llm-key"
        );
        // Exactly the LLM key + the two accepted user vars.
        assert_eq!(env_profile.len(), 3);
    }
}
