//! Unzip an Ornn skill package into `$CODEX_HOME/skills/<name>/` and append a
//! skillset's master prompt to `$CODEX_HOME/AGENTS.md` (issue #114).
//!
//! `codex` discovers Agent Skills under `$CODEX_HOME/skills/<name>/` (a
//! `SKILL.md` + scripts/references) and auto-reads the global
//! `$CODEX_HOME/AGENTS.md` on every run, so this module is the on-disk seam
//! between Ornn's verbatim package format and the per-session codex (#112).
//!
//! Relocated from the control-plane into `fkst-engine` (issue #151) so BOTH the
//! in-process control-plane driver AND the worker's engine executor share ONE
//! install implementation — the worker links `fkst-engine`, not the
//! control-plane. The semantics are byte-identical to the control-plane version;
//! only the error domain changed (`RunnerError` instead of the control-plane's
//! `AppError`), matching the rest of this crate.
//!
//! Security (load-bearing):
//! - Every zip entry path is validated through [`safe_join`], which rejects
//!   absolute paths, backslashes, control chars, `.`/`..`/empty segments, and
//!   symlink escapes — mirroring `materialize.rs::safe_join` (defense in depth
//!   against a malicious package planted in the registry).
//! - Executable bits are restored from the zip entry's recorded unix mode when
//!   present, else `+x` is granted to `scripts/*` entries and to any file
//!   beginning with a `#!` shebang, so a tool's scripts run after install.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::engine::error::RunnerError;

/// The repo-base reader + the pure AGENTS.md body composer (issue #182). Split
/// into its own module so `skills/mod.rs` stays under the 500-line budget; it
/// reuses this module's [`safe_join`] / [`io_err`] via `super::`.
pub mod agents_md;

pub use agents_md::{compose_agents_md, read_repo_agents_md};

/// Per-entry executable bit added when a zip entry carries no recorded unix
/// mode but is heuristically executable (`scripts/*` or a shebang file).
const EXEC_BITS: u32 = 0o755;

/// Default file mode for a regular (non-executable) installed file.
const REGULAR_BITS: u32 = 0o644;

/// Marker prefix for a skillset instruction block in `AGENTS.md`. The full
/// fences are `<!-- ornn-skillset:<name> BEGIN -->` / `... END -->`.
const MARKER_PREFIX: &str = "ornn-skillset:";

/// Map a host-side filesystem failure to the runner's IO error, attaching a
/// short context message so the cause is traceable in the logs (the underlying
/// `io::Error` already carries the OS detail). Never carries a secret.
pub(super) fn io_err(context: &str, error: std::io::Error) -> RunnerError {
    tracing::error!(error = %error, "{context}");
    RunnerError::Io(error)
}

/// Join `rel` (a zip entry path) onto `root`, rejecting every escape vector.
///
/// A self-contained mirror of `materialize::safe_join`: absolute paths,
/// backslashes, control characters, `.`/`..`/empty segments, and symlink escapes
/// are all rejected. The symlink-escape guard canonicalizes the deepest EXISTING
/// ancestor of the target and asserts it stays inside the canonicalized `root`,
/// so a symlink planted inside the install dir that points outside is caught.
pub(super) fn safe_join(root: &Path, rel: &str) -> Result<PathBuf, RunnerError> {
    if rel.is_empty() {
        return Err(RunnerError::InvalidPackage(
            "empty zip entry path".to_string(),
        ));
    }
    if rel.starts_with('/') || Path::new(rel).is_absolute() {
        return Err(RunnerError::InvalidPackage(format!(
            "absolute path not allowed in package: {rel:?}"
        )));
    }
    if rel.contains('\\') {
        return Err(RunnerError::InvalidPackage(format!(
            "invalid path separator (backslash) in package: {rel:?}"
        )));
    }
    if rel.chars().any(char::is_control) {
        return Err(RunnerError::InvalidPackage(format!(
            "control character in package path: {rel:?}"
        )));
    }
    if rel
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(RunnerError::InvalidPackage(format!(
            "unsafe path component in package: {rel:?}"
        )));
    }

    let joined = root.join(rel);

    let canonical_root = root
        .canonicalize()
        .map_err(|error| io_err("failed to canonicalize skill install root", error))?;
    let mut probe = joined.as_path();
    let canonical_ancestor = loop {
        match probe.symlink_metadata() {
            Ok(_) => match probe.canonicalize() {
                Ok(canon) => break canon,
                Err(error) => {
                    return Err(io_err("failed to canonicalize skill path ancestor", error));
                }
            },
            Err(_) => match probe.parent() {
                Some(parent) => probe = parent,
                None => break canonical_root.clone(),
            },
        }
    };
    if !canonical_ancestor.starts_with(&canonical_root) {
        return Err(RunnerError::InvalidPackage(format!(
            "package path escapes the install root: {rel:?}"
        )));
    }
    Ok(joined)
}

/// Decide the unix mode for an installed file: the zip's recorded mode when
/// present, else `+x` for `scripts/*` or shebang files, else a plain `0644`.
/// Pure so the heuristic is unit-testable without a real zip.
fn resolve_mode(entry_rel: &str, recorded_mode: Option<u32>, content: &[u8]) -> u32 {
    if let Some(mode) = recorded_mode {
        // A recorded mode of 0 means the archiver stored no permissions (e.g.
        // a zip written without unix attributes) — fall through to the
        // heuristic rather than chmod the file to 000 (unreadable).
        if mode & 0o777 != 0 {
            return mode & 0o777;
        }
    }
    let under_scripts = entry_rel.starts_with("scripts/") || entry_rel.contains("/scripts/");
    let has_shebang = content.starts_with(b"#!");
    if under_scripts || has_shebang {
        EXEC_BITS
    } else {
        REGULAR_BITS
    }
}

/// Install one skill package zip into `<codex_home>/skills/<name>/`.
///
/// Reads the in-memory `zip_bytes`, writes each entry verbatim under the skill
/// dir (creating parents), and chmods it via [`resolve_mode`]. Path-traversal
/// is rejected per entry by [`safe_join`]. `name` is assumed already validated
/// at the boundary; it is still safe-joined as a single segment here.
pub fn install_skill(codex_home: &Path, name: &str, zip_bytes: &[u8]) -> Result<(), RunnerError> {
    use std::os::unix::fs::PermissionsExt;

    // `safe_join` canonicalizes its `root`, so the `skills/` parent must exist
    // before we resolve `<name>` against it; create it up front.
    let skills_root = codex_home.join("skills");
    std::fs::create_dir_all(&skills_root).map_err(|error| {
        tracing::error!(skill = %name, error = %error, "failed to create skills root");
        RunnerError::Io(error)
    })?;
    let skill_dir = safe_join(&skills_root, name)?;
    std::fs::create_dir_all(&skill_dir).map_err(|error| {
        tracing::error!(skill = %name, error = %error, "failed to create skill dir");
        RunnerError::Io(error)
    })?;

    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|error| {
        // The error text is structural (bad zip), not secret; but the package
        // bytes themselves are sensitive, so we log only the failure shape.
        tracing::error!(skill = %name, error = %error, "failed to open skill package zip");
        RunnerError::InvalidPackage("malformed skill package archive".to_string())
    })?;

    let mut file_count = 0usize;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            tracing::error!(skill = %name, error = %error, "failed to read zip entry");
            RunnerError::InvalidPackage("malformed skill package archive entry".to_string())
        })?;

        // Prefer the archive's declared safe name; reject entries without one.
        let entry_rel = match entry.enclosed_name() {
            Some(path) => path.to_string_lossy().replace('\\', "/"),
            None => {
                return Err(RunnerError::InvalidPackage(format!(
                    "skill package entry has an unsafe name (index {index})"
                )));
            }
        };

        if entry.is_dir() {
            let dir = safe_join(&skill_dir, entry_rel.trim_end_matches('/'))?;
            std::fs::create_dir_all(&dir).map_err(|error| {
                tracing::error!(skill = %name, error = %error, "failed to create skill subdir");
                RunnerError::Io(error)
            })?;
            continue;
        }

        let target = safe_join(&skill_dir, &entry_rel)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                tracing::error!(skill = %name, error = %error, "failed to create skill parent dir");
                RunnerError::Io(error)
            })?;
        }

        let mut content = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut content).map_err(|error| {
            tracing::error!(skill = %name, error = %error, "failed to read skill entry bytes");
            RunnerError::InvalidPackage("failed to read skill package entry".to_string())
        })?;

        let mode = resolve_mode(&entry_rel, entry.unix_mode(), &content);
        std::fs::write(&target, &content).map_err(|error| {
            tracing::error!(skill = %name, error = %error, "failed to write skill file");
            RunnerError::Io(error)
        })?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode)).map_err(
            |error| {
                tracing::error!(skill = %name, error = %error, "failed to chmod skill file");
                RunnerError::Io(error)
            },
        )?;
        file_count += 1;
    }

    tracing::info!(skill = %name, file_count, "installed ornn skill");
    Ok(())
}

/// Idempotently append a skillset's `instructions` to `$CODEX_HOME/AGENTS.md`
/// inside a fenced marker block. On re-pin the existing block for that skillset
/// is REPLACED (deduped), not duplicated. Creates the file if absent.
pub fn append_instructions(
    codex_home: &Path,
    skillset_name: &str,
    instructions: &str,
) -> Result<(), RunnerError> {
    let agents_path = codex_home.join("AGENTS.md");
    let existing = match std::fs::read_to_string(&agents_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(io_err("failed to read AGENTS.md", error));
        }
    };

    let updated = upsert_marker_block(&existing, skillset_name, instructions);
    std::fs::write(&agents_path, updated)
        .map_err(|error| io_err("failed to write AGENTS.md", error))?;
    tracing::info!(skillset = %skillset_name, "appended skillset instructions to AGENTS.md");
    Ok(())
}

/// Pure marker-block upsert: replace an existing `ornn-skillset:<name>` block
/// Render the fenced marker block a skillset's instructions occupy in
/// `AGENTS.md`: `<!-- ornn-skillset:<name> BEGIN -->\n<instructions>\n<!-- … END -->`.
/// The single source of truth for the block format, reused by
/// [`upsert_marker_block`] (the in-process / worker install path) and by the
/// controller's `resolve_plan` (#151), so a resolved dispatch's
/// `agents_md_appends` carries the IDENTICAL bytes the injector would write —
/// and the worker writes them verbatim.
pub fn render_marker_block(skillset_name: &str, instructions: &str) -> String {
    let begin = format!("<!-- {MARKER_PREFIX}{skillset_name} BEGIN -->");
    let end = format!("<!-- {MARKER_PREFIX}{skillset_name} END -->");
    format!("{begin}\n{instructions}\n{end}")
}

/// or append a fresh one. Split out so the dedupe-on-repin logic is testable
/// without filesystem access.
fn upsert_marker_block(existing: &str, skillset_name: &str, instructions: &str) -> String {
    let begin = format!("<!-- {MARKER_PREFIX}{skillset_name} BEGIN -->");
    let end = format!("<!-- {MARKER_PREFIX}{skillset_name} END -->");
    let block = render_marker_block(skillset_name, instructions);

    if let (Some(begin_at), Some(end_at)) = (existing.find(&begin), existing.find(&end)) {
        // Replace the existing block (from the BEGIN marker through the END
        // marker line) with the fresh one, leaving everything else intact.
        let end_line_end = end_at + end.len();
        let mut out = String::with_capacity(existing.len() + block.len());
        out.push_str(&existing[..begin_at]);
        out.push_str(&block);
        out.push_str(&existing[end_line_end..]);
        return out;
    }

    // Append a new block, ensuring a blank-line separation from prior content.
    let mut out = String::with_capacity(existing.len() + block.len() + 2);
    out.push_str(existing);
    if !existing.is_empty() && !existing.ends_with('\n') {
        out.push('\n');
    }
    if !existing.is_empty() {
        out.push('\n');
    }
    out.push_str(&block);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn codex_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("codex home")
    }

    /// Build a zip in memory. Entries: (path, content, optional explicit mode).
    fn build_zip(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut writer = zip::ZipWriter::new(cursor);
            for (path, content, mode) in entries {
                let mut options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
                if let Some(m) = mode {
                    options = options.unix_permissions(*m);
                }
                writer.start_file(*path, options).expect("start file");
                writer.write_all(content).expect("write content");
            }
            writer.finish().expect("finish zip");
        }
        buf
    }

    // ---- resolve_mode -------------------------------------------------------

    #[test]
    fn resolve_mode_prefers_recorded_mode() {
        assert_eq!(resolve_mode("SKILL.md", Some(0o600), b"x"), 0o600);
        assert_eq!(
            resolve_mode("scripts/run.sh", Some(0o700), b"#!/bin/sh"),
            0o700
        );
    }

    #[test]
    fn resolve_mode_falls_back_to_exec_for_scripts_and_shebangs() {
        assert_eq!(resolve_mode("scripts/run.sh", None, b"echo hi"), EXEC_BITS);
        assert_eq!(
            resolve_mode("bin/tool", None, b"#!/usr/bin/env python"),
            EXEC_BITS
        );
        assert_eq!(
            resolve_mode("nested/scripts/x", Some(0), b"plain"),
            EXEC_BITS
        );
    }

    #[test]
    fn resolve_mode_defaults_regular_files_to_644() {
        assert_eq!(resolve_mode("SKILL.md", None, b"# Skill"), REGULAR_BITS);
        assert_eq!(resolve_mode("refs/data.json", Some(0), b"{}"), REGULAR_BITS);
    }

    // ---- install_skill: tree + exec bits ------------------------------------

    #[test]
    fn install_skill_writes_the_tree_and_preserves_exec_bits() {
        let home = codex_home();
        let zip = build_zip(&[
            ("SKILL.md", b"# Demo skill", Some(0o644)),
            ("scripts/run.sh", b"#!/bin/sh\necho hi\n", Some(0o755)),
            ("refs/notes.txt", b"hello", Some(0o644)),
        ]);
        install_skill(home.path(), "demo", &zip).expect("install");

        let base = home.path().join("skills/demo");
        assert_eq!(
            std::fs::read_to_string(base.join("SKILL.md")).unwrap(),
            "# Demo skill"
        );
        assert_eq!(
            std::fs::read(base.join("scripts/run.sh")).unwrap(),
            b"#!/bin/sh\necho hi\n"
        );
        // The script keeps its executable bit.
        let mode = std::fs::metadata(base.join("scripts/run.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o111,
            0o111,
            "script must be executable, got {mode:o}"
        );
        // The plain ref is not executable.
        let ref_mode = std::fs::metadata(base.join("refs/notes.txt"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(ref_mode & 0o111, 0, "ref must not be executable");
    }

    #[test]
    fn install_skill_restores_exec_for_shebang_without_recorded_mode() {
        let home = codex_home();
        // A package built by a lossy archiver records NO usable unix mode
        // (mode `0`); the shebang heuristic must then grant +x. (The zip crate
        // always writes *some* mode, so `0` is how we simulate "none".)
        let zip = build_zip(&[
            ("SKILL.md", b"# x", Some(0)),
            ("entrypoint", b"#!/usr/bin/env bash\n", Some(0)),
        ]);
        install_skill(home.path(), "tool", &zip).expect("install");
        let mode = std::fs::metadata(home.path().join("skills/tool/entrypoint"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "shebang file must be executable");
        // The non-shebang doc stays non-executable under the same heuristic.
        let doc_mode = std::fs::metadata(home.path().join("skills/tool/SKILL.md"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(doc_mode & 0o111, 0, "plain doc must not be executable");
    }

    // ---- install_skill: path traversal --------------------------------------

    #[test]
    fn install_skill_rejects_path_traversal_entry() {
        let home = codex_home();
        // `enclosed_name` normalizes `..`; craft an entry that escapes.
        let zip = build_zip(&[("../escape.txt", b"evil", Some(0o644))]);
        let err = install_skill(home.path(), "demo", &zip).expect_err("must reject traversal");
        assert!(matches!(err, RunnerError::InvalidPackage(_)), "got {err:?}");
        // Nothing escaped the install root.
        assert!(!home.path().join("escape.txt").exists());
    }

    #[test]
    fn install_skill_rejects_absolute_entry() {
        let home = codex_home();
        let zip = build_zip(&[("/etc/evil", b"x", Some(0o644))]);
        // An absolute zip name is normalized by `enclosed_name`; either it is
        // rejected, or it lands safely inside the skill dir — never at /etc.
        let _ = install_skill(home.path(), "demo", &zip);
        assert!(!Path::new("/etc/evil").exists());
    }

    // ---- append_instructions: marker block ----------------------------------

    #[test]
    fn append_instructions_creates_file_with_marker_block() {
        let home = codex_home();
        append_instructions(home.path(), "research", "Use the web tool.").expect("append");
        let text = std::fs::read_to_string(home.path().join("AGENTS.md")).unwrap();
        assert!(text.contains("<!-- ornn-skillset:research BEGIN -->"));
        assert!(text.contains("Use the web tool."));
        assert!(text.contains("<!-- ornn-skillset:research END -->"));
    }

    #[test]
    fn append_instructions_dedupes_on_repin() {
        let home = codex_home();
        append_instructions(home.path(), "research", "v1 prompt").expect("first");
        append_instructions(home.path(), "research", "v2 prompt").expect("repin");
        let text = std::fs::read_to_string(home.path().join("AGENTS.md")).unwrap();
        // Exactly one block for `research`, carrying the latest prompt.
        assert_eq!(text.matches("ornn-skillset:research BEGIN").count(), 1);
        assert!(text.contains("v2 prompt"));
        assert!(!text.contains("v1 prompt"));
    }

    #[test]
    fn append_instructions_keeps_other_skillset_blocks() {
        let home = codex_home();
        append_instructions(home.path(), "alpha", "alpha prompt").expect("alpha");
        append_instructions(home.path(), "beta", "beta prompt").expect("beta");
        append_instructions(home.path(), "alpha", "alpha v2").expect("repin alpha");
        let text = std::fs::read_to_string(home.path().join("AGENTS.md")).unwrap();
        assert!(text.contains("beta prompt"), "beta block must survive");
        assert!(text.contains("alpha v2"));
        assert!(!text.contains("alpha prompt"));
        assert_eq!(text.matches("ornn-skillset:alpha BEGIN").count(), 1);
        assert_eq!(text.matches("ornn-skillset:beta BEGIN").count(), 1);
    }

    #[test]
    fn upsert_marker_block_preserves_surrounding_content() {
        let existing = "# Codex governance\n\nSome existing prose.\n";
        let out = upsert_marker_block(existing, "x", "block body");
        assert!(out.starts_with("# Codex governance"));
        assert!(out.contains("Some existing prose."));
        assert!(out
            .contains("<!-- ornn-skillset:x BEGIN -->\nblock body\n<!-- ornn-skillset:x END -->"));
    }
}
