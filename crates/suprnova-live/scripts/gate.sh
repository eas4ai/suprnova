#!/usr/bin/env bash
set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if ! workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel 2>/dev/null); then
    printf 'Suprnova Live gate cannot resolve its parent Git workspace: %s\n' \
        "${live_root}" >&2
    exit 1
fi
case ${live_root} in
    "${workspace_root}"/*) ;;
    *)
        printf 'Suprnova Live crate root is outside its parent workspace: %s\n' \
            "${live_root}" >&2
        exit 1
        ;;
esac

workspace_manifest=${workspace_root}/Cargo.toml
if [[ ! -f ${workspace_manifest} ]]; then
    printf 'Suprnova workspace manifest is missing: %s\n' "${workspace_manifest}" >&2
    exit 1
fi
workspace_msrv=$(awk -F '"' '/^rust-version[[:space:]]*=/ { print $2; exit }' \
    "${workspace_manifest}")
if [[ -z ${workspace_msrv} ]]; then
    printf 'Suprnova workspace MSRV is missing from %s\n' "${workspace_manifest}" >&2
    exit 1
fi

live_packages=(
    --package suprnova-live
    --package suprnova-macros
    --package suprnova-live-macro-fixture
    --package suprnova-live-test-support
)

phase() {
    printf '\n[%s]\n' "$1"
}

cd "${live_root}"

phase "gate contract"
rtk proxy tests/gate_contract.sh

phase "implementation documentation contract"
rtk proxy tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs

phase "specification structure and archive parity"
rtk node scripts/check-specs.mjs
rtk git diff --check

phase "generated license inventory"
rtk node tests/license_inventory_graph.mjs
rtk node scripts/generate-license-inventory.mjs --check

phase "Rust formatting and lint review"
rtk cargo fmt \
    --manifest-path "${workspace_manifest}" \
    "${live_packages[@]}" \
    -- \
    --check
rtk env CARGO_INCREMENTAL=0 cargo clippy \
    --manifest-path "${workspace_manifest}" \
    "${live_packages[@]}" \
    --all-targets \
    --all-features
rtk node tests/correctness_delay_clippy.mjs
rtk env CARGO_INCREMENTAL=0 cargo clippy \
    --manifest-path "${workspace_manifest}" \
    "${live_packages[@]}" \
    --all-targets \
    --all-features \
    -- \
    -D clippy::disallowed_methods
rtk env CARGO_INCREMENTAL=0 cargo clippy \
    --manifest-path "${live_root}/fuzz/Cargo.toml" \
    --all-targets \
    -- \
    -D clippy::disallowed_methods

phase "Rust fixture, checker, protocol, and security boundaries"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test golden_fixtures \
    --test browser_contract_properties
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test checker_positive \
    --test checker_negative \
    --test checker_regressions
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test compatibility \
    --test protocol_v2
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test security_boundaries
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test security_hostile_context
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-macros \
    --test live_ui

phase "iteration 004 Rust boundaries"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live \
    --test iteration_004_conformance \
    --test iteration_004_adversarial \
    --test iteration_004_exhaustion

phase "Rust MSRV"
rtk env CARGO_INCREMENTAL=0 cargo +"${workspace_msrv}" check \
    --manifest-path "${workspace_manifest}" \
    "${live_packages[@]}" \
    --all-targets \
    --all-features
rtk env CARGO_INCREMENTAL=0 cargo +"${workspace_msrv}" check \
    --manifest-path "${live_root}/tests/fixtures/compile/Cargo.toml" \
    --workspace \
    --all-targets

phase "nightly fuzz build"
rtk cargo +nightly fuzz build --fuzz-dir "${live_root}/fuzz"

phase "browser dependency and conformance gates"
(
    cd browser
    rtk npm ci
    rtk npm run generate:check
    rtk npm run build
    rtk npm run build:check
)

phase "iteration 004 reference host"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live-test-support \
    --test reference_host \
    -- \
    --test-threads=1

phase "Rust all-target and documentation tests"
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    "${live_packages[@]}" \
    --all-targets \
    --all-features \
    --no-fail-fast
rtk env CARGO_INCREMENTAL=0 cargo test \
    --manifest-path "${workspace_manifest}" \
    "${live_packages[@]}" \
    --doc \
    --all-features

phase "correctness-delay scanner"
rtk env CARGO_INCREMENTAL=0 cargo build \
    --manifest-path "${workspace_manifest}" \
    --package suprnova-live-test-support \
    --bin correctness-delay-rust-parser
rtk node tests/correctness_delay_scanner.mjs
rtk node scripts/check-correctness-delays.mjs

(
    cd browser
    rtk npm run format:check
    rtk npm run lint
    rtk npm run typecheck
    phase "iteration 004 browser unit boundaries"
    rtk npm run test:unit -- \
        tests/golden-fixtures.test.ts \
        tests/upload-protocol.test.ts \
        tests/upload-manager.test.ts \
        tests/async-envelope.test.ts \
        tests/async-feature.test.ts \
        tests/async-dispatch.test.ts \
        tests/bounded-resources.test.ts
    rtk npm run test:unit -- \
        tests/feature-host.test.ts \
        tests/document-lifecycle.test.ts \
        tests/optional-artifacts.test.ts \
        tests/build-contract.test.ts \
        tests/budget-contract.test.ts
    phase "browser broad unit suite"
    rtk npm run test:unit
    phase "iteration 004 browser matrix"
    rtk npm run test:browser -- \
        e2e/iteration-004-integration.spec.ts \
        e2e/iteration-004-adversarial.spec.ts \
        e2e/iteration-004-lifecycle.spec.ts \
        e2e/iteration-004-accessibility.spec.ts \
        --project=chromium \
        --project=firefox \
        --project=webkit
    rtk npm run test:browser -- e2e/csp.spec.ts --project=chromium
    phase "real BFCache browser lifecycle"
    rtk npm run test:browser -- \
        e2e/async-lifecycle.spec.ts \
        e2e/iteration-004-lifecycle.spec.ts \
        --project=chrome-bfcache
    phase "browser broad matrix"
    rtk npm run test:browser -- \
        --project=chromium \
        --project=firefox \
        --project=webkit

    if [[ ${SUPRNOVA_LIVE_RELEASE:-0} == 1 ]]; then
        rtk npm run compatibility:check
    else
        rtk npm run compatibility:check -- --allow-unqualified
    fi
)

phase "final worktree diff check"
rtk git diff --check

phase "complete"
printf '%s\n' "Suprnova Live iteration gate passed"
