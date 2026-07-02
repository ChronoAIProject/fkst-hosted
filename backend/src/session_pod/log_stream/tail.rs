//! Incremental file TAILING: track a byte offset per file and, on each poll, emit
//! only the COMPLETE lines that have appeared since the last read.
//!
//! The framework-child + codex log files grow while the session runs; the collector
//! re-reads them on a timer. A naive re-read would re-emit the whole file every
//! time, so this tracks the last-read offset. A partial trailing line (no newline
//! yet) is HELD in a carry so a credential split across a read boundary is never
//! emitted before it can be redacted whole — line framing lives here, redaction is
//! applied by the caller to each returned line.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Per-file read cursor: the byte offset already consumed + the unterminated tail
/// held until its newline arrives.
#[derive(Debug, Default)]
pub struct TailTracker {
    offset: u64,
    carry: String,
}

impl TailTracker {
    /// A fresh tracker positioned at the start of a not-yet-read file.
    pub fn new() -> Self {
        Self::default()
    }

    /// The byte offset consumed so far (exposed for tests + diagnostics).
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Read everything appended to `path` since the last poll and return the newly
    /// COMPLETE lines (without their trailing newline). A partial trailing line is
    /// retained for the next poll. Best-effort: an unreadable/missing file yields no
    /// lines and leaves the cursor unchanged.
    ///
    /// Truncation handling: if the file shrank below our offset (a rotation) the
    /// cursor resets to 0 and the whole file is re-read, so a truncate-in-place never
    /// silently swallows the new content.
    pub fn poll(&mut self, path: &Path) -> Vec<String> {
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };
        let len = match file.metadata() {
            Ok(meta) => meta.len(),
            Err(_) => return Vec::new(),
        };
        if len < self.offset {
            // The file was truncated/rotated under us — restart from the top.
            self.offset = 0;
            self.carry.clear();
        }
        if len == self.offset {
            return Vec::new();
        }
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut fresh = Vec::new();
        if file.read_to_end(&mut fresh).is_err() {
            return Vec::new();
        }
        self.offset += fresh.len() as u64;
        self.frame(&String::from_utf8_lossy(&fresh))
    }

    /// Fold `chunk` into the carry and split off every complete line. Kept separate
    /// from [`poll`](Self::poll) so the line framing is testable without a file.
    pub fn frame(&mut self, chunk: &str) -> Vec<String> {
        self.carry.push_str(chunk);
        let mut lines = Vec::new();
        while let Some(idx) = self.carry.find('\n') {
            let line: String = self.carry.drain(..=idx).collect();
            lines.push(line.trim_end_matches('\n').to_string());
        }
        lines
    }

    /// Emit the final unterminated tail (if any) at end-of-stream, clearing the
    /// carry. Called once during the collector's final flush so a last partial line
    /// is not lost.
    pub fn finish(&mut self) -> Option<String> {
        if self.carry.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.carry))
        }
    }
}

#[cfg(test)]
#[path = "tail_tests.rs"]
mod tests;
