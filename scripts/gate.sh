#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
local_benchmark_result=${repository_root}/benchmarks/local/gate-snapshot-budget-v1.json
local_action_result=${repository_root}/benchmarks/local/gate-action-budget-v1.json

phase() {
    printf '\n[%s]\n' "$1"
}

cd "${repository_root}"

phase "gate contract"
rtk proxy tests/gate_contract.sh

phase "implementation documentation contract"
rtk proxy tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs

phase "specification structure and archive parity"
rtk node scripts/check-specs.mjs
rtk git diff --check

phase "generated license inventory"
rtk node scripts/generate-license-inventory.mjs --check

phase "Rust formatting and lint review"
rtk cargo fmt --all -- --check
rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features

phase "Rust fixture, checker, protocol, and security boundaries"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --test golden_fixtures \
    --test browser_contract_properties
rtk env CARGO_INCREMENTAL=0 cargo test --test checker_positive --test checker_negative --test checker_regressions
rtk env CARGO_INCREMENTAL=0 cargo test --test compatibility --test protocol_v2
rtk env CARGO_INCREMENTAL=0 cargo test --test security_boundaries
rtk env CARGO_INCREMENTAL=0 cargo test --test security_hostile_context
rtk env CARGO_INCREMENTAL=0 cargo test -p suprnova-live-macros --test ui

phase "Rust all-target and documentation tests"
rtk env CARGO_INCREMENTAL=0 cargo test --workspace --all-targets --all-features --no-fail-fast
rtk env CARGO_INCREMENTAL=0 cargo test --workspace --doc --all-features

phase "Rust MSRV"
rtk env CARGO_INCREMENTAL=0 cargo +1.91.1 check --workspace --all-targets --all-features
rtk env CARGO_INCREMENTAL=0 cargo +1.91.1 check --manifest-path tests/fixtures/compile/Cargo.toml --workspace --all-targets

phase "nightly fuzz build"
rtk cargo +nightly fuzz build

phase "browser dependency and conformance gates"
(
    cd browser
    rtk npm ci
    rtk npm run generate:check
    rtk npm run format:check
    rtk npm run lint
    rtk npm run typecheck
    rtk npm run test:unit -- \
        tests/feature-host.test.ts \
        tests/document-lifecycle.test.ts \
        tests/optional-artifacts.test.ts \
        tests/build-contract.test.ts \
        tests/budget-contract.test.ts
    rtk npm run test:unit
    rtk npm run build
    rtk npm run build:check
    rtk npm run test:browser -- e2e/csp.spec.ts --project=chromium
    rtk npm run test:browser -- \
        --project=chromium \
        --project=firefox \
        --project=webkit

    if [[ ${SUPRNOVA_LIVE_RELEASE:-0} == 1 ]]; then
        rtk npm run compatibility:check
        rtk npm run budget:browser -- --release --dedicated
        rtk npm run budget -- --release
    else
        rtk npm run compatibility:check -- --allow-unqualified
        rtk npm run budget
    fi
)

phase "A8/16 snapshot budget"
rtk env \
    CARGO_INCREMENTAL=0 \
    SUPRNOVA_LIVE_BENCH_RESULT="${local_benchmark_result}" \
    scripts/run-snapshot-budget.sh

phase "A8/16 action framework budget"
rtk env \
    CARGO_INCREMENTAL=0 \
    SUPRNOVA_LIVE_BENCH_RESULT="${local_action_result}" \
    scripts/run-action-budget.sh

phase "U4/16 upload framework and browser budget"
if [[ ${SUPRNOVA_LIVE_RELEASE:-0} == 1 ]]; then
    rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-upload-budget.sh
else
    rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-upload-budget.sh
fi

phase "E100/1K and R100 async continuity budgets"
if [[ ${SUPRNOVA_LIVE_RELEASE:-0} == 1 ]]; then
    rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-async-budget.sh
else
    rtk env SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-async-budget.sh
fi

phase "macro expansion and isolated compile budget"
rtk node scripts/check-expansion-budget.mjs

phase "complete"
printf '%s\n' "Suprnova Live iteration gate passed"
