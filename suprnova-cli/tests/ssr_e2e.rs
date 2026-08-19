//! End-to-end proof for T31: `suprnova new` -> `vite build --ssr` ->
//! `suprnova ssr:start` -> a hard-navigation HTML response contains the
//! SSR-rendered body.
//!
//! `#[ignore]`d — needs Node/npm and network access for `npm install`,
//! neither of which the normal `cargo test --workspace` run can assume.
//! Run it explicitly:
//!
//! ```bash
//! cargo test -p suprnova-cli --test ssr_e2e -- --ignored --nocapture
//! ```

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

fn cli_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_suprnova"))
}

fn workspace_framework_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("suprnova-cli must have a workspace parent")
        .join("framework")
}

fn run_ok(mut cmd: Command, what: &str) {
    let status = cmd.status().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(status.success(), "{what} exited with {status}");
}

/// Point the scaffold at the in-tree framework crate — the published
/// `suprnova` tag doesn't carry this task's changes yet. Mirrors
/// `scaffold_snapshot.rs::patch_local_suprnova`.
fn patch_local_suprnova(project: &Path) {
    let cargo_toml = project.join("Cargo.toml");
    let original = std::fs::read_to_string(&cargo_toml).expect("read scaffolded Cargo.toml");
    let mut rewritten = String::with_capacity(original.len());
    let mut replaced = false;
    for line in original.lines() {
        if line.trim_start().starts_with("suprnova = ") {
            rewritten.push_str(&format!(
                "suprnova = {{ path = \"{}\" }}\n",
                workspace_framework_dir().display()
            ));
            replaced = true;
        } else {
            rewritten.push_str(line);
            rewritten.push('\n');
        }
    }
    assert!(
        replaced,
        "scaffolded Cargo.toml must declare a suprnova dependency"
    );
    std::fs::write(&cargo_toml, rewritten).expect("write patched Cargo.toml");
}

/// SSR is off by default (Design Note 4) — this proof has to opt in
/// itself, the same way any app adopting SSR would.
fn enable_ssr(project: &Path) {
    let path = project.join("src/bootstrap.rs");
    let original = std::fs::read_to_string(&path).expect("read scaffolded bootstrap.rs");
    let needle = "InertiaConfig::new().frontend(Frontend::Svelte))";
    assert!(
        original.contains(needle),
        "scaffolded bootstrap.rs no longer matches the expected Inertia::install call \
         (template drifted). bootstrap.rs:\n{original}"
    );
    let patched = original.replace(
        needle,
        "InertiaConfig::new().frontend(Frontend::Svelte).ssr(\"http://127.0.0.1:13714\"))",
    );
    std::fs::write(&path, patched).expect("write patched bootstrap.rs");
}

/// Poll `ssr:check` until it reports the worker healthy or `budget` runs out.
fn wait_for_ssr_healthy(budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let ok = Command::new(cli_binary())
            .args(["ssr:check", "--timeout-ms", "500"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "SSR worker never became healthy within {budget:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn get_body(addr: std::net::SocketAddr) -> String {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .expect("write request");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("read response");
    let text = String::from_utf8_lossy(&response).into_owned();
    text.split("\r\n\r\n")
        .nth(1)
        .unwrap_or_default()
        .to_string()
}

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "e2e: needs npm/node, runs `vite build --ssr`, boots two processes — run manually"]
fn scaffolded_ssr_entry_produces_server_rendered_html() {
    let tmp = TempDir::new().unwrap();
    let project_name = "ssr_e2e_app";

    let mut new_cmd = Command::new(cli_binary());
    new_cmd
        .args([
            "new",
            project_name,
            "--no-interaction",
            "--no-git",
            "--frontend",
            "svelte",
        ])
        .current_dir(tmp.path());
    run_ok(new_cmd, "suprnova new");

    let project = tmp.path().join(project_name);
    let frontend = project.join("frontend");
    patch_local_suprnova(&project);
    enable_ssr(&project);

    let mut npm_install = Command::new("npm");
    npm_install.arg("install").current_dir(&frontend);
    run_ok(npm_install, "npm install");

    let mut npm_build = Command::new("npm");
    npm_build.args(["run", "build"]).current_dir(&frontend);
    run_ok(npm_build, "npm run build");

    let mut npm_build_ssr = Command::new("npm");
    npm_build_ssr
        .args(["run", "build:ssr"])
        .current_dir(&frontend);
    run_ok(npm_build_ssr, "npm run build:ssr");

    assert!(
        frontend.join("bootstrap/ssr/ssr.js").exists(),
        "vite build --ssr must produce frontend/bootstrap/ssr/ssr.js"
    );

    let _ssr_worker = KillOnDrop(
        Command::new(cli_binary())
            .arg("ssr:start")
            .current_dir(&project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ssr:start"),
    );
    wait_for_ssr_healthy(Duration::from_secs(30));

    let backend_port: u16 = 18765;
    let addr: std::net::SocketAddr = format!("127.0.0.1:{backend_port}").parse().unwrap();
    let mut backend_cmd = Command::new(env!("CARGO"));
    backend_cmd
        .args(["run", "--bin", project_name, "--", "serve"])
        .current_dir(&project)
        .env("APP_ENV", "production")
        .env("SERVER_PORT", backend_port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let _backend = KillOnDrop(backend_cmd.spawn().expect("spawn backend"));

    // The backend is a debug `cargo run` compile + boot — give it real
    // time rather than guessing an exact readiness signal.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "backend never started listening on {addr}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    let body = get_body(addr);
    assert!(
        body.contains("data-server-rendered=\"true\""),
        "hard navigation must be server-rendered; body:\n{body}"
    );
}
