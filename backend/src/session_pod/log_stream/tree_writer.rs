//! [`TreeWriter`]: the redacted log tree's write buffer.
//!
//! Extracted from `collector.rs` so that file stays under the 500-line module cap —
//! it had already crossed it, and the collector is where new capture capabilities
//! land. Pure buffering + append-only file I/O, with no knowledge of redaction,
//! bundling, or upload: everything reaching [`TreeWriter::append`] is ALREADY
//! redacted, which is the invariant `collector::append_line` exists to enforce.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::classify::LogClass;

/// Buffers redacted lines per tree-class and appends them to the on-disk tree files.
/// One class == one file under the tree root; flushing appends (never rewrites), so
/// a growing log accretes across flushes.
pub(crate) struct TreeWriter {
    root: PathBuf,
    buffers: HashMap<LogClass, String>,
    pending_bytes: usize,
}

impl TreeWriter {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            buffers: HashMap::new(),
            pending_bytes: 0,
        }
    }

    /// The tree root the bundle is assembled from.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Append one already-redacted line (a newline is added) to its class buffer.
    pub(crate) fn append(&mut self, class: LogClass, redacted_line: &str) {
        let buffer = self.buffers.entry(class).or_default();
        buffer.push_str(redacted_line);
        buffer.push('\n');
        self.pending_bytes += redacted_line.len() + 1;
    }

    /// Bytes buffered since the last flush (drives the size-based flush trigger).
    pub(crate) fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// Append every non-empty class buffer to its tree file (creating parent dirs),
    /// then clear the buffers. A per-class write error propagates but leaves the
    /// unwritten buffer intact for the next attempt.
    pub(crate) fn flush_pending(&mut self) -> std::io::Result<()> {
        for (class, buffer) in self.buffers.iter_mut() {
            if buffer.is_empty() {
                continue;
            }
            let path = self.root.join(class.relative_path());
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(buffer.as_bytes())?;
            buffer.clear();
        }
        self.pending_bytes = 0;
        Ok(())
    }
}
