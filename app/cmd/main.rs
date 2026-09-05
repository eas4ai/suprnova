//! suprnova Application Entry Point

use app::{bootstrap, config, live, migrations, routes, schedule};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .try_routes_async(|| async { live::routes_with_render_cache(routes::register()).await })
        .schedule(schedule::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
