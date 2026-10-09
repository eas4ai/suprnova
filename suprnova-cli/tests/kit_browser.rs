//! Behavioural half of `par-starter-kits`: fresh scaffolds in Chromium.
//! Run: cargo test -p suprnova-cli --test kit_browser -- --ignored --test-threads=1

use std::fs;
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tokio::process::Command as AsyncCommand;

const CLI: &str = env!("CARGO_BIN_EXE_suprnova");
const NAME: &str = "kit_browser_app";
const KITS: [&str; 3] = ["svelte", "react", "vue"];
static BACKEND: OnceLock<Result<Backend, String>> = OnceLock::new();
// Serialises asset swaps even when a caller omits --test-threads=1.
static KIT_LOCK: Mutex<()> = Mutex::new(());

struct Backend {
    root: PathBuf,
    project: PathBuf,
    binary: PathBuf,
    suite: PathBuf,
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("read {}: {error}", path.display()))
}

/// Keep stdout/stderr together in a durable file, including on timeout.
fn run(command: &mut AsyncCommand, log: &Path, timeout: Duration) -> Result<(), String> {
    let description = format!("{command:?}");
    eprintln!("kit browser: {description}; log {}", log.display());
    let file = fs::File::create(log).map_err(|error| error.to_string())?;
    command.stdout(file.try_clone().map_err(|error| error.to_string())?);
    command.stderr(file);
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    command.as_std_mut().process_group(0);
    command.kill_on_drop(true);
    runtime.block_on(async {
        let mut child = command
            .spawn()
            .map_err(|error| format!("{description}: {error}"))?;
        #[cfg(unix)]
        let process_group = child.id();
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) if status.success() => {
                eprintln!("kit browser: {description}: {status}\n{}", read(log));
                Ok(())
            }
            result => {
                // Stop descendants too (Cargo, npm and npx start children).
                #[cfg(unix)]
                if let Some(pid) = process_group {
                    let _ = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(pid as i32),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
                // Reap a timed-out child before reading the final log.
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(format!("{description}: {result:?}\n{}", read(log)))
            }
        }
    })
}

/// As in live_generated_app.rs: use the in-tree facade and detach the scaffold.
fn patch_local_suprnova(project: &Path) -> Result<(), String> {
    let framework = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("workspace root")?
        .join("framework");
    let manifest = project.join("Cargo.toml");
    let original = fs::read_to_string(&manifest).map_err(|error| error.to_string())?;
    let mut replaced = false;
    let mut rewritten = String::new();
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!("suprnova = {{ path = {:?} }}\n", framework));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    if !replaced {
        return Err("scaffold has no suprnova dependency".into());
    }
    if !original.lines().any(|line| line.trim() == "[workspace]") {
        rewritten.push_str("\n[workspace]\n");
    }
    fs::write(manifest, rewritten).map_err(|error| error.to_string())?;

    // The sole backend difference between kits is the frontend enum in
    // bootstrap. Select that configuration at boot to share this build;
    // serve the built assets even under APP_ENV=local. Handlers, middleware,
    // pages and client code stay as scaffolded.
    let bootstrap = project.join("src/bootstrap.rs");
    let source = fs::read_to_string(&bootstrap).map_err(|error| error.to_string())?;
    let pinned = ".frontend(Frontend::Svelte)";
    if source.matches(pinned).count() != 1 {
        return Err("expected one pinned Svelte frontend in bootstrap".into());
    }
    fs::write(
        bootstrap,
        source.replace(
            pinned,
            ".frontend(Frontend::detect_from_env()).development(false)",
        ),
    )
    .map_err(|error| error.to_string())
}

fn prepare() -> Result<Backend, String> {
    fs::create_dir_all(env!("CARGO_TARGET_TMPDIR")).map_err(|error| error.to_string())?;
    // Retain generated projects, command logs and traces for diagnosing a
    // failed mechanism. These live on disk, not the machine's tmpfs.
    let root = tempfile::Builder::new()
        .prefix("kit-browser-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .map_err(|error| error.to_string())?
        .keep();
    eprintln!("kit browser artifacts: {}", root.display());
    for kit in KITS {
        let directory = root.join(kit);
        fs::create_dir(&directory).map_err(|error| error.to_string())?;
        run(
            AsyncCommand::new(CLI)
                .args([
                    "new",
                    NAME,
                    "--no-interaction",
                    "--no-git",
                    "--frontend",
                    kit,
                ])
                .current_dir(&directory),
            &root.join(format!("{kit}-scaffold.log")),
            Duration::from_secs(60),
        )?;
    }
    let project = root.join("backend").join(NAME);
    copy_tree(&root.join("svelte").join(NAME), &project)?;
    patch_local_suprnova(&project)?;
    // Reuse dependency artifacts from the outer Cargo build; only this
    // scaffolded application binary is built once for the three kits.
    let target = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .ok_or("Cargo target directory")?
        .to_path_buf();
    run(
        AsyncCommand::new(env!("CARGO"))
            .args(["build", "--bin", NAME])
            .current_dir(&project)
            .env("CARGO_TARGET_DIR", &target),
        &root.join("backend-build.log"),
        Duration::from_secs(2400),
    )?;
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/kit_browser");
    run(
        AsyncCommand::new("npm").arg("ci").current_dir(&suite),
        &root.join("playwright-npm.log"),
        Duration::from_secs(300),
    )?;
    Ok(Backend {
        binary: target.join("debug").join(NAME),
        root,
        project,
        suite,
    })
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn ready(server: &mut Server, url: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| error.to_string())?;
        let mut ticks = tokio::time::interval(Duration::from_millis(100));
        let wait = async {
            loop {
                ticks.tick().await;
                if let Some(status) = server.0.try_wait().map_err(|error| error.to_string())? {
                    return Err(format!("server exited before readiness: {status}"));
                }
                if client
                    .get(url)
                    .send()
                    .await
                    .is_ok_and(|reply| reply.status().is_success())
                {
                    return Ok(());
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(60), wait)
            .await
            .map_err(|_| format!("server readiness timed out: {url}"))?
    })
}

fn exercise(backend: &Backend, kit: &str, server_log: &Path) -> Result<(), String> {
    let project = backend.root.join(kit).join(NAME);
    let frontend = project.join("frontend");
    run(
        AsyncCommand::new("npm")
            .arg("install")
            .current_dir(&frontend),
        &backend.root.join(format!("{kit}-npm.log")),
        Duration::from_secs(300),
    )?;
    run(
        AsyncCommand::new("npm")
            .args(["run", "build"])
            .current_dir(&frontend),
        &backend.root.join(format!("{kit}-build.log")),
        Duration::from_secs(300),
    )?;
    if project != backend.project {
        for relative in ["public/assets", "frontend/src/pages"] {
            let destination = backend.project.join(relative);
            if destination.exists() {
                fs::remove_dir_all(&destination).map_err(|error| error.to_string())?;
            }
            copy_tree(&project.join(relative), &destination)?;
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    drop(listener);
    let url = format!("http://127.0.0.1:{port}");
    let database = format!(
        "sqlite://{}?mode=rwc",
        backend.root.join(format!("{kit}.sqlite")).display()
    );
    let log = fs::File::create(server_log).map_err(|error| error.to_string())?;
    let mut server = Server(
        Command::new(&backend.binary)
            .arg("web:run")
            .current_dir(&backend.project)
            .env("APP_ENV", "local")
            .env("APP_URL", &url)
            .env("SERVER_HOST", "127.0.0.1")
            .env("SERVER_PORT", port.to_string())
            .env("SESSION_SECURE", "false")
            .env("SESSION_COOKIE_PREFIX", "")
            .env("DATABASE_URL", database)
            .env("SUPRNOVA_FRONTEND", kit)
            // Exercise real mail composition without an external SMTP dependency.
            .env("MAIL_DRIVER", "memory")
            .stdout(log.try_clone().map_err(|error| error.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|error| error.to_string())?,
    );
    ready(&mut server, &url)?;
    run(
        AsyncCommand::new("npx")
            .args(["--no-install", "playwright", "test"])
            .current_dir(&backend.suite)
            .env("KIT_BASE_URL", &url)
            .env("KIT_NAME", kit)
            .env(
                "KIT_BROWSER_RESULTS",
                backend.root.join(format!("{kit}-results")),
            ),
        &backend.root.join(format!("{kit}-playwright.log")),
        Duration::from_secs(1200),
    )
}

fn kit_in_a_browser(kit: &str) {
    let _lock = KIT_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let backend = BACKEND
        .get_or_init(prepare)
        .as_ref()
        .unwrap_or_else(|error| panic!("{kit}: backend setup failed:\n{error}"));
    let log = backend.root.join(format!("{kit}-server.log"));
    if let Err(error) = exercise(backend, kit, &log) {
        let server_output = if log.exists() {
            read(&log)
        } else {
            "Server was not started; setup failed before server startup.".into()
        };
        panic!(
            "{kit}: browser mechanism failed:\n{error}\nserver log ({}):\n{}\nartifacts: {}",
            log.display(),
            server_output,
            backend.root.display()
        );
    }
}

#[test]
#[ignore = "acceptance: builds fresh kits and runs Chromium; slow"]
fn kit_svelte_in_a_browser() {
    kit_in_a_browser("svelte");
}

#[test]
#[ignore = "acceptance: builds fresh kits and runs Chromium; slow"]
fn kit_react_in_a_browser() {
    kit_in_a_browser("react");
}

#[test]
#[ignore = "acceptance: builds fresh kits and runs Chromium; slow"]
fn kit_vue_in_a_browser() {
    kit_in_a_browser("vue");
}
