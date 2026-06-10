//! Packages domain: data models, validation, and MongoDB persistence for
//! fkst lua packages.
//!
//! A *package* is the on-disk shape the fkst engine consumes: a directory
//! tree of `departments/<name>/main.lua`, optional `raisers/<name>.lua`,
//! optional `core.lua`, and optional `composed.deps`. This module owns the
//! `packages` collection shape and its validation gate.

pub mod error;
pub mod model;

pub use error::PackageError;
pub use model::{
    NewPackage, Package, PackageFile, MAX_COMPOSED_DEPS, MAX_COMPOSED_DEP_LEN, MAX_FILES,
    MAX_FILE_CONTENT_BYTES, MAX_FILE_PATH_LEN, MAX_TOTAL_CONTENT_BYTES, PACKAGES_COLLECTION,
};
