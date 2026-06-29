//! Repo-scoped package loading (issue #115): clone the goal's GitHub repo into
//! a per-session project root and resolve its `.fkst/packages/<name>/` package
//! directories — the source of truth that replaces the old Mongo package store.
//!
//! Secret handling: the GitHub App installation token is NEVER placed in the
//! clone argv (`/proc/<pid>/cmdline` is readable by same-uid processes) nor in
//! the resulting `.git/config` remote URL. Instead we reuse the issue-#107
//! credential-helper machinery — a `0600` token file plus the
//! `git-credential-fkst` script wired in via `GIT_CONFIG_*` env — exactly as the
//! in-session engine authenticates. The token is exposed only to write the
//! `0600` file and is never logged.
//!
//! Safety: a cloned repo is UNTRUSTED user content. Each resolved package dir
//! must (a) have a basename that fully matches `[A-Za-z0-9_-]+` and is not the
//! reserved `host`, (b) be unique within the session, and (c) canonicalize to a
//! path still strictly under the canonicalized `.fkst/packages/` root — closing
//! the symlink-escape vector before the dir is read host-side or handed to the
//! engine.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use secrecy::SecretString;
use tempfile::TempDir;
use tokio::process::Command;

use crate::engine::error::RunnerError;
use crate::engine::goal_token::{git_config_entries, write_token_file, HELPER_SCRIPT_NAME};
use crate::engine::materialize::materialize_helper_script;
use crate::engine::util::is_valid_name;
use crate::models::RepoRef;

/// Reserved package basename the substrate engine owns; a repo package may never
/// claim it.
///
/// `pub` so submit-time pre-flight (#179) can reject the reserved name BY
/// REFERENCE to this single source of truth instead of duplicating the literal.
/// Pure re-export: the value and every use site here are unchanged.
pub const RESERVED_PACKAGE_NAME: &str = "host";

/// Relative directory, inside a cloned repo, that holds the repo-scoped packages.
const PACKAGES_SUBDIR: &str = ".fkst/packages";

/// A cloned repo plus its resolved package roots, ready to hand to the engine.
///
/// The [`TempDir`] guards are held by the caller for the session's lifetime: the
/// clone dir and the transient credential dir are both removed on drop (RAII),
/// so neither the working tree nor the `0600` token file outlives the session.
pub struct ClonedRepo {
    /// `--project-root`: the canonicalized repo working-tree root.
    pub project_root: PathBuf,
    /// One canonicalized `--package-root` per requested package, in request
    /// order. Each is `<project_root>/.fkst/packages/<name>` and is guaranteed to
    /// exist and to be safely contained.
    pub package_roots: Vec<PathBuf>,
    /// Held for the session lifetime; dropping it removes the clone working tree.
    _clone_guard: TempDir,
    /// Held for the session lifetime; dropping it removes the transient
    /// credential dir (the `0600` token file + helper script used only to clone).
    _credential_guard: TempDir,
}

/// Clone `repo` into a fresh temp project root under `base`, authenticating with
/// `token` via the credential helper, then resolve the requested
/// `<project_root>/.fkst/packages/<name>` package dirs.
///
/// Fails with [`RunnerError::InvalidPackage`] for an absent/unsafe/ill-named
/// package dir and with [`RunnerError::CloneFailed`] when git itself fails.
pub async fn clone_repo_packages(
    base: &Path,
    repo: &RepoRef,
    token: &SecretString,
    package_names: &[String],
    framework_bin_for_helper: &Path,
) -> Result<ClonedRepo, RunnerError> {
    // Transient credential dir (token file + helper), separate from the clone so
    // the secret never lands inside the working tree git might commit/push.
    let credential_guard = tempfile::Builder::new()
        .prefix("fkst-clone-cred-")
        .tempdir_in(base)
        .map_err(RunnerError::Io)?;
    let token_path = credential_guard.path().join("clone-token");
    // The credential helper reads `{token, expires_at}` from this 0600 file; the
    // expiry is unused for a one-shot clone (set far enough out that the helper's
    // JIT-refresh window never trips), so a fixed +1h is fine here.
    write_token_file(
        &token_path,
        token,
        std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
    )?;
    let helper_path = materialize_helper_script(credential_guard.path())?;

    // Fresh clone destination. `tempdir_in` makes an empty dir; `git clone <url>
    // <dir>` clones into an existing empty target.
    let clone_guard = tempfile::Builder::new()
        .prefix("fkst-repo-")
        .tempdir_in(base)
        .map_err(RunnerError::Io)?;

    run_clone(repo, &clone_guard, &helper_path, &token_path).await?;

    let project_root = clone_guard.path().canonicalize().map_err(RunnerError::Io)?;
    let package_roots = resolve_package_roots(&project_root, package_names)?;

    tracing::info!(
        owner = %repo.owner,
        name = %repo.name,
        package_count = package_roots.len(),
        "session.clone: repo cloned and packages resolved"
    );

    // `framework_bin_for_helper` is reserved for a future helper that may need
    // the engine binary; today the helper script is self-contained.
    let _ = framework_bin_for_helper;

    Ok(ClonedRepo {
        project_root,
        package_roots,
        _clone_guard: clone_guard,
        _credential_guard: credential_guard,
    })
}

/// Run `git clone` into `dest`, authenticating via the credential helper wired
/// through `GIT_CONFIG_*` (token only in the `0600` file, never in argv or
/// `.git/config`). A shallow single-branch clone is enough — packages are read
/// from the default branch's working tree, never from history.
async fn run_clone(
    repo: &RepoRef,
    dest: &TempDir,
    helper_path: &Path,
    token_path: &Path,
) -> Result<(), RunnerError> {
    // HTTPS URL with NO embedded credential; the helper supplies it.
    let url = format!("https://github.com/{}/{}.git", repo.owner, repo.name);

    let mut command = Command::new("git");
    command
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--single-branch")
        .arg(&url)
        .arg(dest.path())
        // The helper resolves the token from the 0600 file at credential time.
        .env("FKST_GITHUB_TOKEN_FILE", token_path)
        // Belt-and-braces: never let git fall into an interactive prompt that
        // would hang the driver if the helper somehow yields no credential.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Wire the credential helper via GIT_CONFIG_* (same mechanism as the engine
    // child uses) so the token is never on the command line or in .git/config.
    let entries = git_config_entries(helper_path);
    command.env("GIT_CONFIG_COUNT", entries.len().to_string());
    for (i, entry) in entries.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{i}"), &entry.key);
        command.env(format!("GIT_CONFIG_VALUE_{i}"), &entry.value);
    }

    let output = command.output().await.map_err(RunnerError::Spawn)?;
    if !output.status.success() {
        // stderr may carry the auth-failure reason but NEVER the token (it is in
        // the 0600 file, not the argv/url). Surface a bounded, generic message;
        // the full stderr is logged for diagnostics.
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(
            owner = %repo.owner,
            name = %repo.name,
            code = ?output.status.code(),
            stderr = %stderr,
            "session.clone: git clone failed"
        );
        return Err(RunnerError::CloneFailed {
            repo: format!("{}/{}", repo.owner, repo.name),
        });
    }
    Ok(())
}

/// Resolve each requested package name to `<project_root>/.fkst/packages/<name>`,
/// enforcing the name rule, uniqueness, the reserved-name ban, existence, and
/// containment (symlink-escape guard). Order matches `package_names`.
fn resolve_package_roots(
    project_root: &Path,
    package_names: &[String],
) -> Result<Vec<PathBuf>, RunnerError> {
    let packages_root = project_root.join(PACKAGES_SUBDIR);
    // Canonicalize the packages root once; an absent dir means the repo carries
    // no packages, which is a clear, actionable spawn failure.
    let canonical_packages_root = packages_root.canonicalize().map_err(|error| {
        tracing::error!(
            packages_root = %packages_root.display(),
            error = %error,
            "session.clone: repo has no .fkst/packages directory"
        );
        RunnerError::InvalidPackage(format!(
            "repo has no {PACKAGES_SUBDIR}/ directory (cannot resolve packages)"
        ))
    })?;

    let mut seen = HashSet::with_capacity(package_names.len());
    let mut roots = Vec::with_capacity(package_names.len());
    for name in package_names {
        if !is_valid_name(name) {
            return Err(RunnerError::InvalidPackage(format!(
                "invalid package name {name:?}: must fully match [A-Za-z0-9_-]+"
            )));
        }
        if name == RESERVED_PACKAGE_NAME {
            return Err(RunnerError::InvalidPackage(format!(
                "reserved package name not allowed: {RESERVED_PACKAGE_NAME:?}"
            )));
        }
        if !seen.insert(name.as_str()) {
            return Err(RunnerError::InvalidPackage(format!(
                "duplicate package name in session: {name:?}"
            )));
        }

        // `name` passed is_valid_name, so it cannot contain a path separator,
        // `.`/`..`, or NUL — the join stays a single child segment.
        let candidate = canonical_packages_root.join(name);
        let canonical = candidate.canonicalize().map_err(|error| {
            tracing::warn!(
                package = %name,
                path = %candidate.display(),
                error = %error,
                "session.clone: named package directory is absent in the repo"
            );
            RunnerError::InvalidPackage(format!(
                "package not found in repo {PACKAGES_SUBDIR}/: {name:?}"
            ))
        })?;
        // Symlink-escape guard: a symlinked package dir pointing outside the
        // packages root is rejected (the canonical target must stay contained).
        if !canonical.starts_with(&canonical_packages_root) {
            return Err(RunnerError::InvalidPackage(format!(
                "package {name:?} escapes {PACKAGES_SUBDIR}/"
            )));
        }
        if !canonical.is_dir() {
            return Err(RunnerError::InvalidPackage(format!(
                "package {name:?} is not a directory"
            )));
        }
        roots.push(canonical);
    }
    Ok(roots)
}

/// True when `name` is a valid repo-scoped package name (the engine's identity
/// rule `^[A-Za-z0-9_-]+$`).
///
/// Thin `pub` alias over [`crate::engine::util::is_valid_name`], named for submit-time
/// pre-flight (#179) so callers checking a *package* name read against the same
/// predicate `resolve_package_roots` enforces here. Pure re-export, no behavior
/// change: it forwards verbatim to the single name-rule implementation.
pub fn is_valid_package_name(name: &str) -> bool {
    is_valid_name(name)
}

/// The credential-helper script file name, re-exported for callers wiring the
/// engine child's git config to the same helper.
pub const CLONE_HELPER_SCRIPT_NAME: &str = HELPER_SCRIPT_NAME;

/// Read every file under a resolved package root into the canonical
/// [`crate::engine::materialize::PackageFile`] shape (relative `path` + verbatim
/// `content`), for the journaling content fingerprint (#25 redo). Paths are
/// recorded relative to `package_root` with `/` separators (the engine
/// convention). Non-UTF-8 files are decoded lossily — the fingerprint only needs
/// to be a stable function of the bytes, not a faithful round-trip. Errors
/// reading an individual entry are logged and skipped so a single unreadable
/// file never fails the spawn (the engine's conformance is the real gate).
pub fn read_package_files(package_root: &Path) -> Vec<crate::engine::materialize::PackageFile> {
    let mut files = Vec::new();
    collect_files(package_root, package_root, &mut files);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// Depth-first walk collecting regular files as `PackageFile`s. Symlinks are not
/// followed (the resolved root was already containment-checked; following links
/// here would be a needless escape vector for a read).
fn collect_files(base: &Path, dir: &Path, out: &mut Vec<crate::engine::materialize::PackageFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!(dir = %dir.display(), error = %error, "session.clone.fingerprint: read_dir failed (skipped)");
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_files(base, &path, out);
        } else if meta.is_file() {
            let rel = match path.strip_prefix(base) {
                Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            let content = match std::fs::read(&path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(error) => {
                    tracing::warn!(path = %path.display(), error = %error, "session.clone.fingerprint: read failed (skipped)");
                    continue;
                }
            };
            out.push(crate::engine::materialize::PackageFile { path: rel, content });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay down a minimal valid repo tree with the given package basenames under
    /// `.fkst/packages/`, each containing `departments/<name>/main.lua`.
    fn fixture_repo(names: &[&str]) -> TempDir {
        let dir = tempfile::tempdir().expect("repo dir");
        for name in names {
            let pkg = dir.path().join(PACKAGES_SUBDIR).join(name);
            std::fs::create_dir_all(pkg.join("departments").join(name)).expect("mkdir");
            std::fs::write(
                pkg.join("departments").join(name).join("main.lua"),
                "return {}\n",
            )
            .expect("write");
        }
        dir
    }

    #[test]
    fn resolves_present_packages_in_request_order() {
        let repo = fixture_repo(&["alpha", "beta"]);
        let root = repo.path().canonicalize().expect("canon");
        let roots = resolve_package_roots(&root, &["beta".to_string(), "alpha".to_string()])
            .expect("resolve");
        assert_eq!(roots.len(), 2);
        assert!(roots[0].ends_with(".fkst/packages/beta"), "{roots:?}");
        assert!(roots[1].ends_with(".fkst/packages/alpha"), "{roots:?}");
    }

    #[test]
    fn absent_package_fails_with_clear_error() {
        let repo = fixture_repo(&["alpha"]);
        let root = repo.path().canonicalize().expect("canon");
        let err = resolve_package_roots(&root, &["missing".to_string()])
            .expect_err("absent package must fail");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn missing_packages_dir_fails() {
        let repo = tempfile::tempdir().expect("empty repo");
        let root = repo.path().canonicalize().expect("canon");
        let err = resolve_package_roots(&root, &["alpha".to_string()])
            .expect_err("no .fkst/packages must fail");
        assert!(err.to_string().contains(".fkst/packages"), "{err}");
    }

    #[test]
    fn rejects_invalid_name_reserved_and_duplicate() {
        let repo = fixture_repo(&["alpha", "host"]);
        let root = repo.path().canonicalize().expect("canon");
        // Invalid characters.
        assert!(matches!(
            resolve_package_roots(&root, &["a/b".to_string()]),
            Err(RunnerError::InvalidPackage(_))
        ));
        // Reserved 'host' basename, even though the dir exists.
        let reserved = resolve_package_roots(&root, &["host".to_string()])
            .expect_err("reserved name must fail");
        assert!(reserved.to_string().contains("reserved"), "{reserved}");
        // Duplicate in the requested set.
        let dup = resolve_package_roots(&root, &["alpha".to_string(), "alpha".to_string()])
            .expect_err("duplicate must fail");
        assert!(dup.to_string().contains("duplicate"), "{dup}");
    }

    #[test]
    fn rejects_symlink_escaping_packages_root() {
        let repo = fixture_repo(&["alpha"]);
        let outside = tempfile::tempdir().expect("outside");
        std::fs::create_dir_all(outside.path().join("departments")).expect("mkdir");
        // Plant a symlink inside .fkst/packages pointing outside the repo.
        let link = repo.path().join(PACKAGES_SUBDIR).join("evil");
        std::os::unix::fs::symlink(outside.path(), &link).expect("symlink");
        let root = repo.path().canonicalize().expect("canon");
        let err = resolve_package_roots(&root, &["evil".to_string()])
            .expect_err("symlink escape must fail");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert!(err.to_string().contains("escapes"), "{err}");
    }
}
