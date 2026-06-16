//! The worker's engine layer (issue #151, increment 4).
//!
//! [`executor::execute_dispatch`] turns a controller-resolved [`ResolvedDispatch`]
//! into a spawned, running engine — the worker-side mirror of the control-plane
//! driver's start path (`fkst-control-plane/src/sessions/service.rs::drive_inner`:
//! clone → optional CODEX_HOME (config.toml + ornn) → GoalContext → `.mint-nonce`
//! → `StartSpec` → `start_with_spec`). It deliberately stops AT the spawn: the
//! supervise loop, status reporting, and credential refresh are the NEXT
//! increment. This increment only spawns and registers a session.
//!
//! ## The `Cloner` seam
//!
//! [`clone_repo_packages`] runs a real `git clone https://github.com/…`, so it
//! cannot run offline. The clone step is therefore behind the one-method
//! [`Cloner`] trait: the production [`RealCloner`] calls `clone_repo_packages`
//! verbatim (so the dormant prod dispatch path is byte-identical to the
//! control-plane driver), while a test can inject a fake cloner that materializes
//! the package tree on disk — letting the REST of `execute_dispatch` (codex home,
//! ornn install, GoalContext, nonce, StartSpec, spawn) be exercised end to end
//! without a network. The seam abstracts ONLY the clone; the spec build and the
//! engine spawn are unchanged.

pub mod executor;
pub mod refresh;
pub mod supervise;

#[cfg(test)]
pub mod supervise_test_support;

use std::any::Any;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use secrecy::SecretString;

use fkst_engine::{clone_repo_packages, RunnerError};
use fkst_shared::models::RepoRef;

pub use executor::{execute_dispatch, ExecError, ExecutedSession};

/// A cloned repo's resolved roots plus an opaque RAII guard that owns the
/// on-disk working tree (and, for the real clone, the transient credential dir).
/// Dropping the handle removes those dirs — so the executor holds it for the
/// session's lifetime, exactly as the control-plane driver holds the
/// `ClonedRepo`'s `TempDir` guards.
///
/// The guard is type-erased (`Box<dyn Any + Send>`) because the real
/// [`fkst_engine::clone::ClonedRepo`] keeps its `TempDir`s private (it cannot be
/// constructed by a fake), so the seam exposes only what the executor consumes
/// (`project_root` + `package_roots`) and a drop-guard. The guard is never
/// downcast — only dropped.
pub struct ClonedHandle {
    /// `--project-root`: the canonicalized repo working-tree root.
    pub project_root: PathBuf,
    /// One canonicalized `<project_root>/.fkst/packages/<name>` per requested
    /// package, in request order.
    pub package_roots: Vec<PathBuf>,
    /// Owns the on-disk dirs for the session lifetime; dropping it cleans them.
    _guard: Box<dyn Any + Send>,
}

// Hand-written so the opaque `Box<dyn Any>` guard (which is not `Debug`) does
// not block deriving it — only the consumed paths are rendered.
impl std::fmt::Debug for ClonedHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClonedHandle")
            .field("project_root", &self.project_root)
            .field("package_roots", &self.package_roots)
            .finish_non_exhaustive()
    }
}

impl ClonedHandle {
    /// Wrap an arbitrary owner of the on-disk clone as the handle's guard. The
    /// production path passes the real [`fkst_engine::clone::ClonedRepo`]; a test
    /// passes the `TempDir` it built the tree in.
    pub fn new(
        project_root: PathBuf,
        package_roots: Vec<PathBuf>,
        guard: Box<dyn Any + Send>,
    ) -> Self {
        Self {
            project_root,
            package_roots,
            _guard: guard,
        }
    }
}

/// The injectable clone step. One method, matching [`clone_repo_packages`]'s
/// arguments exactly, so a future change to its contract forces a compile error
/// here rather than letting the prod path silently diverge.
#[async_trait]
pub trait Cloner: Send + Sync {
    /// Clone `repo`'s named packages under `base`, authenticating with `token`,
    /// and resolve their `<repo>/.fkst/packages/<name>` roots.
    async fn clone_packages(
        &self,
        base: &Path,
        repo: &RepoRef,
        token: &SecretString,
        package_names: &[String],
        framework_bin: &Path,
    ) -> Result<ClonedHandle, RunnerError>;
}

/// Production cloner: a thin, verbatim pass-through to [`clone_repo_packages`].
/// Its body is ONLY the real call plus wrapping the returned `ClonedRepo` as the
/// handle's drop-guard, so there is nothing to drift from the driver's clone.
pub struct RealCloner;

#[async_trait]
impl Cloner for RealCloner {
    async fn clone_packages(
        &self,
        base: &Path,
        repo: &RepoRef,
        token: &SecretString,
        package_names: &[String],
        framework_bin: &Path,
    ) -> Result<ClonedHandle, RunnerError> {
        let cloned = clone_repo_packages(base, repo, token, package_names, framework_bin).await?;
        // Copy out the two argv inputs the runner needs, then move the whole
        // `ClonedRepo` (with its private TempDir guards) into the handle so the
        // working tree + credential dir live for the session lifetime.
        let project_root = cloned.project_root.clone();
        let package_roots = cloned.package_roots.clone();
        Ok(ClonedHandle::new(
            project_root,
            package_roots,
            Box::new(cloned),
        ))
    }
}
