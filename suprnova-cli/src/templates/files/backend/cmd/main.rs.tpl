//! Suprnova Application Entry Point

use suprnova::Application;

use {package_name}::{bootstrap, config, live, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .try_routes_async(|| async { live::routes_with_render_cache(routes::register()).await })
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
