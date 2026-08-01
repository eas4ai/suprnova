#!/usr/bin/env bash
#
# Phase 2.1 — how much traffic does a route carry, and what does latency
# look like on the way up?
#
# Two passes, because one tool cannot answer both questions honestly.
#
#   Capacity (oha, closed loop). Concurrency sweep: N in-flight requests,
#   each client sending the next only after the last returns. Answers "how
#   many requests per second can this route retire", and where adding
#   clients stops helping.
#
#   Latency (vegeta, open loop). Fixed arrival rate regardless of whether
#   the server is keeping up. This is the pass whose percentiles mean
#   something: a closed-loop harness cannot report a delay it never sent a
#   request during, so its tail looks better the worse the server behaves.
#   That artefact has a name — coordinated omission — and it is why the
#   headline latency numbers here come from the open-loop pass and not
#   from the sweep that found the throughput.
#
# # Where the generator runs
#
# On the same host as the system under test, because there is one host —
# and that host is otherwise idle. Nothing here is pinned or capped:
# not the SUT, not the generator. There is no contention to schedule
# around, so there is nothing to schedule around it with.
#
# An earlier revision confined the generator to a six-core cpuset. It
# spent that entire budget and the sweep measured `oha` rather than the
# server. Capping any part of a benchmark on an idle machine buys
# nothing and costs the run.
#
# Generator CPU is still sampled next to every result. Recording is not
# throttling — it is how a reader tells a server number from a generator
# number.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

: "${TARGET_HOST:=127.0.0.1}"
: "${TARGET_PORT:=18080}"
: "${DURATION:=30s}"
: "${WARMUP:=10s}"
: "${CONCURRENCIES:=1 2 4 8 16 32 64 128 256 512}"

BASE="http://${TARGET_HOST}:${TARGET_PORT}"

# Three tiers, chosen to separate the framework's own cost from what an
# application does on top of it. Reporting only one of them would let a
# reader attribute the whole number to the wrong layer.
#
#   health   routing + middleware + response. No database, no rendering.
#            The framework's floor: nothing an app writes is in this path.
#   api      a database read serialised to JSON. Adds the pool, a query,
#            and serialisation.
#   page     a full Inertia page response. Adds prop assembly on top.
declare -A ROUTES=(
    [health]="/_suprnova/health"
    [api]="/api/users"
    [page]="/"
)
ROUTE_ORDER=(health api page)

RESULTS="$(results_dir throughput)"
echo "==> results: $RESULTS"

require_stack

need() {
    command -v "$1" >/dev/null 2>&1 || {
        echo "SETUP FAILURE: $1 not installed" >&2
        exit 2
    }
}
need oha
need vegeta

# A run against a route that is not answering 200 measures an error page.
echo "==> preflight"
for name in "${ROUTE_ORDER[@]}"; do
    path="${ROUTES[$name]}"
    code="$(curl -s -o /dev/null -w '%{http_code}' "${BASE}${path}")"
    bytes="$(curl -s "${BASE}${path}" | wc -c)"
    printf '    %-8s %-22s %s  %sB\n' "$name" "$path" "$code" "$bytes"
    if [[ "$code" != "200" ]]; then
        echo "SETUP FAILURE: ${path} answered ${code}, not 200" | tee "$RESULTS/verdict.txt"
        exit 2
    fi
done

# Sample the generator's own CPU while it runs — as data, not as a limit.
# With nothing pinned, the generator is only a suspect if it approaches the
# whole machine.
gen_cpu_watch() {
    local out="$1" pid="$2"
    ( while kill -0 "$pid" 2>/dev/null; do
          ps -o %cpu= -p "$pid" 2>/dev/null | tr -d ' '
          sleep 1
      done ) >"$out" 2>/dev/null &
    echo $!
}

peak_of() {
    [[ -s "$1" ]] || { echo "0"; return; }
    sort -g "$1" | tail -1
}

# ---------------------------------------------------------------------
# Pass 1 — capacity
# ---------------------------------------------------------------------

echo
echo "==> pass 1: capacity sweep (oha, closed loop, ${DURATION} per step)"
printf 'route\tconcurrency\trps\tp50_ms\tp99_ms\terrors\tgen_cpu_peak\n' >"$RESULTS/capacity.tsv"

declare -A BEST_RPS
declare -A BEST_CONC

for name in "${ROUTE_ORDER[@]}"; do
    path="${ROUTES[$name]}"
    echo "  -- ${name} (${path})"

    # Warm the route once per tier: first-touch costs (pool fill, lazy
    # statics, page cache) belong to startup, not to steady state.
    oha -z "$WARMUP" -c 32 --no-tui "${BASE}${path}" >/dev/null 2>&1 || true

    BEST_RPS[$name]=0
    BEST_CONC[$name]=0

    for c in $CONCURRENCIES; do
        json="$RESULTS/oha-${name}-c${c}.json"
        cpu="$RESULTS/gencpu-${name}-c${c}.txt"

        oha -z "$DURATION" -c "$c" --no-tui \
            --output-format json "${BASE}${path}" >"$json" 2>"${json%.json}.err" &
        oha_pid=$!
        watcher="$(gen_cpu_watch "$cpu" "$oha_pid")"
        wait "$oha_pid" || true
        kill "$watcher" 2>/dev/null || true

        if ! read -r rps p50 p99 errs < <(python3 "$SCRIPT_DIR/oha_summary.py" "$json"); then
            {
                echo "SETUP FAILURE: could not read oha output for ${name} c=${c}"
                echo "stderr from oha:"
                sed 's/^/    /' "${json%.json}.err" | head -5
            } | tee "$RESULTS/verdict.txt"
            exit 2
        fi
        gen_peak="$(peak_of "$cpu")"
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$name" "$c" "$rps" "$p50" "$p99" "$errs" "$gen_peak" >>"$RESULTS/capacity.tsv"
        printf '     c=%-4s %8s rps   p50 %7sms   p99 %8sms   err %-5s gen %s%%\n' \
            "$c" "$rps" "$p50" "$p99" "$errs" "$gen_peak"

        if (( $(echo "$rps > ${BEST_RPS[$name]}" | bc -l) )); then
            BEST_RPS[$name]="$rps"
            BEST_CONC[$name]="$c"
        fi
    done
done

# ---------------------------------------------------------------------
# Pass 2 — latency at a fraction of capacity
# ---------------------------------------------------------------------
#
# Rates are taken from pass 1 rather than guessed, so each tier is
# measured at a load that means the same thing for that tier. Percentiles
# at 50% and 80% of capacity are the numbers an operator can act on: the
# figure at 100% is a saturation artefact and says only that the queue is
# growing.

echo
echo "==> pass 2: latency at fixed arrival rates (vegeta, open loop)"
printf 'route\ttarget_rps\tachieved_rps\tp50_ms\tp95_ms\tp99_ms\tp999_ms\tsuccess_pct\n' \
    >"$RESULTS/latency.tsv"

for name in "${ROUTE_ORDER[@]}"; do
    path="${ROUTES[$name]}"
    cap="${BEST_RPS[$name]%.*}"
    [[ "$cap" -gt 0 ]] || continue

    for frac in 50 80; do
        rate=$(( cap * frac / 100 ))
        [[ "$rate" -gt 0 ]] || continue
        bin="$RESULTS/vegeta-${name}-${frac}pct.bin"

        echo "GET ${BASE}${path}" \
            | vegeta attack \
                -duration "$DURATION" -rate "${rate}/1s" -max-workers 2000 \
            >"$bin" 2>"${bin%.bin}.err"

        if ! read -r achieved p50 p95 p99 p999 success < <(
            vegeta report -type=json "$bin" | python3 "$SCRIPT_DIR/vegeta_summary.py"
        ); then
            echo "SETUP FAILURE: could not read vegeta report for ${name} at ${frac}%" \
                | tee -a "$RESULTS/verdict.txt"
            exit 2
        fi
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$name" "$rate" "$achieved" "$p50" "$p95" "$p99" "$p999" "$success" \
            >>"$RESULTS/latency.tsv"
        printf '     %-6s %3s%% of cap (%6s rps) -> p50 %6sms  p95 %7sms  p99 %8sms  p99.9 %9sms  ok %s%%\n' \
            "$name" "$frac" "$rate" "$p50" "$p95" "$p99" "$p999" "$success"
    done
done

# ---------------------------------------------------------------------

{
    echo "experiment: 2.1-throughput"
    echo "duration_per_step: $DURATION"
    echo "cpu_pinning: none — generator and SUT both unpinned"
    for name in "${ROUTE_ORDER[@]}"; do
        echo "peak_${name}_rps: ${BEST_RPS[$name]} (at concurrency ${BEST_CONC[$name]})"
    done
} | tee "$RESULTS/verdict.txt"

# Nothing is capped, so the only remaining way for the generator to be the
# bottleneck is for it to run out of machine. That is a report, not a limit:
# the fix is a second host, never a smaller budget for either side.
GEN_MAX="$(cut -f7 "$RESULTS/capacity.tsv" | tail -n +2 | sort -g | tail -1)"
MACHINE_PCT="$(( $(nproc) * 100 ))"
{
    echo "generator_peak_cpu_pct: $GEN_MAX (whole machine: $MACHINE_PCT)"
    if (( $(echo "$GEN_MAX > $MACHINE_PCT * 0.5" | bc -l) )); then
        echo "WARNING: the load generator took more than half the machine, so it was"
        echo "contending with the server for the same cores. These figures are a lower"
        echo "bound. Drive the load from a second host before quoting them."
    else
        echo "generator headroom: OK — the generator was not the bottleneck"
    fi
} | tee -a "$RESULTS/verdict.txt"

echo
echo "==> capacity.tsv and latency.tsv written to $RESULTS"
