//! Real application server for the Live browser scenario.
//!
//! The Playwright suite in `crates/suprnova-live/browser` starts this binary
//! and drives the dogfood routes exactly as a deployment would: the
//! production global middleware stack from `bootstrap::register_http_stack`,
//! the guarded reserved Live routes from `app::live::routes`, an in-memory
//! database with the application's migrations, and one demo user the browser
//! signs in as through `/live/demo-login`.

use std::sync::Arc;

use app::live::components::activity_feed::ActivityPosted;
use app::live::providers::upload_finalizer::AppUploadFinalizer;
use app::migrations::Migrator;
use app::models::users::User;
use app::providers::DatabaseUserProvider;
use sea_orm_migration::MigratorTrait as _;
use suprnova::live::{CanonicalValue, LiveEventTarget, LiveStreams, LiveUploadHost};
use suprnova::{
    App, Auth, AuthConfig, AuthManager, Crypt, EloquentUserProvider, EncryptionKey, HttpResponse,
    Model, Request, Router, Server, UserProvider, attrs, bind,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = std::env::var("SUPRNOVA_LIVE_DOGFOOD_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(4178);
    Crypt::init(EncryptionKey::generate());
    App::init();

    let connection = sea_orm::Database::connect("sqlite::memory:").await?;
    Migrator::up(&connection, None).await?;
    App::singleton(suprnova::DbConnection::from_raw(connection));
    bind!(dyn UserProvider, DatabaseUserProvider);
    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))?;
    let demo = User::create(attrs! {
        name: "Live Demo",
        email: "live-demo@example.suprnova.app",
        password: "not-a-real-password",
    })
    .await?;
    let demo_id = demo.id.to_string();

    App::singleton(app::live::registry()?);
    App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
    app::live::providers::authorize_live();
    app::bootstrap::register_http_stack();

    let router: Router = Router::new()
        .get("/health", |_request: Request| async {
            Ok(HttpResponse::text("ok"))
        })
        .get("/live/demo-login", move |_request: Request| {
            let demo_id = demo_id.clone();
            async move {
                Auth::login_using_id(&demo_id, false)
                    .await
                    .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
                Ok(HttpResponse::new().status(303).header("Location", "/live"))
            }
        })
        .get("/live/demo-post", |_request: Request| async {
            let streams = LiveStreams::resolve()
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            streams
                .event::<ActivityPosted>(
                    "activity",
                    LiveEventTarget::Island,
                    CanonicalValue::String("posted".to_owned()),
                )
                .await
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            streams
                .refresh("activity")
                .await
                .map_err(|error| HttpResponse::text(error.to_string()).status(500))?;
            Ok(HttpResponse::text("posted"))
        })
        .into();
    let router = app::live::routes(router)?;
    Server::new(router).host("127.0.0.1").port(port).run().await
}
