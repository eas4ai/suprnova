//! Resolves the database connection from the container by its type, so the
//! plan must show `database`.

use suprnova::DatabaseConnection;
use suprnova::live::{LiveComponent, live};

/// Whether the application has a database connection bound.
#[derive(LiveComponent)]
#[live(name = "acme.inventory", view = "acme-ui/inventory/inventory.html")]
pub struct Inventory {
    /// Whether a connection resolved.
    #[public]
    connected: bool,
}

#[live]
impl Inventory {
    /// Resolves the connection once.
    #[mount]
    pub fn mount() -> Self {
        let connection = suprnova::App::make::<DatabaseConnection>();
        Self {
            connected: connection.is_ok(),
        }
    }
}
