//! The on-branch log TREE: the fixed set of destination files and the pure rules
//! that route a source (a channel record or a tailed file) into one of them.
//!
//! The pod captures a heterogeneous set of streams — the driver's own records, the
//! `supervise` child's stdout/stderr, the framework-child log files, the codex log
//! dir — and folds them into a small, stable tree so a reader of the pushed branch
//! always finds logs in the same place. This module is pure: given a path (and the
//! two anchor dirs) it yields the destination class, with no I/O, so the routing is
//! exhaustively unit-testable.

use std::path::{Path, PathBuf};

/// A destination file in the pushed log tree. One class == one file under an
/// instance dir; every captured record is redacted then appended to its class file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogClass {
    /// The `run-substrate` driver's own ("fkst-hosted") records.
    HostedDriver,
    /// The framework `supervise` child stream + the framework-child log files.
    Supervise,
    /// The codex log dir.
    Codex,
    /// Anything captured but not otherwise classified.
    Misc,
}

impl LogClass {
    /// Every class, for iterating the tree deterministically.
    pub const ALL: [LogClass; 4] = [
        LogClass::HostedDriver,
        LogClass::Supervise,
        LogClass::Codex,
        LogClass::Misc,
    ];

    /// The class's destination path RELATIVE to an instance dir. Kept stable — a
    /// reader (and the control-plane backup) depends on this layout.
    pub fn relative_path(self) -> &'static str {
        match self {
            LogClass::HostedDriver => "fkst-hosted/driver.log",
            LogClass::Supervise => "fkst-substrate/framework/supervise.log",
            LogClass::Codex => "fkst-substrate/codex/codex.log",
            LogClass::Misc => "fkst-substrate/etc/misc.log",
        }
    }
}

/// The in-pod anchor directories the classifier + the source discovery use. The
/// framework writes its child logs under `<runtime_root>/logs/...`; codex writes
/// under `<codex_home>/log`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeAnchors {
    /// `<runtime_root>/logs` — the top-level session log dir.
    pub logs_dir: PathBuf,
    /// `<runtime_root>/logs/framework-child` — one file per framework child.
    pub framework_child_dir: PathBuf,
    /// `<codex_home>/log` — codex's own log dir.
    pub codex_log_dir: PathBuf,
}

impl TreeAnchors {
    /// Resolve the anchors from the pod's runtime root + codex home.
    pub fn new(runtime_root: &Path, codex_home: &Path) -> Self {
        let logs_dir = runtime_root.join("logs");
        Self {
            framework_child_dir: logs_dir.join("framework-child"),
            codex_log_dir: codex_home.join("log"),
            logs_dir,
        }
    }
}

/// Route a tailed file to its class by which anchor dir contains it. A file under
/// the framework-child dir is `Supervise`; one under the codex log dir is `Codex`;
/// anything else that surfaced under the logs tree is `Misc`. The check is
/// prefix-based so it never touches the filesystem.
pub fn classify_file(path: &Path, anchors: &TreeAnchors) -> LogClass {
    if path.starts_with(&anchors.framework_child_dir) {
        LogClass::Supervise
    } else if path.starts_with(&anchors.codex_log_dir) {
        LogClass::Codex
    } else {
        LogClass::Misc
    }
}

/// Discover the tailable source files under the anchor dirs, each paired with the
/// class it routes to. Best-effort: a missing/unreadable dir contributes nothing
/// (the framework or codex may not have written anything yet). Only `*.log` files
/// are tailed so a stray socket/pid file is never streamed.
pub fn discover_sources(anchors: &TreeAnchors) -> Vec<(PathBuf, LogClass)> {
    let mut sources = Vec::new();
    // Codex + framework-child are the two structured dirs; the top-level logs dir
    // catches anything the framework drops there directly (→ Misc).
    collect_logs(&anchors.framework_child_dir, anchors, &mut sources);
    collect_logs(&anchors.codex_log_dir, anchors, &mut sources);
    collect_logs(&anchors.logs_dir, anchors, &mut sources);
    sources
}

/// Append every `*.log` FILE directly under `dir` (non-recursive) to `out`,
/// classified via [`classify_file`]. A read error on the dir is swallowed (the dir
/// may simply not exist yet). Nested dirs are skipped here — the framework-child
/// and codex dirs are enumerated on their own passes.
fn collect_logs(dir: &Path, anchors: &TreeAnchors, out: &mut Vec<(PathBuf, LogClass)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if is_file && path.extension().and_then(|e| e.to_str()) == Some("log") {
            // De-dup: the top-level pass must not re-classify a file already picked
            // up under a sub-anchor.
            if out.iter().any(|(existing, _)| existing == &path) {
                continue;
            }
            let class = classify_file(&path, anchors);
            out.push((path, class));
        }
    }
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
