#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
gate_path=${repository_root}/scripts/gate.sh

if [[ ! -f ${gate_path} ]]; then
    printf '%s\n' "gate contract: scripts/gate.sh is missing" >&2
    exit 1
fi

gate_source=$(<"${gate_path}")

require_text() {
    local description=$1
    local needle=$2
    if [[ ${gate_source} != *"${needle}"* ]]; then
        printf 'gate contract: missing %s (%s)\n' "${description}" "${needle}" >&2
        exit 1
    fi
}

require_text "incremental-build disablement" "CARGO_INCREMENTAL=0"
require_text "structural specification check" "node scripts/check-specs.mjs"
require_text "Rust fixture parity" "cargo test --test golden_fixtures"
require_text "TypeScript fixture parity" "npm test"
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
require_text "MSRV check" "cargo +1.91.1 check"
require_text "compile-fixture MSRV check" "--manifest-path tests/fixtures/compile/Cargo.toml"
require_text "license gate" "node scripts/generate-license-inventory.mjs --check"

if [[ ${gate_source} == *"-D warnings"* ]]; then
    printf '%s\n' "gate contract: blanket -D warnings is forbidden" >&2
    exit 1
fi

printf '%s\n' "gate contract ok"
