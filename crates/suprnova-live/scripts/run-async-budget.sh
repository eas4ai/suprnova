#!/usr/bin/env bash
set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel)
case ${live_root} in
    "${workspace_root}"/*) ;;
    *)
        printf 'async budget: Live root is outside the Suprnova workspace (%s)\n' \
            "${live_root}" >&2
        exit 70
        ;;
esac
workspace_manifest=${workspace_root}/Cargo.toml
profile=${SUPRNOVA_LIVE_BUDGET_PROFILE:-reduced}
browser_result=${SUPRNOVA_LIVE_ASYNC_BUDGET_RESULT:-"${live_root}/browser/benchmarks/local/async-budget-v1.json"}
server_result=${SUPRNOVA_LIVE_ASYNC_SERVER_RESULT:-"${live_root}/benchmarks/local/async-server-v1.json"}
baseline=${SUPRNOVA_LIVE_ASYNC_BUDGET_BASELINE:-"${live_root}/browser/benchmarks/baselines/async-budget-v1.json"}

if [[ ${profile} != reduced && ${profile} != qualified ]]; then
    printf '%s\n' "async budget profile must be reduced or qualified" >&2
    exit 64
fi

if [[ ${profile} == qualified && ${SUPRNOVA_LIVE_B1_DEDICATED:-0} != 1 ]]; then
    printf '%s\n' "qualified E100/1K and R100 require explicit B1 dedicated-runner attestation" >&2
    exit 1
fi

cd "${live_root}/browser"
rtk env \
    SUPRNOVA_LIVE_B1_DEDICATED="${SUPRNOVA_LIVE_B1_DEDICATED:-0}" \
    npm run budget:async -- \
    --profile "${profile}" \
    --baseline "${baseline}" \
    --server-output "${server_result}" \
    --output "${browser_result}"

cd "${live_root}"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test async_budget_contract
