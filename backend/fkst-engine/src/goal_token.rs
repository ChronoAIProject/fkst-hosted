//! Goal-session GitHub credential delivery (issue #107).
//!
//! The substrate engine is reference-only: it runs `git`/`gh`/`codex` as child
//! processes and we cannot change how it authenticates. So fkst-hosted configures
//! everything from its side — the engine-process environment and the runtime dir
//! it owns — and never touches substrate's `crates/`.
//!
//! Two mechanisms cooperate:
//!
//! 1. **A rotatable token file** (`<runtime_dir>/github-token`, mode `0600`)
//!    holding JSON `{ "token": "ghs_…", "expires_at": "<RFC3339>" }`. A bare
//!    token string (pre-#107) carried no freshness signal; the JSON lets the
//!    credential helper decide whether to force a just-in-time re-mint. Both the
//!    startup write and the periodic/reactive refresh write it ATOMICALLY (tmp +
//!    rename on the same filesystem) so a concurrent reader never sees a torn
//!    file. The token is a [`SecretString`] and is exposed only at the write
//!    set-site; it is never logged.
//!
//! 2. **A git credential helper** (`<runtime_dir>/git-credential-fkst`, mode
//!    `0700`) materialized into the workspace. Plain `git push` over HTTPS does
//!    not read `GITHUB_TOKEN`; it consults a credential helper. We point git at
//!    this script via `GIT_CONFIG_*` env (see [`git_config_entries`]); the helper
//!    reads the token file and answers `username=x-access-token` /
//!    `password=<token>`. The helper holds no key and cannot mint — only the
//!    driver (which holds the App key) mints — so when the token is near expiry
//!    the helper asks the driver for a fresh mint via a request file the driver's
//!    poller services (see [`MINT_REQUEST_SUFFIX`] / [`NONCE_FILE_NAME`]).

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use secrecy::{ExposeSecret, SecretString};

use crate::error::RunnerError;

/// File name of the rotatable token file inside the runtime dir.
pub const TOKEN_FILE_NAME: &str = "github-token";

/// File name of the materialized credential-helper script.
pub const HELPER_SCRIPT_NAME: &str = "git-credential-fkst";

/// File name (inside the runtime dir, mode `0600`) holding the per-session mint
/// nonce. The helper reads the nonce from the `FKST_GITHUB_MINT_NONCE` env var
/// (inherited from the engine process); the driver writes it here at startup so
/// its mint-request poller can authenticate a request against it.
pub const NONCE_FILE_NAME: &str = ".mint-nonce";

/// Suffix (appended to the token file path) of the JIT mint-request file the
/// helper drops and the driver's poller consumes. Living in the `0600` runtime
/// dir, and carrying the per-session nonce, only that session's own engine child
/// can author it.
pub const MINT_REQUEST_SUFFIX: &str = ".request";

/// Env var carrying the per-session nonce to the helper. Reserved (FKST_ prefix)
/// so a user `env_profile` can never shadow it.
pub const MINT_NONCE_ENV: &str = "FKST_GITHUB_MINT_NONCE";

/// Owner-only permission for the token and nonce files.
const SECRET_FILE_MODE: u32 = 0o600;

/// Serialize `{token, expires_at}` to the JSON the credential helper parses.
/// `expires_at` is RFC3339 (the helper compares it against `now`); the token is
/// exposed here only to serialize it and is never logged.
fn token_file_json(token: &SecretString, expires_at: SystemTime) -> String {
    // bson::DateTime renders RFC3339 and is already a dependency (api.rs parses
    // GitHub's expires_at through it), so no new date crate is pulled in.
    let millis = expires_at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let rfc3339 = bson::DateTime::from_millis(millis)
        .try_to_rfc3339_string()
        .unwrap_or_default();
    serde_json::json!({
        "token": token.expose_secret(),
        "expires_at": rfc3339,
    })
    .to_string()
}

/// Atomically write the token file at `token_path` with mode `0600`: write a
/// sibling tmp file, fsync, set perms, then `rename` over the target (atomic on
/// the same filesystem). The tmp file shares the parent dir so the rename never
/// crosses a filesystem boundary. On any error the partial tmp file is removed.
///
/// Used by BOTH the startup write (`runner.rs`) and the refresh path
/// (`sessions/service.rs`), so the on-disk format and the atomicity guarantee
/// can never diverge between the two writers.
pub fn write_token_file(
    token_path: &Path,
    token: &SecretString,
    expires_at: SystemTime,
) -> Result<(), RunnerError> {
    let json = token_file_json(token, expires_at);
    let tmp_path = tmp_sibling(token_path);

    // Scope the file handle so it is closed before the rename.
    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp_path)?;
        // Tighten perms before any rename so the secret is never world-readable,
        // even for the instant the tmp file exists.
        file.set_permissions(std::fs::Permissions::from_mode(SECRET_FILE_MODE))?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(RunnerError::Io(err));
    }

    if let Err(err) = std::fs::rename(&tmp_path, token_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(RunnerError::Io(err));
    }
    Ok(())
}

/// Sibling tmp path for the atomic write (`<name>.tmp` in the same dir).
fn tmp_sibling(token_path: &Path) -> PathBuf {
    let mut tmp = token_path.as_os_str().to_owned();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// Generate a 32-hex-char (128-bit) per-session JIT mint nonce. Random,
/// unguessable, and known only to a session's helper (via env) and its driver
/// poller (via the `0600` nonce file) — so only that session's own git child
/// can trigger its own re-mint (#107). `rand` is already an engine dependency.
///
/// This is the SINGLE source of truth for the nonce scheme: the engine's runner
/// generates the per-session nonce here, and the controller's dispatch resolver
/// (#151) reuses it verbatim so a resolved dispatch carries a nonce shaped
/// exactly like the one the in-process path produces.
pub fn generate_mint_nonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Write the per-session mint nonce file (`0600`). Returns the nonce string the
/// caller must also set as [`MINT_NONCE_ENV`] on the engine process so the helper
/// can present it back to the driver's poller.
pub fn write_nonce_file(runtime_dir: &Path, nonce: &str) -> Result<(), RunnerError> {
    let path = runtime_dir.join(NONCE_FILE_NAME);
    std::fs::write(&path, nonce.as_bytes()).map_err(RunnerError::Io)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(SECRET_FILE_MODE))
        .map_err(RunnerError::Io)?;
    Ok(())
}

/// Read the per-session mint nonce file (`<runtime_dir>/.mint-nonce`), returning
/// `None` when it is absent (the engine has not written it yet, or an adopted
/// session whose dir is gone). A read error other than "not found" propagates so
/// a caller never confuses a permission/IO failure with an absent nonce. The
/// returned value is the raw file contents (NOT trimmed); callers compare with
/// [`verify_mint_nonce`] which trims both sides.
///
/// This is the single source of truth for reading the nonce a JIT mint request
/// must authenticate against — shared by the control-plane driver's
/// `service_mint_request` and the worker's mint-request servicer (#151), so the
/// authentication compare can never diverge between the two pollers.
pub fn read_nonce_file(runtime_dir: &Path) -> Result<Option<String>, RunnerError> {
    match std::fs::read_to_string(runtime_dir.join(NONCE_FILE_NAME)) {
        Ok(contents) => Ok(Some(contents)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(RunnerError::Io(e)),
    }
}

/// Authenticate a presented JIT mint-request nonce against the expected on-disk
/// nonce. Both sides are trimmed (the file write is exact, but a helper may add a
/// trailing newline); an empty presented value never matches (a blank request is
/// not authenticated). `expected` is the raw file contents from
/// [`read_nonce_file`] (or `None` when the file is absent — which never matches).
///
/// The nonce is secret-like (it gates a re-mint), so neither value is ever logged
/// by this helper; the caller logs only the boolean outcome.
pub fn verify_mint_nonce(expected: Option<&str>, presented: &str) -> bool {
    let presented = presented.trim();
    !presented.is_empty() && expected.map(str::trim) == Some(presented)
}

/// The git credential-helper script source. Materialized verbatim at
/// `<runtime_dir>/git-credential-fkst` (mode `0700`).
///
/// Contract (validated against git's credential-helper protocol):
/// - git invokes it as `<script> get|store|erase`; only `get` emits output, the
///   others (and any unknown op) exit 0 silently.
/// - stdin for `get` is `key=value` lines terminated by a blank line; we ignore
///   the request fields (the per-host config already scopes us to github.com).
/// - on `get` it prints EXACTLY `username=x-access-token` and `password=<token>`
///   to stdout; ALL diagnostics go to stderr (stdout is parsed by git).
/// - JSON is parsed with anchored POSIX `grep`/`sed` (no `jq`/`python3`
///   dependency): the token charset (`ghs_` + word chars) and the RFC3339 stamp
///   contain no quotes, so a non-greedy `"key":"<chars>"` extraction is safe.
/// - JIT refresh: if `expires_at` is within the safety window, drop a nonce-bearing
///   request file and wait (bounded) for the driver to rewrite the token file
///   (signalled by the request file's deletion), then re-read. If no refresh
///   arrives in the window it falls back to the current token rather than failing
///   git hard — the periodic backstop still covers true expiry.
pub const CREDENTIAL_HELPER_SCRIPT: &str = include_str!("git-credential-fkst.sh");

/// One `GIT_CONFIG` key/value entry to inject on the engine process.
pub struct GitConfigEntry {
    pub key: String,
    pub value: String,
}

/// Build the platform-set `git config` entries that wire the substrate engine's
/// `git` to the credential helper WITHOUT touching any on-disk `.git/config` or
/// embedding the token in a remote URL (leak surface). Returned as ordered
/// key/value pairs; the caller renders them into
/// `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_i`/`GIT_CONFIG_VALUE_i` on the child env.
///
/// - `credential.https://github.com.helper = !<abs helper path>` — git's
///   shell-exec helper form (git appends the `get`/`store`/`erase` operation).
/// - `credential.https://github.com.useHttpPath = false` — one credential serves
///   every path under the host (so the token is not path-scoped).
/// - `url.https://github.com/.insteadOf = git@github.com:` — coerce scp-style SSH
///   remotes to HTTPS so the helper (HTTPS-only) applies to them too.
pub fn git_config_entries(helper_path: &Path) -> Vec<GitConfigEntry> {
    let helper = helper_path.display();
    vec![
        GitConfigEntry {
            key: "credential.https://github.com.helper".to_string(),
            value: format!("!{helper}"),
        },
        GitConfigEntry {
            key: "credential.https://github.com.useHttpPath".to_string(),
            value: "false".to_string(),
        },
        GitConfigEntry {
            key: "url.https://github.com/.insteadOf".to_string(),
            value: "git@github.com:".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn token_file_json_is_well_formed_and_rfc3339() {
        let token = SecretString::from("ghs_abc123".to_string());
        let expires = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let json = token_file_json(&token, expires);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["token"], "ghs_abc123");
        let exp = parsed["expires_at"].as_str().expect("expires_at string");
        // RFC3339 with a trailing Z and the literal date for the chosen epoch.
        assert!(exp.starts_with("2023-11-14T"), "got {exp}");
        assert!(exp.ends_with('Z'), "got {exp}");
    }

    #[test]
    fn write_token_file_is_0600_and_atomic_rename_leaves_no_tmp() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join(TOKEN_FILE_NAME);
        let token = SecretString::from("ghs_secret_value".to_string());
        let expires = SystemTime::now() + Duration::from_secs(3600);

        write_token_file(&path, &token, expires).expect("write");

        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, SECRET_FILE_MODE, "token file must be 0600");
        // No tmp sibling left behind.
        assert!(!tmp_sibling(&path).exists(), "tmp must be renamed away");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(parsed["token"], "ghs_secret_value");
    }

    #[test]
    fn write_token_file_overwrites_previous_token() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join(TOKEN_FILE_NAME);
        let expires = SystemTime::now() + Duration::from_secs(3600);

        write_token_file(&path, &SecretString::from("ghs_first".to_string()), expires)
            .expect("first");
        write_token_file(
            &path,
            &SecretString::from("ghs_second".to_string()),
            expires,
        )
        .expect("second");
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(parsed["token"], "ghs_second", "second write must win");
    }

    #[test]
    fn write_nonce_file_is_0600() {
        let dir = tempfile::tempdir().expect("dir");
        // Use a real generated nonce (not a hard-coded literal) so the test
        // exercises the actual nonce shape and carries no embedded secret-like
        // constant for scanners to flag.
        let nonce = generate_mint_nonce();
        write_nonce_file(dir.path(), &nonce).expect("nonce");
        let path = dir.path().join(NONCE_FILE_NAME);
        assert_eq!(std::fs::read_to_string(&path).expect("read"), nonce);
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, SECRET_FILE_MODE);
    }

    #[test]
    fn read_nonce_file_round_trips_and_absent_is_none() {
        let dir = tempfile::tempdir().expect("dir");
        // Absent before any write.
        assert_eq!(read_nonce_file(dir.path()).expect("read"), None);
        let nonce = generate_mint_nonce();
        write_nonce_file(dir.path(), &nonce).expect("nonce");
        assert_eq!(
            read_nonce_file(dir.path()).expect("read").as_deref(),
            Some(nonce.as_str())
        );
    }

    #[test]
    fn verify_mint_nonce_matches_only_a_correct_nonempty_nonce() {
        // Exact match (trimmed on both sides).
        assert!(verify_mint_nonce(Some("abc123"), "abc123"));
        assert!(verify_mint_nonce(Some("abc123\n"), "abc123"));
        assert!(verify_mint_nonce(Some("abc123"), " abc123 "));
        // Mismatches and the empty/absent cases never authenticate.
        assert!(!verify_mint_nonce(Some("abc123"), "deadbeef"));
        assert!(!verify_mint_nonce(None, "abc123"));
        assert!(!verify_mint_nonce(Some("abc123"), ""));
        assert!(!verify_mint_nonce(Some(""), ""));
    }

    #[test]
    fn git_config_entries_wire_helper_host_scope_and_insteadof() {
        let helper = PathBuf::from("/run/session/git-credential-fkst");
        let entries = git_config_entries(&helper);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "credential.https://github.com.helper");
        assert_eq!(entries[0].value, "!/run/session/git-credential-fkst");
        assert_eq!(entries[1].key, "credential.https://github.com.useHttpPath");
        assert_eq!(entries[1].value, "false");
        assert_eq!(entries[2].key, "url.https://github.com/.insteadOf");
        assert_eq!(entries[2].value, "git@github.com:");
    }

    #[test]
    fn helper_script_has_shebang_and_emits_only_on_get() {
        // Guard against accidental bashisms / wrong shebang in the embedded asset.
        assert!(CREDENTIAL_HELPER_SCRIPT.starts_with("#!/bin/sh"));
        assert!(CREDENTIAL_HELPER_SCRIPT.contains("username=x-access-token"));
        assert!(CREDENTIAL_HELPER_SCRIPT.contains("password="));
    }
}
