#!/usr/bin/env bash
set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel)
case ${live_root} in
    "${workspace_root}"/*) ;;
    *)
        printf 'upload budget: Live root is outside the Suprnova workspace (%s)\n' \
            "${live_root}" >&2
        exit 70
        ;;
esac
workspace_manifest=${workspace_root}/Cargo.toml
profile=${SUPRNOVA_LIVE_BUDGET_PROFILE:-reduced}
cpu_set=${SUPRNOVA_LIVE_S1_CPUSET:-0-7}
server_result=${SUPRNOVA_LIVE_UPLOAD_SERVER_RESULT:-"${live_root}/benchmarks/local/upload-server-v1.json"}
browser_result=${SUPRNOVA_LIVE_UPLOAD_BUDGET_RESULT:-"${live_root}/browser/benchmarks/local/upload-budget-v1.json"}
baseline=${SUPRNOVA_LIVE_UPLOAD_BUDGET_BASELINE:-"${live_root}/browser/benchmarks/baselines/upload-budget-v1.json"}

if [[ ${profile} != reduced && ${profile} != qualified ]]; then
    printf '%s\n' "upload budget profile must be reduced or qualified" >&2
    exit 64
fi

if [[ ${profile} == qualified ]]; then
    if [[ ${SUPRNOVA_LIVE_S1_DEDICATED:-0} != 1 || ${SUPRNOVA_LIVE_B1_DEDICATED:-0} != 1 ]]; then
        printf '%s\n' "qualified U4/16 requires explicit S1 and B1 dedicated-runner attestations" >&2
        exit 1
    fi
fi

cd "${live_root}"

printf '%s\n' "[upload-budget] U4/16 server control path profile=${profile} CPU set ${cpu_set}"
rtk node browser/scripts/run-upload-server-processes.mjs \
    --profile "${profile}" \
    --cpu-set "${cpu_set}" \
    --baseline "${baseline}" \
    --output "${server_result}"

printf '%s\n' "[upload-budget] U4/16 production browser upload path profile=${profile}"
(
    cd browser
    rtk env \
        SUPRNOVA_LIVE_B1_DEDICATED="${SUPRNOVA_LIVE_B1_DEDICATED:-0}" \
        taskset -c "${cpu_set}" \
        npm run budget:upload -- \
        --profile "${profile}" \
        --server-result "${server_result}" \
        --baseline "${baseline}" \
        --output "${browser_result}"
)

printf '%s\n' "[upload-budget] checked schema and wiring contract"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test upload_budget_contract
