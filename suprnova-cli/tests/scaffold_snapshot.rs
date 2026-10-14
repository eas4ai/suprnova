//! Golden tests for `suprnova new`.
//!
//! Codex review finding #6 (S4 follow-up): the generated `--api` and
//! default Inertia starters previously shipped TODO-laden bodies, stub
//! `all_example` / `find_example` helpers, and an in-memory Torii config
//! that was unsafe for production. These tests make the absence of those
//! markers a hard guarantee:
//!
//! 1. Walk every generated `.rs`, `.toml`, and template-driven config
//!    file and reject occurrences of `TODO`, `FIXME`, `unimplemented!`,
//!    `panic!(`, or the historical stub helpers.
//! 2. Confirm that a freshly scaffolded project compiles end-to-end by
//!    rewriting its `suprnova` dependency to point at the in-tree
//!    framework crate and running `cargo check`. Marked `#[ignore]` so
//!    `cargo test --workspace` stays fast - run with
//!    `cargo test --workspace -- --ignored` to exercise them.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// Substring markers that must NEVER appear in scaffolder output. We
/// search literal substrings rather than comment-only patterns because
/// the production templates have no legitimate reason to include any
/// of these - every prior occurrence was a punted implementation.
const FORBIDDEN_MARKERS: &[&str] = &[
    "TODO",
    "FIXME",
    "unimplemented!",
    "panic!(",
    "all_example",
    "find_example",
    "sqlite_in_memory",
];

/// File extensions whose contents we audit. The scaffolder writes
/// frontend files (`.tsx`, `.svelte`, `.vue`, `.ts`, `.json`,
/// `.html`, `.css`) plus Rust source and TOML manifests - every one is
/// fair game for stub markers. We deliberately do *not* audit binary
/// blobs or lockfiles.
const AUDITED_EXTENSIONS: &[&str] = &[
    "rs", "toml", "ts", "tsx", "svelte", "vue", "json", "html", "css", "yml", "yaml",
];

/// Paths whose substrings indicate generated / external content we
/// don't own. `target/` and `node_modules/` won't exist before
/// `cargo check` / `npm install`, but the filter is cheap and matches
/// what users would see after a real boot.
const SKIP_PATH_FRAGMENTS: &[&str] = &["/target/", "/node_modules/"];

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_suprnova"))
}

/// Run `suprnova new ...` inside `tmp` and return the project root.
fn scaffold(tmp: &TempDir, project_name: &str, extra_args: &[&str]) {
    let mut cmd = Command::new(cli_binary());
    cmd.arg("new")
        .arg(project_name)
        .arg("--no-interaction")
        .arg("--no-git");
    for a in extra_args {
        cmd.arg(a);
    }
    let output = cmd
        .current_dir(tmp.path())
        .output()
        .expect("`suprnova new` should run");
    assert!(
        output.status.success(),
        "`suprnova new {project_name} {extra_args:?}` failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Walk the scaffolded project and collect any forbidden-marker hits.
fn collect_marker_hits(root: &Path) -> Vec<(PathBuf, &'static str, usize, String)> {
    let mut hits = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let path_str = path.to_string_lossy();
        if SKIP_PATH_FRAGMENTS
            .iter()
            .any(|frag| path_str.contains(frag))
        {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !AUDITED_EXTENSIONS.contains(&ext) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for (line_idx, line) in content.lines().enumerate() {
            for marker in FORBIDDEN_MARKERS {
                if line.contains(marker) {
                    hits.push((
                        path.to_owned(),
                        *marker,
                        line_idx + 1,
                        line.trim().to_string(),
                    ));
                }
            }
        }
    }
    hits
}

fn assert_no_marker_hits(root: &Path) {
    let hits = collect_marker_hits(root);
    assert!(
        hits.is_empty(),
        "scaffolded project under {} contains forbidden stub markers:\n{}",
        root.display(),
        hits.iter()
            .map(|(p, marker, line, text)| {
                format!("  {}:{} [{}] {}", p.display(), line, marker, text)
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
}

// ---------------------------------------------------------------------------
// Stub-marker audits (fast - no compilation involved).
// ---------------------------------------------------------------------------

#[test]
fn api_starter_has_no_stub_markers() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "marker_api", &["--api"]);
    assert_no_marker_hits(&tmp.path().join("marker_api"));
}

#[test]
fn default_starter_has_no_stub_markers_svelte() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "marker_svelte", &["--frontend", "svelte"]);
    assert_no_marker_hits(&tmp.path().join("marker_svelte"));
}

#[test]
fn default_starter_has_no_stub_markers_react() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "marker_react", &["--frontend", "react"]);
    assert_no_marker_hits(&tmp.path().join("marker_react"));
}

#[test]
fn default_starter_has_no_stub_markers_vue() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "marker_vue", &["--frontend", "vue"]);
    assert_no_marker_hits(&tmp.path().join("marker_vue"));
}

#[test]
fn default_starter_has_no_stub_markers_live() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "marker_live", &["--frontend", "live"]);
    assert_no_marker_hits(&tmp.path().join("marker_live"));
}

// ---------------------------------------------------------------------------
// Auth contract audits - the scaffolded model must keep the trait overrides
// the login flow depends on.
// ---------------------------------------------------------------------------

/// The scaffolded `User` must override `Authenticatable::get_auth_password`
/// to expose the stored hash. The trait default returns `None`, which makes
/// `EloquentUserProvider::validate_credentials` reject every password - a
/// freshly scaffolded app's `Auth::attempt` login 422s on correct
/// credentials. The project still compiles without the override, so only a
/// content check catches the regression.
#[test]
fn scaffolded_user_model_exposes_auth_password() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "auth_pw", &["--frontend", "svelte"]);
    let model = tmp.path().join("auth_pw").join("src/models/user.rs");
    let body = std::fs::read_to_string(&model).expect("scaffolded user model should exist");
    assert!(
        body.contains("fn get_auth_password"),
        "scaffolded User must override get_auth_password or Auth::attempt \
         rejects every password:\n{body}"
    );
    assert!(
        body.contains("Some(&self.password)"),
        "get_auth_password must return the stored hash:\n{body}"
    );
}

// ---------------------------------------------------------------------------
// Inertia client pin - the scaffolds must ship a current `@inertiajs/*`
// adapter. A typo or accidental downgrade compiles fine (npm isn't part of
// `cargo check`), so only a content assertion catches drift.
// ---------------------------------------------------------------------------

/// The expected `@inertiajs/*` caret pin every frontend scaffold ships.
/// Bump this in lockstep with the `package.json.tpl` files.
const EXPECTED_INERTIA_PIN: &str = "^3.8.0";

fn assert_inertia_pin(tmp: &TempDir, project: &str, frontend: &str, adapter: &str) {
    scaffold(tmp, project, &["--frontend", frontend]);
    let pkg = tmp
        .path()
        .join(project)
        .join("frontend")
        .join("package.json");
    let body = std::fs::read_to_string(&pkg).expect("scaffolded package.json should exist");
    let needle = format!("\"@inertiajs/{adapter}\": \"{EXPECTED_INERTIA_PIN}\"");
    assert!(
        body.contains(&needle),
        "{frontend} scaffold must pin @inertiajs/{adapter} at {EXPECTED_INERTIA_PIN}:\n{body}"
    );
}

#[test]
fn scaffold_pins_current_inertia_client() {
    let tmp = TempDir::new().unwrap();
    assert_inertia_pin(&tmp, "inertia_svelte", "svelte", "svelte");
    assert_inertia_pin(&tmp, "inertia_react", "react", "react");
    assert_inertia_pin(&tmp, "inertia_vue", "vue", "vue3");
}

// ---------------------------------------------------------------------------
// make:* generator audits - ensure the in-place commands emit clean code.
// ---------------------------------------------------------------------------

#[test]
fn make_middleware_generates_clean_code() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "make_mid", &["--frontend", "svelte"]);
    let project = tmp.path().join("make_mid");

    let status = Command::new(cli_binary())
        .args(["make:middleware", "RateLimit"])
        .current_dir(&project)
        .status()
        .expect("`suprnova make:middleware` should run");
    assert!(status.success(), "make:middleware should succeed");

    assert_no_marker_hits(&project.join("src/middleware/rate_limit.rs"));
}

#[test]
fn make_action_generates_clean_code() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "make_act", &["--frontend", "svelte"]);
    let project = tmp.path().join("make_act");

    let status = Command::new(cli_binary())
        .args(["make:action", "SendInvoice"])
        .current_dir(&project)
        .status()
        .expect("`suprnova make:action` should run");
    assert!(status.success(), "make:action should succeed");

    assert_no_marker_hits(&project.join("src/actions/send_invoice_action.rs"));
}

#[test]
fn make_task_generates_clean_code() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "make_tsk", &["--frontend", "svelte"]);
    let project = tmp.path().join("make_tsk");

    // make:task creates the tasks directory itself if missing.
    let status = Command::new(cli_binary())
        .args(["make:task", "RotateAuditLog"])
        .current_dir(&project)
        .status()
        .expect("`suprnova make:task` should run");
    assert!(status.success(), "make:task should succeed");

    // Walk the project - the generated task file is what we want clean,
    // but a broader sweep also confirms nothing else regressed.
    assert_no_marker_hits(&project);
}

// ---------------------------------------------------------------------------
// --with-portless contract: portless.json present with the flag, absent without.
// ---------------------------------------------------------------------------

#[test]
fn with_portless_emits_portless_json() {
    let tmp = TempDir::new().unwrap();
    scaffold(
        &tmp,
        "portless_app",
        &["--frontend", "svelte", "--with-portless"],
    );
    let pj = tmp.path().join("portless_app").join("portless.json");
    assert!(
        pj.is_file(),
        "portless.json should be present when --with-portless is passed"
    );
    let body = std::fs::read_to_string(&pj).unwrap();
    assert!(
        body.contains("\"name\": \"portless_app\""),
        "portless.json name field wrong:\n{body}"
    );
    assert!(
        body.contains("\"appPort\": 8765"),
        "portless.json appPort field wrong:\n{body}"
    );
}

#[test]
fn without_portless_omits_portless_json() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "plain_app", &["--frontend", "svelte"]);
    assert!(
        !tmp.path().join("plain_app").join("portless.json").exists(),
        "portless.json must be absent without --with-portless"
    );
}

// ---------------------------------------------------------------------------
// Compile checks - slow (full transitive build); ignored by default.
// ---------------------------------------------------------------------------

/// Rewrite the scaffolded `Cargo.toml` so it builds against the
/// in-tree `framework/` crate instead of the published `suprnova`
/// release. The published crate exists for end users but isn't
/// resolvable inside this workspace's test harness; swapping the dep
/// line for a `path =` reference is what lets `cargo check` succeed
/// before the crate is on crates.io.
fn patch_local_suprnova(project: &Path) {
    let framework_dir = workspace_framework_dir();
    let cargo_toml = project.join("Cargo.toml");
    let original = std::fs::read_to_string(&cargo_toml).expect("read scaffolded Cargo.toml");

    let mut rewritten = String::with_capacity(original.len() + 14);
    let mut replaced = false;
    let mut has_workspace = false;
    for line in original.lines() {
        if line.trim() == "[workspace]" {
            has_workspace = true;
        }
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!(
                "suprnova = {{ path = \"{}\" }}\n",
                framework_dir.display(),
            ));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert!(
        replaced,
        "scaffolded Cargo.toml must contain a `suprnova = ...` dependency line"
    );
    if !has_workspace {
        rewritten.push_str("\n[workspace]\n");
    }
    std::fs::write(&cargo_toml, rewritten).expect("write patched Cargo.toml");
}

fn workspace_framework_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` points at `suprnova-cli/`, so the sibling
    // workspace member lives one level up.
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .expect("suprnova-cli must have a workspace parent")
        .join("framework")
}

fn run_cargo_check(project: &Path) {
    let output = Command::new(env!("CARGO"))
        .arg("check")
        .current_dir(project)
        .output()
        .expect("cargo check should run");
    assert!(
        output.status.success(),
        "scaffolded project at {} failed cargo check.\n\
         stdout:\n{}\nstderr:\n{}",
        project.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
#[ignore = "compile check - runs `cargo check` on a scaffolded project; slow"]
fn api_starter_compiles_with_cargo_check() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "compile_api", &["--api"]);
    let project = tmp.path().join("compile_api");
    patch_local_suprnova(&project);
    run_cargo_check(&project);
}

#[test]
#[ignore = "compile check - runs `cargo check` on a scaffolded project; slow"]
fn default_starter_compiles_with_cargo_check() {
    let tmp = TempDir::new().unwrap();
    scaffold(&tmp, "compile_default", &["--frontend", "svelte"]);
    let project = tmp.path().join("compile_default");
    patch_local_suprnova(&project);
    run_cargo_check(&project);
}

// ---------------------------------------------------------------------------
// The Live starter kit (PAR-192): compiled, served, and its documents read.
// ---------------------------------------------------------------------------

/// The build directory the Live kit's scaffold shares with this package:
/// the parent of `CARGO_TARGET_TMPDIR`, as `kit_browser.rs` does, so the
/// framework this workspace already compiled is reused and only the
/// scaffolded application builds.
fn shared_target_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .expect("CARGO_TARGET_TMPDIR has a parent")
        .to_path_buf()
}

/// Runs `cargo <args>` in `project` against the shared build directory.
fn run_cargo_shared(project: &Path, args: &[&str]) {
    let output = Command::new(env!("CARGO"))
        .args(args)
        .current_dir(project)
        .env("CARGO_TARGET_DIR", shared_target_dir())
        .output()
        .expect("cargo should run");
    assert!(
        output.status.success(),
        "`cargo {}` failed for the Live scaffold at {}.\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        project.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// The scaffold's server process, stopped when the test ends however it
/// ends.
struct LiveServer(std::process::Child);

impl Drop for LiveServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// One `GET` over HTTP/1.1 to the local server, as a browser asks for a
/// page: the status and the body.
fn get(port: u16, path: &str) -> std::io::Result<(u16, String)> {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/html\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    Ok((status, body))
}

/// PAR-192: a fresh `--frontend live` scaffold compiles against the
/// in-tree framework, its own document test renders every page for its
/// visitor, and its server answers each guest page with a Live document in
/// its layout. The scaffold builds its debug binary, the one `suprnova
/// serve` runs, into the shared build directory.
#[test]
#[ignore = "compile check - builds and serves a Live kit scaffold; slow"]
fn live_kit_compiles_and_renders() {
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let name = "live_render";
    scaffold(&tmp, name, &["--frontend", "live"]);
    let project = tmp.path().join(name);
    patch_local_suprnova(&project);
    // The workspace's lock, so the scaffold resolves the dependency
    // versions this workspace builds and tests rather than whatever is
    // newest today; cargo drops the entries the scaffold does not use.
    let workspace_lock = workspace_framework_dir()
        .parent()
        .expect("framework sits in the workspace")
        .join("Cargo.lock");
    std::fs::copy(&workspace_lock, project.join("Cargo.lock")).expect("copy Cargo.lock");

    run_cargo_shared(&project, &["check"]);
    // The kit's own `tests/documents.rs`: every document rendered through
    // the test client as its visitor sees it, the account pages as a
    // signed-in user with a note, the error page and the profile endpoint.
    run_cargo_shared(&project, &["test", "--test", "documents"]);
    run_cargo_shared(&project, &["build", "--bin", name]);

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port();
    let database = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("live_render.sqlite").display()
    );
    let log_path = tmp.path().join("server.log");
    let log = std::fs::File::create(&log_path).expect("server log");
    let mut server = LiveServer(
        Command::new(shared_target_dir().join("debug").join(name))
            .arg("web:run")
            .current_dir(&project)
            .env("APP_ENV", "local")
            .env("APP_URL", format!("http://127.0.0.1:{port}"))
            .env("SERVER_HOST", "127.0.0.1")
            .env("SERVER_PORT", port.to_string())
            .env("SESSION_SECURE", "false")
            .env("SESSION_COOKIE_PREFIX", "")
            .env("DATABASE_URL", database)
            .env("MAIL_DRIVER", "memory")
            .stdout(log.try_clone().expect("server log"))
            .stderr(log)
            .spawn()
            .expect("start the scaffold's server"),
    );
    let server_log = || std::fs::read_to_string(&log_path).unwrap_or_default();

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if let Some(status) = server.0.try_wait().expect("poll the server") {
            panic!(
                "the server exited before it answered: {status}\n{}",
                server_log()
            );
        }
        if matches!(get(port, "/"), Ok((200, _))) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the server did not answer `/` in time:\n{}",
            server_log()
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    for (path, layout, island) in [
        ("/", "app", None),
        ("/login", "guest", Some("login-form")),
        ("/register", "guest", Some("register-form")),
        ("/forgot-password", "guest", Some("forgot-password-form")),
    ] {
        let (status, body) = get(port, path).unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(status, 200, "GET {path}:\n{body}\n{}", server_log());
        // The bootstrap tags: the inert configuration element and the
        // runtime the framework serves under the reserved namespace.
        assert!(
            body.contains(r#"<script id="suprnova-live-config" type="application/json">"#),
            "GET {path} has no Live configuration element:\n{body}"
        );
        assert!(
            body.contains(r#"<script type="module" src="/__live/assets/"#),
            "GET {path} loads no Live runtime:\n{body}"
        );
        assert!(
            body.contains(&format!(r#"data-layout="{layout}""#)),
            "GET {path} is not in the {layout} layout:\n{body}"
        );
        if let Some(island) = island {
            assert!(
                body.contains(&format!(
                    r#"data-suprnova-live-component="{name}.{island}""#
                )),
                "GET {path} mounts no {island} island:\n{body}"
            );
        }
    }

    // The library's stylesheets and the kit's own are served, a page that
    // needs a user sends a guest to sign in, and an unknown path is the
    // error page.
    for path in ["/suprnova-ui/card/card.css", "/css/app.css"] {
        let (status, _) = get(port, path).unwrap_or_else(|e| panic!("GET {path}: {e}"));
        assert_eq!(status, 200, "GET {path}");
    }
    let (status, _) = get(port, "/dashboard").expect("GET /dashboard");
    assert_eq!(status, 302, "a guest is sent from the dashboard to sign in");
    let (status, body) = get(port, "/no-such-page").expect("GET /no-such-page");
    assert_eq!(status, 404);
    assert!(
        body.contains(r#"<h1 class="kit-error-status" id="error-status">404</h1>"#),
        "{body}"
    );

    drop(server);
}
