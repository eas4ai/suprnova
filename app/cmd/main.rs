//! suprnova Application Entry Point

use app::{agents, bootstrap, config, live, migrations, routes, schedule};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        // The routes with the MCP endpoint and both agents plugins
        // (`src/agents.rs`), then Live and the RenderCache. Serving the
        // agents fails without the signing key, which stops the server
        // here rather than on an agent's first request.
        .try_routes_async(|| async {
            live::routes_with_render_cache(agents::serve(routes::register())?).await
        })
        .schedule(schedule::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
