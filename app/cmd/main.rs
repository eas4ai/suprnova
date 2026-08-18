//! suprnova Application Entry Point

use app::{bootstrap, config, migrations, routes, schedule};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .schedule(schedule::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
