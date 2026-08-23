#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
gate_path=${repository_root}/scripts/gate.sh

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

require_text "incremental-build disablement" "CARGO_INCREMENTAL=0"
require_text "exact browser lockfile install" "npm ci"
require_text "browser contract generation drift" "npm run generate:check"
require_text "browser format check" "npm run format:check"
require_text "browser lint" "npm run lint"
require_text "browser typecheck" "npm run typecheck"
require_text "Vitest suite" "npm run test:unit"
require_text "production browser build" "npm run build"
require_text "deterministic browser assets" "npm run build:check"
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
require_text "MSRV check" "cargo +1.91.1 check"
require_text "compile-fixture MSRV check" "--manifest-path tests/fixtures/compile/Cargo.toml"
require_text "license gate" "node scripts/generate-license-inventory.mjs --check"

require_file "CSP Playwright coverage" "browser/e2e/csp.spec.ts"
require_file "accessibility Playwright coverage" "browser/e2e/accessibility.spec.ts"
require_file "leak Playwright coverage" "browser/e2e/leaks.spec.ts"
require_file "bfcache Playwright coverage" "browser/e2e/bfcache.spec.ts"
require_file "shared v1 fixture manifest" "fixtures/v1/manifest.sha256"
require_file "shared v2 fixture manifest" "fixtures/v2/manifest.sha256"
require_file "shared v3 fixture manifest" "fixtures/v3/manifest.sha256"

if [[ ${gate_source} == *"-D warnings"* ]]; then
    printf '%s\n' "gate contract: blanket -D warnings is forbidden" >&2
    exit 1
fi

before_install=${gate_source%%"npm ci"*}
before_build=${gate_source%%"npm run build:check"*}
before_browser=${gate_source%%"npm run test:browser"*}
if (( ${#before_install} >= ${#before_build} || ${#before_build} >= ${#before_browser} )); then
    printf '%s\n' \
        "gate contract: browser dependency order must be ci, build, then Playwright" >&2
    exit 1
fi

printf '%s\n' "gate contract ok"
