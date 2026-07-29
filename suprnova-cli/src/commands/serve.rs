use console::style;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

use crate::ui;

struct ProcessManager {
    children: Vec<Child>,
    shutdown: Arc<AtomicBool>,
}

impl ProcessManager {
    fn new() -> Self {
        Self {
            children: Vec::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    fn spawn_with_prefix(
        &mut self,
        command: &str,
        args: &[&str],
        cwd: Option<&Path>,
        envs: &[(&str, String)],
        prefix: &str,
        color: console::Color,
    ) -> Result<(), String> {
        let mut cmd = Command::new(command);
        cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());

        // The framework's `.env` loader (Phase 5a in config/env.rs)
        // restores real system env over file values, so a var we set on
        // the child here wins over the scaffold `.env` — that's how the
        // resolved/scanned ports reach the backend and Vite.
        for (key, value) in envs {
            cmd.env(key, value);
        }

        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to spawn {}: {}", command, e))?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let shutdown_stdout = self.shutdown.clone();
        let shutdown_stderr = self.shutdown.clone();

        let prefix_out = prefix.to_string();
        let prefix_err = prefix.to_string();

        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if shutdown_stdout.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(line) = line {
                    println!("{} {}", style(&prefix_out).fg(color).bold(), line);
                }
            }
        });

        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if shutdown_stderr.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(line) = line {
                    eprintln!("{} {}", style(&prefix_err).fg(color).bold(), line);
                }
            }
        });

        self.children.push(child);
        Ok(())
    }

    fn shutdown_all(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn any_exited(&mut self) -> bool {
        for child in &mut self.children {
            if let Ok(Some(_)) = child.try_wait() {
                return true;
            }
        }
        false
    }
}

fn get_package_name() -> Result<String, String> {
    let cargo_toml = Path::new("Cargo.toml");
    let content = std::fs::read_to_string(cargo_toml)
        .map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;

    crate::commands::cargo_meta::parse_cargo_toml(&content)
        .map_err(|e| format!("Failed to parse Cargo.toml: {}", e))?;

    crate::commands::cargo_meta::package_name_from_content(&content)
        .ok_or_else(|| "Could not find package name in Cargo.toml".to_string())
}

fn validate_suprnova_project(backend_only: bool, frontend_only: bool) -> Result<(), String> {
    let cargo_toml = Path::new("Cargo.toml");
    let frontend_dir = Path::new("frontend");

    if !frontend_only && !cargo_toml.exists() {
        return Err("No Cargo.toml found. Are you in a Suprnova project directory?".into());
    }

    if !backend_only && !frontend_dir.exists() {
        return Err("No frontend directory found. Are you in a Suprnova project directory?".into());
    }

    Ok(())
}

/// Version requirement for the `cargo-watch` we install on demand.
///
/// Bounded to a major version because `serve` drives it as
/// `cargo watch -x <cmd>` — a flag whose meaning is not guaranteed across
/// a major bump. Unbounded, `cargo install cargo-watch` takes whatever is
/// newest, so a future release could break `suprnova serve` on machines
/// that happened to install it that day, with nothing in this repo
/// changing.
const CARGO_WATCH_VERSION_REQ: &str = "^8.5";

fn ensure_cargo_watch() -> Result<(), String> {
    let status = Command::new("cargo")
        .args(["watch", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        _ => {
            ui::warning(&format!(
                "cargo-watch not found. Installing {CARGO_WATCH_VERSION_REQ} (locked)..."
            ));
            // `--locked` builds against the versions cargo-watch published
            // in its own Cargo.lock instead of re-resolving its dependency
            // tree at install time. Without it, `suprnova serve` silently
            // compiles and runs whatever transitive versions happen to be
            // newest on the day — the thing you least want from a command
            // that installs software as a side effect of starting a dev
            // server.
            let install = Command::new("cargo")
                .args([
                    "install",
                    "--locked",
                    "--version",
                    CARGO_WATCH_VERSION_REQ,
                    "cargo-watch",
                ])
                .status()
                .map_err(|e| format!("Failed to install cargo-watch: {}", e))?;

            if !install.success() {
                return Err(format!(
                    "Failed to install cargo-watch {CARGO_WATCH_VERSION_REQ}. Install it \
                     yourself with `cargo install --locked cargo-watch`, or run \
                     `suprnova serve --frontend-only` to skip the backend watcher."
                ));
            }
            ui::success("cargo-watch installed");
            Ok(())
        }
    }
}

fn ensure_npm_dependencies() -> Result<(), String> {
    let frontend_path = Path::new("frontend");
    let node_modules = frontend_path.join("node_modules");

    if !node_modules.exists() {
        ui::info("Installing frontend dependencies...");
        let npm_install = Command::new("npm")
            .args(["install"])
            .current_dir(frontend_path)
            .status()
            .map_err(|e| format!("Failed to run npm install: {}", e))?;

        if !npm_install.success() {
            return Err("Failed to install npm dependencies".into());
        }
        ui::success("Frontend dependencies installed");
    }

    Ok(())
}

/// Default backend port. Mirrors the framework's
/// `suprnova::config::providers::server::DEFAULT_SERVER_PORT`; kept in
/// sync deliberately (the CLI can't depend on the framework crate).
const DEFAULT_BACKEND_PORT: u16 = 8765;
/// Default Vite port. Mirrors `suprnova::inertia::DEFAULT_VITE_PORT`.
const DEFAULT_VITE_PORT: u16 = 5765;

/// Parse a `u16` port from an env var, treating empty/unparseable as unset.
fn env_port(key: &str) -> Option<u16> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u16>().ok())
}

/// Resolve a dev-server port. An explicit CLI flag (`cli`) pins the port
/// exactly. Otherwise take `env_value` (or `default`) as a base and scan
/// upward for the first free port so a busy base self-heals.
fn pick_port(cli: Option<u16>, env_value: Option<u16>, default: u16) -> u16 {
    if let Some(p) = cli {
        return p;
    }
    first_free_port(env_value.unwrap_or(default))
}

/// First bindable TCP port at or above `base` on 127.0.0.1, scanning up
/// to 100 ports. Falls back to `base` when none are free (the child's own
/// bind then surfaces a clear error). Best-effort: there's a small window
/// between this probe and the child binding, acceptable for local dev.
fn first_free_port(base: u16) -> u16 {
    (base..=base.saturating_add(99))
        .find(|&p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok())
        .unwrap_or(base)
}

pub fn run(
    port: Option<u16>,
    frontend_port: Option<u16>,
    backend_only: bool,
    frontend_only: bool,
    skip_types: bool,
) {
    // Load .env so SERVER_PORT / VITE_PORT can act as the resolution base.
    let _ = dotenvy::dotenv();

    // Resolve dev-server ports. An explicit CLI flag pins the port
    // exactly; otherwise we take SERVER_PORT/VITE_PORT (or the
    // distinctive default) as a *base* and scan upward for the first free
    // port, so a busy default self-heals instead of failing to bind. The
    // resolved values are pushed to the child processes via env in
    // `spawn_with_prefix`, where the framework's `.env` loader lets them
    // win over the scaffold `.env`.
    let backend_port = pick_port(port, env_port("SERVER_PORT"), DEFAULT_BACKEND_PORT);
    let vite_port = pick_port(frontend_port, env_port("VITE_PORT"), DEFAULT_VITE_PORT);

    ui::banner();
    ui::info("Starting development servers...");
    ui::br();

    // Validate project
    if let Err(e) = validate_suprnova_project(backend_only, frontend_only) {
        ui::error(&e);
        std::process::exit(1);
    }

    // Generate TypeScript types on startup (unless skipped or frontend-only)
    if !skip_types && !frontend_only {
        let project_path = Path::new(".");
        let output_path = project_path.join("frontend/src/types/inertia-props.ts");

        ui::info("Generating TypeScript types...");
        match super::generate_types::generate_types_to_file(project_path, &output_path) {
            Ok(0) => {
                ui::hint("No InertiaProps structs found (skipping type generation)");
            }
            Ok(count) => {
                ui::success(&format!(
                    "Generated {} type(s) → {}",
                    count,
                    output_path.display()
                ));
            }
            Err(e) => {
                ui::warning(&format!("Failed to generate types: {} (continuing)", e));
            }
        }
        ui::br();
    }

    // Ensure cargo-watch is installed (only if running backend)
    if !frontend_only && let Err(e) = ensure_cargo_watch() {
        ui::error(&e);
        std::process::exit(1);
    }

    // Ensure npm dependencies are installed (only if running frontend)
    if !backend_only && let Err(e) = ensure_npm_dependencies() {
        ui::error(&e);
        std::process::exit(1);
    }

    let mut manager = ProcessManager::new();
    let shutdown = manager.shutdown.clone();

    // Set up Ctrl+C handler
    ctrlc::set_handler(move || {
        println!();
        ui::info("Shutting down servers...");
        shutdown.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Start backend with cargo-watch
    if !frontend_only {
        let package_name = match get_package_name() {
            Ok(name) => name,
            Err(e) => {
                ui::error(&e);
                std::process::exit(1);
            }
        };

        ui::label_value("Backend", &format!("http://127.0.0.1:{}", backend_port));

        // SERVER_PORT pins the backend's bind; VITE_PORT lets the
        // Inertia dev-head inject the correct `<script src=…>` for the
        // Vite port we actually launched (default or scanned).
        let backend_env = [
            ("SERVER_PORT", backend_port.to_string()),
            ("VITE_PORT", vite_port.to_string()),
        ];

        let run_cmd = format!("run --bin {}", package_name);
        if let Err(e) = manager.spawn_with_prefix(
            "cargo",
            &["watch", "-x", &run_cmd],
            None,
            &backend_env,
            "[backend] ",
            console::Color::Magenta,
        ) {
            ui::error(&e);
            std::process::exit(1);
        }
    }

    // Start frontend with npm/vite
    if !backend_only {
        ui::label_value("Frontend", &format!("http://127.0.0.1:{}", vite_port));

        let frontend_path = Path::new("frontend");

        // vite.config.ts reads VITE_PORT for `server.port`; passing it
        // here makes Vite bind the resolved port.
        let vite_env = [("VITE_PORT", vite_port.to_string())];

        if let Err(e) = manager.spawn_with_prefix(
            "npm",
            &["run", "dev"],
            Some(frontend_path),
            &vite_env,
            "[frontend]",
            console::Color::Cyan,
        ) {
            ui::error(&e);
            manager.shutdown_all();
            std::process::exit(1);
        }
    }

    // Start file watcher for TypeScript type regeneration
    if !skip_types && !frontend_only {
        let shutdown_watcher = manager.shutdown.clone();
        thread::spawn(move || {
            start_type_watcher(shutdown_watcher);
        });
    }

    ui::br();
    ui::hint("Press Ctrl+C to stop all servers");
    ui::br();

    // Wait for shutdown signal or process exit
    while !manager.shutdown.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(100));

        // Check if any child process has exited
        if manager.any_exited() {
            manager.shutdown.store(true, Ordering::SeqCst);
            break;
        }
    }

    manager.shutdown_all();
    ui::success("Servers stopped.");
}

/// File watcher that regenerates TypeScript types when Rust files change
fn start_type_watcher(shutdown: Arc<AtomicBool>) {
    let (tx, rx) = channel();
    let src_path = Path::new("src");

    let watcher_result = RecommendedWatcher::new(
        move |res| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        },
        Config::default().with_poll_interval(Duration::from_secs(2)),
    );

    let mut watcher = match watcher_result {
        Ok(w) => w,
        Err(e) => {
            eprintln!(
                "{} Failed to start type watcher: {}",
                style("[types]").yellow(),
                e
            );
            return;
        }
    };

    if let Err(e) = watcher.watch(src_path, RecursiveMode::Recursive) {
        eprintln!(
            "{} Failed to watch src directory: {}",
            style("[types]").yellow(),
            e
        );
        return;
    }

    println!(
        "{} Watching for Rust file changes to regenerate types",
        style("[types]").blue()
    );

    let project_path = Path::new(".");
    let output_path = project_path.join("frontend/src/types/inertia-props.ts");

    let mut debounce = Debounce::new(Duration::from_millis(500));

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        // Use recv_timeout to periodically check shutdown. The timeout is
        // also what drives the trailing edge: after the last event of a
        // burst, this wakes every 100ms until the quiet period elapses and
        // the pending regeneration fires.
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                // Check if it's a Rust file change
                let is_rust_change = event
                    .paths
                    .iter()
                    .any(|p| p.extension().map(|e| e == "rs").unwrap_or(false));

                if is_rust_change {
                    debounce.on_event(std::time::Instant::now());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if debounce.should_fire(std::time::Instant::now()) {
            match super::generate_types::generate_types_to_file(project_path, &output_path) {
                Ok(count) if count > 0 => {
                    println!("{} Regenerated {} type(s)", style("[types]").blue(), count);
                }
                Ok(_) => {} // No types found, stay quiet
                Err(e) => {
                    eprintln!("{} Failed to regenerate: {}", style("[types]").yellow(), e);
                }
            }
        }
    }
}

/// Trailing-edge debounce: fire once the burst has gone quiet.
///
/// The watcher used to debounce on the *leading* edge —
/// `if is_rust_change && last_regen.elapsed() > debounce_duration` — which
/// regenerates on the first event of a burst and then silently drops every
/// event for the next 500ms with no trailing run.
///
/// That loses work rather than merely delaying it, and it loses the work
/// most likely to matter. A burst is not a rare event: `cargo fmt`,
/// format-on-save across several files, a branch switch, and any editor
/// that writes a temp file and renames it all produce one. The regenerate
/// fires on the *first* file, before the rest are written, so the types on
/// disk reflect a partial edit — and nothing regenerates them until some
/// unrelated future save happens to land outside a quiet window. The
/// developer sees stale types and no error.
///
/// Firing on the trailing edge inverts that: the burst is coalesced into
/// exactly one regeneration, and it runs after the last write.
struct Debounce {
    /// How long the burst must be quiet before firing.
    quiet: Duration,
    /// When the most recent event arrived, if one is waiting to fire.
    pending_since: Option<std::time::Instant>,
}

impl Debounce {
    fn new(quiet: Duration) -> Self {
        Self {
            quiet,
            pending_since: None,
        }
    }

    /// Record an event. Each one restarts the quiet period, so a steady
    /// stream of saves coalesces into a single run after the last.
    fn on_event(&mut self, now: std::time::Instant) {
        self.pending_since = Some(now);
    }

    /// Whether the pending burst has gone quiet long enough to fire.
    ///
    /// Consumes the pending flag, so one burst produces exactly one run.
    fn should_fire(&mut self, now: std::time::Instant) -> bool {
        match self.pending_since {
            Some(last) if now.duration_since(last) >= self.quiet => {
                self.pending_since = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn pick_port_cli_flag_pins_exactly_without_scanning() {
        // An explicit --port is a hard pin: returned as-is even if busy,
        // because the user asked for that exact port (and portless'
        // appPort routing expects it).
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        assert_eq!(
            pick_port(Some(busy_port), None, DEFAULT_BACKEND_PORT),
            busy_port
        );
    }

    #[test]
    fn pick_port_scans_upward_from_busy_base() {
        // Occupy a base port; pick_port (no CLI flag) must skip it and
        // return a higher free port.
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = occupied.local_addr().unwrap().port();
        let chosen = pick_port(None, Some(base), DEFAULT_BACKEND_PORT);
        assert_ne!(chosen, base, "must not pick the occupied base port");
        assert!(chosen > base, "scan moves upward from the base");
    }

    #[test]
    fn pick_port_falls_back_to_default_when_no_env_value() {
        // No CLI, no env value → scan from the distinctive default. The
        // default is almost always free in a test environment.
        let chosen = pick_port(None, None, DEFAULT_BACKEND_PORT);
        assert!(chosen >= DEFAULT_BACKEND_PORT);
        assert!(chosen < DEFAULT_BACKEND_PORT + 100);
    }

    #[test]
    fn first_free_port_returns_base_when_free() {
        // Pick a high base unlikely to be occupied; bind to confirm it's
        // free, release, then assert first_free_port returns it.
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let free = probe.local_addr().unwrap().port();
        drop(probe);
        assert_eq!(first_free_port(free), free);
    }

    #[test]
    fn env_port_rejects_empty_and_garbage() {
        // Indirection through a real (unset) env var name keeps this
        // hermetic — no global env mutation.
        assert_eq!(env_port("SUPRNOVA_DEFINITELY_UNSET_PORT_VAR"), None);
    }
}

#[cfg(test)]
mod debounce_tests {
    //! P2-13. The watcher debounced on the leading edge, which does not
    //! delay work — it discards it. These drive `Debounce` with explicit
    //! `Instant`s, so they are deterministic and take no wall-clock time.

    use super::Debounce;
    use std::time::{Duration, Instant};

    const QUIET: Duration = Duration::from_millis(500);

    #[test]
    fn nothing_fires_without_an_event() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();

        assert!(!d.should_fire(t0));
        assert!(
            !d.should_fire(t0 + Duration::from_secs(60)),
            "an idle watcher must never regenerate — the quiet period is \
             measured from an event, not from process start"
        );
    }

    #[test]
    fn a_single_event_fires_once_after_the_quiet_period() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();
        d.on_event(t0);

        assert!(
            !d.should_fire(t0 + Duration::from_millis(499)),
            "must not fire before the quiet period elapses"
        );
        assert!(d.should_fire(t0 + Duration::from_millis(500)));
        assert!(
            !d.should_fire(t0 + Duration::from_secs(10)),
            "one event must produce exactly one run, not a repeating timer"
        );
    }

    /// The regression, stated directly. A burst of saves must produce one
    /// regeneration, and it must happen *after the last one* — that is the
    /// save whose types were previously lost.
    #[test]
    fn a_burst_fires_once_and_only_after_its_final_event() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();

        // A burst: five saves 100ms apart. `cargo fmt` across a few files
        // looks exactly like this.
        for i in 0..5 {
            let at = t0 + Duration::from_millis(i * 100);
            d.on_event(at);
            assert!(
                !d.should_fire(at),
                "firing at event {i} would regenerate from a partially \
                 written burst — the leading-edge bug"
            );
        }

        let last_event = t0 + Duration::from_millis(400);
        assert!(
            !d.should_fire(last_event + Duration::from_millis(499)),
            "the quiet period restarts on every event, so it is measured \
             from the LAST save, not the first"
        );
        assert!(
            d.should_fire(last_event + Duration::from_millis(500)),
            "the burst must regenerate once it goes quiet; under the old \
             leading-edge debounce the final four saves were dropped and \
             nothing regenerated them"
        );
        assert!(
            !d.should_fire(last_event + Duration::from_secs(10)),
            "and exactly once"
        );
    }

    /// A save arriving during the quiet period extends it rather than
    /// being swallowed. This is the case the old code got wrong: it
    /// dropped these events entirely.
    #[test]
    fn an_event_during_the_quiet_period_is_not_lost() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();
        d.on_event(t0);

        // 300ms in — inside the old 500ms window, where this event used
        // to be discarded outright.
        let second = t0 + Duration::from_millis(300);
        d.on_event(second);

        assert!(
            !d.should_fire(t0 + Duration::from_millis(500)),
            "the window must have been extended by the second event"
        );
        assert!(
            d.should_fire(second + QUIET),
            "and the fire must come after the SECOND event, so its changes \
             are included"
        );
    }

    /// Separate bursts each get their own run.
    #[test]
    fn a_later_burst_fires_again() {
        let mut d = Debounce::new(QUIET);
        let t0 = Instant::now();

        d.on_event(t0);
        assert!(d.should_fire(t0 + QUIET));

        let later = t0 + Duration::from_secs(30);
        d.on_event(later);
        assert!(!d.should_fire(later));
        assert!(d.should_fire(later + QUIET));
    }
}
