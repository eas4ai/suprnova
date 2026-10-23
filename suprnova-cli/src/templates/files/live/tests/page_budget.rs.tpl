//! Public entry pages and their same-origin assets share the default budget.

use std::process::Command;
use std::sync::Arc;
use suprnova::testing::{PageBudget, TestClient, TestDatabase};
use suprnova::{App, AppConfig, Auth, AuthConfig, AuthManager, CacheStore, Config, EloquentUserProvider, InMemoryCache, MiddlewareRegistry};

#[test]
fn budget_public_pages() -> Result<(), Box<dyn std::error::Error>> {
    // Set the environment before the child starts its runtime and framework threads.
    if std::env::var_os("SUPRNOVA_BUDGET_TEST_CHILD").is_none() {
        let status = Command::new(std::env::current_exe()?)
            .args(["--exact", "budget_public_pages", "--nocapture"])
            .env("SUPRNOVA_BUDGET_TEST_CHILD", "1")
            .env("APP_ENV", "testing")
            .env("APP_URL", "http://localhost")
            .env("SESSION_DRIVER", "database")
            .status()?;
        if !status.success() { return Err("page budget test child failed".into()); }
        return Ok(());
    }
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        suprnova::testing::install_test_encryption_key();
        suprnova::testing::install_test_agent_auth_key();
        Config::register(AppConfig::builder().url("http://localhost").build());
        let _db = TestDatabase::fresh::<{package_name}::migrations::Migrator>().await?;
        App::singleton(AuthManager::new(AuthConfig::from_env()));
        Auth::register_provider("users", Arc::new(EloquentUserProvider::<{package_name}::models::user::User>::new()))?;
        App::bind_if_absent::<dyn CacheStore>(Arc::new(InMemoryCache::new()));
        App::singleton({package_name}::live::registry()?);
        {package_name}::live::authorize();
        {package_name}::bootstrap::try_register_http_stack()?;
        let router = {package_name}::live::routes_with_render_cache(
            {package_name}::agents::serve({package_name}::routes::register())?
        ).await?;
        App::singleton(suprnova::live::testing::prepare_live_router_for_test(&router)?);
        let client = TestClient::new(router, MiddlewareRegistry::from_global());
        for page in ["/", "/login", "/register", "/forgot-password", "/reset-password"] {
            client.audit_page(page, &PageBudget::default()).await?.ensure_passes()?;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    Ok(())
}
