//! Shared application state passed to every handler.

use crate::config::Config;
use crate::db::Db;

/// Clonable state shared across the router. Both members are cheap to clone
/// (`Db` is `Arc`-backed inside the driver).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Db,
}
