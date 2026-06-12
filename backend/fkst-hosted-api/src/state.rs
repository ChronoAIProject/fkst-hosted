//! Shared application state passed to every handler.

use crate::auth::AuthMode;
use crate::config::Config;
use crate::db::Db;
use crate::packages::PackageRepository;
use crate::sessions::SessionService;

/// Clonable state shared across the router. Every member is cheap to clone
/// (`Db` and the repository's `Collection` are `Arc`-backed inside the
/// driver; the session service is an `Arc` handle).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Db,
    /// Repository over the `packages` collection (domain layer owned by the
    /// packages module); HTTP handlers go through this, never raw Mongo.
    pub packages: PackageRepository,
    /// Single-pod session orchestration (sessions module); HTTP handlers go
    /// through this, never raw Mongo or the engine runner.
    pub sessions: SessionService,
    /// Authentication mode: disabled (local dev) or enabled with NyxID
    /// settings. Determines whether the JWT middleware is active.
    pub auth_mode: AuthMode,
}
