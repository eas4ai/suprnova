//! {project_name} - JSON:API Server Entry Point

use suprnova::Application;

use {package_name}::{agents, bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .try_routes(|| agents::serve(routes::register()))
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
