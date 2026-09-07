#!/usr/bin/env bash
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

if [[ ${SUPRNOVA_LIVE_SKIP_WORKLOADS:-1} != 1 ]]; then
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
        "[render-cache-budget] framework workloads skipped (SUPRNOVA_LIVE_SKIP_WORKLOADS)"
fi

printf '%s\n' "[render-cache-budget] checked-result contract"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test benchmark_contract
