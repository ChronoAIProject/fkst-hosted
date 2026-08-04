//! Whole-file capture: copy each health report into the bundle under its own name.
//!
//! A second source KIND beside the line-appending tail, sharing everything that
//! matters — one collector, one [`Redactor`], one flush cadence, one bundle. The
//! tailing path cannot serve reports: [`TreeWriter`] buffers lines per
//! [`LogClass`](super::classify::LogClass) and appends them to that class's single
//! file, so routing reports through it would concatenate every report into one blob
//! and destroy the per-report boundaries the whole feature depends on.
//!
//! # Redaction is mandatory and non-negotiable
//!
//! A report is authored by a codex that has read the session's logs, so it can quote a
//! credential verbatim. Every byte passes through [`Redactor::redact_line`] before it
//! touches the tree — the same choke point `collector::append_line` guarantees for
//! tailed records. There is deliberately no path in this module that writes unredacted
//! bytes.
//!
//! # Bounds: a session must not be able to fill the bundle
//!
//! The producer runs inside the session, so its output is untrusted in volume even
//! when it is trusted in intent. Three independent ceilings apply, and exceeding any
//! of them logs a redacted warning and skips — never crashes the collector, never
//! blocks `supervise`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::session_health::HEALTH_DIR_NAME;

use super::classify::{discover_copied_files, TreeAnchors};
use super::redact::Redactor;
use super::tree_writer::TreeWriter;

/// Bundle directory the reports are copied into, one entry per report.
///
/// Named for the producing package rather than for `health` so a reader of a bundle
/// can tell at a glance that these files came from a package, not from the framework.
pub(crate) const COPIED_TREE_DIR: &str = "fkst-health";

/// Largest single report copied. A report is a page of markdown; anything larger is
/// anomalous, and the v1 contract publishes the same number so a well-formed report is
/// never dropped downstream.
pub(crate) const MAX_FILE_BYTES: u64 = 256 * 1024;

/// Most reports copied, newest by filename — which is newest by time, because the
/// contract's stamp sorts lexically.
pub(crate) const MAX_FILES: usize = 200;

/// Hard ceiling on this source's total contribution to the bundle over a pod's life.
///
/// Counted in REDACTED bytes — what actually lands in the bundle. An over-long line
/// collapses to the redactor's overflow mask and therefore costs almost nothing here;
/// [`MAX_FILES`] is what bounds the read cost of that case.
pub(crate) const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

/// One report that was just copied into the tree.
///
/// Carries the redacted bytes so a downstream consumer publishes exactly what landed
/// in the bundle — never a second read of the source file, which could by then have
/// been rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopiedFile {
    /// The report's own filename, which is also its bundle entry name.
    pub(crate) file_name: String,
    /// The redacted contents written into the tree.
    pub(crate) redacted: String,
}

/// What a source file looked like when it was last copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

/// Tracks which reports have already been copied, so an unchanged file is not
/// re-copied on every 500 ms poll.
#[derive(Debug, Default)]
pub(crate) struct CopiedFileTracker {
    seen: HashMap<PathBuf, FileStamp>,
    total_bytes: usize,
    copies: usize,
    judged: usize,
    over_budget_reported: bool,
}

impl CopiedFileTracker {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// How many copies have been performed. Test observability for the
    /// "unchanged files are not re-copied" property, which is otherwise invisible.
    #[cfg(test)]
    pub(crate) fn copies(&self) -> usize {
        self.copies
    }

    /// How many distinct (path, stamp) pairs have been judged at all — including the
    /// ones skipped for a bound or an unsafe name. Makes "decided once, not once per
    /// poll" observable, which is otherwise only visible as log volume.
    #[cfg(test)]
    pub(crate) fn judged(&self) -> usize {
        self.judged
    }

    /// Copy every new-or-changed report into the tree and return what was copied.
    ///
    /// Best-effort throughout: an unreadable dir or file contributes nothing and does
    /// not raise. Copied bytes are accounted to [`TreeWriter`] so a new report
    /// participates in the existing size-based flush trigger AND marks the tree dirty
    /// — without that, a flush cycle would consider the tree unchanged and never
    /// re-upload the bundle.
    pub(crate) fn sweep(
        &mut self,
        anchors: &TreeAnchors,
        redactor: &Redactor,
        tree: &mut TreeWriter,
    ) -> Vec<CopiedFile> {
        let mut discovered = discover_copied_files(anchors);
        // Newest by filename. Older reports stay in the pod (the producer prunes
        // them); they simply stop being carried in the bundle.
        if discovered.len() > MAX_FILES {
            discovered.drain(..discovered.len() - MAX_FILES);
        }

        let mut copied = Vec::new();
        for path in discovered {
            match self.copy_one(&path, redactor, tree) {
                Ok(Some(file)) => copied.push(file),
                Ok(None) => {}
                Err(detail) => log_redacted(redactor, &detail),
            }
        }
        copied
    }

    /// `Ok(None)` means "nothing to do" (unchanged, or deliberately skipped);
    /// `Err(detail)` carries a message for the caller to log REDACTED.
    fn copy_one(
        &mut self,
        path: &Path,
        redactor: &Redactor,
        tree: &mut TreeWriter,
    ) -> Result<Option<CopiedFile>, String> {
        let metadata = std::fs::metadata(path)
            .map_err(|error| format!("log-stream: {HEALTH_DIR_NAME} stat failed: {error}"))?;
        let stamp = FileStamp {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        };
        if self.seen.get(path) == Some(&stamp) {
            return Ok(None);
        }
        // Record BEFORE deciding — for EVERY skip reason, including an unsafe name.
        // A producer legitimately keeps a working file in this directory (the health
        // reporter writes its codex context as a dotfile there), and this poll runs
        // twice a second: re-deciding without recording re-warns ~2x/s for the pod's
        // whole life and floods the very bundle this module exists to fill. Observed
        // on a real session: 411 identical warnings in four minutes.
        self.seen.insert(path.to_path_buf(), stamp);
        self.judged += 1;

        let Some(file_name) = safe_file_name(path) else {
            return Err(format!(
                "log-stream: unsafe {HEALTH_DIR_NAME} filename skipped: {path:?}"
            ));
        };

        if metadata.len() > MAX_FILE_BYTES {
            return Err(format!(
                "log-stream: {HEALTH_DIR_NAME} report {file_name} is {} bytes (limit {MAX_FILE_BYTES}); skipped",
                metadata.len()
            ));
        }
        if self.total_bytes >= MAX_TOTAL_BYTES {
            if self.over_budget_reported {
                return Ok(None);
            }
            self.over_budget_reported = true;
            return Err(format!(
                "log-stream: {HEALTH_DIR_NAME} capture budget of {MAX_TOTAL_BYTES} bytes reached; skipping further reports"
            ));
        }

        let raw = std::fs::read(path)
            .map_err(|error| format!("log-stream: {HEALTH_DIR_NAME} read failed: {error}"))?;
        let redacted = redact_document(&String::from_utf8_lossy(&raw), redactor);

        let destination = tree.root().join(COPIED_TREE_DIR).join(&file_name);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!("log-stream: {HEALTH_DIR_NAME} tree dir creation failed: {error}")
            })?;
        }
        std::fs::write(&destination, redacted.as_bytes())
            .map_err(|error| format!("log-stream: {HEALTH_DIR_NAME} copy failed: {error}"))?;

        self.total_bytes += redacted.len();
        self.copies += 1;
        tree.note_external_bytes(redacted.len());

        Ok(Some(CopiedFile {
            file_name,
            redacted,
        }))
    }
}

/// Redact a whole document line by line, preserving its line structure exactly.
///
/// Splitting on `\n` and rejoining with `\n` keeps the newline count (and any `\r`)
/// byte-identical for content that holds no secret, which is what lets the parsed
/// report body round-trip.
fn redact_document(text: &str, redactor: &Redactor) -> String {
    text.split('\n')
        .map(|line| redactor.redact_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A defence-in-depth traversal guard.
///
/// `read_dir` cannot yield a name containing a separator, so this is belt-and-braces
/// — but the name becomes a path segment AND (downstream) an object key, and the cost
/// of being wrong once is arbitrary file write. Anything not a plain, visible file
/// name is refused.
fn safe_file_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if name.is_empty()
        || name.starts_with('.')
        || name.contains('/')
        || name.contains('\\')
        || name.contains(['\0', '\n', '\r'])
    {
        return None;
    }
    Some(name.to_string())
}

fn log_redacted(redactor: &Redactor, message: &str) {
    tracing::warn!(detail = %redactor.redact_line(message));
}

#[cfg(test)]
#[path = "copied_tests.rs"]
mod tests;
