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
# On the same host as the system under test, because there is one host.
# The SUT is deliberately unpinned — it gets the whole machine — while the
# generator is confined to a small cpuset so it cannot starve the thing it
# is measuring. That is not free, and the script records generator CPU
# alongside every result: if the generator saturates its own cores, the
# throughput figure is a floor for the framework, not a ceiling, and the
# verdict says so rather than leaving it for a reader to notice.

set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

: "${TARGET_HOST:=127.0.0.1}"
: "${TARGET_PORT:=18080}"
: "${DURATION:=30s}"
: "${WARMUP:=10s}"
# Cores for the load generator. The SUT is not pinned at all; this only
# stops the generator from taking the whole box.
: "${GEN_CPUS:=24-29}"
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
need taskset

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

# Sample the generator's own CPU while it runs. A generator pegged at 100%
# of its cpuset is the bottleneck, and every number from that run is a
# lower bound on the server rather than a measurement of it.
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
    taskset -c "$GEN_CPUS" oha -z "$WARMUP" -c 32 --no-tui "${BASE}${path}" >/dev/null 2>&1 || true

    BEST_RPS[$name]=0
    BEST_CONC[$name]=0

    for c in $CONCURRENCIES; do
        json="$RESULTS/oha-${name}-c${c}.json"
        cpu="$RESULTS/gencpu-${name}-c${c}.txt"

        taskset -c "$GEN_CPUS" oha -z "$DURATION" -c "$c" --no-tui -j \
            "${BASE}${path}" >"$json" 2>/dev/null &
        oha_pid=$!
        watcher="$(gen_cpu_watch "$cpu" "$oha_pid")"
        wait "$oha_pid" || true
        kill "$watcher" 2>/dev/null || true

        read -r rps p50 p99 errs < <(python3 - "$json" <<'PY'
import json, sys
try:
    d = json.load(open(sys.argv[1]))
except Exception:
    print("0 0 0 0"); raise SystemExit
s = d.get("summary", {})
rps = s.get("requestsPerSec", 0)
pct = d.get("latencyPercentiles", {})
p50 = pct.get("p50", 0) * 1000
p99 = pct.get("p99", 0) * 1000
codes = d.get("statusCodeDistribution", {})
ok = sum(v for k, v in codes.items() if k.startswith("2"))
total = sum(codes.values()) or 1
print(f"{rps:.0f} {p50:.2f} {p99:.2f} {total - ok}")
PY
        )
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
            | taskset -c "$GEN_CPUS" vegeta attack \
                -duration "$DURATION" -rate "${rate}/1s" -max-workers 2000 \
            >"$bin" 2>/dev/null

        read -r achieved p50 p95 p99 p999 success < <(
            vegeta report -type=json "$bin" | python3 -c '
import json, sys
d = json.load(sys.stdin)
ms = lambda ns: ns / 1e6
lat = d["latencies"]
print(f"{d[\"throughput\"]:.0f} {ms(lat[\"50th\"]):.2f} {ms(lat[\"95th\"]):.2f} "
      f"{ms(lat[\"99th\"]):.2f} {ms(lat.get(\"999th\", lat[\"max\"])):.2f} "
      f"{d[\"success\"] * 100:.2f}")'
        )
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
    echo "generator_cpus: $GEN_CPUS"
    echo "sut_cpus: unpinned (whole machine)"
    for name in "${ROUTE_ORDER[@]}"; do
        echo "peak_${name}_rps: ${BEST_RPS[$name]} (at concurrency ${BEST_CONC[$name]})"
    done
} | tee "$RESULTS/verdict.txt"

# The one thing that would invalidate everything above.
GEN_MAX="$(cut -f7 "$RESULTS/capacity.tsv" | tail -n +2 | sort -g | tail -1)"
CPU_COUNT="$(python3 -c "
import sys
lo, hi = '${GEN_CPUS}'.split('-')
print((int(hi) - int(lo) + 1) * 100)")"
{
    echo "generator_peak_cpu_pct: $GEN_MAX (of $CPU_COUNT available)"
    if (( $(echo "$GEN_MAX > $CPU_COUNT * 0.9" | bc -l) )); then
        echo "WARNING: the load generator saturated its own cores. Every throughput"
        echo "figure above is a lower bound on the server, not a measurement of it."
        echo "Widen GEN_CPUS or drive the load from a second host before quoting these."
    else
        echo "generator headroom: OK — the generator was not the bottleneck"
    fi
} | tee -a "$RESULTS/verdict.txt"

echo
echo "==> capacity.tsv and latency.tsv written to $RESULTS"
