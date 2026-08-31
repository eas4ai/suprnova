#!/usr/bin/env bash
set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel)
case ${live_root} in
    "${workspace_root}"/*) ;;
    *)
        printf 'snapshot budget: Live root is outside the Suprnova workspace (%s)\n' \
            "${live_root}" >&2
        exit 70
        ;;
esac
workspace_manifest=${workspace_root}/Cargo.toml
cpu_set=${SUPRNOVA_LIVE_S1_CPUSET:-0-7}
result_path=${SUPRNOVA_LIVE_BENCH_RESULT:-"${live_root}/benchmarks/snapshot-budget-v1.json"}

cd "${live_root}"

printf '%s\n' "[snapshot-budget] release A8/16 benchmark on CPU set ${cpu_set}"
rtk env \
    CARGO_INCREMENTAL=0 \
    SUPRNOVA_LIVE_BENCH_RESULT="${result_path}" \
    taskset -c "${cpu_set}" \
    cargo bench \
        --manifest-path "${workspace_manifest}" \
        --package suprnova-live \
        --bench snapshot_budget

printf '%s\n' "[snapshot-budget] checked-result contract"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test benchmark_contract
