#!/usr/bin/env bash
#
# On-demand RenderCache budget run; never a gate step.
#
# Runs the engine bench (the `C64` Complete L0 hot hit and `C64+4`
# Composite assembly, with the counting allocator), then the framework
# workload bench, then the checked-result contract. Both benches run by
# default. `SUPRNOVA_LIVE_SKIP_WORKLOADS=1` runs the engine bench alone.
#
# `SUPRNOVA_LIVE_S1_CPUSET` (default `0-7`) pins both benches.
# `SUPRNOVA_LIVE_BENCH_RESULT` and `SUPRNOVA_LIVE_WORKLOADS_RESULT`
# redirect the two result files away from the checked-in ones.
#
# `PG_TEST_URL` and `REDIS_TEST_URL` each add a run to the workload bench's
# result: PostgreSQL for the database tier's second dialect, Redis for the
# accelerator tier. Both servers must be disposable, since the run drops and
# recreates every table and flushes every key it uses.
#
# A full run of this script needs both of them, because the checked-result
# contract requires all three recorded profiles (SQLite, PostgreSQL, Redis).
# A partial run - one server, or neither - must redirect both result files
# under `benchmarks/local/` (gitignored) with `SUPRNOVA_LIVE_BENCH_RESULT`
# and `SUPRNOVA_LIVE_WORKLOADS_RESULT`; otherwise it overwrites the
# checked-in results with a shorter file and then fails its own contract.

set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel)
case ${live_root} in
    "${workspace_root}"/*) ;;
    *)
        printf 'render-cache budget: Live root is outside the Suprnova workspace (%s)\n' \
            "${live_root}" >&2
        exit 70
        ;;
esac
workspace_manifest=${workspace_root}/Cargo.toml
cpu_set=${SUPRNOVA_LIVE_S1_CPUSET:-0-7}
result_path=${SUPRNOVA_LIVE_BENCH_RESULT:-"${live_root}/benchmarks/render-cache-budget-v1.json"}
workloads_path=${SUPRNOVA_LIVE_WORKLOADS_RESULT:-"${live_root}/benchmarks/render-cache-workloads-v1.json"}

cd "${live_root}"

printf '%s\n' "[render-cache-budget] release C64 and C64+4 engine benchmark on CPU set ${cpu_set}"
rtk env \
    CARGO_INCREMENTAL=0 \
    SUPRNOVA_LIVE_BENCH_RESULT="${result_path}" \
    taskset -c "${cpu_set}" \
    cargo bench \
        --manifest-path "${workspace_manifest}" \
        --package suprnova-live \
        --bench render_cache_budget

if [[ ${SUPRNOVA_LIVE_SKIP_WORKLOADS:-0} != 1 ]]; then
    printf '%s\n' "[render-cache-budget] release framework workloads on CPU set ${cpu_set}"
    rtk env \
        CARGO_INCREMENTAL=0 \
        SUPRNOVA_LIVE_WORKLOADS_RESULT="${workloads_path}" \
        taskset -c "${cpu_set}" \
        cargo bench \
            --manifest-path "${workspace_manifest}" \
            --package suprnova \
            --bench render_cache_workloads
else
    printf '%s\n' \
        "[render-cache-budget] framework workloads skipped: SUPRNOVA_LIVE_SKIP_WORKLOADS=1"
fi

printf '%s\n' "[render-cache-budget] checked-result contract"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test benchmark_contract
