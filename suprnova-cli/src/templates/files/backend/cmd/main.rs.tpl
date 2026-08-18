//! Suprnova Application Entry Point

use suprnova::Application;

use {package_name}::{bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
