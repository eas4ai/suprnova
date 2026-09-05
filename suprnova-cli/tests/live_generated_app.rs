//! A freshly generated application is Live-ready: it ships the Live module,
//! binds the registry, installs the guarded reserved routes, scaffolds a
//! component through `live:make`, depends only on `suprnova`, and (in the
//! ignored acceptance test) builds and passes the integrated checker through
//! the real console binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_suprnova");

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref())
        .unwrap_or_else(|e| panic!("read {}: {e}", path.as_ref().display()))
}

fn scaffold(tmp: &Path, name: &str) -> PathBuf {
    let output = Command::new(BIN)
        .args([
            "new",
            name,
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp)
        .output()
        .expect("suprnova new");
    assert!(
        output.status.success(),
        "suprnova new failed: {}",
        combined(&output)
    );
    tmp.join(name)
}

fn live_make(project: &Path, name: &str) -> Output {
    Command::new(BIN)
        .args(["live:make", name])
        .current_dir(project)
        .output()
        .expect("suprnova live:make")
}

#[test]
fn a_generated_application_is_live_ready() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(tmp.path(), "live_ready");

    let module = read(project.join("src/live/mod.rs"));
    assert!(module.contains("pub fn registry()"), "{module}");
    assert!(module.contains("pub fn routes(router: Router)"), "{module}");
    assert!(module.contains("try_live_with"), "{module}");
    // The RenderCache install is the asynchronous half, separate from
    // `routes` because it probes for the generation ledger before it
    // assembles a runtime.
    assert!(
        module.contains("pub async fn routes_with_render_cache(router: Router)"),
        "{module}"
    );
    assert!(
        module.contains("RenderCache::install(routes(router)?, RenderCacheConfig::from_env())"),
        "{module}"
    );
    assert!(module.contains("AuthMiddleware::new()"), "{module}");
    assert!(module.contains("LiveTenantMiddleware"), "{module}");
    assert!(module.contains("RateLimitMiddleware::new("), "{module}");
    assert_eq!(module.matches(".build()").count(), 1);

    let lib = read(project.join("src/lib.rs"));
    assert_eq!(lib.matches("pub mod live;").count(), 1, "{lib}");
    let bootstrap = read(project.join("src/bootstrap.rs"));
    assert!(bootstrap.contains("crate::live::registry()"), "{bootstrap}");
    // Live verifies the browser's origin proof on its own; the scaffold keeps
    // the default policy so ordinary routes keep token validation.
    assert!(
        bootstrap.contains("global_middleware!(CsrfMiddleware::new());"),
        "{bootstrap}"
    );
    assert!(!bootstrap.contains("with_origin_policy"), "{bootstrap}");
    let main = read(project.join("cmd/main.rs"));
    assert!(
        main.contains(
            ".try_routes_async(|| async { live::routes_with_render_cache(routes::register()).await })"
        ),
        "{main}"
    );
    // Without the framework's own migration in the generated Migrator, the
    // install's boot probe fails the first time a generated application
    // runs `suprnova serve`.
    let migrator = read(project.join("src/migrations/mod.rs"));
    assert!(
        migrator.contains("Box::new(suprnova::render_cache::migration::Migration),"),
        "{migrator}"
    );

    let output = live_make(&project, "Counter");
    assert!(output.status.success(), "{}", combined(&output));
    let module = read(project.join("src/live/mod.rs"));
    assert!(module.contains("pub mod counter;"), "{module}");
    assert!(
        module.contains(".register::<counter::Counter>()?"),
        "{module}"
    );
    assert_eq!(module.matches(".build()").count(), 1);
    assert!(project.join("src/live/counter.rs").exists());
    assert!(project.join("templates/live/counter.html").exists());
    let component = read(project.join("src/live/counter.rs"));
    assert!(
        component.contains("name = \"live_ready.counter\""),
        "{component}"
    );

    let manifest = read(project.join("Cargo.toml"));
    let framework_lines = manifest
        .lines()
        .filter(|line| line.trim_start().starts_with("suprnova"))
        .count();
    assert_eq!(
        framework_lines, 1,
        "exactly one framework dependency: {manifest}"
    );
    for forbidden in [
        "suprnova-live",
        "suprnova_live",
        "askama",
        "suprnova-macros",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "{forbidden} must not be a direct dependency"
        );
    }
    for file in [
        "src/live/mod.rs",
        "src/live/counter.rs",
        "src/bootstrap.rs",
        "cmd/main.rs",
    ] {
        let source = read(project.join(file));
        assert!(
            !source.contains("suprnova_live"),
            "{file} names only the public facade"
        );
        assert!(
            !source.contains("askama"),
            "{file} names no Askama internals"
        );
    }
}

/// Point the scaffolded manifest at the in-tree framework and detach it from
/// the surrounding workspace, exactly as `scaffold_snapshot.rs` does.
fn patch_local_suprnova(project: &Path) {
    let framework_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("framework");
    let cargo_toml = project.join("Cargo.toml");
    let original = read(&cargo_toml);
    let mut rewritten = String::with_capacity(original.len() + 32);
    let mut replaced = false;
    let mut has_workspace = false;
    for line in original.lines() {
        if line.trim() == "[workspace]" {
            has_workspace = true;
        }
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!(
                "suprnova = {{ path = \"{}\" }}\n",
                framework_dir.display()
            ));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert!(
        replaced,
        "scaffolded Cargo.toml must declare the suprnova dependency"
    );
    if !has_workspace {
        rewritten.push_str("\n[workspace]\n");
    }
    fs::write(&cargo_toml, rewritten).expect("write patched Cargo.toml");
}

#[test]
#[ignore = "acceptance: builds a generated application and its console; slow"]
fn a_generated_live_application_builds_and_passes_the_integrated_checker() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project = scaffold(tmp.path(), "live_accept");
    let output = live_make(&project, "Counter");
    assert!(output.status.success(), "{}", combined(&output));
    patch_local_suprnova(&project);
    let database = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("live_accept.sqlite").display()
    );

    // `live:check` builds and runs the application's console binary, so it is
    // also the compile proof: the component, its registration, the guarded
    // routes, and the bootstrap all compile against the public facade.
    let check = Command::new(BIN)
        .args(["live:check", "--timeout-secs", "2400"])
        .current_dir(&project)
        .env("DATABASE_URL", &database)
        .env("APP_ENV", "testing")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("suprnova live:check");
    let text = combined(&check);
    assert!(check.status.success(), "{text}");
    assert!(text.contains("1 component"), "{text}");
    assert!(text.contains("Every Live view is proved"), "{text}");

    let inspect = Command::new(BIN)
        .args(["live:inspect", "--json", "--timeout-secs", "2400"])
        .current_dir(&project)
        .env("DATABASE_URL", &database)
        .env("APP_ENV", "testing")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("suprnova live:inspect");
    assert!(inspect.status.success(), "{}", combined(&inspect));
    let report: serde_json::Value = serde_json::from_slice(&inspect.stdout).expect("inspect JSON");
    assert_eq!(report["runtime"]["registry_bound"], true);
    assert_eq!(report["runtime"]["components"], 1);
    assert_eq!(report["components"][0]["name"], "live_accept.counter");
    assert_eq!(report["components"][0]["view"], "live/counter.html");
}
