//! The `suprnova` CLI's `ssr:start`, `ssr:stop` and `ssr:check`.
//!
//! The CLI runs the project's application binary's command of the same name
//! (PAR-061), so the `inssr_` tests run the CLI binary in a temporary
//! project whose `cargo` is a shell script earlier on `PATH`, the pattern
//! `serve_dev_processes.rs` uses: the script records the arguments and the
//! directory it was called with, and answers with the output, the exit
//! status or the signal handling a test gives it. What the application's
//! commands do with the installed configuration is tested where they live,
//! in the framework's `tests/console/ssr.rs`.
//!
//! The end-to-end proof for T31 - `suprnova new` -> `vite build --ssr` ->
//! `suprnova ssr:start` -> a hard-navigation HTML response contains the
//! SSR-rendered body - is `#[ignore]`d: it needs Node/npm and network access
//! for `npm install`, neither of which the normal `cargo test --workspace`
//! run can assume. Run it explicitly:
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

/// Point the scaffold at the in-tree framework crate - the published
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

/// SSR is off by default (Design Note 4) - this proof has to opt in
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

/// Poll `ssr:check` in `project` until it reports the worker healthy or
/// `budget` runs out.
fn wait_for_ssr_healthy(project: &Path, budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let ok = Command::new(cli_binary())
            .arg("ssr:check")
            .current_dir(project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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

/// Stops `suprnova ssr:start` as a supervisor does, with `SIGTERM`, which
/// the CLI forwards to the application and the application to its worker.
/// A `SIGKILL` would end the CLI alone and leave both running.
struct TerminateOnDrop(Child);
impl Drop for TerminateOnDrop {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(self.0.id()) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        #[cfg(not(unix))]
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "e2e: needs npm/node, runs `vite build --ssr`, boots two processes - run manually"]
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

    // `ssr:start` and `ssr:check` run the application binary through
    // `cargo run`; building it first keeps the compile out of the health
    // check's budget.
    let mut build = Command::new(env!("CARGO"));
    build
        .args(["build", "--bin", project_name])
        .current_dir(&project);
    run_ok(build, "cargo build");

    let _ssr_worker = TerminateOnDrop(
        Command::new(cli_binary())
            .arg("ssr:start")
            .current_dir(&project)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ssr:start"),
    );
    wait_for_ssr_healthy(&project, Duration::from_secs(30));

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

    // The backend is a debug `cargo run` compile + boot - give it real
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

// ---------------------------------------------------------------------------
// PAR-061: the CLI runs the application binary's command
// ---------------------------------------------------------------------------

/// The CLI links no part of the framework crate: no table a build of the
/// CLI reads names the `suprnova` package, under its own name or another.
/// The tests may link it, so `[dev-dependencies]` is not read.
#[test]
fn inssr_the_cli_does_not_depend_on_the_framework_crate() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest: toml::Table = std::fs::read_to_string(&path)
        .expect("read suprnova-cli/Cargo.toml")
        .parse()
        .expect("parse suprnova-cli/Cargo.toml");

    let mut tables = vec![
        ("[dependencies]".to_owned(), manifest.get("dependencies")),
        (
            "[build-dependencies]".to_owned(),
            manifest.get("build-dependencies"),
        ),
    ];
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, table) in targets {
            for kind in ["dependencies", "build-dependencies"] {
                tables.push((format!("[target.'{target}'.{kind}]"), table.get(kind)));
            }
        }
    }
    assert!(
        tables[0].1.is_some(),
        "the manifest has a [dependencies] table to read"
    );
    for (name, table) in tables {
        let Some(table) = table.and_then(toml::Value::as_table) else {
            continue;
        };
        for (key, spec) in table {
            let package = spec
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            assert_ne!(
                package, "suprnova",
                "{name} names the framework crate as `{key}`"
            );
        }
    }
}

#[cfg(unix)]
mod inssr {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;

    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    use tempfile::TempDir;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines};
    use tokio::process::{Child, ChildStdout};

    use super::cli_binary;

    /// The package of the test project, so the binary the CLI must name.
    const PACKAGE: &str = "ssr_app";

    /// The three commands, each of which runs the application's own.
    const COMMANDS: [&str; 3] = ["ssr:start", "ssr:stop", "ssr:check"];

    /// How `cargo` was called.
    #[derive(Debug, PartialEq, Eq)]
    struct Call {
        args: Vec<String>,
        dir: PathBuf,
    }

    /// A temporary project, a `bin` directory whose `cargo` goes on `PATH`
    /// before the real one, and the record that `cargo` writes, side by
    /// side so the record is not inside the project.
    struct Project {
        dir: TempDir,
    }

    impl Project {
        /// A project whose manifest names the package [`PACKAGE`].
        fn new() -> Self {
            Self::with_manifest(Some(&format!(
                "[package]\nname = \"{PACKAGE}\"\nversion = \"0.1.0\"\n"
            )))
        }

        /// A project directory with `manifest` as its `Cargo.toml`, or with
        /// none.
        fn with_manifest(manifest: Option<&str>) -> Self {
            let project = Self {
                dir: tempfile::tempdir().expect("a temporary directory"),
            };
            for dir in [project.root(), project.bin(), project.record()] {
                std::fs::create_dir_all(dir).expect("create a directory");
            }
            if let Some(manifest) = manifest {
                std::fs::write(project.root().join("Cargo.toml"), manifest)
                    .expect("write the manifest");
            }
            project
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("project")
        }

        fn bin(&self) -> PathBuf {
            self.dir.path().join("bin")
        }

        fn record(&self) -> PathBuf {
            self.dir.path().join("record")
        }

        /// Put a `cargo` in `bin` that records its arguments and working
        /// directory, then runs `body`.
        fn cargo(&self, body: &str) {
            let path = self.bin().join("cargo");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\n\
                     pwd -P > '{dir}'\n\
                     for arg in \"$@\"; do printf '%s\\n' \"$arg\"; done > '{args}'\n\
                     {body}\n",
                    dir = self.record().join("dir").display(),
                    args = self.record().join("args").display(),
                ),
            )
            .expect("write the fake cargo");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make the fake cargo executable");
        }

        /// How `cargo` was last called, or `None` when it never ran.
        fn call(&self) -> Option<Call> {
            let args = std::fs::read_to_string(self.record().join("args")).ok()?;
            let dir = std::fs::read_to_string(self.record().join("dir"))
                .expect("cargo recorded its directory before its arguments");
            Some(Call {
                args: args.lines().map(str::to_owned).collect(),
                dir: PathBuf::from(dir.trim_end()),
            })
        }

        /// Forget the last call, so the next one is recorded alone.
        fn forget(&self) {
            for name in ["args", "dir"] {
                match std::fs::remove_file(self.record().join(name)) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => panic!("forget the call: {e}"),
                }
            }
        }

        /// The CLI with `args`, in the project, with `bin` first on `PATH`.
        fn cli(&self, args: &[&str]) -> tokio::process::Command {
            let path = format!(
                "{}:{}",
                self.bin().display(),
                std::env::var("PATH").unwrap_or_default()
            );
            self.cli_with_path(args, &path)
        }

        /// The CLI with `args`, in the project, with `path` as all of `PATH`.
        fn cli_with_path(&self, args: &[&str], path: &str) -> tokio::process::Command {
            let mut command = tokio::process::Command::new(cli_binary());
            command
                .args(args)
                .current_dir(self.root())
                .env("PATH", path)
                .stdin(Stdio::null())
                .kill_on_drop(true);
            command
        }
    }

    /// The arguments the CLI must give `cargo` for `app_args`.
    fn cargo_args(app_args: &[&str]) -> Vec<String> {
        ["run", "--bin", PACKAGE, "--"]
            .iter()
            .chain(app_args)
            .map(|arg| (*arg).to_owned())
            .collect()
    }

    /// What a command exited with and printed.
    #[derive(Debug)]
    struct Ran {
        code: i32,
        out: String,
        err: String,
    }

    async fn run(command: &mut tokio::process::Command) -> Ran {
        let output = command.output().await.expect("run the CLI");
        Ran {
            code: output
                .status
                .code()
                .expect("the CLI exited, not killed by a signal"),
            out: String::from_utf8_lossy(&output.stdout).into_owned(),
            err: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    // -- what the CLI runs ----------------------------------------------------

    #[tokio::test]
    async fn inssr_cli_start_runs_the_applications_ssr_start_with_the_runtime() {
        let project = Project::new();
        project.cargo("exit 0");

        let ran = run(&mut project.cli(&["ssr:start", "--runtime", "bun"])).await;

        assert_eq!(ran.code, 0, "{ran:?}");
        let call = project.call().expect("the CLI ran cargo");
        assert_eq!(call.args, cargo_args(&["ssr:start", "--runtime=bun"]));
    }

    #[tokio::test]
    async fn inssr_cli_start_turns_no_environment_into_flags() {
        let project = Project::new();
        project.cargo("exit 0");

        let ran = run(project
            .cli(&["ssr:start"])
            .env("SUPRNOVA_SSR_URL", "http://127.0.0.1:1")
            .env("SUPRNOVA_SSR_RUNTIME", "deno")
            .env("SUPRNOVA_SSR_BUNDLE", "build/ssr.js")
            .env("SUPRNOVA_SSR_ENSURE_RUNTIME_EXISTS", "true"))
        .await;

        assert_eq!(ran.code, 0, "{ran:?}");
        let call = project.call().expect("the CLI ran cargo");
        assert_eq!(
            call.args,
            cargo_args(&["ssr:start"]),
            "the installed configuration decides the URL, the runtime and the bundle"
        );
    }

    #[tokio::test]
    async fn inssr_cli_stop_passes_graceful_through() {
        let project = Project::new();
        project.cargo("exit 0");

        for args in [&["ssr:stop", "--graceful"][..], &["ssr:stop"]] {
            project.forget();
            let ran = run(&mut project.cli(args)).await;

            assert_eq!(ran.code, 0, "{args:?}: {ran:?}");
            let call = project.call().expect("the CLI ran cargo");
            assert_eq!(call.args, cargo_args(args), "{args:?}");
        }
    }

    #[tokio::test]
    async fn inssr_cli_check_runs_the_applications_ssr_check() {
        let project = Project::new();
        project.cargo("exit 0");

        let ran = run(&mut project.cli(&["ssr:check"])).await;

        assert_eq!(ran.code, 0, "{ran:?}");
        let call = project.call().expect("the CLI ran cargo");
        assert_eq!(call.args, cargo_args(&["ssr:check"]));
    }

    #[tokio::test]
    async fn inssr_cli_runs_the_application_from_the_project_directory() {
        let project = Project::new();
        project.cargo("exit 0");
        let root = std::fs::canonicalize(project.root()).expect("the project's real path");

        for command in COMMANDS {
            project.forget();
            let ran = run(&mut project.cli(&[command])).await;

            assert_eq!(ran.code, 0, "{command}: {ran:?}");
            let call = project.call().expect("the CLI ran cargo");
            assert_eq!(call.dir, root, "{command}");
        }
    }

    #[tokio::test]
    async fn inssr_cli_has_no_configuration_flags_of_its_own() {
        let project = Project::new();
        project.cargo("exit 0");

        for args in [
            &["ssr:start", "--bundle", "build/ssr.js"][..],
            &["ssr:start", "--url", "http://127.0.0.1:13714"],
            &["ssr:start", "--ensure-runtime-exists"],
            &["ssr:stop", "--url", "http://127.0.0.1:13714"],
            &["ssr:stop", "--timeout-ms", "500"],
            &["ssr:check", "--url", "http://127.0.0.1:13714"],
            &["ssr:check", "--timeout-ms", "500"],
        ] {
            let ran = run(&mut project.cli(args)).await;

            assert_eq!(ran.code, 2, "{args:?}: the flag is refused: {ran:?}");
            assert!(
                ran.err.contains("unexpected argument"),
                "{args:?}: {}",
                ran.err
            );
            assert_eq!(project.call(), None, "{args:?}: cargo never ran");
        }
    }

    // -- what the CLI passes back ---------------------------------------------

    #[tokio::test]
    async fn inssr_cli_relays_the_applications_stdout_and_stderr() {
        let project = Project::new();
        project.cargo("printf 'rendering\\n'\nprintf 'a problem\\n' >&2\nexit 0");

        for command in COMMANDS {
            let ran = run(&mut project.cli(&[command])).await;

            assert_eq!(ran.code, 0, "{command}: {ran:?}");
            assert_eq!(
                ran.out, "rendering\n",
                "{command}: the application's output"
            );
            assert_eq!(
                ran.err, "a problem\n",
                "{command}: the application's errors"
            );
        }
    }

    #[tokio::test]
    async fn inssr_cli_exits_with_the_applications_status() {
        let project = Project::new();
        project.cargo("exit 3");

        for command in COMMANDS {
            let ran = run(&mut project.cli(&[command])).await;

            assert_eq!(ran.code, 3, "{command}: {ran:?}");
        }
    }

    #[tokio::test]
    async fn inssr_cli_exits_128_plus_the_signal_that_ended_the_application() {
        let project = Project::new();
        project.cargo("kill -KILL $$");

        for command in COMMANDS {
            let ran = run(&mut project.cli(&[command])).await;

            assert_eq!(ran.code, 128 + 9, "{command}: {ran:?}");
        }
    }

    // -- what the CLI refuses ---------------------------------------------------

    #[tokio::test]
    async fn inssr_cli_fails_outside_a_project_naming_the_reason() {
        let project = Project::with_manifest(None);
        project.cargo("exit 0");

        for command in COMMANDS {
            let ran = run(&mut project.cli(&[command])).await;

            assert_eq!(ran.code, 1, "{command}: {ran:?}");
            assert!(
                ran.err.contains("No Cargo.toml found"),
                "{command}: {}",
                ran.err
            );
            assert_eq!(project.call(), None, "{command}: cargo never ran");
        }
    }

    #[tokio::test]
    async fn inssr_cli_fails_on_a_manifest_without_a_package_naming_the_reason() {
        let project = Project::with_manifest(Some("[workspace]\nmembers = []\n"));
        project.cargo("exit 0");

        for command in COMMANDS {
            let ran = run(&mut project.cli(&[command])).await;

            assert_eq!(ran.code, 1, "{command}: {ran:?}");
            assert!(
                ran.err
                    .contains("Could not find package name in Cargo.toml"),
                "{command}: {}",
                ran.err
            );
            assert_eq!(project.call(), None, "{command}: cargo never ran");
        }
    }

    #[tokio::test]
    async fn inssr_cli_names_the_cargo_run_it_could_not_start() {
        // `bin` holds no `cargo`, and it is all of `PATH`.
        let project = Project::new();
        let path = project.bin().display().to_string();

        for command in COMMANDS {
            let ran = run(&mut project.cli_with_path(&[command], &path)).await;

            assert_eq!(ran.code, 1, "{command}: {ran:?}");
            assert!(
                ran.err
                    .contains(&format!("cargo run --bin {PACKAGE} -- {command}")),
                "{command}: the error names what could not run: {}",
                ran.err
            );
        }
    }

    // -- signals ----------------------------------------------------------------

    /// A `cargo` that says `ready` once it runs, prints `application got
    /// <signal> <count>` for each `SIGINT` or `SIGTERM` it receives, and
    /// exits with 7 at the `stop_after`th. It writes its own process id and
    /// its sleeper's to `pids`.
    fn signal_recording_cargo(pids: &Path, stop_after: usize) -> String {
        format!(
            "n=0\n\
             got() {{\n\
             \x20 n=$((n + 1))\n\
             \x20 echo \"application got $1 $n\"\n\
             \x20 if [ \"$n\" -ge {stop_after} ]; then kill \"$sleeper\"; exit 7; fi\n\
             }}\n\
             trap 'got INT' INT\n\
             trap 'got TERM' TERM\n\
             sleep 1000 >/dev/null 2>&1 &\n\
             sleeper=$!\n\
             echo \"$$ $sleeper\" > '{pids}'\n\
             echo ready\n\
             while kill -0 \"$sleeper\" 2>/dev/null; do wait \"$sleeper\"; done",
            pids = pids.display()
        )
    }

    /// Read the CLI's stdout into `out` up to the line `awaited`. `false`
    /// when the CLI exited or its output ended first.
    async fn read_until(
        lines: &mut Lines<BufReader<ChildStdout>>,
        child: &mut Child,
        out: &mut String,
        awaited: &str,
    ) -> bool {
        loop {
            let line = tokio::select! {
                line = lines.next_line() => line.expect("read the CLI's stdout"),
                _ = child.wait() => return false,
            };
            let Some(line) = line else {
                return false;
            };
            out.push_str(&line);
            out.push('\n');
            if line == awaited {
                return true;
            }
        }
    }

    /// Run `ssr:start` under a `cargo` that records the signals it receives,
    /// send the CLI alone each of `signals`, the first once the application
    /// is ready and each next one once the application reported the one
    /// before, and return what the CLI printed and exited with.
    ///
    /// A CLI that did not forward a signal dies of it and leaves the
    /// application running, holding the output pipe open; the application
    /// is then killed by the ids it wrote so the test can finish and report.
    async fn signal_a_running_start(signals: &[Signal]) -> Ran {
        let project = Project::new();
        let pids = project.record().join("pids");
        project.cargo(&signal_recording_cargo(&pids, signals.len()));
        let mut child = project
            .cli(&["ssr:start"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn the CLI");
        let cli =
            Pid::from_raw(i32::try_from(child.id().expect("the CLI runs")).expect("a process id"));

        let mut stderr = child.stderr.take().expect("the CLI's stderr");
        let errors = tokio::spawn(async move {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text).await;
            text
        });
        let mut lines = BufReader::new(child.stdout.take().expect("the CLI's stdout")).lines();
        let mut out = String::new();

        let mut awaited = "ready".to_owned();
        for (sent, signal) in signals.iter().enumerate() {
            if !read_until(&mut lines, &mut child, &mut out, &awaited).await {
                break;
            }
            kill(cli, *signal).expect("signal the CLI");
            awaited = format!(
                "application got {} {}",
                signal.as_str().trim_start_matches("SIG"),
                sent + 1
            );
        }

        let status = child.wait().await.expect("wait for the CLI");
        if status.code().is_none() {
            let ids = std::fs::read_to_string(&pids).unwrap_or_default();
            for id in ids
                .split_whitespace()
                .filter_map(|id| id.parse::<i32>().ok())
            {
                let _ = kill(Pid::from_raw(id), Signal::SIGKILL);
            }
        }
        while let Some(line) = lines.next_line().await.expect("read the CLI's stdout") {
            out.push_str(&line);
            out.push('\n');
        }
        let err = errors.await.expect("read the CLI's stderr");
        Ran {
            code: status
                .code()
                .unwrap_or_else(|| panic!("the CLI died of a signal: {out} {err}")),
            out,
            err,
        }
    }

    #[tokio::test]
    async fn inssr_cli_start_forwards_sigint_to_the_application() {
        let ran = signal_a_running_start(&[Signal::SIGINT]).await;

        assert_eq!(ran.code, 7, "the application's status: {ran:?}");
        assert_eq!(ran.out, "ready\napplication got INT 1\n");
    }

    #[tokio::test]
    async fn inssr_cli_start_forwards_sigterm_to_the_application() {
        let ran = signal_a_running_start(&[Signal::SIGTERM]).await;

        assert_eq!(ran.code, 7, "the application's status: {ran:?}");
        assert_eq!(ran.out, "ready\napplication got TERM 1\n");
    }

    /// The application's `ssr:start` kills its worker at a second signal, so
    /// the CLI forwards every signal, not the first alone.
    #[tokio::test]
    async fn inssr_cli_start_forwards_a_second_signal_as_well() {
        let ran = signal_a_running_start(&[Signal::SIGINT, Signal::SIGINT]).await;

        assert_eq!(ran.code, 7, "the application's status: {ran:?}");
        assert_eq!(
            ran.out,
            "ready\napplication got INT 1\napplication got INT 2\n"
        );
    }
}
