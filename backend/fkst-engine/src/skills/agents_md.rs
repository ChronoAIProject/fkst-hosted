//! Seed the repo's own `.fkst/AGENTS.md` as the verbatim base of the per-session
//! `$CODEX_HOME/AGENTS.md`, then layer the Ornn skillset marker blocks on top
//! (issue #182).
//!
//! This is the SINGLE source of truth for both halves of that assembly:
//! - [`read_repo_agents_md`] reads `<project_root>/.fkst/AGENTS.md` from the
//!   cloned working tree, through the parent module's [`super::safe_join`]
//!   containment guard, capped at [`REPO_AGENTS_MD_CAP_BYTES`].
//! - [`compose_agents_md`] is the filesystem-free body composer: the repo `base`
//!   first (verbatim), then the Ornn marker-block `tail` below it.
//!
//! Both the ACTIVE worker executor and the DORMANT in-process control-plane
//! driver call these, so the two paths emit byte-identical `AGENTS.md` content.
//! Precedence is FIXED here: repo base above Ornn blocks.

use std::path::Path;

use crate::error::RunnerError;

/// Maximum byte size of a repo-supplied `.fkst/AGENTS.md` seeded as the AGENTS.md
/// base. User-authored repo content (not secret); a pathological file is
/// truncated to this cap with a logged warning so the engine never writes an
/// absurd CODEX_HOME file. 256 KiB is generous for human-authored markdown
/// instructions while still bounding a runaway file.
///
/// We deliberately choose a larger cap than the 64 KiB env-value cap (#102/#138):
/// that cap bounds a single injected env *value*, whereas this is a whole
/// instruction document that a user reasonably grows over time.
const REPO_AGENTS_MD_CAP_BYTES: usize = 256 * 1024;

/// Read the repo's `.fkst/AGENTS.md` from the cloned working tree, capped at
/// [`REPO_AGENTS_MD_CAP_BYTES`]. Returns `Ok(None)` when the file is absent.
///
/// `.fkst/AGENTS.md` is a fixed two-segment relative path, resolved through the
/// parent module's [`super::safe_join`] for defense-in-depth containment: a
/// `.fkst` symlink that escapes `project_root` is rejected as
/// [`RunnerError::InvalidPackage`] (its existing behavior). On a file larger than
/// the cap the content is truncated at a UTF-8 char boundary and a warning is
/// logged with the byte sizes — NEVER the content (it is user repo content).
pub fn read_repo_agents_md(project_root: &Path) -> Result<Option<String>, RunnerError> {
    // Defense in depth: the path is fixed, but routing it through `safe_join`
    // reuses the symlink-escape guard so a planted `.fkst -> /etc` symlink is
    // caught and mapped to `InvalidPackage`.
    let path = super::safe_join(project_root, ".fkst/AGENTS.md")?;

    let content = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(super::io_err("failed to read repo .fkst/AGENTS.md", error)),
    };

    if content.len() > REPO_AGENTS_MD_CAP_BYTES {
        // Truncate on a UTF-8 char boundary at or below the cap so we never split
        // a multi-byte codepoint. `floor_char_boundary` is unstable, so walk the
        // char indices and take the last boundary that still fits.
        let cut = content
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= REPO_AGENTS_MD_CAP_BYTES)
            .last()
            .unwrap_or(0);
        tracing::warn!(
            repo_agents_md_bytes = content.len(),
            cap = REPO_AGENTS_MD_CAP_BYTES,
            "repo .fkst/AGENTS.md exceeds cap; truncating"
        );
        return Ok(Some(content[..cut].to_string()));
    }

    Ok(Some(content))
}

/// Compose the final AGENTS.md body: the repo `base` first (verbatim), then a
/// blank-line separator, then the Ornn marker-block `tail`. Either side may be
/// empty; the precedence (repo base above Ornn blocks) is FIXED here.
///
/// Filesystem-free so it is unit-testable in isolation, and the single assembly
/// rule shared by both call paths. The separator mirrors
/// [`super::upsert_marker_block`]'s append rules (`mod.rs`): exactly one blank
/// line between a non-empty base and a non-empty tail, and a single trailing
/// newline is preserved.
pub fn compose_agents_md(base: Option<&str>, tail: &str) -> String {
    let base = base.unwrap_or("");
    let base_empty = base.is_empty();
    let tail_empty = tail.is_empty();

    // Neither side present: empty body (callers treat this as "do not write").
    if base_empty && tail_empty {
        return String::new();
    }

    let mut out = String::with_capacity(base.len() + tail.len() + 2);

    if !base_empty {
        out.push_str(base);
        if !tail_empty {
            // Exactly one blank line between base and tail, mirroring
            // `upsert_marker_block`: a single '\n' to end the base line (if it
            // lacks one) plus a second '\n' for the blank separator line.
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push('\n');
        }
    }

    if !tail_empty {
        out.push_str(tail);
    }

    // Preserve a single trailing newline (matches the worker's
    // `format!("{}\n", …)` and `upsert_marker_block`'s trailing newline).
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        tempfile::tempdir().expect("project root")
    }

    fn write_agents(root: &Path, content: &[u8]) {
        let fkst = root.join(".fkst");
        std::fs::create_dir_all(&fkst).expect("mkdir .fkst");
        std::fs::write(fkst.join("AGENTS.md"), content).expect("write AGENTS.md");
    }

    // ---- read_repo_agents_md ------------------------------------------------

    #[test]
    fn read_returns_none_when_absent() {
        let root = project();
        // No .fkst dir at all.
        assert_eq!(read_repo_agents_md(root.path()).expect("read"), None);

        // A .fkst dir exists but no AGENTS.md inside it.
        std::fs::create_dir_all(root.path().join(".fkst")).expect("mkdir");
        assert_eq!(read_repo_agents_md(root.path()).expect("read"), None);
    }

    #[test]
    fn read_returns_content_when_present() {
        let root = project();
        write_agents(root.path(), b"# Repo base\n\nFollow house rules.\n");
        assert_eq!(
            read_repo_agents_md(root.path()).expect("read"),
            Some("# Repo base\n\nFollow house rules.\n".to_string())
        );
    }

    #[test]
    fn read_truncates_oversize_on_char_boundary() {
        let root = project();
        // Build a file just over the cap whose boundary at the cap falls in the
        // middle of a multi-byte char, so a naive byte slice would panic.
        let mut content = "a".repeat(REPO_AGENTS_MD_CAP_BYTES - 1);
        // A 3-byte char ('€') straddles the cap: byte index CAP-1 starts it, so
        // the only valid boundary <= CAP is CAP-1.
        content.push('€');
        content.push_str("trailing");
        assert!(content.len() > REPO_AGENTS_MD_CAP_BYTES);

        let out = read_repo_agents_md_with(root.path(), content.as_bytes());
        let out = out.expect("read").expect("some");
        // Truncated strictly below the cap, on a valid boundary (no panic), and
        // the multibyte char that straddled the cap was dropped whole.
        assert!(out.len() <= REPO_AGENTS_MD_CAP_BYTES);
        assert!(out.is_char_boundary(out.len()));
        assert_eq!(out, "a".repeat(REPO_AGENTS_MD_CAP_BYTES - 1));
        assert!(!out.contains('€'));
    }

    /// Helper: write `content` as the repo's `.fkst/AGENTS.md` and read it back.
    fn read_repo_agents_md_with(
        root: &Path,
        content: &[u8],
    ) -> Result<Option<String>, RunnerError> {
        write_agents(root, content);
        read_repo_agents_md(root)
    }

    #[test]
    fn read_rejects_fkst_symlink_escaping_project_root() {
        // `.fkst` is a symlink pointing OUTSIDE project_root; `safe_join` must
        // reject it as InvalidPackage rather than read the escaped file.
        let root = project();
        let outside = project();
        std::fs::create_dir_all(outside.path()).expect("outside dir");
        std::fs::write(outside.path().join("AGENTS.md"), b"escaped secret").expect("write outside");

        std::os::unix::fs::symlink(outside.path(), root.path().join(".fkst"))
            .expect("symlink .fkst -> outside");

        let err = read_repo_agents_md(root.path()).expect_err("must reject escape");
        assert!(matches!(err, RunnerError::InvalidPackage(_)), "got {err:?}");
    }

    // ---- compose_agents_md: all four base/tail combinations -----------------

    #[test]
    fn compose_neither_is_empty() {
        // The degenerate "neither" case: empty body so the caller writes nothing.
        assert_eq!(compose_agents_md(None, ""), "");
        assert_eq!(compose_agents_md(Some(""), ""), "");
    }

    #[test]
    fn compose_base_only_is_just_the_base() {
        // A base with a trailing newline is preserved verbatim.
        assert_eq!(
            compose_agents_md(Some("# Base\nrules\n"), ""),
            "# Base\nrules\n"
        );
        // A base WITHOUT a trailing newline gains exactly one.
        assert_eq!(
            compose_agents_md(Some("# Base\nrules"), ""),
            "# Base\nrules\n"
        );
    }

    #[test]
    fn compose_tail_only_is_just_the_tail() {
        // Ornn-only: identical to the worker's `format!("{}\n", tail)` shape.
        let tail = "<!-- ornn-skillset:x BEGIN -->\nbody\n<!-- ornn-skillset:x END -->";
        assert_eq!(compose_agents_md(None, tail), format!("{tail}\n"));
        assert_eq!(compose_agents_md(Some(""), tail), format!("{tail}\n"));
    }

    #[test]
    fn compose_base_then_tail_with_one_blank_line() {
        let tail = "<!-- ornn-skillset:x BEGIN -->\nbody\n<!-- ornn-skillset:x END -->";
        // Base already newline-terminated => exactly one blank line, then tail.
        assert_eq!(
            compose_agents_md(Some("# Base\n"), tail),
            format!("# Base\n\n{tail}\n")
        );
        // Base NOT newline-terminated => still exactly one blank line between.
        assert_eq!(
            compose_agents_md(Some("# Base"), tail),
            format!("# Base\n\n{tail}\n")
        );
    }

    /// The load-bearing #182 invariant: the ACTIVE worker path composes the body
    /// in ONE shot (`compose_agents_md(base, blocks.join("\n\n"))`), while the
    /// DORMANT in-process path seeds the base (`compose_agents_md(base, "")`) then
    /// `append_instructions`-upserts each Ornn block. Both MUST emit identical
    /// bytes. We assert it directly here, against the real `append_instructions`,
    /// for both a newline-terminated and a non-terminated base.
    #[test]
    fn worker_oneshot_matches_inprocess_seed_then_append() {
        for base in ["# Repo rules\nFollow them.\n", "# Repo rules\nFollow them."] {
            let block_a = super::super::render_marker_block("alpha", "alpha body");
            let block_b = super::super::render_marker_block("beta", "beta body");

            // Worker one-shot: base + joined blocks, one trailing newline.
            let tail = [block_a.clone(), block_b.clone()].join("\n\n");
            let worker = compose_agents_md(Some(base), &tail);

            // In-process: seed the base, then upsert each block in order into the
            // file `append_instructions` writes.
            let home = tempfile::tempdir().expect("home");
            let seeded = compose_agents_md(Some(base), "");
            std::fs::write(home.path().join("AGENTS.md"), &seeded).expect("seed");
            super::super::append_instructions(home.path(), "alpha", "alpha body").expect("a");
            super::super::append_instructions(home.path(), "beta", "beta body").expect("b");
            let inprocess =
                std::fs::read_to_string(home.path().join("AGENTS.md")).expect("read back");

            assert_eq!(worker, inprocess, "worker and in-process paths must agree");
        }
    }
}
