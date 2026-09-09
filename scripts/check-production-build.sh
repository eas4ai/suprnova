#!/usr/bin/env bash
# Proves the production build shape (design doc section 5, vocabulary):
# `default-features = false` plus the nine non-`testing` defaults.
#
# 1. `framework/tests/fixtures/testing-off-probe/` references, by path,
#    every `pub` test seam the framework declares. Checking it with no
#    features must fail - every reference must be unresolved.
# 2. Checking the same probe with `--features with-testing` must pass,
#    proving the list names real items rather than typos that would "pass"
#    step 1 for the wrong reason.
# 3. `cargo build -p app --bin app --bin console` builds the dogfood
#    binaries in the production shape by construction (app/Cargo.toml's
#    `[dependencies]` entry).
# 4. The built `app` binary runs its migrations against a fresh SQLite
#    file.
# 5. The built `app` binary serves one request on `/_suprnova/health/live`
#    (touches nothing - manual/deployment.md, "Health check").
#
# ## Error classes step 1 accepts, and why there are two more than the
# ## design text names
#
# A missing free function inside an otherwise-always-present module fails
# with E0425 ("cannot find function ... in module ..."). A missing module
# gated as a whole fails with E0433 ("cannot find `X` in `Y`") - resolution
# stops at the missing module segment, so it does not matter which item
# past it is named. Both are the design's own two codes. A missing
# associated function or method on a type that itself always exists -
# `RenderCache::shell_for_test`, `Context::test_set_query`,
# `Storage::fake`, and most of the framework's actual `_for_test` seams -
# fails with E0599 ("no function or associated item named ... found"), not
# E0425 or E0433. Confirmed empirically (a throwaway single-file `rustc`
# compile of the three shapes, kept out of this script) before writing
# `probe/src/main.rs`. Rewriting every associated-function seam as a free
# function to fit two error codes would be a framework API change section
# 5 does not ask for, so this script accepts E0599 alongside E0425/E0433 -
# a deliberate deviation from the literal design text, recorded here
# rather than silently widened.
#
# `probe/src/main.rs`'s own header comment records two pairs of items the
# design lists as seams that are not actually gated in the framework today
# (`RenderCacheConfig::with_clock_for_test` / `with_coordinator_for_test`,
# and `render_cache::console::epoch_advance_report_for_test` /
# `inspect_report_for_test`) - they are referenced anyway, for completeness
# against the design's list, and contribute nothing to this script's
# compile-failure assertion below.

set -euo pipefail
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

PROBE_DIR="framework/tests/fixtures/testing-off-probe"
ALLOWED_ERROR_CODES=("E0425" "E0433" "E0599")

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

is_allowed_code() {
    local code="$1" allowed
    for allowed in "${ALLOWED_ERROR_CODES[@]}"; do
        [[ "$code" == "$allowed" ]] && return 0
    done
    return 1
}

echo "==> production build shape: probe fails to compile without \`testing\`"
PROBE_LOG="$TMP_DIR/probe-without-testing.log"
if (cd "$PROBE_DIR" && cargo check) >"$PROBE_LOG" 2>&1; then
    echo "error: the probe compiled without \`testing\` - a test seam leaked" >&2
    echo "       into the production build shape, or the probe stopped" >&2
    echo "       naming a real item. Full output:" >&2
    cat "$PROBE_LOG" >&2
    exit 1
fi

mapfile -t found_codes < <(grep -oE 'error\[E[0-9]+\]' "$PROBE_LOG" | sed -E 's/error\[(E[0-9]+)\]/\1/' | sort -u)
if [[ "${#found_codes[@]}" -eq 0 ]]; then
    echo "error: the probe failed to compile, but reported no error[Exxxx] code." >&2
    echo "       Full output:" >&2
    cat "$PROBE_LOG" >&2
    exit 1
fi
for code in "${found_codes[@]}"; do
    if ! is_allowed_code "$code"; then
        echo "error: the probe's failure without \`testing\` included $code," >&2
        echo "       which is not an unresolved-item error (${ALLOWED_ERROR_CODES[*]} only)." >&2
        echo "       Full output:" >&2
        cat "$PROBE_LOG" >&2
        exit 1
    fi
done
echo "    every reported error is an unresolved-item error (${found_codes[*]})"

echo "==> production build shape: probe compiles with \`testing\` back on"
if ! (cd "$PROBE_DIR" && cargo check --features with-testing) >"$TMP_DIR/probe-with-testing.log" 2>&1; then
    echo "error: the probe failed to compile with \`--features with-testing\`," >&2
    echo "       so its reference list does not name real items. Full output:" >&2
    cat "$TMP_DIR/probe-with-testing.log" >&2
    exit 1
fi

echo "==> production build shape: build the dogfood binaries"
cargo build -p app --bin app --bin console

APP_BIN="$REPO_ROOT/target/debug/app"
if [[ ! -x "$APP_BIN" ]]; then
    echo "error: expected a built binary at $APP_BIN" >&2
    exit 1
fi

DB_FILE="$TMP_DIR/production-build-probe.sqlite"
export DATABASE_URL="sqlite://$DB_FILE"
export APP_ENV="local"

echo "==> production build shape: run migrations"
"$APP_BIN" migrate

echo "==> production build shape: serve and answer one request"
# A random high port, checked against `/dev/tcp` (a bash builtin - no
# extra tool needed, matching this step's declared capabilities) before
# use, so a stray local listener does not make this flaky.
pick_port() {
    local attempt candidate
    for attempt in $(seq 1 20); do
        candidate=$((20000 + RANDOM % 20000))
        if ! (exec 3<>"/dev/tcp/127.0.0.1/$candidate") 2>/dev/null; then
            printf '%s' "$candidate"
            return 0
        fi
    done
    echo "error: could not find a free local port" >&2
    return 1
}
SERVER_PORT="$(pick_port)"
export SERVER_HOST="127.0.0.1"
export SERVER_PORT
export APP_URL="http://127.0.0.1:${SERVER_PORT}"

"$APP_BIN" serve --no-migrate >"$TMP_DIR/server.log" 2>&1 &
SERVER_PID=$!

stop_server() {
    if kill -0 "$SERVER_PID" 2>/dev/null; then
        kill -TERM "$SERVER_PID" 2>/dev/null || true
        for _ in $(seq 1 10); do
            kill -0 "$SERVER_PID" 2>/dev/null || break
            sleep 1
        done
        kill -0 "$SERVER_PID" 2>/dev/null && kill -KILL "$SERVER_PID" 2>/dev/null || true
    fi
    wait "$SERVER_PID" 2>/dev/null || true
}
trap 'stop_server; rm -rf "$TMP_DIR"' EXIT

answered=0
for _ in $(seq 1 60); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then
        echo "error: the server exited before it answered a request. Log:" >&2
        cat "$TMP_DIR/server.log" >&2
        exit 1
    fi
    status="$(curl --silent --output /dev/null --write-out '%{http_code}' \
        "http://127.0.0.1:${SERVER_PORT}/_suprnova/health/live" 2>/dev/null || true)"
    if [[ "$status" =~ ^[23][0-9][0-9]$ ]]; then
        answered=1
        break
    fi
    sleep 1
done

if [[ "$answered" -ne 1 ]]; then
    echo "error: /_suprnova/health/live never answered with 2xx/3xx. Log:" >&2
    cat "$TMP_DIR/server.log" >&2
    exit 1
fi

echo "    answered with HTTP $status"
echo
echo "Production build shape passed."
