//! Control-plane facade over the on-disk Ornn skill-install machinery.
//!
//! The install implementation (unzip a skill package into
//! `$CODEX_HOME/skills/<name>/` and append a skillset's master prompt to
//! `$CODEX_HOME/AGENTS.md`) MOVED into `fkst-engine` (issue #151,
//! `fkst_engine::skills`) so BOTH the in-process control-plane driver AND the
//! worker's engine executor share ONE implementation. This module is now a thin
//! adapter that keeps the control-plane's `AppError` surface (and the
//! `crate::ornn::inject::{install_skill, append_instructions, render_marker_block}`
//! call sites in [`crate::ornn::inject_pins`] / `resolve_plan`) unchanged while
//! delegating to the relocated engine functions. The install + marker semantics
//! are byte-identical — only the error domain is mapped back to `AppError`.

use std::path::Path;

use fkst_engine::RunnerError;

use crate::error::AppError;

/// Map the engine runner's error onto the control-plane's `AppError`, preserving
/// the original classification: a structural/bad-input failure
/// ([`RunnerError::InvalidPackage`]) is a `422 Unprocessable`; a host filesystem
/// failure ([`RunnerError::Io`]) is a `503 Unavailable`. Other variants are
/// host-side failures and map to `Unavailable` as well. Never carries a secret.
fn map_runner_error(error: RunnerError) -> AppError {
    match error {
        RunnerError::InvalidPackage(message) => AppError::Unprocessable(message),
        other => AppError::Unavailable(other.to_string()),
    }
}

/// Install one skill package zip into `<codex_home>/skills/<name>/`.
///
/// Delegates to [`fkst_engine::skills::install_skill`]; see it for the full
/// path-traversal + exec-bit contract. Errors are mapped back to `AppError` so
/// the session driver's start path is unchanged.
pub fn install_skill(codex_home: &Path, name: &str, zip_bytes: &[u8]) -> Result<(), AppError> {
    fkst_engine::skills::install_skill(codex_home, name, zip_bytes).map_err(map_runner_error)
}

/// Idempotently append a skillset's `instructions` to `$CODEX_HOME/AGENTS.md`
/// inside a fenced marker block (deduped on re-pin). Delegates to
/// [`fkst_engine::skills::append_instructions`].
pub fn append_instructions(
    codex_home: &Path,
    skillset_name: &str,
    instructions: &str,
) -> Result<(), AppError> {
    fkst_engine::skills::append_instructions(codex_home, skillset_name, instructions)
        .map_err(map_runner_error)
}

/// Render the fenced marker block a skillset's instructions occupy in
/// `AGENTS.md`. The single source of truth for the block format lives in
/// `fkst-engine` ([`fkst_engine::skills::render_marker_block`]); this re-export
/// lets the controller's `resolve_plan` (#151) put the IDENTICAL block bytes the
/// in-process injector writes into a dispatch's `agents_md_appends`, which the
/// worker then writes verbatim.
pub fn render_marker_block(skillset_name: &str, instructions: &str) -> String {
    fkst_engine::skills::render_marker_block(skillset_name, instructions)
}
