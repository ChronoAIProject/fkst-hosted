//! Packages domain: data models, validation, and MongoDB persistence for
//! fkst lua packages.
//!
//! A *package* is the on-disk shape the fkst engine consumes: a directory
//! tree of `departments/<name>/main.lua`, optional `raisers/<name>.lua`,
//! optional `core.lua`, and optional `composed.deps`. This module owns the
//! `packages` collection shape and its validation gate.

pub mod archive;
pub mod error;
pub mod model;
pub mod repository;
pub mod shares;

pub use archive::package_from_zip;
pub use error::PackageError;
pub use model::{
    is_valid_name, NewPackage, Package, PackageFile, MAX_COMPOSED_DEPS, MAX_COMPOSED_DEP_LEN,
    MAX_FILES, MAX_FILE_CONTENT_BYTES, MAX_FILE_PATH_LEN, MAX_TOTAL_CONTENT_BYTES,
    PACKAGES_COLLECTION,
};
pub use repository::PackageRepository;
pub use shares::{GranteeKind, ShareDoc, ShareLevel, ShareRepo, PACKAGE_SHARES_COLLECTION};
