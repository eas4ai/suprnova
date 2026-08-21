#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
local_benchmark_result=${repository_root}/benchmarks/local/gate-snapshot-budget-v1.json

phase() {
    printf '\n[%s]\n' "$1"
}

cd "${repository_root}"

phase "gate contract"
rtk proxy tests/gate_contract.sh

phase "specification structure and archive parity"
rtk node scripts/check-specs.mjs
rtk git diff --check

phase "generated license inventory"
rtk node scripts/generate-license-inventory.mjs --check

phase "Rust formatting and lint review"
rtk cargo fmt --all -- --check
rtk env CARGO_INCREMENTAL=0 cargo clippy --all-targets --all-features

phase "Rust fixture and security boundaries"
rtk env CARGO_INCREMENTAL=0 cargo test --test golden_fixtures
rtk env CARGO_INCREMENTAL=0 cargo test --test security_boundaries

phase "Rust all-target and documentation tests"
rtk env CARGO_INCREMENTAL=0 cargo test --all-targets --all-features --no-fail-fast
rtk env CARGO_INCREMENTAL=0 cargo test --doc --all-features

phase "Rust MSRV"
rtk env CARGO_INCREMENTAL=0 cargo +1.91.1 check --all-targets --all-features

phase "nightly fuzz build"
rtk cargo +nightly fuzz build

phase "browser dependency and conformance gates"
(
    cd browser
    rtk npm ci
    rtk npm run format:check
    rtk npm run lint
    rtk npm run typecheck
    rtk npm test
    rtk npm run build
    rtk npm run budget
)

phase "A8/16 snapshot budget"
rtk env \
    CARGO_INCREMENTAL=0 \
    SUPRNOVA_LIVE_BENCH_RESULT="${local_benchmark_result}" \
    scripts/run-snapshot-budget.sh

phase "complete"
printf '%s\n' "Suprnova Live iteration gate passed"
