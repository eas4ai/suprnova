//! Session storage drivers

pub mod database;
// The MongoDB driver (PAR-188), behind `database-mongodb`.
#[cfg(feature = "database-mongodb")]
pub mod mongodb;

#[cfg(feature = "database-mongodb")]
pub use self::mongodb::{DEFAULT_MONGO_SESSIONS_COLLECTION, MongoSessionDriver};
pub use database::DatabaseSessionDriver;

use std::sync::Arc;

use super::config::{SessionConfig, SessionDriver};
use super::store::SessionStore;

/// The store [`SessionConfig::driver`] names, built as the session
/// middleware builds it. Infallible, as the middleware's constructor is: a
/// driver whose backend is missing fails its first call, naming why.
pub(crate) fn configured_store(config: &SessionConfig) -> Arc<dyn SessionStore> {
    match config.driver {
        SessionDriver::Database => Arc::new(DatabaseSessionDriver::with_configured_table(
            config.lifetime,
            config.table_name.clone(),
        )),
        #[cfg(feature = "database-mongodb")]
        SessionDriver::MongoDb => Arc::new(MongoSessionDriver::from_config(config)),
    }
}
