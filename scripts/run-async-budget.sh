#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
profile=${SUPRNOVA_LIVE_BUDGET_PROFILE:-reduced}
browser_result=${SUPRNOVA_LIVE_ASYNC_BUDGET_RESULT:-"${repository_root}/browser/benchmarks/local/async-budget-v1.json"}
server_result=${SUPRNOVA_LIVE_ASYNC_SERVER_RESULT:-"${repository_root}/benchmarks/local/async-server-v1.json"}
baseline=${SUPRNOVA_LIVE_ASYNC_BUDGET_BASELINE:-"${repository_root}/browser/benchmarks/baselines/async-budget-v1.json"}

if [[ ${profile} != reduced && ${profile} != qualified ]]; then
    printf '%s\n' "async budget profile must be reduced or qualified" >&2
    exit 64
fi

if [[ ${profile} == qualified && ${SUPRNOVA_LIVE_B1_DEDICATED:-0} != 1 ]]; then
    printf '%s\n' "qualified E100/1K and R100 require explicit B1 dedicated-runner attestation" >&2
    exit 1
fi

cd "${repository_root}/browser"
rtk env \
    SUPRNOVA_LIVE_B1_DEDICATED="${SUPRNOVA_LIVE_B1_DEDICATED:-0}" \
    npm run budget:async -- \
    --profile "${profile}" \
    --baseline "${baseline}" \
    --server-output "${server_result}" \
    --output "${browser_result}"

cd "${repository_root}"
rtk env CARGO_INCREMENTAL=0 cargo test --test async_budget_contract
