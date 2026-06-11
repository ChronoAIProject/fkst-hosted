//! Shared application state passed to every handler.

use crate::config::Config;
use crate::db::Db;
use crate::packages::PackageRepository;

/// Clonable state shared across the router. Every member is cheap to clone
/// (`Db` and the repository's `Collection` are `Arc`-backed inside the
/// driver).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Db,
    /// Repository over the `packages` collection (domain layer owned by the
    /// packages module); HTTP handlers go through this, never raw Mongo.
    pub packages: PackageRepository,
}
