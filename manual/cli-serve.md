# suprnova serve

`suprnova serve` runs your backend and the Vite dev server together with hot
reload on both sides, plus automatic TypeScript type regeneration whenever you
touch a `#[derive(InertiaProps)]` struct. It's the one command you keep open
in a terminal while you're building.

```bash
suprnova serve
```

Both processes stream their stdout into the same terminal with coloured
`[backend]` and `[frontend]` prefixes so you can tell who said what. `Ctrl+C`
shuts them both down cleanly.

## Usage

```bash
suprnova serve [OPTIONS]
```

| Option | Default | Description |
|---|---|---|
| `-p, --port <PORT>` | `8765` (CLI) / `$SERVER_PORT` (env) | Backend HTTP port |
| `--frontend-port <PORT>` | `5765` (CLI) / `$VITE_PORT` (env) | Vite dev server port |
| `--backend-only` | `false` | Skip the Vite dev server |
| `--frontend-only` | `false` | Skip the backend, just run Vite |
| `--skip-types` | `false` | Don't regenerate TypeScript types on Rust changes |
| `--no-restart` | `false` | Don't respawn a crashed dev process - tear the whole session down instead (the old behaviour) |
| `--restart-tries <N>` | `5` | Give up retrying a process after this many consecutive crashes. Ignored with `--no-restart`, which already ends the session on the first crash. |
| `--timestamps` | `false` | Prefix each output line with an `HH:MM:SS` clock time |
| `--json` | `false` | Emit one JSON object per line (NDJSON) on stdout instead of prefixed text - see [JSON output](#json-output). Combining with `--timestamps` isn't an error; `--timestamps` has no extra effect, since every event already carries its own timestamp. |

The CLI flags take precedence over environment variables, which take precedence
over the built-in defaults. A scaffolded `.env` ships with `SERVER_PORT=8765`
and `VITE_PORT=5765`; you'll see those values used unless you override with
`--port`.

## Examples

### Default - both servers

```bash
suprnova serve
```

Output:

```
Backend  http://127.0.0.1:8765
Frontend http://127.0.0.1:5765
[backend] Compiling my-app v0.1.0 ...
[frontend] VITE v6.3.0  ready in 312 ms
```

Hit `http://127.0.0.1:8765` in your browser. The backend serves the Inertia
HTML shell and proxies asset requests through to Vite, so you don't need to
visit the Vite URL directly.

### Custom ports

```bash
suprnova serve --port 3000 --frontend-port 3001
```

Or set them in `.env` and run without flags:

```env
SERVER_PORT=3000
VITE_PORT=3001
```

### Backend only

```bash
suprnova serve --backend-only
```

Good for working on an API-only project, or when your frontend is already
running in another terminal (or another machine, or a deployed preview).

### Frontend only

```bash
suprnova serve --frontend-only
```

Good for working on UI without paying the cost of a Rust rebuild on every
save, or when the backend is running in another shell (or in Docker).

### Skip type generation

```bash
suprnova serve --skip-types
```

Disables the TypeScript regeneration watcher. Use this when you're managing
`frontend/src/types/inertia-props.ts` by hand, or when you're working far
from any Inertia code and want quieter output.

## What it actually does

When you run `suprnova serve`, the CLI:

1. Loads `.env` from the current directory.
2. Resolves backend and frontend ports (CLI flag → env var → default).
3. Verifies you're in a Suprnova project - `Cargo.toml` must exist (unless
   `--frontend-only`) and a `frontend/` directory must exist (unless
   `--backend-only`).
4. Regenerates TypeScript types from any `#[derive(InertiaProps)]` structs
   it finds in `src/`, writing them to `frontend/src/types/inertia-props.ts`.
5. Installs `cargo-watch` via `cargo install --locked --version "^8.5"
   cargo-watch` if it isn't on the PATH yet (one-time, with an
   "Installing..." notice). Skipped under `--frontend-only`.
   The version is bounded because `serve` drives `cargo watch -x`, whose
   meaning is not guaranteed across a major bump; `--locked` builds the
   dependency tree cargo-watch published rather than re-resolving it at
   install time. A command that installs software as a side effect of
   starting a dev server should not also be choosing versions for you.
6. Runs `npm install` in `frontend/` if `node_modules` doesn't exist yet.
   Skipped under `--backend-only`.
7. Spawns `cargo watch -x 'run --bin <package-name>'` for the backend.
   `cargo-watch` re-runs the binary whenever a `.rs` file changes.
8. Spawns `npm run dev` in `frontend/` for Vite, which gives you HMR for
   Svelte/React/Vue components and Tailwind classes.
9. Spawns every extra process declared in the project's `Suprnova.toml`
   (see [Extra dev processes](#extra-dev-processes) below), each with its
   own `[name]` prefix - queue workers, log tailers, anything else you'd
   otherwise juggle in another terminal.
10. Starts a file watcher on `src/` that re-runs the type generator whenever
    a `.rs` file changes, once the burst of saves has been quiet for 500 ms.
    The debounce is trailing-edge, so a burst - `cargo fmt`, format-on-save
    across several files, a branch switch - coalesces into exactly one
    regeneration that runs *after* the last write, rather than one that
    fires on the first file and misses the rest.
11. Forwards every child's stdout/stderr to your terminal with a `[name]`
    prefix (`[backend]`, `[frontend]`, or the process's configured name),
    optionally timestamped with `--timestamps` - or, with `--json`, as
    NDJSON events instead (see [JSON output](#json-output) below).

`Ctrl+C` signals the manager to set its shutdown flag, kill every child,
and exit. If a child exits on its own - a Rust compile error too severe
for `cargo watch` to recover, a crashed Vite process, a `Suprnova.toml`
process that failed - it's respawned after a short backoff (200ms,
doubling on each consecutive crash, capped at 5s; a process that stayed
up 30s resets the climb) instead of tearing the session down. Pass
`--no-restart` to get the old behaviour back: any child exiting shuts the
whole session down immediately.

A process that keeps crashing doesn't retry forever: `--restart-tries`
(default `5`) caps how many consecutive crashes `serve` retries before
giving up on that one process - a fresh 30s of uptime resets the count,
same as the backoff delay. Giving up prints an actionable message and
stops retrying *only* that process; the others (and the session itself)
keep running, matching Laravel's own `concurrently --restart-tries=5`
default. See [Troubleshooting](#a-process-keeps-crash-looping).

### Why Suprnova diverges

Laravel users typically run `php artisan serve` for the backend and `npm
run dev` in another terminal, and most teams paper over the two-terminal
split with a `Procfile` and `foreman`/`overmind`. Suprnova ships that
multiplexer as a first-class CLI command. You get one terminal, one
`Ctrl+C`, automatic toolchain bootstrap (`cargo-watch`, `npm install`),
and a typed-Inertia bridge that regenerates `frontend/src/types/inertia-props.ts`
on the fly so your Svelte/React/Vue components always see the current
prop shape without manual type sync.

Laravel's `dev` command also offers `--tabs` and `--stream` modes, each
rendering output through a small Node TUI (`@laravel/multiplex`).
Suprnova doesn't ship the TUI: single-terminal, prefixed output is the
norm across the Rust dev-tooling ecosystem (`cargo watch`, `bacon`,
`just`), and a process registry with colored prefixes already gives you
the "which process said this" signal a TUI provides. `--stream`'s
underlying job - one scriptable, real-time event stream - ships as
`--json` (see [JSON output](#json-output)); `--tabs`' multi-pane TUI is
the deliberate no, not a gap - a second interaction model and a second
library to keep working across terminals for a problem this page already
solves. See the corresponding row in
[Parity](parity.md#what-we-won-t-ship-and-why).

## Hot reload

**Backend.** `cargo watch -x 'run --bin <package>'` is the loop. It rebuilds
and restarts the server on every `.rs` change in the project. Cold rebuilds
after touching a heavy crate can take several seconds; incremental changes
in a single file are usually sub-second.

**Frontend.** Vite's HMR injects component changes in place without a full
reload, preserving component state. Tailwind classes update live via the
Tailwind v4 watcher.

**TypeScript types.** Whenever a `.rs` file changes, the type watcher re-runs
the generator. If new `#[derive(InertiaProps)]` structs appear (or existing
ones change shape), the regenerated `frontend/src/types/inertia-props.ts`
triggers Vite's HMR for the component that imports them.

## Extra dev processes

`suprnova serve` always runs the backend and Vite, but most projects have
more than two things to keep running - a queue worker, a log tailer, a
mail-catcher. Declare them in a `Suprnova.toml` at the project root and
`serve` spawns, prefixes, and auto-restarts them right alongside the
backend and frontend:

```toml
[[serve.process]]
name = "queue"
command = "cargo"
args = ["run", "--bin", "console", "--", "queue:work"]
color = "yellow"

[[serve.process]]
name = "logs"
command = "tail"
args = ["-f", "storage/logs/app.log"]
```

Each entry needs `name` and `command`; `args` defaults to none, `color`
defaults to one of green/yellow/blue/white assigned in declaration order
(or pick one of the eight named `console` colors - black, red, green,
yellow, blue, magenta, cyan, white). Names must be unique. `Suprnova.toml`
is entirely optional; a project without one runs exactly as before.

### Why Suprnova diverges

Laravel registers extra `dev` processes from PHP -
`DevCommands::register($command, $name)`, typically in a service
provider's `boot()` - because `php artisan dev` execs a multiplexer from
inside the same process that already booted the application.
`suprnova serve` is a separate binary from your app; it never links or
runs your Rust code, and only ever shells out to `cargo watch` and `npm`.
There's no application boot to hook into, so registration has to be data
the CLI reads rather than a call your code makes - hence `Suprnova.toml`
instead of a `DevProcesses::register()` API.

## JSON output

Pass `--json` and `suprnova serve` writes one JSON object per line
(NDJSON) to stdout instead of colored `[name]`-prefixed text - nothing
else goes to stdout while it's active, so you can pipe straight into
`jq` or any other line-oriented JSON consumer. Every line has a `type`
field:

| `type` | Fields | Meaning |
|---|---|---|
| `started` | `ts`, `name`, `pid` | A process (backend, frontend, or a `Suprnova.toml` entry) was spawned for the first time. |
| `output` | `ts`, `name`, `stream` (`"stdout"` or `"stderr"`), `line` | One line of a child's output, carried as a field rather than passed through raw. |
| `exited` | `ts`, `name`, `code` (nullable) | A process exited. `code` is `null` if it was killed by a signal rather than returning a status. |
| `restart_scheduled` | `ts`, `name`, `delay_ms` | A crashed process will be respawned after `delay_ms` (see the backoff schedule above). |
| `restart_succeeded` | `ts`, `name`, `pid` | A scheduled respawn succeeded; the process is running again under a new PID. |
| `gave_up` | `ts`, `name`, `tries` | The process crashed `tries` consecutive times (`--restart-tries`) and `serve` stopped retrying it. The session, and every other process, keep running. |
| `types_regenerated` | `ts`, `artifact` (`"inertia_props"` or `"lang_keys"`), `count` | The file watcher regenerated a TypeScript artifact in response to a `.rs`/`.ftl` change. |
| `shutdown` | `ts` | The session is shutting down. Always the last line. |

For example, a Vite crash and its respawn look like:

```json
{"type":"exited","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","code":1}
{"type":"restart_scheduled","ts":"2026-08-18T10:15:23.456-07:00","name":"frontend","delay_ms":200}
{"type":"restart_succeeded","ts":"2026-08-18T10:15:23.657-07:00","name":"frontend","pid":48391}
```

`--json` composes with `--timestamps` rather than conflicting with it:
combining them isn't an error, but `--timestamps` has no additional
effect, since every event already carries its own `ts` field.

This is machine-readable output other tools parse - field names and
`type` values won't be renamed or removed without a note in the
changelog. Treat an unrecognized `type` or an unexpected extra field as
something to ignore, not an error, so a future release can extend the
schema without breaking your consumer.

## Troubleshooting

### Port already in use

```text
[backend] Error: Address already in use (os error 98)
```

Find and kill the process, or pick another port:

```bash
lsof -i :8765
kill -9 <pid>

# or
suprnova serve --port 8081
```

### `cargo-watch` install fails

The CLI runs `cargo install cargo-watch` if it isn't already on PATH. If
that install fails (no network, restricted environment), install it manually
once:

```bash
cargo install cargo-watch
```

After that, `suprnova serve` will find it and won't try to install again.

### Frontend dependencies stuck

If `npm install` fails mid-bootstrap, fix the cause (npm registry reachable,
disk space, lockfile in good shape) and run it manually:

```bash
cd frontend && npm install
```

Then re-run `suprnova serve`. The CLI only auto-runs `npm install` when
`node_modules` is missing, so a successful manual install lets it skip that
step.

### Type regeneration not picking up changes

The watcher polls every 2 seconds (using `notify` with a poll interval -
chosen for cross-platform reliability over inotify quirks) and debounces
regeneration to once every 500 ms. If a change isn't showing up:

- Confirm the file is under `src/` (the watcher doesn't recurse into
  `crates/`, `cmd/`, or `migrations/`).
- Confirm the struct actually has `#[derive(InertiaProps)]`.
- Restart `suprnova serve` and watch for the `Generated N type(s)` startup
  message - if you see `No InertiaProps structs found`, the scanner didn't
  find anything to emit.

### A process keeps crash-looping

If a child - backend, frontend, or a `Suprnova.toml` entry - can't start
(bad code, a missing binary, a port conflict), it respawns on the backoff
schedule described above instead of stopping. Look at the `[name]` lines
right before each "respawning in …ms" notice for the real error (a rustc
`error[E…]`, an ENOENT, whatever the child printed). Fix the cause; the
next respawn attempt picks it up automatically. To stop the retries and
see the failure once, re-run with `--no-restart` - the session then tears
down on the first crash, same as `suprnova serve` behaved before this
existed.

After `--restart-tries` (default `5`) consecutive crashes, `serve` stops
retrying that process on its own and prints a message naming it:

```text
gave up restarting `backend` after 5 attempts; fix the error and run `suprnova serve` again
```

The other processes, and the session itself, keep running - fix the
cause and re-run `suprnova serve` to bring the given-up process back; you
don't need to restart the whole session for it.

## Next

- [Installation](installation.md) - get the CLI on your machine
- [Quickstart](quickstart.md) - a full first-app walkthrough
- [Directory Structure](structure.md) - what `suprnova new` scaffolded
- [Generators](cli-generators.md) - `make:controller`, `make:action`, etc.
- [Console](console.md) - the per-project `cargo run --bin console` binary
