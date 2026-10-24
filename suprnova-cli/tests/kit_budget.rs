//! The page-bearing scaffolds ship the test and serve their assets anonymously.

use std::process::Command;
use suprnova::testing::{PageBudget, TestClient, TestDatabase};
use suprnova::{
    CsrfMiddleware, HttpResponse, MiddlewareRegistry, Request, Router, SessionConfig,
    SessionMiddleware, StaticFiles, fallback,
};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

#[tokio::test]
async fn budget_every_page_kit_ships_the_public_page_test_and_origin_policy() {
    suprnova::testing::install_test_encryption_key();
    for kit in ["react", "vue", "svelte", "live"] {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(BIN)
            .args([
                "new",
                "budget_app",
                "--no-interaction",
                "--no-git",
                "--frontend",
                kit,
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let root = dir.path().join("budget_app");
        let test = std::fs::read_to_string(root.join("tests/page_budget.rs")).unwrap();
        if kit != "live" {
            assert_eq!(
                test,
                suprnova_cli::templates::page_budget_test("budget_app")
            );
        } else {
            for path in [
                "src/bootstrap.rs",
                "src/migrations/mod.rs",
                "src/models/user.rs",
                "src/live/mod.rs",
            ] {
                assert!(
                    root.join(path).exists(),
                    "missing Live bootstrap dependency: {path}"
                );
            }
            for required in [
                "use budget_app::models::user::User;",
                "use budget_app::{agents, bootstrap, live, migrations, routes};",
                "live::registry()",
                "migrations::Migrator",
                "prepare_live_router_for_test",
                "routes_with_render_cache",
            ] {
                assert!(test.contains(required), "Live budget test needs {required}");
            }
        }
        for required in [
            "routes::register()",
            "bootstrap::try_register_http_stack()",
            "PageBudget::default()",
            "audit_page",
            "/login",
            "/register",
            "/forgot-password",
            "/reset-password",
        ] {
            assert!(test.contains(required), "{kit}: missing {required}");
        }
        assert!(!test.contains("{crate}"));
        syn::parse_file(&test).unwrap();
        let routes = std::fs::read_to_string(root.join("src/routes.rs")).unwrap();
        assert!(routes.contains("fallback!(StaticFiles::public().handler())"));
        // A scaffold has source files before Vite runs. Serve its real entry source
        // under a build-shaped name to exercise the origin policy without a build.
        let entry = if kit == "live" {
            "public/css/app.css"
        } else if kit == "react" {
            "frontend/src/main.tsx"
        } else {
            "frontend/src/main.ts"
        };
        let source = std::fs::read(root.join(entry)).unwrap();
        if kit == "live" {
            assert!(
                std::fs::read_to_string(root.join("src/models/mod.rs"))
                    .unwrap()
                    .contains("user")
            );
        }
        let public = root.join("public");
        std::fs::write(public.join("main-BjmpJyAU.js"), &source).unwrap();
        std::fs::write(public.join("icon.svg"), "<svg/>").unwrap();
        if kit != "live" {
            std::fs::create_dir_all(public.join("css")).unwrap();
            std::fs::write(public.join("css/app.css"), "body{}").unwrap();
        }
        let _db = TestDatabase::sqlite_memory().await.unwrap();
        suprnova::session::migrations::CreateSessionsTable::new(
            suprnova::session::migrations::SessionUserKey::Integer,
        )
        .up(&sea_orm_migration::SchemaManager::new(_db.conn()))
        .await
        .unwrap();
        let config = SessionConfig::default();
        let router = fallback!(StaticFiles::from_dir(&public).handler()).register(Router::new().get("/", |_req: Request| async {
            Ok(HttpResponse::html("<script src='/main-BjmpJyAU.js'></script><link rel='icon' href='/icon.svg'><link rel='stylesheet' href='/css/app.css'>"))
        }).into());
        let client = TestClient::new(
            router,
            MiddlewareRegistry::new()
                .append(SessionMiddleware::new(config.clone()))
                .append(CsrfMiddleware::new().with_session_config(&config)),
        );
        client
            .audit_page("/", &PageBudget::default())
            .await
            .unwrap()
            .ensure_passes()
            .unwrap();
        let asset = client.get("/main-BjmpJyAU.js").send().await;
        assert_eq!(
            asset.header("Cache-Control"),
            Some("public, max-age=31536000, immutable")
        );
        assert_eq!(asset.header("Set-Cookie"), None);
    }
}

/// Each page kit's own `tests/page_budget.rs` builds and passes on a fresh
/// scaffold. The test above only reads that file; this one runs it, against
/// the in-tree framework and this workspace's lock, in the build directory
/// this package already uses, so the framework is not compiled again.
#[test]
#[ignore = "scaffolds and compiles four applications; slow"]
fn budget_every_page_kit_passes_its_own_budget_test() {
    let framework = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI sits in the workspace")
        .join("framework");
    let lock = framework
        .parent()
        .expect("the framework sits in the workspace")
        .join("Cargo.lock");
    let target = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR has a parent")
        .to_path_buf();
    for kit in ["react", "vue", "svelte", "live"] {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(BIN)
            .args([
                "new",
                "budget_app",
                "--no-interaction",
                "--no-git",
                "--frontend",
                kit,
            ])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "{kit}: `suprnova new` failed");
        let root = dir.path().join("budget_app");
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let mut patched = String::new();
        for line in manifest.lines() {
            if line.trim_start().starts_with("suprnova = ") {
                patched.push_str(&format!(
                    "suprnova = {{ path = \"{}\" }}\n",
                    framework.display()
                ));
            } else {
                patched.push_str(line);
                patched.push('\n');
            }
        }
        if !manifest.lines().any(|line| line.trim() == "[workspace]") {
            patched.push_str("\n[workspace]\n");
        }
        std::fs::write(root.join("Cargo.toml"), patched).unwrap();
        std::fs::copy(&lock, root.join("Cargo.lock")).unwrap();
        let run = Command::new(env!("CARGO"))
            .args(["test", "--test", "page_budget"])
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", &target)
            .output()
            .unwrap();
        assert!(
            run.status.success(),
            "{kit}: the generated page budget test failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn budget_hyphenated_project_names_use_the_rust_crate_name() {
    let test = suprnova_cli::templates::page_budget_test("example_app");
    assert!(test.contains("use example_app::{agents, bootstrap, live, migrations, routes};"));
    syn::parse_file(&test).unwrap();
}

use sea_orm_migration::MigrationTrait;
