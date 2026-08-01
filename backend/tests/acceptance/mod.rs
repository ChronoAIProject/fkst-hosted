//! The milestone acceptance harness: the requirement-to-test matrix, the
//! discovery pass that proves each named test still exists, and the evidence
//! artifact the closure gate produces.
//!
//! This module is deliberately *data driven*. The claim "requirement `AUTH-03`
//! is covered" is worthless if it lives only in a prose table, because a rename
//! silently breaks it and nothing notices. Here the claim is a row in
//! `acceptance/requirement-matrix.toml`, and the linter re-derives it from the
//! working tree on every run.

#![allow(dead_code)]

pub mod discovery;
pub mod lint;
pub mod model;
pub mod report;

/// The repository root, derived from the crate manifest directory.
///
/// The matrix spans backend, frontend, and deployment suites, so every path in
/// it is repository-relative rather than crate-relative.
pub fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the backend crate always has a parent directory")
        .to_path_buf()
}

/// Where generated evidence lands.
///
/// Honours `CARGO_TARGET_DIR` so a shared target directory does not scatter
/// artifacts, and stays inside `target/` so nothing is ever committed.
pub fn artifact_dir() -> std::path::PathBuf {
    let target = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target"),
    };
    target.join("acceptance")
}
