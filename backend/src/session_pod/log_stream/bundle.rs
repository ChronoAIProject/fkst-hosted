//! Assemble the redacted log tree into a single gzip'd tar for the storage sink.
//!
//! The collector writes the redacted tree to disk (bounding memory) as a set of
//! files under a staging root (`fkst-hosted/…`, `fkst-substrate/…`, `meta.json`,
//! `README.md`). On each flush this folds that whole tree into ONE `tar.gz` the
//! [`super::sink::LogSink`] uploads to `logs/<session>/latest.tar.gz`. Kept pure
//! (a directory in, an in-memory `tar.gz` out) so the archive shape is unit-testable
//! without a network sink.
//!
//! why in-memory: the redacted tree is already gzip-friendly text and is bounded by
//! the flush cadence; a single buffer keeps the collector's I/O simple and its
//! footprint proportional to the COMPRESSED log size rather than the raw stream.

use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;

/// Tar + gzip every file under `root` into an in-memory buffer, with archive paths
/// rooted at the tree top (so an extractor recreates `fkst-hosted/driver.log`,
/// `fkst-substrate/framework/supervise.log`, … exactly as the collector wrote them).
///
/// An empty/absent `root` yields a valid (empty) gzip stream rather than an error,
/// so a first flush before anything is captured is still safe to upload.
pub fn tar_gz_dir(root: &Path) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        // Follow no symlinks: the tree only ever holds regular files the collector
        // wrote, so this just guards against a surprise link being archived by ref.
        builder.follow_symlinks(false);
        if root.is_dir() {
            // "" roots the entries at the archive top (no leading `./`).
            builder.append_dir_all("", root)?;
        }
        builder.finish()?;
    }
    encoder.finish()
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
