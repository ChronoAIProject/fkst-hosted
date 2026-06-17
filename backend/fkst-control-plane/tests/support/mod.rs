//! Shared integration-test support: real-engine binary resolution.
//!
//! Lives in `tests/support/` (a subdirectory, NOT a `tests/*.rs` file) so
//! cargo does not compile it as its own test target; each integration test
//! that needs it pulls it in with `mod support;`.
//!
//! Resolution order (identical for every consumer):
//! 1. `FKST_ENGINE_BIN` (path to a runnable `fkst-framework`).
//! 2. `/usr/local/bin/fkst-framework` when executable (the engine image).
//! 3. Linux with Docker: extract the binary from the engine image
//!    (`FKST_ENGINE_IMAGE`, default `fkst-hosted-api:engine-dev`) into a
//!    cached temp path via `docker create` + `docker cp`.
//! 4. Otherwise none. The docker-extracted binary is a LINUX binary — on
//!    macOS it cannot run on the host, so without `FKST_ENGINE_BIN` the
//!    consuming suites self-skip there.
// This module is compiled into every integration-test binary that needs ANY of
// its helpers; a given binary rarely uses all of them. Allow the resulting
// per-binary "unused" diagnostics so the shared module stays a single source of
// truth instead of being split per consumer.
#![allow(dead_code, unused_macros, unused_imports)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use fkst_control_plane::vault::{VaultLimits, VaultService};

/// Build the in-memory `VaultService` (#138) with the default per-scope limits.
/// The controller is datastore-free (#143), so this takes no handle.
pub fn test_vault() -> VaultService {
    VaultService::new(VaultLimits::default())
}

/// Default binary location inside the engine image / engine-based pods.
pub const IMAGE_ENGINE_BIN: &str = "/usr/local/bin/fkst-framework";

/// Default engine image for the Docker extraction path (overridable via
/// `FKST_ENGINE_IMAGE`). Only the Linux extraction path consumes it.
#[cfg(target_os = "linux")]
const DEFAULT_ENGINE_IMAGE: &str = "fkst-hosted-api:engine-dev";

/// Resolve (once per test binary) the real engine, or `None` to skip.
pub fn engine_bin() -> Option<PathBuf> {
    static ENGINE: OnceLock<Option<PathBuf>> = OnceLock::new();
    ENGINE.get_or_init(resolve_engine).clone()
}

fn resolve_engine() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("FKST_ENGINE_BIN") {
        let path = PathBuf::from(custom);
        if is_executable(&path) {
            return Some(path);
        }
        eprintln!(
            "FKST_ENGINE_BIN is set but not an executable file: {}",
            path.display()
        );
        return None;
    }
    let image_bin = Path::new(IMAGE_ENGINE_BIN);
    if is_executable(image_bin) {
        return Some(image_bin.to_path_buf());
    }
    extract_from_docker()
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Linux-only: pull the engine binary out of the Docker image into a cached
/// temp path. The extracted binary is a Linux ELF, so this path is gated to
/// Linux hosts (macOS cannot exec it; see the module docs).
#[cfg(target_os = "linux")]
fn extract_from_docker() -> Option<PathBuf> {
    use std::process::Command;

    let image =
        std::env::var("FKST_ENGINE_IMAGE").unwrap_or_else(|_| DEFAULT_ENGINE_IMAGE.to_string());
    let docker_ok = Command::new("docker")
        .arg("version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !docker_ok {
        eprintln!("docker not available; cannot extract the engine binary");
        return None;
    }

    let cache_dir = std::env::temp_dir().join("fkst-engine-it");
    let target = cache_dir.join("fkst-framework");
    if is_executable(&target) {
        return Some(target); // cached from a previous run
    }
    std::fs::create_dir_all(&cache_dir).ok()?;

    let create = Command::new("docker")
        .args(["create", &image])
        .output()
        .ok()?;
    if !create.status.success() {
        eprintln!(
            "docker create {image} failed: {}",
            String::from_utf8_lossy(&create.stderr)
        );
        return None;
    }
    let cid = String::from_utf8_lossy(&create.stdout).trim().to_string();

    let cp = Command::new("docker")
        .args([
            "cp",
            &format!("{cid}:{IMAGE_ENGINE_BIN}"),
            &target.to_string_lossy(),
        ])
        .output();
    let _ = Command::new("docker").args(["rm", "-f", &cid]).output();

    match cp {
        Ok(out) if out.status.success() && is_executable(&target) => Some(target),
        Ok(out) => {
            eprintln!(
                "docker cp of the engine binary failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            None
        }
        Err(err) => {
            eprintln!("docker cp of the engine binary failed: {err}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn extract_from_docker() -> Option<PathBuf> {
    eprintln!(
        "non-Linux host: the docker-extracted engine binary cannot run here; \
         set FKST_ENGINE_BIN to a runnable fkst-framework to enable this suite"
    );
    None
}

/// Resolve the engine or `return` out of the calling test with a SKIP line.
macro_rules! require_engine {
    () => {
        match crate::support::engine_bin() {
            Some(bin) => bin,
            None => {
                eprintln!(
                    "SKIP: no real fkst-framework available \
                     (FKST_ENGINE_BIN / {} / Linux+Docker image)",
                    crate::support::IMAGE_ENGINE_BIN
                );
                return;
            }
        }
    };
}
pub(crate) use require_engine;
