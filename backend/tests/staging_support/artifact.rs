//! Writes staging evidence next to the requirement report.
//!
//! Separate from the requirement artifact because it is produced by a different
//! tier: the requirement report is rendered on every pull request, this one only
//! exists when the gated staging tier actually ran. Keeping them apart means a
//! reviewer can tell "the staging tier ran and here is what it saw" from "the
//! staging tier was not run", which a merged document could not express.

use std::path::PathBuf;

use serde_json::Value;

/// Where generated evidence lands. Mirrors `acceptance::artifact_dir` — the two
/// tiers do not share a module because they live in different test binaries.
pub fn dir() -> PathBuf {
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
    };
    target.join("acceptance")
}

/// Write `document` as `name` under the artifact directory.
///
/// A write failure is reported and swallowed on purpose: losing the artifact
/// must not turn a successful staging round trip into a red build, and the
/// message names the failure so it is not invisible.
pub fn write(name: &str, document: &Value) {
    let dir = dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("acceptance: could not create the artifact directory: {error}");
        return;
    }
    let rendered = match serde_json::to_string_pretty(document) {
        Ok(rendered) => rendered,
        Err(error) => {
            eprintln!("acceptance: could not render {name}: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::write(dir.join(name), rendered) {
        eprintln!("acceptance: could not write {name}: {error}");
    }
}
