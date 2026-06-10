//! Shared application state passed to every handler.

use crate::config::Config;

/// Clonable state shared across the router.
///
/// Currently holds only the runtime configuration; this is the seam where
/// later issues add shared handles (e.g. the Mongo client).
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
}
