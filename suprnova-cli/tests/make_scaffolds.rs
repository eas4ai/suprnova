//! The `make:*` generators against Laravel's: `make:middleware` with a
//! pass-through body, nested names and `--test`, `make:view`, and
//! `make:inertia` with nested names, `--force` and `--test`.
//!
//! Each test runs the built `suprnova` binary in a fresh temporary
//! directory laid out as much like a `suprnova new` project as the command
//! needs.

mod laravel_http_gaps {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};

    use tempfile::TempDir;

    /// What `suprnova new` writes to `src/middleware/mod.rs`.
    const MIDDLEWARE_MOD: &str = "//! Application middleware\n//!\n//! Each middleware has its own dedicated file following the framework convention.\n\npub mod authenticate;\nmod logging;\n\npub use logging::LoggingMiddleware;\n";

    /// What `suprnova new` writes to `src/lib.rs`.
    const LIB_RS: &str = "pub mod actions;\npub mod bootstrap;\npub mod commands;\npub mod config;\npub mod controllers;\npub mod live;\npub mod middleware;\npub mod migrations;\npub mod models;\npub mod props;\npub mod routes;\n";

    fn run(dir: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .args(args)
            .env_remove("SUPRNOVA_FRONTEND")
            .current_dir(dir)
            .output()
            .expect("the suprnova binary runs")
    }

    fn succeeds(dir: &Path, args: &[&str]) -> String {
        let output = run(dir, args);
        let text = combined(&output);
        assert!(
            output.status.success(),
            "`suprnova {args:?}` failed:\n{text}"
        );
        text
    }

    fn combined(output: &Output) -> String {
        String::from_utf8_lossy(&output.stdout).into_owned()
            + &String::from_utf8_lossy(&output.stderr)
    }

    fn read(path: impl AsRef<Path>) -> String {
        let path = path.as_ref();
        fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    fn write(path: impl AsRef<Path>, content: &str) {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create the parent directory");
        }
        fs::write(path, content).expect("write the fixture");
    }

    /// A project root as `suprnova new` lays one out, as far as the
    /// generators read it.
    fn project() -> TempDir {
        let tmp = TempDir::new().expect("a temporary directory");
        write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"shop-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        );
        write(tmp.path().join("src/lib.rs"), LIB_RS);
        write(tmp.path().join("src/middleware/mod.rs"), MIDDLEWARE_MOD);
        write(tmp.path().join("templates/app.html"), "<!DOCTYPE html>\n");
        fs::create_dir_all(tmp.path().join("frontend/src/pages")).expect("the pages directory");
        tmp
    }

    /// Every file under `root`, relative to it, sorted.
    fn files(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).expect("read a directory") {
                let path = entry.expect("a directory entry").path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    found.push(path.strip_prefix(root).expect("under root").to_path_buf());
                }
            }
        }
        found.sort();
        found
    }

    fn count(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    // ── make:middleware ─────────────────────────────────────────────

    /// The body of the generated `handle`, between its braces.
    fn handle_body(source: &str) -> String {
        let start = source
            .find("async fn handle(")
            .expect("the middleware has a handle method");
        let open = start + source[start..].find('{').expect("handle opens a body");
        let close = open + source[open..].find('}').expect("handle closes its body");
        source[open + 1..close].trim().to_string()
    }

    #[test]
    fn make_middleware_writes_a_handle_that_returns_next() {
        let tmp = TempDir::new().expect("a temporary directory");
        succeeds(tmp.path(), &["make:middleware", "Audit"]);

        let source = read(tmp.path().join("src/middleware/audit.rs"));
        assert!(!source.contains("println!"), "{source}");
        assert_eq!(handle_body(&source), "next(request).await", "{source}");
        assert!(source.contains("pub struct AuditMiddleware;"), "{source}");
        assert!(
            source.contains("impl Middleware for AuditMiddleware"),
            "{source}"
        );

        let module = read(tmp.path().join("src/middleware/mod.rs"));
        assert!(module.contains("mod audit;"), "{module}");
        assert!(
            module.contains("pub use audit::AuditMiddleware;"),
            "{module}"
        );
    }

    #[test]
    fn make_middleware_nested_writes_under_its_directory_and_declares_it() {
        let tmp = project();
        let before = files(tmp.path());
        succeeds(tmp.path(), &["make:middleware", "Admin/EnsureRole"]);

        let source = read(tmp.path().join("src/middleware/admin/ensure_role.rs"));
        assert_eq!(handle_body(&source), "next(request).await", "{source}");
        assert!(
            source.contains("pub struct EnsureRoleMiddleware;"),
            "{source}"
        );

        let admin = read(tmp.path().join("src/middleware/admin/mod.rs"));
        assert!(admin.contains("mod ensure_role;"), "{admin}");
        assert!(
            admin.contains("pub use ensure_role::EnsureRoleMiddleware;"),
            "{admin}"
        );
        let root = read(tmp.path().join("src/middleware/mod.rs"));
        assert!(root.contains("pub mod admin;"), "{root}");
        assert!(
            root.contains("pub mod authenticate;\nmod logging;"),
            "the existing declarations stay: {root}"
        );
        assert!(
            root.contains("pub use logging::LoggingMiddleware;"),
            "{root}"
        );

        let created: Vec<PathBuf> = files(tmp.path())
            .into_iter()
            .filter(|path| !before.contains(path))
            .collect();
        assert_eq!(
            created,
            [
                PathBuf::from("src/middleware/admin/ensure_role.rs"),
                PathBuf::from("src/middleware/admin/mod.rs"),
            ],
            "nothing is written outside src/middleware/admin/"
        );

        // A second middleware in the same directory joins the first.
        succeeds(tmp.path(), &["make:middleware", "Admin\\AuditTrail"]);
        let admin = read(tmp.path().join("src/middleware/admin/mod.rs"));
        assert!(
            admin.contains("mod ensure_role;\nmod audit_trail;"),
            "{admin}"
        );
        assert!(
            admin.contains("pub use audit_trail::AuditTrailMiddleware;"),
            "{admin}"
        );
        let root = read(tmp.path().join("src/middleware/mod.rs"));
        assert_eq!(count(&root, "pub mod admin;"), 1, "{root}");
    }

    #[test]
    fn make_middleware_declares_each_level_of_a_deeper_name() {
        let tmp = project();
        succeeds(tmp.path(), &["make:middleware", "Admin/Billing/EnsurePaid"]);
        assert!(
            tmp.path()
                .join("src/middleware/admin/billing/ensure_paid.rs")
                .is_file()
        );
        assert!(read(tmp.path().join("src/middleware/mod.rs")).contains("pub mod admin;"));
        assert!(read(tmp.path().join("src/middleware/admin/mod.rs")).contains("pub mod billing;"));
        let billing = read(tmp.path().join("src/middleware/admin/billing/mod.rs"));
        assert!(
            billing.contains("pub use ensure_paid::EnsurePaidMiddleware;"),
            "{billing}"
        );
    }

    #[test]
    fn make_middleware_test_flag_writes_a_test_that_runs_it() {
        let tmp = project();
        succeeds(
            tmp.path(),
            &["make:middleware", "Admin/EnsureRole", "--test"],
        );

        let test = read(tmp.path().join("tests/admin_ensure_role_middleware.rs"));
        assert!(
            test.contains("use shop_app::middleware::admin::EnsureRoleMiddleware;"),
            "{test}"
        );
        assert!(test.contains("TestClient::new("), "{test}");
        assert!(
            test.contains("MiddlewareRegistry::new().append(EnsureRoleMiddleware)"),
            "{test}"
        );
        assert!(test.contains("#[tokio::test]"), "{test}");

        succeeds(tmp.path(), &["make:middleware", "Audit", "--test"]);
        let test = read(tmp.path().join("tests/audit_middleware.rs"));
        assert!(
            test.contains("use shop_app::middleware::AuditMiddleware;"),
            "{test}"
        );
    }

    #[test]
    fn make_middleware_test_flag_outside_a_project_writes_nothing() {
        let tmp = TempDir::new().expect("a temporary directory");
        let output = run(tmp.path(), &["make:middleware", "Audit", "--test"]);
        assert!(!output.status.success(), "{}", combined(&output));
        assert!(
            combined(&output).contains("Cargo.toml"),
            "{}",
            combined(&output)
        );
        assert!(files(tmp.path()).is_empty(), "nothing is written");
    }

    #[test]
    fn make_middleware_keeps_an_existing_file_and_refuses_bad_names() {
        let tmp = project();
        succeeds(tmp.path(), &["make:middleware", "Audit"]);
        write(tmp.path().join("src/middleware/audit.rs"), "// edited\n");
        let output = run(tmp.path(), &["make:middleware", "Audit"]);
        assert!(!output.status.success());
        assert_eq!(
            read(tmp.path().join("src/middleware/audit.rs")),
            "// edited\n"
        );

        let before = files(tmp.path());
        for bad in [
            "Admin//X",
            "../X",
            "Admin/type/X",
            "Admin/1st/X",
            "Middleware",
        ] {
            let output = run(tmp.path(), &["make:middleware", bad]);
            assert!(!output.status.success(), "{bad}: {}", combined(&output));
        }
        assert_eq!(files(tmp.path()), before, "a refused name writes nothing");
    }

    // ── make:view ───────────────────────────────────────────────────

    #[test]
    fn make_view_dotted_and_slashed_names_write_the_same_view() {
        for name in ["admin.dashboard", "admin/dashboard"] {
            let tmp = project();
            succeeds(tmp.path(), &["make:view", name]);

            let template = read(tmp.path().join("templates/admin/dashboard.html"));
            assert!(template.contains("{{ title }}"), "{name}: {template}");
            let view = read(tmp.path().join("src/views/admin/dashboard.rs"));
            assert!(
                view.contains("#[suprnova::view(path = \"admin/dashboard.html\")]"),
                "{name}: {view}"
            );
            assert!(
                view.contains("pub struct DashboardView {"),
                "{name}: {view}"
            );
            assert!(view.contains("pub title: String,"), "{name}: {view}");

            let views = read(tmp.path().join("src/views/mod.rs"));
            assert!(views.contains("pub mod admin;"), "{name}: {views}");
            let admin = read(tmp.path().join("src/views/admin/mod.rs"));
            assert!(admin.contains("pub mod dashboard;"), "{name}: {admin}");
            let lib = read(tmp.path().join("src/lib.rs"));
            assert!(lib.contains("pub mod views;"), "{name}: {lib}");
            assert!(lib.starts_with(LIB_RS.trim_end()), "{name}: {lib}");
        }
    }

    #[test]
    fn make_view_keeps_existing_files_unless_forced() {
        let tmp = project();
        succeeds(tmp.path(), &["make:view", "admin.dashboard"]);
        let template = tmp.path().join("templates/admin/dashboard.html");
        let view = tmp.path().join("src/views/admin/dashboard.rs");
        write(&template, "<p>edited</p>\n");
        let generated_view = read(&view);

        let text = succeeds(tmp.path(), &["make:view", "admin/dashboard"]);
        assert!(
            text.contains("--force"),
            "the refusal names the flag: {text}"
        );
        assert_eq!(read(&template), "<p>edited</p>\n", "not overwritten");
        assert_eq!(read(&view), generated_view);

        // The view file alone existing keeps the template from being written.
        fs::remove_file(&template).expect("remove the template");
        succeeds(tmp.path(), &["make:view", "admin.dashboard"]);
        assert!(
            !template.exists(),
            "nothing is written while one file exists"
        );

        write(&template, "<p>edited</p>\n");
        succeeds(tmp.path(), &["make:view", "admin.dashboard", "--force"]);
        assert!(
            read(&template).contains("{{ title }}"),
            "--force overwrites"
        );
        let lib = read(tmp.path().join("src/lib.rs"));
        assert_eq!(count(&lib, "pub mod views;"), 1, "{lib}");
        let views = read(tmp.path().join("src/views/mod.rs"));
        assert_eq!(count(&views, "pub mod admin;"), 1, "{views}");
        let admin = read(tmp.path().join("src/views/admin/mod.rs"));
        assert_eq!(count(&admin, "pub mod dashboard;"), 1, "{admin}");
    }

    #[test]
    fn make_view_test_flag_writes_a_test_that_renders_it() {
        let tmp = project();
        succeeds(tmp.path(), &["make:view", "admin.dashboard", "--test"]);
        let test = read(tmp.path().join("tests/admin_dashboard_view.rs"));
        assert!(
            test.contains("use shop_app::views::admin::dashboard::DashboardView;"),
            "{test}"
        );
        assert!(test.contains("use suprnova::view::ViewTemplate;"), "{test}");
        assert!(test.contains(".render_view(&mut html)"), "{test}");
        assert!(test.contains("<h1>Rendered heading</h1>"), "{test}");

        succeeds(tmp.path(), &["make:view", "welcome", "--test"]);
        let test = read(tmp.path().join("tests/welcome_view.rs"));
        assert!(
            test.contains("use shop_app::views::welcome::WelcomeView;"),
            "{test}"
        );
    }

    #[test]
    fn make_view_refuses_bad_names_and_writes_nothing() {
        let tmp = project();
        let before = files(tmp.path());
        for bad in ["admin..x", "admin/type", "../etc", "admin/1st", "a b"] {
            let output = run(tmp.path(), &["make:view", bad]);
            assert!(!output.status.success(), "{bad}: {}", combined(&output));
        }
        assert_eq!(files(tmp.path()), before);
    }

    // ── make:inertia ────────────────────────────────────────────────

    #[test]
    fn make_inertia_nested_with_force_and_test_is_accepted() {
        let tmp = project();
        for _ in 0..2 {
            succeeds(
                tmp.path(),
                &["make:inertia", "Admin/Users", "--force", "--test"],
            );
        }
        let page = read(tmp.path().join("frontend/src/pages/Admin/UsersPage.svelte"));
        assert!(
            page.contains("frontend/src/pages/Admin/UsersPage.svelte"),
            "{page}"
        );
        let test = read(tmp.path().join("tests/admin_users_page.rs"));
        assert!(
            test.contains("inertia_response!(&req, \"Admin/UsersPage\", {})"),
            "{test}"
        );
        assert!(
            test.contains(".component_exists(\"Admin/UsersPage\", true)"),
            "{test}"
        );
    }

    #[test]
    fn make_inertia_keeps_an_existing_page_unless_forced() {
        let tmp = project();
        succeeds(tmp.path(), &["make:inertia", "Admin/Users"]);
        let page = tmp.path().join("frontend/src/pages/Admin/UsersPage.svelte");
        write(&page, "<p>edited</p>\n");

        let text = succeeds(tmp.path(), &["make:inertia", "Admin/Users"]);
        assert!(text.contains("--force"), "{text}");
        assert_eq!(read(&page), "<p>edited</p>\n");

        succeeds(tmp.path(), &["make:inertia", "Admin/Users", "--force"]);
        assert!(read(&page).contains("UsersPage"), "--force overwrites");
    }

    #[test]
    fn make_inertia_nested_react_page_names_its_function_by_the_last_segment() {
        let tmp = project();
        let output = Command::new(env!("CARGO_BIN_EXE_suprnova"))
            .args(["make:inertia", "Admin/Users"])
            .env("SUPRNOVA_FRONTEND", "react")
            .current_dir(tmp.path())
            .output()
            .expect("the suprnova binary runs");
        assert!(output.status.success(), "{}", combined(&output));
        let page = read(tmp.path().join("frontend/src/pages/Admin/UsersPage.tsx"));
        assert!(
            page.contains("export default function UsersPage()"),
            "{page}"
        );
    }

    #[test]
    fn make_inertia_refuses_test_with_data_and_bad_directories() {
        let tmp = project();
        let before = files(tmp.path());
        let output = run(
            tmp.path(),
            &["make:inertia", "UserProps", "--data", "--test"],
        );
        assert!(!output.status.success(), "{}", combined(&output));
        for bad in ["../Users", "Admin//Users", "1st/Users", "Admin Area/Users"] {
            let output = run(tmp.path(), &["make:inertia", bad]);
            assert!(!output.status.success(), "{bad}: {}", combined(&output));
        }
        assert_eq!(files(tmp.path()), before);
    }

    // ── what every generator writes ─────────────────────────────────

    /// `scaffold_snapshot` rejects these in `suprnova new`'s output; the
    /// generators that add to a project hold the same line.
    #[test]
    fn generated_files_carry_no_stub_markers() {
        let tmp = project();
        let before = files(tmp.path());
        succeeds(
            tmp.path(),
            &["make:middleware", "Admin/EnsureRole", "--test"],
        );
        succeeds(tmp.path(), &["make:view", "admin.dashboard", "--test"]);
        succeeds(tmp.path(), &["make:inertia", "Admin/Users", "--test"]);
        for path in files(tmp.path()) {
            if before.contains(&path) && path != Path::new("src/middleware/mod.rs") {
                continue;
            }
            let content = read(tmp.path().join(&path));
            for marker in ["TODO", "FIXME", "unimplemented!", "panic!("] {
                assert!(
                    !content.contains(marker),
                    "{} contains {marker}:\n{content}",
                    path.display()
                );
            }
        }
    }
}
