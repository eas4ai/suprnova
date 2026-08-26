use console::style;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

use crate::ui;

/// Floor of the respawn backoff: how long to wait before the *first*
/// respawn attempt after a crash.
const BACKOFF_FLOOR: Duration = Duration::from_millis(200);
/// Ceiling the backoff climbs to and stays at through a sustained crash
/// loop, so a broken process retries at a bounded rate instead of an
/// ever-slower one.
const BACKOFF_CAP: Duration = Duration::from_secs(5);
/// How long a process must stay up since its last spawn before the *next*
/// crash is treated as new rather than a continuation of an old crash
/// loop, resetting the backoff to the floor.
const HEALTHY_AFTER: Duration = Duration::from_secs(30);

/// Exponential backoff between respawn attempts for one crashed dev
/// process: 200ms, doubling on each consecutive crash, capped at 5s.
/// [`Self::record_spawn`] marks when a (re)spawn happened; if the process
/// survives 30s before its next crash, [`Self::next_delay`] treats that
/// crash as new rather than continuing the climb from wherever an old
/// crash loop left off. [`Self::tries`] tracks the same "is this a
/// continuation of the same crash loop" window for `--restart-tries`: it
/// resets to 0 at exactly the moments `current` resets to the floor, so
/// a process that stays up `HEALTHY_AFTER` gets a fresh try budget too.
struct RestartBackoff {
    current: Duration,
    spawned_at: Option<Instant>,
    tries: u32,
}

impl RestartBackoff {
    fn new() -> Self {
        Self {
            current: BACKOFF_FLOOR,
            spawned_at: None,
            tries: 0,
        }
    }

    /// Record that the process was (re)spawned at `now`.
    fn record_spawn(&mut self, now: Instant) {
        self.spawned_at = Some(now);
    }

    /// The process just exited at `now`. Returns how long to wait before
    /// respawning it, and advances the backoff for the *next* crash.
    fn next_delay(&mut self, now: Instant) -> Duration {
        if let Some(at) = self.spawned_at
            && now.duration_since(at) >= HEALTHY_AFTER
        {
            self.current = BACKOFF_FLOOR;
            self.tries = 0;
        }
        self.tries += 1;
        let delay = self.current;
        self.current = (self.current * 2).min(BACKOFF_CAP);
        delay
    }

    /// Consecutive crashes (or failed respawn attempts) since the backoff
    /// last reset to the floor. `--restart-tries` gives up once this
    /// exceeds the configured limit.
    fn tries(&self) -> u32 {
        self.tries
    }
}

/// Colors auto-assigned to `Suprnova.toml` dev processes that don't set
/// one, rotating so several unstyled entries stay visually distinct.
/// Skips magenta/cyan - those are the backend/frontend prefixes.
const DEV_PROCESS_PALETTE: [console::Color; 4] = [
    console::Color::Green,
    console::Color::Yellow,
    console::Color::Blue,
    console::Color::White,
];

/// The only keys a `[[serve.process]]` entry recognizes. An entry
/// carrying any other key (a `colour` typo, say) is a hard error rather
/// than a silently ignored one - the same "fail fast on a malformed file"
/// stance as every other check in `parse_dev_processes`.
const KNOWN_PROCESS_KEYS: [&str; 4] = ["name", "command", "args", "color"];

fn color_from_name(name: &str) -> Option<console::Color> {
    match name.to_ascii_lowercase().as_str() {
        "black" => Some(console::Color::Black),
        "red" => Some(console::Color::Red),
        "green" => Some(console::Color::Green),
        "yellow" => Some(console::Color::Yellow),
        "blue" => Some(console::Color::Blue),
        "magenta" => Some(console::Color::Magenta),
        "cyan" => Some(console::Color::Cyan),
        "white" => Some(console::Color::White),
        _ => None,
    }
}

/// One extra dev process declared in the project's `Suprnova.toml`, run
/// alongside the backend and frontend under `suprnova serve`. Suprnova's
/// answer to Laravel's `DevCommands::register($command, $name)`: Laravel
/// registers from inside the same PHP process that then execs the
/// multiplexer, but `suprnova serve` is a separate binary that never
/// links or runs the app's Rust code, so registration is declarative data
/// the CLI reads instead of a call the CLI makes into the app.
#[derive(Debug, Clone, PartialEq)]
struct DevProcessConfig {
    name: String,
    command: String,
    args: Vec<String>,
    color: console::Color,
}

/// Load `[[serve.process]]` entries from `Suprnova.toml` at `path`. A
/// missing file means "nothing configured" (`Ok(vec![])`) - most projects
/// will never have one. A file that exists but is malformed, or has a
/// broken entry, is a hard error: silently skipping it would start
/// `serve` without a process the developer believes is running.
fn load_dev_processes(path: &Path) -> Result<Vec<DevProcessConfig>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("Failed to read {}: {}", path.display(), e)),
    };
    parse_dev_processes(&content)
}

fn parse_dev_processes(content: &str) -> Result<Vec<DevProcessConfig>, String> {
    let table: toml::Table = content
        .parse()
        .map_err(|e| format!("Suprnova.toml is not valid TOML: {e}"))?;

    let Some(entries) = table
        .get("serve")
        .and_then(|s| s.get("process"))
        .and_then(|p| p.as_array())
    else {
        return Ok(Vec::new());
    };

    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(entries.len());

    for (i, entry) in entries.iter().enumerate() {
        if let Some(table) = entry.as_table() {
            for key in table.keys() {
                if !KNOWN_PROCESS_KEYS.contains(&key.as_str()) {
                    return Err(format!(
                        "Suprnova.toml: serve.process[{i}] has an unknown key \"{key}\"; \
                         expected one of: {}",
                        KNOWN_PROCESS_KEYS.join(", ")
                    ));
                }
            }
        }

        // Trimmed before the emptiness check so a whitespace-only value
        // ("   ") is caught here, with a message naming the file and the
        // entry, instead of passing parsing and surfacing later as an
        // opaque OS spawn error.
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                format!("Suprnova.toml: serve.process[{i}] is missing a non-empty `name`")
            })?
            .to_string();

        if !seen.insert(name.clone()) {
            return Err(format!(
                "Suprnova.toml: duplicate serve.process name \"{name}\""
            ));
        }

        let command = entry
            .get("command")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                format!("Suprnova.toml: serve.process \"{name}\" is missing a non-empty `command`")
            })?
            .to_string();

        let args = match entry.get("args") {
            None => Vec::new(),
            Some(v) => v
                .as_array()
                .ok_or_else(|| {
                    format!(
                        "Suprnova.toml: serve.process \"{name}\" `args` must be an array of strings"
                    )
                })?
                .iter()
                .map(|a| {
                    a.as_str().map(str::to_string).ok_or_else(|| {
                        format!(
                            "Suprnova.toml: serve.process \"{name}\" `args` must all be strings"
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        };

        let color = match entry.get("color").and_then(|v| v.as_str()) {
            Some(color_name) => color_from_name(color_name).ok_or_else(|| {
                format!(
                    "Suprnova.toml: serve.process \"{name}\" has an unknown color \"{color_name}\"; \
                     expected one of: black, red, green, yellow, blue, magenta, cyan, white"
                )
            })?,
            None => DEV_PROCESS_PALETTE[i % DEV_PROCESS_PALETTE.len()],
        };

        out.push(DevProcessConfig {
            name,
            command,
            args,
            color,
        });
    }

    Ok(out)
}

/// How `suprnova serve` renders what child processes and the manager
/// itself are doing. The two modes are mutually exclusive on stdout:
/// `Prefixed` writes colored `[name] line` text (optionally
/// timestamped); `Json` writes one [`DevEvent`] per line and nothing
/// else - no banner, no `[name]` prefixes, no hints, no "installing
/// dependencies" notices. `stderr` (`ui::warning`/`ui::error`) is
/// unaffected by this choice in either mode - see the design notes
/// above for why that split is deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Prefixed { timestamps: bool },
    Json,
}

impl OutputMode {
    fn is_json(self) -> bool {
        matches!(self, OutputMode::Json)
    }
}

/// Which of a child's two output streams a [`DevEvent::Output`] line
/// came from.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutputStream {
    Stdout,
    Stderr,
}

/// Which generated TypeScript artifact a [`DevEvent::TypesRegenerated`]
/// event describes.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum TypesArtifact {
    /// `frontend/src/types/inertia-props.ts`, from
    /// `#[derive(InertiaProps)]` structs under `src/`.
    InertiaProps,
    /// `frontend/src/types/lang-keys.ts`, from `.ftl` catalogs under
    /// `lang/`.
    LangKeys,
}

/// One `--json` NDJSON event: exactly one of these, serialized as a
/// single JSON object, per line on stdout. This is the entire contract
/// of `--json` mode - nothing else is written to stdout while it's
/// active.
///
/// # Stability
/// This is machine-readable output other programs parse. The `type` tag
/// values and the field names below are a stable contract: they will
/// not be renamed, retyped, or removed without a note in
/// `CHANGELOG.md`. New variants, and new fields on existing variants,
/// may be added; a consumer must ignore an unrecognized `type` or an
/// unexpected extra field rather than erroring on it.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DevEvent {
    /// A process (backend, frontend, or a `Suprnova.toml` entry) was
    /// spawned for the first time this session.
    Started { ts: String, name: String, pid: u32 },
    /// One line of a child's stdout or stderr. Carried as a field
    /// instead of passed through raw so a consumer never has to
    /// distinguish "a JSON event" from "a raw child line" on the same
    /// stream - every stdout line `suprnova serve --json` writes is one
    /// of these variants, full stop.
    Output {
        ts: String,
        name: String,
        stream: OutputStream,
        line: String,
    },
    /// A process exited. `code` is `None` when it was killed by a
    /// signal rather than returning a status (matches
    /// `std::process::ExitStatus::code`).
    Exited {
        ts: String,
        name: String,
        code: Option<i32>,
    },
    /// A crashed process will be respawned after `delay_ms` - the
    /// backoff [`RestartBackoff::next_delay`] just computed.
    RestartScheduled {
        ts: String,
        name: String,
        delay_ms: u64,
    },
    /// A scheduled respawn succeeded; the process is running again
    /// under a new PID.
    RestartSucceeded { ts: String, name: String, pid: u32 },
    /// A process crashed `--restart-tries` consecutive times without
    /// staying up long enough to reset the count (the same
    /// [`HEALTHY_AFTER`] window the backoff itself uses), and
    /// `suprnova serve` stopped retrying it. The other processes, and
    /// the session itself, keep running - matches Laravel's own
    /// `concurrently --restart-tries=5` default, which doesn't tear the
    /// whole session down on one process giving up either.
    GaveUp {
        ts: String,
        name: String,
        tries: u32,
    },
    /// The watcher on `src/` (or `lang/`) regenerated a TypeScript
    /// artifact in response to a file change. `count` is how many items
    /// (types or message ids) were written; a `count` of 0 - nothing to
    /// regenerate, or the file was cleaned up - never fires this event,
    /// same as the equivalent human-readable line stays quiet in that
    /// case.
    TypesRegenerated {
        ts: String,
        artifact: TypesArtifact,
        count: u32,
    },
    /// The whole `serve` session is shutting down (`Ctrl+C`, or a crash
    /// under `--no-restart`) and every child is being killed. Emitted
    /// from [`ProcessManager::shutdown_all`], the one chokepoint every
    /// shutdown path already funnels through, so it fires exactly once
    /// and is always the last line `--json` mode writes.
    Shutdown { ts: String },
}

/// Extracts the bare process name from a display prefix like
/// `"[backend] "` or `"[queue]"`, for use in `DevEvent` payloads - the
/// brackets and padding that line up prefixed terminal output are noise
/// there.
fn bare_name(prefix: &str) -> String {
    prefix
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string()
}

/// Current local time as RFC 3339 with millisecond precision - the `ts`
/// field on every `DevEvent`.
fn now_ts() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// Write `event` as one NDJSON line on stdout, or do nothing outside
/// `--json` mode. The sole place that touches stdout for
/// `OutputMode::Json` - every event in this module funnels through
/// here, so "one JSON object per line, nothing else" is enforced in one
/// spot rather than trusted at every call site.
fn emit_event(mode: OutputMode, event: DevEvent) {
    if mode.is_json()
        && let Ok(line) = serde_json::to_string(&event)
    {
        println!("{line}");
    }
}

/// Everything needed to spawn (or respawn) one dev process identically.
struct ProcessSpec {
    command: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    envs: Vec<(String, String)>,
    /// Display prefix for `Prefixed` output, e.g. `"[backend] "`.
    prefix: String,
    /// Bare name for `DevEvent` payloads, e.g. `"backend"` - derived
    /// once from `prefix` at spawn time via [`bare_name`].
    name: String,
    color: console::Color,
}

/// A managed process is either a live child, exited and waiting for its
/// backoff delay to elapse before it's respawned, or - once
/// `--restart-tries` consecutive crashes are exhausted - permanently
/// given up on.
enum ProcessState {
    Running(Child),
    PendingRestart {
        respawn_at: Instant,
    },
    /// `poll` stopped retrying this process; there's no child to manage
    /// and this state never transitions again. The other entries in
    /// [`ProcessManager::processes`] are unaffected.
    GaveUp,
}

/// One dev process under supervision: its spec (for respawning), its
/// current state, and its restart backoff.
struct ManagedProcess {
    spec: ProcessSpec,
    state: ProcessState,
    backoff: RestartBackoff,
}

struct ProcessManager {
    processes: Vec<ManagedProcess>,
    shutdown: Arc<AtomicBool>,
    /// `--no-restart`, inverted: `true` means a crashed child gets
    /// respawned; `false` restores the pre-restart behaviour, where any
    /// exit tears the whole session down.
    restart: bool,
    /// `--restart-tries`: give up retrying a process once its backoff's
    /// [`RestartBackoff::tries`] exceeds this many consecutive crashes.
    /// Irrelevant when `restart` is `false` - `--no-restart` already
    /// ends the session on the very first crash, before this is ever
    /// consulted.
    restart_tries: u32,
    /// How output is rendered: colored `[name]`-prefixed text
    /// (optionally timestamped), or `--json` NDJSON events. See
    /// [`OutputMode`].
    mode: OutputMode,
}

impl ProcessManager {
    fn new(restart: bool, restart_tries: u32, mode: OutputMode) -> Self {
        Self {
            processes: Vec::new(),
            shutdown: Arc::new(AtomicBool::new(false)),
            restart,
            restart_tries,
            mode,
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
        let spec = ProcessSpec {
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.map(Path::to_path_buf),
            envs: envs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            prefix: prefix.to_string(),
            name: bare_name(prefix),
            color,
        };
        let child = spawn_child_and_stream(&spec, self.shutdown.clone(), self.mode)?;
        let pid = child.id();
        emit_event(
            self.mode,
            DevEvent::Started {
                ts: now_ts(),
                name: spec.name.clone(),
                pid,
            },
        );
        let mut backoff = RestartBackoff::new();
        backoff.record_spawn(Instant::now());
        self.processes.push(ManagedProcess {
            spec,
            state: ProcessState::Running(child),
            backoff,
        });
        Ok(())
    }

    fn shutdown_all(&mut self) {
        // The single chokepoint every shutdown path (Ctrl+C, a
        // --no-restart crash, a fatal spawn failure) already calls, so
        // `Shutdown` fires exactly once regardless of cause.
        emit_event(self.mode, DevEvent::Shutdown { ts: now_ts() });
        self.shutdown.store(true, Ordering::SeqCst);
        for mp in &mut self.processes {
            if let ProcessState::Running(child) = &mut mp.state {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// Reap exited children, schedule or perform respawns, and report
    /// whether a crash means the whole session must shut down
    /// (`--no-restart` only - with restart enabled, a crash is handled
    /// here and never asks the caller to stop). `stderr` diagnostics
    /// (`ui::warning`/`ui::error`) print unconditionally, same as
    /// before this task; the `DevEvent`s alongside them are the
    /// `--json`-mode equivalent on stdout, and `emit_event` no-ops
    /// outside `--json` so both call sites are always safe to reach.
    fn poll(&mut self) -> bool {
        let now = Instant::now();
        let mut must_shutdown = false;

        for mp in &mut self.processes {
            match &mut mp.state {
                ProcessState::Running(child) => {
                    let Ok(Some(status)) = child.try_wait() else {
                        continue;
                    };
                    emit_event(
                        self.mode,
                        DevEvent::Exited {
                            ts: now_ts(),
                            name: mp.spec.name.clone(),
                            code: status.code(),
                        },
                    );
                    if !self.restart {
                        ui::warning(&format!(
                            "{} exited ({status}); --no-restart is set, shutting down",
                            mp.spec.prefix
                        ));
                        must_shutdown = true;
                        continue;
                    }
                    let delay = mp.backoff.next_delay(now);
                    if mp.backoff.tries() > self.restart_tries {
                        give_up(mp, self.mode, self.restart_tries);
                        continue;
                    }
                    ui::warning(&format!(
                        "{} exited ({status}); respawning in {}ms",
                        mp.spec.prefix,
                        delay.as_millis()
                    ));
                    emit_event(
                        self.mode,
                        DevEvent::RestartScheduled {
                            ts: now_ts(),
                            name: mp.spec.name.clone(),
                            delay_ms: delay.as_millis() as u64,
                        },
                    );
                    mp.state = ProcessState::PendingRestart {
                        respawn_at: now + delay,
                    };
                }
                ProcessState::PendingRestart { respawn_at } => {
                    if now < *respawn_at {
                        continue;
                    }
                    match spawn_child_and_stream(&mp.spec, self.shutdown.clone(), self.mode) {
                        Ok(child) => {
                            mp.backoff.record_spawn(now);
                            let pid = child.id();
                            mp.state = ProcessState::Running(child);
                            emit_event(
                                self.mode,
                                DevEvent::RestartSucceeded {
                                    ts: now_ts(),
                                    name: mp.spec.name.clone(),
                                    pid,
                                },
                            );
                        }
                        Err(e) => {
                            ui::error(&format!("Failed to respawn {}: {}", mp.spec.prefix, e));
                            let delay = mp.backoff.next_delay(now);
                            if mp.backoff.tries() > self.restart_tries {
                                give_up(mp, self.mode, self.restart_tries);
                                continue;
                            }
                            emit_event(
                                self.mode,
                                DevEvent::RestartScheduled {
                                    ts: now_ts(),
                                    name: mp.spec.name.clone(),
                                    delay_ms: delay.as_millis() as u64,
                                },
                            );
                            mp.state = ProcessState::PendingRestart {
                                respawn_at: now + delay,
                            };
                        }
                    }
                }
                // Permanently given up on; nothing left to poll for this
                // entry. The other processes in `self.processes` (and the
                // session itself) are unaffected - see `give_up`'s doc
                // comment for why.
                ProcessState::GaveUp => {}
            }
        }

        must_shutdown
    }
}

/// Stop retrying `mp` after its `--restart-tries` budget is exhausted:
/// print an actionable terminal message naming the process and what to
/// do, emit the machine-readable equivalent, and mark it given up so
/// `poll` never revisits it.
///
/// This does *not* tear the whole `serve` session down - only `mp`
/// stops. Laravel's own `concurrently --restart-tries=5` (no
/// `--kill-others-on-fail` in that branch - see
/// `DevCommand::buildConcurrentlyCommand`) behaves the same way: a
/// process exhausting its retries doesn't kill its siblings, only
/// `--no-restart`'s immediate-crash-ends-everything path does that. A
/// `--json` consumer needs this event precisely because a permanently
/// dead process retrying invisibly forever - the bug this whole flag
/// exists to fix - is exactly the failure mode it otherwise cannot see.
fn give_up(mp: &mut ManagedProcess, mode: OutputMode, restart_tries: u32) {
    ui::error(&format!(
        "gave up restarting `{}` after {restart_tries} attempts; fix the error and run \
         `suprnova serve` again",
        mp.spec.name
    ));
    emit_event(
        mode,
        DevEvent::GaveUp {
            ts: now_ts(),
            name: mp.spec.name.clone(),
            tries: restart_tries,
        },
    );
    mp.state = ProcessState::GaveUp;
}

/// Spawn `spec`'s command and wire its stdout/stderr to either prefixed,
/// optionally timestamped lines (`OutputMode::Prefixed`) or `DevEvent`
/// NDJSON lines (`OutputMode::Json`) - both stdout- and stderr-sourced
/// lines are carried as `DevEvent::Output` payloads on *our* stdout in
/// `--json` mode, never passed through raw, so a consumer never has to
/// also watch our stderr for child output. Used for both the first spawn
/// of a process and every respawn - a respawned child gets fresh reader
/// threads because its pipes are new.
fn spawn_child_and_stream(
    spec: &ProcessSpec,
    shutdown: Arc<AtomicBool>,
    mode: OutputMode,
) -> Result<Child, String> {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // The framework's `.env` loader (Phase 5a in config/env.rs) restores
    // real system env over file values, so a var we set on the child here
    // wins over the scaffold `.env` - that's how the resolved/scanned
    // ports reach the backend and Vite.
    for (key, value) in &spec.envs {
        cmd.env(key, value);
    }
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn {}: {}", spec.command, e))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let shutdown_stdout = shutdown.clone();
    let shutdown_stderr = shutdown;
    let prefix_out = spec.prefix.clone();
    let prefix_err = spec.prefix.clone();
    let name_out = spec.name.clone();
    let name_err = spec.name.clone();
    let color = spec.color;

    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if shutdown_stdout.load(Ordering::SeqCst) {
                break;
            }
            if let Ok(line) = line {
                match mode {
                    OutputMode::Prefixed { timestamps } => {
                        print_prefixed(&prefix_out, color, &line, timestamps)
                    }
                    OutputMode::Json => emit_event(
                        mode,
                        DevEvent::Output {
                            ts: now_ts(),
                            name: name_out.clone(),
                            stream: OutputStream::Stdout,
                            line,
                        },
                    ),
                }
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
                match mode {
                    OutputMode::Prefixed { timestamps } => {
                        eprint_prefixed(&prefix_err, color, &line, timestamps)
                    }
                    // Note: a stderr-sourced line still becomes a
                    // DevEvent::Output on *our* stdout, not stderr - see
                    // the schema's doc comment on why.
                    OutputMode::Json => emit_event(
                        mode,
                        DevEvent::Output {
                            ts: now_ts(),
                            name: name_err.clone(),
                            stream: OutputStream::Stderr,
                            line,
                        },
                    ),
                }
            }
        }
    });

    Ok(child)
}

fn print_prefixed(prefix: &str, color: console::Color, line: &str, timestamps: bool) {
    if timestamps {
        println!(
            "{} {} {}",
            style(chrono::Local::now().format("%H:%M:%S")).dim(),
            style(prefix).fg(color).bold(),
            line
        );
    } else {
        println!("{} {}", style(prefix).fg(color).bold(), line);
    }
}

fn eprint_prefixed(prefix: &str, color: console::Color, line: &str, timestamps: bool) {
    if timestamps {
        eprintln!(
            "{} {} {}",
            style(chrono::Local::now().format("%H:%M:%S")).dim(),
            style(prefix).fg(color).bold(),
            line
        );
    } else {
        eprintln!("{} {}", style(prefix).fg(color).bold(), line);
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

fn validate_suprnova_project(frontend_only: bool) -> Result<(), String> {
    let cargo_toml = Path::new("Cargo.toml");

    if !frontend_only && !cargo_toml.exists() {
        return Err("No Cargo.toml found. Are you in a Suprnova project directory?".into());
    }

    Ok(())
}

/// Whether this project has a frontend for `serve` to run.
///
/// Laravel registers its Vite pane only when `base_path('package.json')`
/// exists, and the same question applies here: did the scaffolder write a
/// frontend? `suprnova new --api` writes none at all. The `package.json`
/// half of the check earns its keep because type generation creates
/// `frontend/src/types/` on its own, so a bare `frontend/` directory can
/// exist without ever having been an npm project.
fn frontend_present() -> bool {
    let frontend_dir = Path::new("frontend");
    frontend_dir.is_dir() && frontend_dir.join("package.json").is_file()
}

/// Which dev panes this invocation runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DevPanes {
    /// Run `cargo watch` on the app binary.
    backend: bool,
    /// Run `npm run dev` in `frontend/`, and install its dependencies
    /// first.
    frontend: bool,
}

/// Decide which panes to run from the two flags and whether the project
/// actually has a frontend.
///
/// A frontend-less project used to be rejected as "not a Suprnova project"
/// unless the user remembered `--backend-only`. It is a valid project; it
/// simply has no frontend pane. `--frontend-only` stays an error there,
/// because it asks for the one pane that does not exist.
fn decide_panes(
    backend_only: bool,
    frontend_only: bool,
    frontend_present: bool,
) -> Result<DevPanes, String> {
    if frontend_only && !frontend_present {
        return Err(
            "`--frontend-only` needs a `frontend/` directory with a package.json, and \
                    this project has none. A JSON:API-only project (`suprnova new --api`) has \
                    no frontend: run `suprnova serve` without the flag to serve the backend."
                .into(),
        );
    }

    Ok(DevPanes {
        backend: !frontend_only,
        frontend: !backend_only && frontend_present,
    })
}

/// Version requirement for the `cargo-watch` we install on demand.
///
/// Bounded to a major version because `serve` drives it as
/// `cargo watch -x <cmd>` - a flag whose meaning is not guaranteed across
/// a major bump. Unbounded, `cargo install cargo-watch` takes whatever is
/// newest, so a future release could break `suprnova serve` on machines
/// that happened to install it that day, with nothing in this repo
/// changing.
const CARGO_WATCH_VERSION_REQ: &str = "^8.5";

fn ensure_cargo_watch(json: bool) -> Result<(), String> {
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
            // newest on the day - the thing you least want from a command
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
            if !json {
                ui::success("cargo-watch installed");
            }
            Ok(())
        }
    }
}

fn ensure_npm_dependencies(json: bool) -> Result<(), String> {
    let frontend_path = Path::new("frontend");
    let node_modules = frontend_path.join("node_modules");

    if !node_modules.exists() {
        if !json {
            ui::info("Installing frontend dependencies...");
        }
        let npm_install = Command::new("npm")
            .args(["install"])
            .current_dir(frontend_path)
            .status()
            .map_err(|e| format!("Failed to run npm install: {}", e))?;

        if !npm_install.success() {
            return Err("Failed to install npm dependencies".into());
        }
        if !json {
            ui::success("Frontend dependencies installed");
        }
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

// Arguments mirror `Commands::Serve`'s CLI flags one-to-one, dispatched
// verbatim from `main.rs` like every other `commands::*::run` - grouping
// them into a struct would just move the flag list, not shrink it.
#[allow(clippy::too_many_arguments)]
pub fn run(
    port: Option<u16>,
    frontend_port: Option<u16>,
    backend_only: bool,
    frontend_only: bool,
    skip_types: bool,
    no_restart: bool,
    restart_tries: u32,
    timestamps: bool,
    json: bool,
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

    if !json {
        ui::banner();
        ui::info("Starting development servers...");
        ui::br();
    }

    // Validate project
    if let Err(e) = validate_suprnova_project(frontend_only) {
        ui::error(&e);
        std::process::exit(1);
    }

    let has_frontend = frontend_present();
    let panes = match decide_panes(backend_only, frontend_only, has_frontend) {
        Ok(panes) => panes,
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    };
    if !backend_only && !has_frontend && !json {
        ui::hint("No frontend/package.json found - serving the backend only.");
    }

    // Extra dev processes from the project's Suprnova.toml (Laravel's
    // `DevCommands::register`). Loaded early so a malformed file fails
    // fast, before any real process is spawned.
    let dev_processes = match load_dev_processes(Path::new("Suprnova.toml")) {
        Ok(procs) => procs,
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    };

    // Generate TypeScript types on startup (unless skipped or frontend-only)
    if !skip_types && !frontend_only && has_frontend {
        let project_path = Path::new(".");
        let output_path = project_path.join("frontend/src/types/inertia-props.ts");

        if !json {
            ui::info("Generating TypeScript types...");
        }
        match super::generate_types::generate_types_to_file(project_path, &output_path) {
            Ok(0) => {
                if !json {
                    ui::hint("No InertiaProps structs found (skipping type generation)");
                }
            }
            Ok(count) => {
                if !json {
                    ui::success(&format!(
                        "Generated {} type(s) → {}",
                        count,
                        output_path.display()
                    ));
                }
            }
            Err(e) => {
                ui::warning(&format!("Failed to generate types: {} (continuing)", e));
            }
        }

        // lang-keys.ts stays quiet on Ok(0) (no `lang/` dir, or zero
        // message ids) rather than getting the "no structs found" hint
        // InertiaProps gets above - most projects aren't localized at
        // all, and printing that on every single `serve` would be
        // permanent noise for the common case.
        let lang_keys_output = project_path.join("frontend/src/types/lang-keys.ts");
        match super::generate_types::generate_lang_keys_to_file(project_path, &lang_keys_output) {
            Ok(0) => {}
            Ok(count) => {
                if !json {
                    ui::success(&format!(
                        "Generated {} message id(s) → {}",
                        count,
                        lang_keys_output.display()
                    ));
                }
            }
            Err(e) => {
                ui::warning(&format!("Failed to generate lang-keys: {} (continuing)", e));
            }
        }
        if !json {
            ui::br();
        }
    }

    // Ensure cargo-watch is installed (only if running backend)
    if panes.backend
        && let Err(e) = ensure_cargo_watch(json)
    {
        ui::error(&e);
        std::process::exit(1);
    }

    // Ensure npm dependencies are installed (only if running frontend)
    if panes.frontend
        && let Err(e) = ensure_npm_dependencies(json)
    {
        ui::error(&e);
        std::process::exit(1);
    }

    let mode = if json {
        OutputMode::Json
    } else {
        OutputMode::Prefixed { timestamps }
    };
    let mut manager = ProcessManager::new(!no_restart, restart_tries, mode);
    let shutdown = manager.shutdown.clone();

    // Set up Ctrl+C handler. The blank line and "Shutting down..." notice
    // are human decoration only - under --json they'd land on stdout
    // ahead of the final `Shutdown` NDJSON event `shutdown_all` emits,
    // breaking "every stdout line is a DevEvent" for whatever's parsing
    // it. `json` is `Copy`, so capturing it here doesn't disturb its use
    // later in this function.
    ctrlc::set_handler(move || {
        if !json {
            println!();
            ui::info("Shutting down servers...");
        }
        shutdown.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Start backend with cargo-watch
    if panes.backend {
        let package_name = match get_package_name() {
            Ok(name) => name,
            Err(e) => {
                ui::error(&e);
                std::process::exit(1);
            }
        };

        if !json {
            ui::label_value("Backend", &format!("http://127.0.0.1:{}", backend_port));
        }

        // SERVER_PORT pins the backend's bind; VITE_PORT lets the
        // Inertia dev-head inject the correct `<script src=…>` for the
        // Vite port we actually launched (default or scanned).
        let backend_env = [
            ("SERVER_PORT", backend_port.to_string()),
            ("VITE_PORT", vite_port.to_string()),
        ];

        let run_cmd = format!("run --bin {}", package_name);
        let watch_args = backend_watch_args(Path::new("."), &run_cmd);
        let watch_args: Vec<&str> = watch_args.iter().map(String::as_str).collect();
        if let Err(e) = manager.spawn_with_prefix(
            "cargo",
            &watch_args,
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
    if panes.frontend {
        if !json {
            ui::label_value("Frontend", &format!("http://127.0.0.1:{}", vite_port));
        }

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

    // Extra dev processes declared in Suprnova.toml. Always run,
    // independent of --backend-only/--frontend-only - a queue worker or
    // log tailer isn't "the frontend" or "the backend".
    for proc in &dev_processes {
        if !json {
            ui::label_value(&proc.name, &proc.command);
        }
        let args: Vec<&str> = proc.args.iter().map(String::as_str).collect();
        let prefix = format!("[{}]", proc.name);
        if let Err(e) =
            manager.spawn_with_prefix(&proc.command, &args, None, &[], &prefix, proc.color)
        {
            ui::error(&e);
            manager.shutdown_all();
            std::process::exit(1);
        }
    }

    // Start file watcher for TypeScript type regeneration
    if !skip_types && !frontend_only && has_frontend {
        let shutdown_watcher = manager.shutdown.clone();
        thread::spawn(move || {
            start_type_watcher(shutdown_watcher, mode);
        });
    }

    if !json {
        ui::br();
        ui::hint("Press Ctrl+C to stop all servers");
        ui::br();
    }

    // Wait for shutdown signal, or a crash that must end the session
    // (only when --no-restart is set; otherwise crashes are respawned
    // inside poll() and the loop keeps going).
    while !manager.shutdown.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(100));

        if manager.poll() {
            manager.shutdown.store(true, Ordering::SeqCst);
            break;
        }
    }

    manager.shutdown_all();
    if !json {
        ui::success("Servers stopped.");
    }
}

/// Paths, relative to the project root, whose contents the backend
/// process actually depends on - in the order they are handed to
/// `cargo watch`.
///
/// `src`, `Cargo.toml`, and `Cargo.lock` are the build inputs. `.env` is
/// read once by `Config::init` at boot, and `lang/` once by
/// `Localization::bootstrap`, which compiles every `lang/<locale>/*.ftl`
/// catalog into the translator - neither is re-read at request time, so a
/// change to either only takes effect on a restart.
const BACKEND_WATCH_PATHS: [&str; 5] = ["src", "Cargo.toml", "Cargo.lock", ".env", "lang"];

/// Build the `cargo watch` argument list for the backend pane.
///
/// Without any `-w`, cargo-watch watches the whole non-gitignored project,
/// which means every Vite component save and every regenerated
/// `frontend/src/types/*.ts` restarted the backend - a full framework
/// rebuild triggered by editing a `.svelte` file. Naming the paths the
/// backend is actually built from is the entire fix; no `-i` ignore list
/// is needed once the scope is right.
///
/// A `-w` path that does not exist makes cargo-watch refuse to start
/// outright (`Path error: couldn't canonicalize ...`), so each candidate is
/// included only when it is present at spawn time. That is not a rare case:
/// a freshly scaffolded project has no `Cargo.lock` until its first build.
/// A candidate that appears later is picked up by the next `serve`, the
/// same watcher-registration-time gap the type watcher has with `lang/`.
fn backend_watch_args(project: &Path, run_cmd: &str) -> Vec<String> {
    let mut args = vec!["watch".to_string()];
    for candidate in BACKEND_WATCH_PATHS {
        if project.join(candidate).exists() {
            args.push("-w".to_string());
            args.push(candidate.to_string());
        }
    }
    args.push("-x".to_string());
    args.push(run_cmd.to_string());
    args
}

/// What one filesystem event asks the type watcher to regenerate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WatchTrigger {
    /// Regenerate `inertia-props.ts`: a `.rs` file changed.
    rust: bool,
    /// Regenerate `lang-keys.ts`: a `.ftl` catalog changed.
    ftl: bool,
}

impl WatchTrigger {
    /// Nothing to regenerate.
    const NONE: Self = Self {
        rust: false,
        ftl: false,
    };
}

/// Classify one `notify` event into what it should regenerate.
///
/// The event *kind* gate is what keeps this watcher from feeding itself.
/// `notify`'s inotify backend registers `WatchMask::OPEN` on every watched
/// directory, so the kernel reports a plain read of a file as
/// `Access(Open(..))` followed by `Access(Close(Read))`. Regeneration
/// reads every `.rs` file under `src/` - the exact tree this watcher is
/// watching - so each run emits a burst of those `Access` events on the
/// watcher's own channel. Counting them as changes re-arms the
/// trailing-edge debounce, which fires another regeneration 500ms later,
/// which reads the tree again: a project nobody has touched rebuilds its
/// types (and, through cargo-watch, its backend) forever.
///
/// Only kinds that mean the bytes on disk are different now count:
/// `Create`, `Modify`, `Remove`, and `Any` (the backends' "we do not know
/// what this was" catch-all, which must stay conservative). `Access` and
/// `Other` never do.
fn watch_trigger(event: &notify::Event) -> WatchTrigger {
    let is_change = matches!(
        event.kind,
        notify::EventKind::Create(_)
            | notify::EventKind::Modify(_)
            | notify::EventKind::Remove(_)
            | notify::EventKind::Any
    );
    if !is_change {
        return WatchTrigger::NONE;
    }

    let has_extension = |ext: &str| {
        event
            .paths
            .iter()
            .any(|p| p.extension().map(|e| e == ext).unwrap_or(false))
    };

    WatchTrigger {
        rust: has_extension("rs"),
        ftl: has_extension("ftl"),
    }
}

/// File watcher that regenerates TypeScript types when Rust files change,
/// and `lang-keys.ts` when `.ftl` catalogs under `lang/` change. `mode`
/// governs stdout exactly like everywhere else in this module: purely
/// decorative "watching for changes" notices are suppressed under
/// `--json` (nothing a machine consumer would act on), while a
/// regeneration - which carries a count a consumer would want - becomes
/// a [`DevEvent::TypesRegenerated`] instead of a suppressed print.
/// Failure notices stay on stderr unconditionally, same as every other
/// diagnostic in this file.
fn start_type_watcher(shutdown: Arc<AtomicBool>, mode: OutputMode) {
    let (tx, rx) = channel();
    let src_path = Path::new("src");
    let lang_path = Path::new("lang");

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

    if !mode.is_json() {
        println!(
            "{} Watching for Rust file changes to regenerate types",
            style("[types]").blue()
        );
    }

    // `lang/` is optional and, unlike `src/`, may not exist at all for a
    // non-localized project - `notify` can't watch a path that isn't
    // there, so this watch is skipped rather than treated as an error.
    // A project that adds `lang/` after `serve` started needs a restart
    // to pick it up, same as any other watcher-registration-time gap.
    let watch_lang = lang_path.is_dir();
    if watch_lang {
        if let Err(e) = watcher.watch(lang_path, RecursiveMode::Recursive) {
            eprintln!(
                "{} Failed to watch lang directory: {}",
                style("[types]").yellow(),
                e
            );
        } else if !mode.is_json() {
            println!(
                "{} Watching lang/ for .ftl changes to regenerate lang-keys",
                style("[types]").blue()
            );
        }
    }

    let project_path = Path::new(".");
    let output_path = project_path.join("frontend/src/types/inertia-props.ts");
    let lang_keys_output = project_path.join("frontend/src/types/lang-keys.ts");

    let mut debounce = Debounce::new(Duration::from_millis(500));
    // Independent debounce/regeneration for `lang-keys.ts`, mirroring the
    // Rust-file one exactly (same quiet period, same trailing-edge fire) -
    // separate because a `.rs` save shouldn't reparse every `.ftl` file
    // and vice versa; the two artifacts have nothing to do with each
    // other beyond sharing this watcher loop.
    let mut lang_debounce = Debounce::new(Duration::from_millis(500));

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
                let trigger = watch_trigger(&event);
                if trigger.rust {
                    debounce.on_event(std::time::Instant::now());
                }
                if trigger.ftl {
                    lang_debounce.on_event(std::time::Instant::now());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if debounce.should_fire(std::time::Instant::now()) {
            match super::generate_types::generate_types_to_file(project_path, &output_path) {
                Ok(count) if count > 0 => {
                    if mode.is_json() {
                        emit_event(
                            mode,
                            DevEvent::TypesRegenerated {
                                ts: now_ts(),
                                artifact: TypesArtifact::InertiaProps,
                                count: count as u32,
                            },
                        );
                    } else {
                        println!("{} Regenerated {} type(s)", style("[types]").blue(), count);
                    }
                }
                Ok(_) => {} // No types found, stay quiet
                Err(e) => {
                    eprintln!("{} Failed to regenerate: {}", style("[types]").yellow(), e);
                }
            }
        }

        if lang_debounce.should_fire(std::time::Instant::now()) {
            match super::generate_types::generate_lang_keys_to_file(project_path, &lang_keys_output)
            {
                Ok(count) if count > 0 => {
                    if mode.is_json() {
                        emit_event(
                            mode,
                            DevEvent::TypesRegenerated {
                                ts: now_ts(),
                                artifact: TypesArtifact::LangKeys,
                                count: count as u32,
                            },
                        );
                    } else {
                        println!(
                            "{} Regenerated {} message id(s)",
                            style("[types]").blue(),
                            count
                        );
                    }
                }
                Ok(_) => {} // No message ids (or lang/ removed); stale file already cleaned up
                Err(e) => {
                    eprintln!(
                        "{} Failed to regenerate lang-keys: {}",
                        style("[types]").yellow(),
                        e
                    );
                }
            }
        }
    }
}

/// Trailing-edge debounce: fire once the burst has gone quiet.
///
/// The watcher used to debounce on the *leading* edge -
/// `if is_rust_change && last_regen.elapsed() > debounce_duration` - which
/// regenerates on the first event of a burst and then silently drops every
/// event for the next 500ms with no trailing run.
///
/// That loses work rather than merely delaying it, and it loses the work
/// most likely to matter. A burst is not a rare event: `cargo fmt`,
/// format-on-save across several files, a branch switch, and any editor
/// that writes a temp file and renames it all produce one. The regenerate
/// fires on the *first* file, before the rest are written, so the types on
/// disk reflect a partial edit - and nothing regenerates them until some
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
        // hermetic - no global env mutation.
        assert_eq!(env_port("SUPRNOVA_DEFINITELY_UNSET_PORT_VAR"), None);
    }

    #[test]
    fn an_api_project_runs_the_backend_pane_only() {
        let panes = decide_panes(false, false, false).expect("an api project is servable");
        assert!(panes.backend);
        assert!(!panes.frontend, "there is no frontend to run");
    }

    #[test]
    fn a_fullstack_project_runs_both_panes() {
        let panes = decide_panes(false, false, true).expect("a fullstack project is servable");
        assert!(panes.backend);
        assert!(panes.frontend);
    }

    #[test]
    fn the_flags_still_win_over_a_present_frontend() {
        assert_eq!(
            decide_panes(true, false, true).expect("backend-only"),
            DevPanes {
                backend: true,
                frontend: false,
            }
        );
        assert_eq!(
            decide_panes(false, true, true).expect("frontend-only"),
            DevPanes {
                backend: false,
                frontend: true,
            }
        );
    }

    #[test]
    fn frontend_only_without_a_frontend_is_an_error_that_names_the_api_case() {
        let err = decide_panes(false, true, false).expect_err("nothing to run");
        assert!(err.contains("--frontend-only"), "{err}");
        assert!(err.contains("--api"), "{err}");
        assert!(
            !err.contains("Are you in a Suprnova project directory?"),
            "the old misdiagnosis must not survive: {err}"
        );
    }

    #[test]
    fn a_backend_only_project_still_validates_without_a_frontend() {
        // The Cargo.toml half of the check is all that is left, and it
        // does not care about the frontend at all.
        assert!(validate_suprnova_project(true).is_ok());
    }
}

#[cfg(test)]
mod backend_watch_args_tests {
    use super::*;

    const RUN: &str = "run --bin app";

    #[test]
    fn a_bare_project_watches_only_what_it_actually_has() {
        // cargo-watch refuses to start when a `-w` path does not exist,
        // so a project without a lockfile, a `.env`, or `lang/` must not
        // be handed those paths.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("src")).expect("create src");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("create manifest");

        assert_eq!(
            backend_watch_args(dir.path(), RUN),
            vec!["watch", "-w", "src", "-w", "Cargo.toml", "-x", RUN]
        );
    }

    #[test]
    fn a_full_project_watches_sources_manifests_env_and_catalogs() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("src")).expect("create src");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("create manifest");
        std::fs::write(dir.path().join("Cargo.lock"), "\n").expect("create lockfile");
        std::fs::write(dir.path().join(".env"), "APP_KEY=x\n").expect("create .env");
        std::fs::create_dir(dir.path().join("lang")).expect("create lang");

        assert_eq!(
            backend_watch_args(dir.path(), RUN),
            vec![
                "watch",
                "-w",
                "src",
                "-w",
                "Cargo.toml",
                "-w",
                "Cargo.lock",
                "-w",
                ".env",
                "-w",
                "lang",
                "-x",
                RUN
            ]
        );
    }

    #[test]
    fn the_frontend_and_the_generated_types_are_never_watched() {
        // This is the whole point of scoping: a Vite component save, and
        // the `.ts` the type generator writes, must not restart the
        // backend.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("src")).expect("create src");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("create manifest");
        std::fs::create_dir_all(dir.path().join("frontend/src/types")).expect("create frontend");

        let args = backend_watch_args(dir.path(), RUN);
        assert!(
            !args.iter().any(|a| a.contains("frontend")),
            "frontend must stay out of the backend watch scope: {args:?}"
        );
    }
}

#[cfg(test)]
mod watch_trigger_tests {
    use super::*;
    use notify::EventKind;
    use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind, RemoveKind};

    fn event_on(kind: EventKind, path: &str) -> notify::Event {
        notify::Event::new(kind).add_path(PathBuf::from(path))
    }

    /// The three kinds that mean "the bytes on disk are different now".
    fn real_changes() -> [EventKind; 3] {
        [
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            EventKind::Create(CreateKind::File),
            EventKind::Remove(RemoveKind::File),
        ]
    }

    #[test]
    fn reading_a_rust_file_is_not_a_change() {
        // This is the loop: the generator opens and reads every `.rs`
        // file under the tree this watcher is watching, so its own reads
        // must not schedule the next regeneration.
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ] {
            assert_eq!(
                watch_trigger(&event_on(kind, "src/a.rs")),
                WatchTrigger::NONE,
                "{kind:?} on a .rs file must not trigger a regeneration"
            );
        }
    }

    #[test]
    fn writing_creating_or_removing_a_rust_file_triggers_types() {
        for kind in real_changes() {
            let trigger = watch_trigger(&event_on(kind, "src/a.rs"));
            assert!(trigger.rust, "{kind:?} on a .rs file must regenerate types");
            assert!(!trigger.ftl, "{kind:?} on a .rs file is not a lang change");
        }
    }

    #[test]
    fn writing_creating_or_removing_a_catalog_triggers_lang_keys() {
        for kind in real_changes() {
            let trigger = watch_trigger(&event_on(kind, "lang/en/x.ftl"));
            assert!(
                trigger.ftl,
                "{kind:?} on a .ftl file must regenerate lang keys"
            );
            assert!(
                !trigger.rust,
                "{kind:?} on a .ftl file is not a Rust change"
            );
        }
    }

    #[test]
    fn a_file_that_is_neither_rust_nor_fluent_triggers_nothing() {
        assert_eq!(
            watch_trigger(&event_on(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                "src/notes.md"
            )),
            WatchTrigger::NONE
        );
    }

    /// End-to-end against a real inotify watcher, because the bug lived in
    /// the gap between what `notify` emits and what the classifier looked
    /// at: nothing that only builds `Event` values by hand can prove the
    /// kernel does not hand us an `Access` event for a plain read.
    ///
    /// Linux-only: the `OPEN`/`CLOSE` watch mask is an inotify detail, and
    /// the other backends do not report reads at all.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_read_of_a_watched_file_produces_no_trigger_but_a_write_does() {
        use std::sync::mpsc::RecvTimeoutError;

        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).expect("create src");
        let file = src.join("a.rs");
        std::fs::write(&file, "pub struct A;\n").expect("seed a.rs");

        let (tx, rx) = channel();
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
                println!("skipping: no filesystem watcher available ({e})");
                return;
            }
        };
        if let Err(e) = watcher.watch(&src, RecursiveMode::Recursive) {
            println!("skipping: cannot watch a temp dir ({e})");
            return;
        }

        // Drain whatever registering the watch produced.
        while rx.recv_timeout(Duration::from_millis(200)).is_ok() {}

        // A read is exactly what the generator does to every file under
        // the watched tree on each run.
        let _ = std::fs::read_to_string(&file).expect("read a.rs");
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => assert_eq!(
                    watch_trigger(&event),
                    WatchTrigger::NONE,
                    "reading a watched file must not schedule a regeneration, got {:?}",
                    event.kind
                ),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }

        // A write must still get through, or the fix would have traded a
        // loop for a dead watcher.
        std::fs::write(&file, "pub struct A;\npub struct B;\n").expect("rewrite a.rs");
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut saw_change = false;
        while !saw_change && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => saw_change = watch_trigger(&event).rust,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        assert!(saw_change, "a write to a watched .rs file must trigger");
    }
}

#[cfg(test)]
mod debounce_tests {
    //! P2-13. The watcher debounced on the leading edge, which does not
    //! delay work - it discards it. These drive `Debounce` with explicit
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
            "an idle watcher must never regenerate - the quiet period is \
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
    /// regeneration, and it must happen *after the last one* - that is the
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
                 written burst - the leading-edge bug"
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

        // 300ms in - inside the old 500ms window, where this event used
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

#[cfg(test)]
mod restart_backoff_tests {
    //! T15. `RestartBackoff` is the pure state machine behind crash
    //! respawn: exact numbers only, driven by explicit `Instant`s so the
    //! test takes no wall-clock time - same style as `Debounce`'s tests
    //! above.

    use super::{BACKOFF_CAP, BACKOFF_FLOOR, RestartBackoff};
    use std::time::{Duration, Instant};

    #[test]
    fn first_crash_waits_the_floor_delay() {
        let mut b = RestartBackoff::new();
        assert_eq!(b.next_delay(Instant::now()), BACKOFF_FLOOR);
    }

    #[test]
    fn consecutive_crashes_double_up_to_the_cap_and_stay_there() {
        let mut b = RestartBackoff::new();
        let t0 = Instant::now();
        for ms in [200, 400, 800, 1600, 3200, 5000, 5000] {
            assert_eq!(b.next_delay(t0), Duration::from_millis(ms));
        }
        assert_eq!(BACKOFF_CAP, Duration::from_secs(5));
    }

    #[test]
    fn thirty_seconds_of_uptime_resets_the_backoff_to_the_floor() {
        let mut b = RestartBackoff::new();
        let t0 = Instant::now();

        // Two rapid crashes climb the backoff past the floor.
        assert_eq!(b.next_delay(t0), Duration::from_millis(200));
        assert_eq!(b.next_delay(t0), Duration::from_millis(400));

        // Respawned, then survives a full healthy window before crashing again.
        b.record_spawn(t0);
        let healthy_crash = t0 + Duration::from_secs(30);
        assert_eq!(
            b.next_delay(healthy_crash),
            Duration::from_millis(200),
            "30s of uptime must reset the backoff to the floor"
        );
    }

    #[test]
    fn a_crash_before_the_healthy_window_keeps_climbing() {
        let mut b = RestartBackoff::new();
        let t0 = Instant::now();
        b.next_delay(t0); // 200ms; internal state now at 400ms
        b.record_spawn(t0);

        let early_crash = t0 + Duration::from_secs(29);
        assert_eq!(
            b.next_delay(early_crash),
            Duration::from_millis(400),
            "under 30s of uptime must not reset the backoff"
        );
    }

    /// `--restart-tries` needs a consecutive-crash counter alongside the
    /// delay, reset by the exact same 30s-healthy-uptime rule so a
    /// process that recovers gets a fresh try budget, not one still
    /// depleted from an old, unrelated crash loop.
    #[test]
    fn tries_counts_consecutive_crashes_and_resets_with_the_backoff() {
        let mut b = RestartBackoff::new();
        let t0 = Instant::now();
        assert_eq!(b.tries(), 0, "no crash yet");

        b.next_delay(t0);
        assert_eq!(b.tries(), 1);
        b.next_delay(t0);
        assert_eq!(b.tries(), 2);
        b.next_delay(t0);
        assert_eq!(b.tries(), 3);

        // Respawned, then survives a full healthy window before crashing
        // again - both the delay and the tries count must reset.
        b.record_spawn(t0);
        let healthy_crash = t0 + Duration::from_secs(30);
        b.next_delay(healthy_crash);
        assert_eq!(
            b.tries(),
            1,
            "30s of uptime must reset the tries count, not just the delay"
        );
    }
}

#[cfg(test)]
mod dev_process_config_tests {
    //! T15. `Suprnova.toml`'s `[[serve.process]]` array - the declarative
    //! registry a project uses in place of Laravel's `DevCommands::register`.

    use super::{DevProcessConfig, parse_dev_processes};

    #[test]
    fn no_serve_process_table_is_an_empty_registry_not_an_error() {
        assert_eq!(
            parse_dev_processes("").unwrap(),
            Vec::<DevProcessConfig>::new()
        );
        assert_eq!(
            parse_dev_processes("[package]\nname = \"x\"\n").unwrap(),
            Vec::new()
        );
    }

    #[test]
    fn parses_a_full_entry() {
        let toml = r#"
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"
"#;
        let procs = parse_dev_processes(toml).unwrap();
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].name, "queue");
        assert_eq!(procs[0].command, "cargo");
        assert_eq!(
            procs[0].args,
            vec!["run", "--bin", "console", "--", "queue:work"]
        );
        assert_eq!(procs[0].color, console::Color::Yellow);
    }

    #[test]
    fn missing_color_gets_a_palette_default_not_an_error() {
        let toml = "[[serve.process]]\nname = \"logs\"\ncommand = \"tail\"\n";
        let procs = parse_dev_processes(toml).unwrap();
        assert_eq!(procs[0].args, Vec::<String>::new());
        let _ = procs[0].color; // no panic, no error - palette assigned one
    }

    #[test]
    fn malformed_entries_are_hard_errors_not_silently_skipped() {
        let cases = [
            ("[[serve.process]]\nname = \"queue\"\n", "command"),
            (
                "[[serve.process]]\nname = \"queue\"\ncommand = \"a\"\ncolor = \"chartreuse\"\n",
                "chartreuse",
            ),
            (
                "[[serve.process]]\nname = \"q\"\ncommand = \"a\"\n\
                 [[serve.process]]\nname = \"q\"\ncommand = \"b\"\n",
                "duplicate",
            ),
            // Whitespace-only `name`/`command` must be caught here, with
            // an actionable message naming the file and the entry -
            // rather than passing parsing and surfacing later as an
            // opaque OS "No such file or directory" spawn error.
            (
                "[[serve.process]]\nname = \"   \"\ncommand = \"a\"\n",
                "name",
            ),
            (
                "[[serve.process]]\nname = \"queue\"\ncommand = \"   \"\n",
                "command",
            ),
            // An unrecognized key (a `colour` typo, say) must be rejected
            // rather than silently ignored.
            (
                "[[serve.process]]\nname = \"queue\"\ncommand = \"a\"\ncolour = \"green\"\n",
                "colour",
            ),
        ];
        for (toml, expect) in cases {
            let err = parse_dev_processes(toml).unwrap_err();
            assert!(err.contains(expect), "toml {toml:?} -> error {err:?}");
        }
    }
}

#[cfg(test)]
mod dev_event_json_tests {
    //! T15 `--json`. `DevEvent`'s NDJSON shape is the stability contract
    //! for whatever parses `suprnova serve --json`'s stdout, so these
    //! tests pin the exact serialized bytes for every variant, not just
    //! "it round-trips".

    use super::{DevEvent, OutputStream, TypesArtifact};

    #[test]
    fn started_serializes_to_the_documented_shape() {
        let event = DevEvent::Started {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            name: "backend".to_string(),
            pid: 48213,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"started","ts":"2026-08-18T10:15:23.456-07:00","name":"backend","pid":48213}"#
        );
    }

    #[test]
    fn output_serializes_with_its_stream_and_carries_the_line_verbatim() {
        let event = DevEvent::Output {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            name: "frontend".to_string(),
            stream: OutputStream::Stderr,
            line: "warning: something".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"output","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","stream":"stderr","line":"warning: something"}"#
        );
    }

    #[test]
    fn exited_carries_a_nullable_code() {
        let event = DevEvent::Exited {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            name: "frontend".to_string(),
            code: None,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":null}"#
        );
    }

    #[test]
    fn restart_scheduled_carries_the_backoff_delay_in_milliseconds() {
        let event = DevEvent::RestartScheduled {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            name: "frontend".to_string(),
            delay_ms: 1600,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":1600}"#
        );
    }

    #[test]
    fn restart_succeeded_carries_the_new_pid() {
        let event = DevEvent::RestartSucceeded {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            name: "frontend".to_string(),
            pid: 48391,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","pid":48391}"#
        );
    }

    #[test]
    fn gave_up_carries_the_configured_tries_limit() {
        let event = DevEvent::GaveUp {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            name: "backend".to_string(),
            tries: 5,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"gave_up","ts":"2026-08-18T10:15:23.456-07:00","name":"backend","tries":5}"#
        );
    }

    #[test]
    fn types_regenerated_carries_the_artifact_and_count() {
        let event = DevEvent::TypesRegenerated {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            artifact: TypesArtifact::InertiaProps,
            count: 3,
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"types_regenerated","ts":"2026-08-18T10:15:23.456-07:00","artifact":"inertia_props","count":3}"#
        );

        let lang_keys = DevEvent::TypesRegenerated {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
            artifact: TypesArtifact::LangKeys,
            count: 12,
        };
        assert_eq!(
            serde_json::to_string(&lang_keys).unwrap(),
            r#"{"type":"types_regenerated","ts":"2026-08-18T10:15:23.456-07:00","artifact":"lang_keys","count":12}"#
        );
    }

    #[test]
    fn shutdown_carries_nothing_but_a_timestamp() {
        let event = DevEvent::Shutdown {
            ts: "2026-08-18T10:15:23.456-07:00".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"shutdown","ts":"2026-08-18T10:15:23.456-07:00"}"#
        );
    }
}
