#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
gate_path=${repository_root}/scripts/gate.sh
upload_runner_path=${repository_root}/scripts/run-upload-budget.sh
async_runner_path=${repository_root}/scripts/run-async-budget.sh

if [[ ! -f ${gate_path} ]]; then
    printf '%s\n' "gate contract: scripts/gate.sh is missing" >&2
    exit 1
fi

gate_source=$(<"${gate_path}")

require_file() {
    local description=$1
    local relative_path=$2
    if [[ ! -f ${repository_root}/${relative_path} ]]; then
        printf 'gate contract: missing %s (%s)\n' "${description}" "${relative_path}" >&2
        exit 1
    fi
}

require_text() {
    local description=$1
    local needle=$2
    if [[ ${gate_source} != *"${needle}"* ]]; then
        printf 'gate contract: missing %s (%s)\n' "${description}" "${needle}" >&2
        exit 1
    fi
}

require_order() {
    local earlier_description=$1
    local earlier=$2
    local later_description=$3
    local later=$4
    local before_earlier=${gate_source%%"${earlier}"*}
    local before_later=${gate_source%%"${later}"*}

    require_text "${earlier_description}" "${earlier}"
    require_text "${later_description}" "${later}"
    if (( ${#before_earlier} >= ${#before_later} )); then
        printf 'gate contract: %s must run before %s\n' \
            "${earlier_description}" "${later_description}" >&2
        exit 1
    fi
}

require_text "incremental-build disablement" "CARGO_INCREMENTAL=0"
require_text "exact browser lockfile install" "npm ci"
require_text "browser contract generation drift" "npm run generate:check"
require_text "browser format check" "npm run format:check"
require_text "browser lint" "npm run lint"
require_text "browser typecheck" "npm run typecheck"
require_text "Vitest suite" "npm run test:unit"
require_text "focused optional feature lifecycle" "feature-host.test.ts"
require_text "focused document lifecycle" "document-lifecycle.test.ts"
require_text "focused optional artifacts" "optional-artifacts.test.ts"
require_text "focused deterministic build contract" "build-contract.test.ts"
require_text "focused role budget contract" "budget-contract.test.ts"
require_text "production browser build" "npm run build"
require_text "deterministic browser assets" "npm run build:check"
require_text "focused CSP browser coverage" "e2e/csp.spec.ts"
require_text "Chromium browser suite" "--project=chromium"
require_text "Firefox browser suite" "--project=firefox"
require_text "WebKit browser suite" "--project=webkit"
require_text "actual-browser compatibility check" "npm run compatibility:check"
require_text "honest local compatibility classification" "--allow-unqualified"
require_text "release compatibility qualification" "SUPRNOVA_LIVE_RELEASE"
require_text "implementation documentation contract" "tests/documentation_contract.sh"
require_text "implementation documentation links" "node scripts/check-implementation-docs.mjs"
require_text "structural specification check" "node scripts/check-specs.mjs"
require_text "Rust fixture parity" "--test golden_fixtures"
require_text "Rust browser contract properties" "--test browser_contract_properties"
require_text "TypeScript fixture parity" "npm run test:unit"
require_text "macro compile UI contract" "cargo test -p suprnova-live-macros --test ui"
require_text "checked template fixtures" "cargo test --test checker_positive --test checker_negative --test checker_regressions"
require_text "protocol v1/v2 parity" "cargo test --test compatibility --test protocol_v2"
require_text "security-boundary tests" "cargo test --test security_boundaries"
require_text "hostile-context tests" "cargo test --test security_hostile_context"
require_text "nightly fuzz build" "cargo +nightly fuzz build"
require_text "Rust snapshot budget" "scripts/run-snapshot-budget.sh"
require_text "Rust action budget" "scripts/run-action-budget.sh"
require_text "macro expansion budget" "node scripts/check-expansion-budget.mjs"
require_text "browser byte budget" "npm run budget"
require_text "full async release workload budget" "npm run budget:browser -- --release --dedicated"
require_text "MSRV check" "cargo +1.91.1 check"
require_text "compile-fixture MSRV check" "--manifest-path tests/fixtures/compile/Cargo.toml"
require_text "license gate" "node scripts/generate-license-inventory.mjs --check"
require_text "required phase \"U4/16 upload budget\"" 'phase "iteration 004 reduced deterministic budgets"'
require_text "legacy U4/16 budget phase" 'phase "U4/16 upload framework and browser budget"'
require_text "legacy async continuity budget phase" 'phase "E100/1K and R100 async continuity budgets"'
require_text "iteration 004 Rust boundary phase" 'phase "iteration 004 Rust boundaries"'
require_text "iteration 004 conformance test" "--test iteration_004_conformance"
require_text "iteration 004 adversarial test" "--test iteration_004_adversarial"
require_text "iteration 004 exhaustion test" "--test iteration_004_exhaustion"
require_text "iteration 004 reference-host phase" 'phase "iteration 004 reference host"'
require_text "thin Rust reference-host integration" \
    "cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1"
require_text "iteration 004 browser matrix phase" 'phase "iteration 004 browser matrix"'
require_text "iteration 004 browser unit phase" 'phase "iteration 004 browser unit boundaries"'
require_text "broad browser unit phase" 'phase "browser broad unit suite"'
require_text "broad browser matrix phase" 'phase "browser broad matrix"'
require_text "iteration 004 browser integration matrix" "e2e/iteration-004-integration.spec.ts"
require_text "iteration 004 browser adversarial matrix" "e2e/iteration-004-adversarial.spec.ts"
require_text "iteration 004 browser lifecycle matrix" "e2e/iteration-004-lifecycle.spec.ts"
require_text "iteration 004 browser accessibility matrix" "e2e/iteration-004-accessibility.spec.ts"
require_text "reduced upload workload" \
    "SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-upload-budget.sh"
require_text "reduced async workloads" \
    "SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-async-budget.sh"
require_text "qualified U4/16 release phase" 'phase "U4/16 qualified upload budget"'
require_text "qualified U4/16 release workload" \
    "SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-upload-budget.sh"
require_text "qualified E100/1K and R100 release phase" \
    'phase "E100/1K and R100 qualified async budgets"'
require_text "qualified E100/1K and R100 release workloads" \
    "SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-async-budget.sh"
require_text "release workload guard" \
    'if [[ "${SUPRNOVA_LIVE_RELEASE:-0}" == "1" ]]; then'

require_file "CSP Playwright coverage" "browser/e2e/csp.spec.ts"
require_file "accessibility Playwright coverage" "browser/e2e/accessibility.spec.ts"
require_file "leak Playwright coverage" "browser/e2e/leaks.spec.ts"
require_file "bfcache Playwright coverage" "browser/e2e/bfcache.spec.ts"
require_file "shared v1 fixture manifest" "fixtures/v1/manifest.sha256"
require_file "shared v2 fixture manifest" "fixtures/v2/manifest.sha256"
require_file "shared v3 fixture manifest" "fixtures/v3/manifest.sha256"
require_file "shared v4 fixture manifest" "fixtures/v4/manifest.sha256"
require_file "iteration 004 Rust conformance matrix" "tests/iteration_004_conformance.rs"
require_file "iteration 004 Rust adversarial matrix" "tests/iteration_004_adversarial.rs"
require_file "iteration 004 Rust exhaustion matrix" "tests/iteration_004_exhaustion.rs"
require_file "thin Rust reference-host integration" \
    "crates/suprnova-live-test-support/tests/reference_host.rs"
require_file "upload protocol fuzz target" "fuzz/fuzz_targets/upload_protocol.rs"
require_file "upload transition fuzz target" "fuzz/fuzz_targets/upload_state.rs"
require_file "upload media-header fuzz target" "fuzz/fuzz_targets/upload_media_header.rs"
require_file "async envelope fuzz target" "fuzz/fuzz_targets/async_envelope.rs"
require_file "async sequence fuzz target" "fuzz/fuzz_targets/async_sequence.rs"

if [[ ${gate_source} == *"-D warnings"* ]]; then
    printf '%s\n' "gate contract: blanket -D warnings is forbidden" >&2
    exit 1
fi

upload_runner_source=$(<"${upload_runner_path}")
async_runner_source=$(<"${async_runner_path}")
if [[ ${upload_runner_source} != *"SUPRNOVA_LIVE_S1_DEDICATED"* ||
      ${upload_runner_source} != *"SUPRNOVA_LIVE_B1_DEDICATED"* ||
      ${async_runner_source} != *"SUPRNOVA_LIVE_B1_DEDICATED"* ]]; then
    printf '%s\n' \
        "gate contract: qualified workloads must fail closed without B1/S1 attestation" >&2
    exit 1
fi

budget_source=${gate_source#*'phase "iteration 004 reduced deterministic budgets"'}
before_release=${budget_source%%'if [[ "${SUPRNOVA_LIVE_RELEASE:-0}" == "1" ]]; then'*}
release_source=${budget_source#*'if [[ "${SUPRNOVA_LIVE_RELEASE:-0}" == "1" ]]; then'}
release_source=${release_source%%$'\nfi'*}
if [[ ${before_release} != *"SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-upload-budget.sh"* ||
      ${before_release} != *"SUPRNOVA_LIVE_BUDGET_PROFILE=reduced scripts/run-async-budget.sh"* ]]; then
    printf '%s\n' "gate contract: ordinary mode must run both reduced workloads" >&2
    exit 1
fi
if [[ ${release_source} != *"SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-upload-budget.sh"* ||
      ${release_source} != *"SUPRNOVA_LIVE_BUDGET_PROFILE=qualified scripts/run-async-budget.sh"* ]]; then
    printf '%s\n' \
        "gate contract: release mode cannot substitute reduced evidence for qualified workloads" >&2
    exit 1
fi

require_order "browser lockfile install" "npm ci" \
    "deterministic browser build" "npm run build:check"
require_order "deterministic browser build" "npm run build:check" \
    "browser matrix" 'phase "iteration 004 browser matrix"'
require_order "iteration 004 browser unit boundaries" \
    'phase "iteration 004 browser unit boundaries"' \
    "broad browser unit suite" 'phase "browser broad unit suite"'
require_order "iteration 004 browser matrix" 'phase "iteration 004 browser matrix"' \
    "broad browser matrix" 'phase "browser broad matrix"'
require_order "iteration 004 Rust boundaries" 'phase "iteration 004 Rust boundaries"' \
    "broad Rust suite" 'phase "Rust all-target and documentation tests"'
require_order "iteration 004 reference host" 'phase "iteration 004 reference host"' \
    "broad Rust suite" 'phase "Rust all-target and documentation tests"'
require_order "deterministic browser build" "npm run build:check" \
    "reduced deterministic budgets" 'phase "iteration 004 reduced deterministic budgets"'

printf '%s\n' "gate contract ok"
