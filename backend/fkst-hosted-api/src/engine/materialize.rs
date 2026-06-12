//! Package materialization: a [`PreparedPackage`] file array becomes the
//! on-disk plain-directory tree the engine consumes.
//!
//! Per the issue #17 empirical contract the materialized package is a PLAIN
//! directory — no `git init`, no commit, no git identity (spike Q2). The
//! 2-key `fkst.env` is always written defensively (spike Q1): the engine does
//! not need it to start, but packages that call the candidate-git SDK resolve
//! the two HostFacts lazily at runtime.
//!
//! Logging discipline: paths, keys, and counts only — file CONTENT and
//! HostFact VALUES are never logged.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::engine::error::RunnerError;
use crate::packages::model::PackageFile;
use crate::packages::{is_valid_name, Package};

/// Host-owned files at the package root that a package may never supply:
/// `composed.deps` is rendered from `composed_deps` (an empty list means NO
/// file, so a package-supplied one would land verbatim) and `fkst.env` is
/// written by [`write_fkst_env`] with the configured HostFacts.
const RESERVED_HOST_PATHS: [&str; 2] = ["composed.deps", "fkst.env"];

/// Validated runner input, derived from the stored package document. The
/// runner stays Mongo-agnostic: the caller loads the [`Package`] and converts
/// it (the `From` impl below) or builds one by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPackage {
    /// Package identity (`[A-Za-z0-9_-]+`), used in the temp-dir prefix.
    pub package_name: String,
    /// Files written verbatim under the package root. Duplicate paths are
    /// last-writer-wins (documented; upstream package validation already
    /// rejects duplicates at the API edge).
    pub files: Vec<PackageFile>,
    /// Rendered into `composed.deps` (one per line); no file when empty.
    pub composed_deps: Vec<String>,
}

impl From<Package> for PreparedPackage {
    fn from(package: Package) -> Self {
        Self {
            package_name: package.name,
            files: package.files,
            composed_deps: package.composed_deps,
        }
    }
}

impl PreparedPackage {
    /// Structural validation, pure (no filesystem access, no temp dir).
    ///
    /// Rejects with [`RunnerError::InvalidPackage`] when:
    /// - `package_name` or any `composed_deps` entry fails `[A-Za-z0-9_-]+`,
    /// - `files` is empty,
    /// - no entry matches `departments/<name>/main.lua` (the engine's
    ///   conformance pre-flight requires at least one department; a
    ///   raiser-only package fails it with exit 1 — spike Q4),
    /// - any file is pathed exactly at a [`RESERVED_HOST_PATHS`] entry
    ///   (`composed.deps` / `fkst.env` are host-owned).
    ///
    /// Per-path traversal safety is enforced separately by [`safe_join`]
    /// during materialization (defense in depth).
    pub fn validate(&self) -> Result<(), RunnerError> {
        if !is_valid_name(&self.package_name) {
            return Err(RunnerError::InvalidPackage(
                "invalid package name: must fully match [A-Za-z0-9_-]+".to_string(),
            ));
        }
        if self.files.is_empty() {
            return Err(RunnerError::InvalidPackage(
                "package has no files".to_string(),
            ));
        }
        if !self.files.iter().any(|file| is_department_main(&file.path)) {
            return Err(RunnerError::InvalidPackage(
                "no department entry file: need departments/<name>/main.lua".to_string(),
            ));
        }
        for file in &self.files {
            if RESERVED_HOST_PATHS.contains(&file.path.as_str()) {
                return Err(RunnerError::InvalidPackage(format!(
                    "reserved host-owned path: {:?}",
                    file.path
                )));
            }
        }
        for dep in &self.composed_deps {
            if !is_valid_name(dep) {
                return Err(RunnerError::InvalidPackage(format!(
                    "invalid composed_dep {dep:?}: must fully match [A-Za-z0-9_-]+"
                )));
            }
        }
        Ok(())
    }
}

/// True for an anchored `departments/<name>/main.lua` engine entry.
fn is_department_main(path: &str) -> bool {
    let mut parts = path.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some("departments"), Some(name), Some("main.lua"), None)
            if !name.is_empty() && is_valid_name(name)
    )
}

/// Join `rel` onto `root`, rejecting every escape vector:
/// absolute paths, backslashes, control characters, `.`/`..`/empty segments,
/// and symlink escapes (the deepest existing ancestor of the target must
/// canonicalize to a path still inside the canonicalized `root`).
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, RunnerError> {
    if rel.is_empty() {
        return Err(RunnerError::InvalidPackage("empty file path".to_string()));
    }
    if rel.starts_with('/') || Path::new(rel).is_absolute() {
        return Err(RunnerError::InvalidPackage(format!(
            "absolute path not allowed: {rel:?}"
        )));
    }
    if rel.contains('\\') {
        return Err(RunnerError::InvalidPackage(format!(
            "invalid path separator: backslash in {rel:?}"
        )));
    }
    if rel.chars().any(char::is_control) {
        return Err(RunnerError::InvalidPackage(format!(
            "invalid character in path: control character in {rel:?}"
        )));
    }
    if rel
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(RunnerError::InvalidPackage(format!(
            "unsafe path component in {rel:?}"
        )));
    }

    let joined = root.join(rel);

    // Symlink-escape guard: canonicalize the deepest EXISTING ancestor of the
    // target and assert it is still prefixed by the canonicalized root. A
    // symlink planted inside the root that points outside is caught here
    // because the symlink itself is the deepest existing ancestor.
    let canonical_root = root.canonicalize().map_err(RunnerError::Io)?;
    let mut probe = joined.as_path();
    let canonical_ancestor = loop {
        match probe.symlink_metadata() {
            Ok(_) => break probe.canonicalize().map_err(RunnerError::Io)?,
            Err(_) => match probe.parent() {
                Some(parent) => probe = parent,
                None => break canonical_root.clone(),
            },
        }
    };
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(RunnerError::InvalidPackage(format!(
            "path escapes package root: {rel:?}"
        )));
    }

    Ok(joined)
}

/// Materialize the package into a fresh `fkst-pkg-<name>-*` temp dir under
/// `base`: every file written verbatim (UTF-8 bytes, no normalization), plus
/// `composed.deps` when `composed_deps` is non-empty.
///
/// On ANY error the returned `TempDir`-in-progress is dropped, which removes
/// the directory — no partial tree is ever leaked.
pub fn materialize_package(pkg: &PreparedPackage, base: &Path) -> Result<TempDir, RunnerError> {
    pkg.validate()?;

    let dir = tempfile::Builder::new()
        .prefix(&format!("fkst-pkg-{}-", pkg.package_name))
        .tempdir_in(base)
        .map_err(RunnerError::Io)?;

    tracing::info!(
        package_name = %pkg.package_name,
        file_count = pkg.files.len(),
        dir = %dir.path().display(),
        "session.prepare"
    );

    for file in &pkg.files {
        let target = safe_join(dir.path(), &file.path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(RunnerError::Io)?;
        }
        fs::write(&target, file.content.as_bytes()).map_err(RunnerError::Io)?;
        tracing::debug!(
            package_name = %pkg.package_name,
            path = %file.path,
            bytes = file.content.len(),
            "session.prepare.file"
        );
    }

    if !pkg.composed_deps.is_empty() {
        let mut rendered = String::new();
        for dep in &pkg.composed_deps {
            rendered.push_str(dep);
            rendered.push('\n');
        }
        fs::write(dir.path().join("composed.deps"), rendered).map_err(RunnerError::Io)?;
        tracing::debug!(
            package_name = %pkg.package_name,
            dep_count = pkg.composed_deps.len(),
            "session.prepare.composed_deps"
        );
    }

    Ok(dir)
}

/// Write the defensive 2-key `fkst.env` at the package root: exactly two
/// `KEY=VALUE\n` lines in sorted key order. Values are written verbatim
/// (engine contract: plain `KEY=VALUE`, no quoting) and never logged.
pub fn write_fkst_env(
    pkg_root: &Path,
    candidate_prefix: &str,
    candidate_from_sep: &str,
) -> Result<(), RunnerError> {
    // Sorted: FKST_CANDIDATE_FROM_SEP < FKST_CANDIDATE_PREFIX.
    let rendered = format!(
        "FKST_CANDIDATE_FROM_SEP={candidate_from_sep}\nFKST_CANDIDATE_PREFIX={candidate_prefix}\n"
    );
    fs::write(pkg_root.join("fkst.env"), rendered).map_err(RunnerError::Io)?;
    tracing::debug!(
        keys = "FKST_CANDIDATE_FROM_SEP,FKST_CANDIDATE_PREFIX",
        "session.prepare.fkst_env"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> PackageFile {
        PackageFile {
            path: path.to_string(),
            content: content.to_string(),
        }
    }

    fn minimal() -> PreparedPackage {
        PreparedPackage {
            package_name: "demo".to_string(),
            files: vec![
                file("departments/hello/main.lua", "local M = {}\nreturn M\n"),
                file("raisers/tick.lua", "return {}\n"),
            ],
            composed_deps: Vec::new(),
        }
    }

    fn base_dir() -> TempDir {
        tempfile::tempdir().expect("test base dir")
    }

    fn count_entries(base: &Path) -> usize {
        fs::read_dir(base).expect("read base").count()
    }

    // ---- PreparedPackage::validate ------------------------------------------

    #[test]
    fn validate_accepts_the_minimal_package() {
        assert!(minimal().validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_package_name() {
        for name in ["", "a b", "a/b", "a.b", "../x"] {
            let mut pkg = minimal();
            pkg.package_name = name.to_string();
            let err = pkg.validate().expect_err("bad name must be rejected");
            assert!(matches!(err, RunnerError::InvalidPackage(_)), "{name:?}");
        }
    }

    #[test]
    fn validate_rejects_empty_files() {
        let mut pkg = minimal();
        pkg.files.clear();
        let err = pkg.validate().expect_err("empty files must be rejected");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert!(err.to_string().contains("no files"));
    }

    #[test]
    fn validate_rejects_missing_department_main() {
        // Raiser-only: the engine's conformance would fail it with exit 1
        // (spike Q4) — the runner rejects it before any filesystem work.
        let pkg = PreparedPackage {
            package_name: "demo".to_string(),
            files: vec![file("raisers/tick.lua", "return {}\n")],
            composed_deps: Vec::new(),
        };
        let err = pkg.validate().expect_err("raiser-only must be rejected");
        assert!(err.to_string().contains("department entry"));
    }

    #[test]
    fn validate_requires_anchored_department_main() {
        // A nested or unanchored match must not count.
        for path in [
            "evil/departments/x/main.lua",
            "departments/x/y/main.lua",
            "departments/main.lua",
            "departments/x.y/main.lua",
        ] {
            let pkg = PreparedPackage {
                package_name: "demo".to_string(),
                files: vec![file(path, "x")],
                composed_deps: Vec::new(),
            };
            assert!(pkg.validate().is_err(), "must reject {path:?}");
        }
    }

    #[test]
    fn validate_rejects_package_supplied_composed_deps_file() {
        // With an empty composed_deps list the host writes NO composed.deps,
        // so a package-supplied one would land verbatim — forbidden.
        let mut pkg = minimal();
        pkg.files.push(file("composed.deps", "evil-dep\n"));
        let err = pkg.validate().expect_err("composed.deps must be rejected");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert!(
            err.to_string().contains("reserved host-owned path"),
            "{err}"
        );
    }

    #[test]
    fn validate_rejects_package_supplied_fkst_env_file() {
        let mut pkg = minimal();
        pkg.files
            .push(file("fkst.env", "FKST_CANDIDATE_PREFIX=evil/\n"));
        let err = pkg.validate().expect_err("fkst.env must be rejected");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert!(
            err.to_string().contains("reserved host-owned path"),
            "{err}"
        );
    }

    #[test]
    fn validate_allows_reserved_names_in_subdirectories() {
        // Only the exact root paths are host-owned; nested files of the same
        // name are ordinary package content.
        let mut pkg = minimal();
        pkg.files.push(file("departments/hello/fkst.env", "x"));
        pkg.files.push(file("config/composed.deps", "y"));
        assert!(pkg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_composed_dep_names() {
        for dep in ["", "a b", "a/b", "a\nb", "../x"] {
            let mut pkg = minimal();
            pkg.composed_deps = vec![dep.to_string()];
            let err = pkg.validate().expect_err("bad dep must be rejected");
            assert!(matches!(err, RunnerError::InvalidPackage(_)), "{dep:?}");
        }
    }

    #[test]
    fn validate_is_pure_and_creates_nothing() {
        let base = base_dir();
        let mut pkg = minimal();
        pkg.files.clear();
        let _ = pkg.validate();
        assert_eq!(count_entries(base.path()), 0);
    }

    // ---- From<Package> -------------------------------------------------------

    #[test]
    fn prepared_package_derives_from_stored_package() {
        let stored = Package {
            name: "demo".to_string(),
            files: vec![file("departments/hello/main.lua", "x")],
            composed_deps: vec!["dep-a".to_string()],
            owner_user_id: None,
            org_id: None,
            created_at: bson::DateTime::from_millis(0),
            updated_at: bson::DateTime::from_millis(0),
        };
        let prepared = PreparedPackage::from(stored);
        assert_eq!(prepared.package_name, "demo");
        assert_eq!(prepared.files.len(), 1);
        assert_eq!(prepared.composed_deps, vec!["dep-a".to_string()]);
    }

    // ---- safe_join -----------------------------------------------------------

    #[test]
    fn safe_join_accepts_nested_relative_paths() {
        let base = base_dir();
        let joined = safe_join(base.path(), "departments/hello/main.lua").expect("safe path");
        assert!(joined.ends_with("departments/hello/main.lua"));
    }

    #[test]
    fn safe_join_rejects_traversal_and_absolute_paths() {
        let base = base_dir();
        for rel in [
            "../escape.lua",
            "a/../b.lua",
            "..",
            "/etc/passwd",
            "a//b.lua",
            "./x.lua",
            "a/",
            "",
            "dir\\file.lua",
            "a\u{0}b.lua",
        ] {
            let err = safe_join(base.path(), rel).expect_err(rel);
            assert!(matches!(err, RunnerError::InvalidPackage(_)), "{rel:?}");
        }
    }

    #[test]
    fn safe_join_rejects_symlink_escape() {
        let base = base_dir();
        let outside = base_dir();
        // Plant a symlink inside the root that points outside it.
        std::os::unix::fs::symlink(outside.path(), base.path().join("link"))
            .expect("plant symlink");
        let err = safe_join(base.path(), "link/evil.lua").expect_err("symlink escape");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert!(err.to_string().contains("escapes package root"));
    }

    #[test]
    fn safe_join_allows_symlink_inside_root() {
        let base = base_dir();
        fs::create_dir(base.path().join("real")).expect("mkdir");
        std::os::unix::fs::symlink(base.path().join("real"), base.path().join("alias"))
            .expect("plant internal symlink");
        let joined = safe_join(base.path(), "alias/ok.lua").expect("internal symlink is fine");
        assert!(joined.ends_with("alias/ok.lua"));
    }

    // ---- materialize_package ---------------------------------------------------

    #[test]
    fn materialize_writes_the_exact_tree() {
        let base = base_dir();
        let pkg = PreparedPackage {
            package_name: "demo".to_string(),
            files: vec![
                file("departments/hello/main.lua", "local M = {}\nreturn M\n"),
                file("raisers/tick.lua", "return {}\n"),
                file("core.lua", "-- shared\n"),
            ],
            composed_deps: Vec::new(),
        };
        let dir = materialize_package(&pkg, base.path()).expect("materialize");

        let name = dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .expect("dir name");
        assert!(name.starts_with("fkst-pkg-demo-"), "got {name:?}");

        assert_eq!(
            fs::read_to_string(dir.path().join("departments/hello/main.lua")).unwrap(),
            "local M = {}\nreturn M\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("raisers/tick.lua")).unwrap(),
            "return {}\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("core.lua")).unwrap(),
            "-- shared\n"
        );
        // Empty composed_deps => NO composed.deps file.
        assert!(!dir.path().join("composed.deps").exists());
    }

    #[test]
    fn materialize_writes_content_verbatim_without_newline_injection() {
        let base = base_dir();
        let mut pkg = minimal();
        pkg.files.push(file("no-trailing-newline.lua", "return 42"));
        let dir = materialize_package(&pkg, base.path()).expect("materialize");
        assert_eq!(
            fs::read(dir.path().join("no-trailing-newline.lua")).unwrap(),
            b"return 42"
        );
    }

    #[test]
    fn composed_deps_file_is_byte_exact() {
        let base = base_dir();
        let mut pkg = minimal();
        pkg.composed_deps = vec!["other-pkg".to_string(), "third_pkg".to_string()];
        let dir = materialize_package(&pkg, base.path()).expect("materialize");
        assert_eq!(
            fs::read(dir.path().join("composed.deps")).unwrap(),
            b"other-pkg\nthird_pkg\n"
        );
    }

    #[test]
    fn duplicate_paths_are_last_writer_wins() {
        let base = base_dir();
        let mut pkg = minimal();
        pkg.files.push(file("dup.lua", "first"));
        pkg.files.push(file("dup.lua", "second"));
        let dir = materialize_package(&pkg, base.path()).expect("materialize");
        assert_eq!(
            fs::read_to_string(dir.path().join("dup.lua")).unwrap(),
            "second"
        );
    }

    #[test]
    fn validate_failure_creates_no_temp_dir() {
        let base = base_dir();
        let mut pkg = minimal();
        pkg.files.clear();
        assert!(materialize_package(&pkg, base.path()).is_err());
        assert_eq!(count_entries(base.path()), 0, "no fkst-pkg-* may remain");
    }

    #[test]
    fn traversal_failure_mid_write_leaks_no_temp_dir() {
        let base = base_dir();
        let mut pkg = minimal();
        pkg.files.push(file("a/../escape.lua", "x"));
        let err = materialize_package(&pkg, base.path()).expect_err("traversal must fail");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert_eq!(
            count_entries(base.path()),
            0,
            "partial fkst-pkg-* must be cleaned on error"
        );
    }

    // ---- write_fkst_env ---------------------------------------------------------

    #[test]
    fn fkst_env_is_byte_exact_and_sorted() {
        let base = base_dir();
        write_fkst_env(base.path(), "candidate/", "::").expect("write fkst.env");
        assert_eq!(
            fs::read(base.path().join("fkst.env")).unwrap(),
            b"FKST_CANDIDATE_FROM_SEP=::\nFKST_CANDIDATE_PREFIX=candidate/\n"
        );
    }

    #[test]
    fn fkst_env_writes_configured_values_verbatim() {
        let base = base_dir();
        write_fkst_env(base.path(), "cand/", "--").expect("write fkst.env");
        assert_eq!(
            fs::read(base.path().join("fkst.env")).unwrap(),
            b"FKST_CANDIDATE_FROM_SEP=--\nFKST_CANDIDATE_PREFIX=cand/\n"
        );
    }
}
