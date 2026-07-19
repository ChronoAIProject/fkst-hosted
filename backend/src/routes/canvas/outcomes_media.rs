//! Pure media/extension helpers for the outcomes + blob endpoints: the
//! filename → kind / content-type guesses, the `OutcomeFile` projection, the
//! download `Content-Disposition`, and the blob-sha validator. Split out of
//! `outcomes.rs` to keep both files within the source line budget; all `I/O`
//! stays in the handler module.

use crate::error::AppError;
use crate::github_app::PullFileMeta;

use super::OutcomeFile;

/// Project one PR-file meta into the wire shape (kind + size_hint derived here).
pub(super) fn outcome_file(meta: &PullFileMeta) -> OutcomeFile {
    let kind = guess_kind(&meta.filename);
    // A text diff carries a meaningful line delta; a binary/media blob does not.
    let size_hint = (kind == "text").then_some(meta.additions + meta.deletions);
    OutcomeFile {
        filename: meta.filename.clone(),
        status: meta.status.clone(),
        additions: meta.additions,
        deletions: meta.deletions,
        sha: meta.sha.clone(),
        previous_filename: meta.previous_filename.clone(),
        kind: kind.to_string(),
        size_hint,
    }
}

/// The `Content-Disposition` value: `attachment; filename="…"` when downloading,
/// else `inline`. The filename is the sanitized basename (no path, no quotes).
pub(super) fn disposition_header(name: &str, download: bool) -> String {
    if !download {
        return "inline".to_string();
    }
    let base: String = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .chars()
        .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
        .collect();
    if base.is_empty() {
        "attachment".to_string()
    } else {
        format!("attachment; filename=\"{base}\"")
    }
}

/// The lowercased extension (basename after the last `.`), or `None` when the
/// file has no extension (`README`, `LICENSE`, `Dockerfile`).
pub(super) fn extension(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase())
}

/// True for the common text/source extensions a session commits.
fn is_text_ext(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "markdown"
            | "rst"
            | "json"
            | "jsonc"
            | "toml"
            | "yaml"
            | "yml"
            | "xml"
            | "csv"
            | "tsv"
            | "ini"
            | "cfg"
            | "conf"
            | "properties"
            | "env"
            | "lock"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mjs"
            | "cjs"
            | "vue"
            | "svelte"
            | "py"
            | "go"
            | "java"
            | "kt"
            | "kts"
            | "swift"
            | "c"
            | "h"
            | "cc"
            | "cpp"
            | "hpp"
            | "cs"
            | "rb"
            | "php"
            | "pl"
            | "lua"
            | "r"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "sql"
            | "html"
            | "htm"
            | "css"
            | "scss"
            | "sass"
            | "less"
            | "tf"
            | "gradle"
            | "dockerfile"
            | "makefile"
            | "gitignore"
            | "log"
            | "text"
    )
}

/// Coarse media KIND from the filename extension (`text`/`image`/`video`/`audio`/`binary`).
pub(super) fn guess_kind(name: &str) -> &'static str {
    match extension(name).as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "avif") => "image",
        Some("mp4" | "webm" | "mov" | "mkv" | "avi" | "m4v") => "video",
        Some("mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "opus") => "audio",
        Some(ext) if is_text_ext(ext) => "text",
        // Extensionless files (README, LICENSE, Dockerfile, Makefile) are text.
        None => "text",
        // A known-but-not-text extension (pdf, zip, woff, …) is binary.
        Some(_) => "binary",
    }
}

/// Guess a response `Content-Type` from the filename extension. Text/unknown
/// extensionless files serve as UTF-8 text; unknown binary extensions fall back
/// to `application/octet-stream`.
pub(super) fn content_type_for(name: &str) -> &'static str {
    match extension(name).as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("bmp") => "image/bmp",
        Some("ico") => "image/x-icon",
        Some("avif") => "image/avif",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("ogg" | "opus") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("m4a" | "aac") => "audio/mp4",
        Some(ext) if is_text_ext(ext) => "text/plain; charset=utf-8",
        None => "text/plain; charset=utf-8",
        Some(_) => "application/octet-stream",
    }
}

/// A git blob sha is hex (sha-1 or sha-256); reject anything else before it is
/// interpolated into a GitHub URL.
pub(super) fn validate_blob_sha(sha: &str) -> Result<(), AppError> {
    let ok = !sha.is_empty() && sha.len() <= 64 && sha.chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(AppError::Validation(
            "invalid blob sha: must be hex".to_string(),
        ))
    }
}
