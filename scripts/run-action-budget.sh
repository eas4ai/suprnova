#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cpu_set=${SUPRNOVA_LIVE_S1_CPUSET:-0-7}
result_path=${SUPRNOVA_LIVE_BENCH_RESULT:-"${repository_root}/benchmarks/action-budget-v1.json"}

cd "${repository_root}"

printf '%s\n' "[action-budget] release A8/16 framework benchmark on CPU set ${cpu_set}"
rtk env \
    CARGO_INCREMENTAL=0 \
    SUPRNOVA_LIVE_BENCH_RESULT="${result_path}" \
    taskset -c "${cpu_set}" \
    cargo bench --bench action_framework_budget

printf '%s\n' "[action-budget] checked-result contract"
rtk env CARGO_INCREMENTAL=0 cargo test --test benchmark_contract
