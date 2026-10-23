//! Audit the dogfood public entry pages through the application stack.

use std::process::Command;
use std::sync::Arc;
use suprnova::testing::{PageBudget, TestClient, TestDatabase};
use suprnova::{App, MiddlewareRegistry};

#[test]
fn budget_public_pages() -> Result<(), Box<dyn std::error::Error>> {
    // Configure the child before it starts runtime threads, without mutating a live environment.
    if std::env::var_os("SUPRNOVA_BUDGET_TEST_CHILD").is_none() {
        let status = Command::new(std::env::current_exe()?)
            .args(["--exact", "budget_public_pages", "--nocapture"])
            .env("SUPRNOVA_BUDGET_TEST_CHILD", "1")
            .env("APP_ENV", "testing")
            .env("APP_URL", "http://localhost")
            .env("SESSION_DRIVER", "database")
            .status()?;
        if !status.success() {
            return Err("page budget test child failed".into());
        }
        return Ok(());
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            suprnova::testing::install_test_encryption_key();
            suprnova::testing::install_test_agent_auth_key();
            let _db = TestDatabase::fresh::<app::migrations::Migrator>().await?;
            App::bind_if_absent::<dyn suprnova::CacheStore>(Arc::new(
                suprnova::InMemoryCache::new(),
            ));
            app::bootstrap::register_auth()?;
            App::boot_services()?;
            app::bootstrap::try_register_http_stack()?;
            app::bootstrap::register_inertia_shared_data();
            let client =
                TestClient::new(app::routes::register(), MiddlewareRegistry::from_global());
            for page in ["/", "/login", "/register", "/ssr-opt-out-example", "/users"] {
                client
                    .audit_page(page, &PageBudget::default())
                    .await?
                    .ensure_passes()?;
            }
            // The app uses the same fallback as the kits when Vite is absent.
            let public = tempfile::tempdir()
                .map_err(|e| suprnova::FrameworkError::internal(e.to_string()))?;
            std::fs::write(public.path().join("main-BjmpJyAU.js"), "export {};")
                .map_err(|e| suprnova::FrameworkError::internal(e.to_string()))?;
            suprnova::use_public_path(public.path());
            let origin =
                TestClient::new(app::routes::register(), MiddlewareRegistry::from_global());
            let asset = origin.get("/main-BjmpJyAU.js").send().await;
            asset.assert_ok();
            assert_eq!(
                asset.header("Cache-Control"),
                Some("public, max-age=31536000, immutable")
            );
            assert_eq!(asset.header("Set-Cookie"), None);
            Ok::<(), suprnova::FrameworkError>(())
        })?;
    Ok(())
}
