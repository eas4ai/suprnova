//! Application Bootstrap
//!
//! Registers global middleware and services before the server starts accepting requests.
//!
//! Middleware order matters. Requests flow through the stack top-to-bottom;
//! responses travel back bottom-to-top:
//!
//!   BearerTokenMiddleware -> IncludeMiddleware -> handlers
//!
//! # Switching databases
//!
//! SQLite is the default. To use PostgreSQL set `DATABASE_URL` in `.env`:
//!
//!   DATABASE_URL=postgres://user:pass@localhost:5432/{package_name}
//!
//! Magnetar reuses the same connection; no second database configuration is required.
//!
//! # Passkey / WebAuthn
//!
//! Override the relying-party domain and origin via environment variables
//! (`PASSKEY_RP_ID`, `PASSKEY_RP_ORIGIN`) when deploying. Both default to
//! `localhost` / `http://localhost` so local development works without
//! extra config.

use suprnova::{
    BearerTokenMiddleware, DB, IncludeMiddleware, MagnetarConfig, PasskeyConfig,
    boot::initialize_crypt_or_exit, global_middleware, init_magnetar,
};

/// Register global middleware and services.
///
/// Called from `main()` before the server starts.
pub async fn register() {
    // Magnetar's encryptor is constructed during this hook, before the HTTP
    // server exists. Worker and console processes call the same hook, so this
    // is the shared boundary where Crypt must be ready for every real process
    // bootstrap. Console help/version skip the hook and need no APP_KEY.
    initialize_crypt_or_exit();

    // Initialise the database connection pool.
    DB::init().await.expect("Failed to connect to database");

    let db = DB::connection().expect("DB not initialized");
    let magnetar = MagnetarConfig::from_sea_orm(db.inner().clone()).passkey_config(
        PasskeyConfig {
            rp_id: std::env::var("PASSKEY_RP_ID")
                .unwrap_or_else(|_| "localhost".to_string()),
            rp_origin: std::env::var("PASSKEY_RP_ORIGIN")
                .unwrap_or_else(|_| "http://localhost".to_string()),
        },
    );
    init_magnetar(magnetar)
        .await
        .expect("Failed to initialize Magnetar");

    // Bearer-token authentication -- reads Authorization: Bearer <token> and
    // populates the authenticated user in request context.
    global_middleware!(BearerTokenMiddleware);

    // Include middleware -- parses ?include=relation1,relation2 and
    // ?fields[type]=field1,field2 query parameters into task-locals
    // so Resource::collection / Resource::single can resolve them.
    global_middleware!(IncludeMiddleware);
}
