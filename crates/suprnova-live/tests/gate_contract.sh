#!/usr/bin/env bash
set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel)
gate_path=${live_root}/scripts/gate.sh

if [[ ! -f ${gate_path} ]]; then
    printf '%s\n' "gate contract: scripts/gate.sh is missing" >&2
    exit 1
fi

gate_source=$(<"${gate_path}")

require_file() {
    local description=$1
    local relative_path=$2
    if [[ ! -f ${live_root}/${relative_path} ]]; then
        printf 'gate contract: missing %s (%s)\n' "${description}" "${relative_path}" >&2
        exit 1
    fi
}

require_text() {
    local description=$1
    local needle=$2
    if [[ ${gate_source} != *"${needle}"* ]]; then
        local normalized=${gate_source//$'\n'/ }
        while [[ ${normalized} == *"  "* ]]; do
            normalized=${normalized//  / }
        done
        if [[ ${normalized} == *"${needle}"* ]]; then
            return
        fi
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

contains_blanket_warning_denial() {
    local source=$1
    local short_form='(^|[[:space:]])-D[[:space:]]*warnings([[:space:]]|$)'
    local long_form='(^|[[:space:]])--deny([=[:space:]]+)warnings([[:space:]]|$)'
    local unquoted=${source//\"/}
    local assignment
    local cargo_flag_variable
    local cargo_flags
    local line

    unquoted=${unquoted//\'/}
    if [[ ${unquoted} =~ ${short_form} || ${unquoted} =~ ${long_form} ]]; then
        return 0
    fi

    while IFS= read -r line; do
        for cargo_flag_variable in RUSTFLAGS CARGO_ENCODED_RUSTFLAGS; do
            if [[ ${line} =~ (^|[[:space:]])${cargo_flag_variable}[[:space:]]*= ]]; then
                assignment=${BASH_REMATCH[0]}
                cargo_flags=${line#*"${assignment}"}
                cargo_flags=${cargo_flags#"${cargo_flags%%[![:space:]]*}"}
                if [[ ${cargo_flags:0:2} == "\$'" ]]; then
                    cargo_flags=${cargo_flags:2}
                    cargo_flags=${cargo_flags%%\'*}
                else
                    case ${cargo_flags:0:1} in
                        '"')
                            cargo_flags=${cargo_flags:1}
                            cargo_flags=${cargo_flags%%\"*}
                            ;;
                        "'")
                            cargo_flags=${cargo_flags:1}
                            cargo_flags=${cargo_flags%%\'*}
                            ;;
                        *) cargo_flags=${cargo_flags%%[[:space:]]*} ;;
                    esac
                fi
                cargo_flags=${cargo_flags//$'\x1f'/ }
                cargo_flags=${cargo_flags//\\x1f/ }
                cargo_flags=${cargo_flags//\\x1F/ }
                cargo_flags=${cargo_flags//\\037/ }
                if [[ ${cargo_flags} =~ ${short_form} ||
                      ${cargo_flags} =~ ${long_form} ]]; then
                    return 0
                fi
            fi
        done
    done <<<"${source}"

    return 1
}

run_gate_probe() {
    local gate_under_test=$1
    local release_mode=$2
    local trace_path=$3
    local output_path=$4

    : >"${trace_path}"
    if ! PATH="${probe_root}:${PATH}" \
        SUPRNOVA_LIVE_GATE_TRACE="${trace_path}" \
        SUPRNOVA_LIVE_RELEASE="${release_mode}" \
        bash "${gate_under_test}" >"${output_path}" 2>&1; then
        printf 'gate contract: stubbed gate execution failed in release mode %s\n' \
            "${release_mode}" >&2
        exit 1
    fi
}

normalize_gate_trace() {
    local trace_path=$1
    local normalized_path=$2
    local field
    local index
    local start
    local -a fields

    : >"${normalized_path}"
    while IFS=$'\t' read -r -a fields; do
        start=0
        if (( ${#fields[@]} > 0 )) && [[ ${fields[0]} == profile=* ]]; then
            start=1
        fi
        if (( ${#fields[@]} <= start )); then
            continue
        fi
        for ((index = start; index < ${#fields[@]}; index += 1)); do
            field=${fields[index]//${live_root}/<live>}
            field=${field//${workspace_root}/<workspace>}
            if (( index > start )); then
                printf '\t' >>"${normalized_path}"
            fi
            printf '%s' "${field}" >>"${normalized_path}"
        done
        printf '\n' >>"${normalized_path}"
    done <"${trace_path}"
}

write_expected_gate_commands() {
    local release_mode=$1
    local expected_path=$2

    printf '%s\n' \
        $'proxy\ttests/gate_contract.sh' \
        $'proxy\ttests/documentation_contract.sh' \
        $'node\tscripts/check-implementation-docs.mjs' \
        $'node\tscripts/check-specs.mjs' \
        $'git\tdiff\t--check' \
        $'node\ttests/license_inventory_graph.mjs' \
        $'node\tscripts/generate-license-inventory.mjs\t--check' \
        $'cargo\tfmt\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--\t--check' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--all-targets\t--all-features' \
        $'node\ttests/correctness_delay_clippy.mjs' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--all-targets\t--all-features\t--\t-D\tclippy::disallowed_methods' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--manifest-path\t<live>/fuzz/Cargo.toml\t--all-targets\t--\t-D\tclippy::disallowed_methods' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--test\tgolden_fixtures\t--test\tbrowser_contract_properties' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--test\tchecker_positive\t--test\tchecker_negative\t--test\tchecker_regressions' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--test\tcompatibility\t--test\tprotocol_v2' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--test\tsecurity_boundaries' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--test\tsecurity_hostile_context' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-macros\t--test\tlive_ui' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--test\titeration_004_conformance\t--test\titeration_004_adversarial\t--test\titeration_004_exhaustion' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\t+1.94.0\tcheck\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--all-targets\t--all-features' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\t+1.94.0\tcheck\t--manifest-path\t<live>/tests/fixtures/compile/Cargo.toml\t--workspace\t--all-targets' \
        $'cargo\t+nightly\tfuzz\tbuild\t--fuzz-dir\t<live>/fuzz' \
        $'npm\tci' \
        $'npm\trun\tgenerate:check' \
        $'npm\trun\tbuild' \
        $'npm\trun\tbuild:check' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live-test-support\t--test\treference_host\t--\t--test-threads=1' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--all-targets\t--all-features\t--no-fail-fast' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--doc\t--all-features' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tbuild\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live-test-support\t--bin\tcorrectness-delay-rust-parser' \
        $'node\ttests/correctness_delay_scanner.mjs' \
        $'node\tscripts/check-correctness-delays.mjs' \
        $'npm\trun\tformat:check' \
        $'npm\trun\tlint' \
        $'npm\trun\ttypecheck' \
        $'npm\trun\ttest:unit\t--\ttests/golden-fixtures.test.ts\ttests/upload-protocol.test.ts\ttests/upload-manager.test.ts\ttests/async-envelope.test.ts\ttests/async-feature.test.ts\ttests/async-dispatch.test.ts\ttests/bounded-resources.test.ts' \
        $'npm\trun\ttest:unit\t--\ttests/feature-host.test.ts\ttests/document-lifecycle.test.ts\ttests/optional-artifacts.test.ts\ttests/build-contract.test.ts\ttests/budget-contract.test.ts' \
        $'npm\trun\ttest:unit' \
        $'npm\trun\ttest:browser\t--\te2e/iteration-004-integration.spec.ts\te2e/iteration-004-adversarial.spec.ts\te2e/iteration-004-lifecycle.spec.ts\te2e/iteration-004-accessibility.spec.ts\t--project=chromium\t--project=firefox\t--project=webkit' \
        $'npm\trun\ttest:browser\t--\te2e/csp.spec.ts\t--project=chromium' \
        $'npm\trun\ttest:browser\t--\te2e/async-lifecycle.spec.ts\te2e/iteration-004-lifecycle.spec.ts\t--project=chrome-bfcache' \
        $'npm\trun\ttest:browser\t--\t--project=chromium\t--project=firefox\t--project=webkit' \
        >"${expected_path}"

    if [[ ${release_mode} == 1 ]]; then
        printf '%s\n' \
            $'npm\trun\tcompatibility:check' \
            >>"${expected_path}"
    else
        printf '%s\n' \
            $'npm\trun\tcompatibility:check\t--\t--allow-unqualified' \
            >>"${expected_path}"
    fi

    printf '%s\n' \
        $'git\tdiff\t--check' \
        >>"${expected_path}"
}

normalize_phase_trace() {
    local output_path=$1
    local normalized_path=$2
    local line

    : >"${normalized_path}"
    while IFS= read -r line; do
        if [[ ${line} == \[*\] ]]; then
            printf '%s\n' "${line:1:${#line}-2}" >>"${normalized_path}"
        fi
    done <"${output_path}"
}

write_expected_gate_phases() {
    local release_mode=$1
    local expected_path=$2

    printf '%s\n' \
        "gate contract" \
        "implementation documentation contract" \
        "specification structure and archive parity" \
        "generated license inventory" \
        "Rust formatting and lint review" \
        "Rust fixture, checker, protocol, and security boundaries" \
        "iteration 004 Rust boundaries" \
        "Rust MSRV" \
        "nightly fuzz build" \
        "browser dependency and conformance gates" \
        "iteration 004 reference host" \
        "Rust all-target and documentation tests" \
        "correctness-delay scanner" \
        "iteration 004 browser unit boundaries" \
        "browser broad unit suite" \
        "iteration 004 browser matrix" \
        "real BFCache browser lifecycle" \
        "browser broad matrix" \
        >"${expected_path}"

    printf '%s\n' \
        "final worktree diff check" \
        "complete" \
        >>"${expected_path}"
}

gate_execution_trace_is_valid() {
    local release_mode=$1
    local trace_path=$2
    local output_path=$3
    local label=$4
    local actual_commands=${probe_root}/${label}.commands.actual
    local expected_commands=${probe_root}/${label}.commands.expected
    local actual_phases=${probe_root}/${label}.phases.actual
    local expected_phases=${probe_root}/${label}.phases.expected

    normalize_gate_trace "${trace_path}" "${actual_commands}"
    write_expected_gate_commands "${release_mode}" "${expected_commands}"
    normalize_phase_trace "${output_path}" "${actual_phases}"
    write_expected_gate_phases "${release_mode}" "${expected_phases}"
    cmp -s "${expected_commands}" "${actual_commands}" &&
        cmp -s "${expected_phases}" "${actual_phases}"
}

require_gate_execution_trace() {
    local description=$1
    local release_mode=$2
    local trace_path=$3
    local output_path=$4
    local label=$5

    if ! gate_execution_trace_is_valid \
        "${release_mode}" "${trace_path}" "${output_path}" "${label}"; then
        printf 'gate contract: %s executable phase/command trace drifted\n' \
            "${description}" >&2
        diff -u \
            "${probe_root}/${label}.commands.expected" \
            "${probe_root}/${label}.commands.actual" >&2 || true
        diff -u \
            "${probe_root}/${label}.phases.expected" \
            "${probe_root}/${label}.phases.actual" >&2 || true
        exit 1
    fi
}

gate_stops_at_clippy_failure() {
    local gate_under_test=$1
    local trace_path=$2
    local output_path=$3
    local normalized_path=$4
    local expected_last=$'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--manifest-path\t<workspace>/Cargo.toml\t--package\tsuprnova-live\t--package\tsuprnova-macros\t--package\tsuprnova-live-macro-fixture\t--package\tsuprnova-live-test-support\t--all-targets\t--all-features'
    local status
    local last_command

    : >"${trace_path}"
    if PATH="${probe_root}:${PATH}" \
        SUPRNOVA_LIVE_GATE_TRACE="${trace_path}" \
        SUPRNOVA_LIVE_GATE_FAIL_MATCH="cargo clippy" \
        SUPRNOVA_LIVE_RELEASE=0 \
        bash "${gate_under_test}" >"${output_path}" 2>&1; then
        status=0
    else
        status=$?
    fi
    normalize_gate_trace "${trace_path}" "${normalized_path}"
    last_command=$(tail -n 1 "${normalized_path}")

    (( status != 0 )) && [[ ${last_command} == "${expected_last}" ]]
}

write_replacement_mutant() {
    local source_path=$1
    local destination_path=$2
    local needle=$3
    local replacement=$4
    local source
    local mutated

    source=$(<"${source_path}")
    mutated=${source/"${needle}"/"${replacement}"}
    if [[ ${mutated} == "${source}" ]]; then
        printf 'gate contract: could not create gate mutation for %s\n' \
            "${needle}" >&2
        exit 1
    fi
    printf '%s\n' "${mutated}" >"${destination_path}"
}

require_text "incremental-build disablement" "CARGO_INCREMENTAL=0"
require_text "strict shell mode" "set -euo pipefail"
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
require_text "focused browser benchmark runner contract" "budget-contract.test.ts"
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
require_text "crate-root ownership" 'live_root='
require_text "workspace-root ownership" 'workspace_root='
require_text "parent workspace manifest" 'workspace_root}/Cargo.toml'
require_text "explicit Live package scope" '--package suprnova-live'
require_text "macro compile package" "--package suprnova-macros"
require_text "macro compile UI contract" "--test live_ui"
require_text "checked template fixtures" "--test checker_regressions"
require_text "protocol v1/v2 parity" "--test protocol_v2"
require_text "security-boundary tests" "--test security_boundaries"
require_text "hostile-context tests" "--test security_hostile_context"
require_text "nightly fuzz build" 'cargo +nightly fuzz build --fuzz-dir "${live_root}/fuzz"'
require_text "workspace MSRV check" 'cargo +"${workspace_msrv}" check'
require_text "compile-fixture MSRV check" 'live_root}/tests/fixtures/compile/Cargo.toml'
require_text "license gate" "node scripts/generate-license-inventory.mjs --check"
require_text "license inventory graph contract" \
    "node tests/license_inventory_graph.mjs"
require_order "license inventory graph contract" \
    "node tests/license_inventory_graph.mjs" \
    "generated license inventory check" \
    "node scripts/generate-license-inventory.mjs --check"
require_text "correctness-delay scanner phase" 'phase "correctness-delay scanner"'
require_text "correctness-delay Rust parser build" \
    "--bin correctness-delay-rust-parser"
require_text "correctness-delay scanner self-tests" \
    "node tests/correctness_delay_scanner.mjs"
require_text "compiler-resolved correctness-delay self-test" \
    "node tests/correctness_delay_clippy.mjs"
require_text "compiler-resolved correctness-delay lint denial" \
    "-D clippy::disallowed_methods"
require_text "compiler-resolved fuzz correctness-delay lint" \
    'live_root}/fuzz/Cargo.toml'
require_text "correctness-delay repository scan" \
    "node scripts/check-correctness-delays.mjs"
require_order "correctness-delay Rust parser build" \
    "--bin correctness-delay-rust-parser" \
    "correctness-delay scanner self-tests" \
    "node tests/correctness_delay_scanner.mjs"
require_text "iteration 004 Rust boundary phase" 'phase "iteration 004 Rust boundaries"'
require_text "iteration 004 conformance test" "--test iteration_004_conformance"
require_text "iteration 004 adversarial test" "--test iteration_004_adversarial"
require_text "iteration 004 exhaustion test" "--test iteration_004_exhaustion"
require_text "iteration 004 reference-host phase" 'phase "iteration 004 reference host"'
require_text "thin Rust reference-host integration" \
    "--test reference_host"
require_text "serialized reference-host integration" "--test-threads=1"
require_text "iteration 004 browser matrix phase" 'phase "iteration 004 browser matrix"'
require_text "real BFCache browser lifecycle phase" 'phase "real BFCache browser lifecycle"'
require_text "iteration 004 browser unit phase" 'phase "iteration 004 browser unit boundaries"'
require_text "broad browser unit phase" 'phase "browser broad unit suite"'
require_text "broad browser matrix phase" 'phase "browser broad matrix"'
require_text "real BFCache Chromium project" "--project=chrome-bfcache"
require_text "iteration 004 browser integration matrix" "e2e/iteration-004-integration.spec.ts"
require_text "iteration 004 browser adversarial matrix" "e2e/iteration-004-adversarial.spec.ts"
require_text "iteration 004 browser lifecycle matrix" "e2e/iteration-004-lifecycle.spec.ts"
require_text "iteration 004 browser accessibility matrix" "e2e/iteration-004-accessibility.spec.ts"
require_text "final worktree diff phase" 'phase "final worktree diff check"'

for on_demand_tool in \
    scripts/run-snapshot-budget.sh \
    scripts/run-action-budget.sh \
    scripts/run-upload-budget.sh \
    scripts/run-async-budget.sh \
    scripts/check-expansion-budget.mjs \
    "npm run budget"; do
    if [[ ${gate_source} == *"${on_demand_tool}"* ]]; then
        printf 'gate contract: budget tool %s is on-demand and must not run in the gate\n' \
            "${on_demand_tool}" >&2
        exit 1
    fi
done

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
require_file "correctness-delay scanner" "scripts/check-correctness-delays.mjs"
require_file "license inventory Cargo graph module" \
    "scripts/license-inventory-cargo.mjs"
require_file "license inventory Cargo graph contract" \
    "tests/license_inventory_graph.mjs"
require_file "correctness-delay scanner mutation tests" \
    "tests/correctness_delay_scanner.mjs"
require_file "compiler-resolved correctness-delay mutation tests" \
    "tests/correctness_delay_clippy.mjs"
require_file "compiler-resolved correctness-delay policy" "clippy.toml"
require_file "correctness-delay JavaScript parser" \
    "scripts/correctness-delay-javascript.mjs"
require_file "iteration 004 verification-surface manifest" \
    "scripts/iteration-004-verification-surfaces.mjs"
require_file "parser-backed Rust syntax validator" \
    "crates/suprnova-live-test-support/src/bin/correctness-delay-rust-parser.rs"

for budget_runner in \
    scripts/run-snapshot-budget.sh \
    scripts/run-action-budget.sh \
    scripts/run-upload-budget.sh \
    scripts/run-async-budget.sh
do
    runner_source=$(<"${live_root}/${budget_runner}")
    for required_runner_text in \
        'live_root=' \
        'workspace_root=' \
        '--manifest-path "${workspace_manifest}"' \
        '--package suprnova-live'
    do
        if [[ ${runner_source} != *"${required_runner_text}"* ]]; then
            printf 'gate contract: %s is missing relocation contract (%s)\n' \
                "${budget_runner}" "${required_runner_text}" >&2
            exit 1
        fi
    done
    if [[ ${runner_source} == *"repository_root="* ]]; then
        printf 'gate contract: %s retains the standalone-root contract\n' \
            "${budget_runner}" >&2
        exit 1
    fi
done

if [[ ${gate_source} == *"--workspace --all-targets --all-features"* ]]; then
    printf '%s\n' "gate contract: Live gate must not sweep the parent workspace" >&2
    exit 1
fi
if [[ ${gate_source} == *"+1.91.1"* ]]; then
    printf '%s\n' "gate contract: standalone MSRV survived integration" >&2
    exit 1
fi

for manifest in \
    Cargo.toml \
    crates/suprnova-live-macro-fixture/Cargo.toml \
    crates/suprnova-live-test-support/Cargo.toml
do
    manifest_source=$(<"${live_root}/${manifest}")
    if [[ ${manifest_source} != *"rust-version.workspace = true"* ]]; then
        printf 'gate contract: %s does not inherit the workspace MSRV\n' \
            "${manifest}" >&2
        exit 1
    fi
done

production_macro_manifest=${workspace_root}/suprnova-macros/Cargo.toml
production_macro_source=$(<"${production_macro_manifest}")
if [[ ${production_macro_source} != *"rust-version.workspace = true"* ]]; then
    printf 'gate contract: %s does not inherit the workspace MSRV\n' \
        "${production_macro_manifest}" >&2
    exit 1
fi
if [[ -e ${live_root}/crates/suprnova-live-macros ]]; then
    printf '%s\n' "gate contract: retired duplicate Live macro package remains" >&2
    exit 1
fi

workspace_manifest_source=$(<"${workspace_root}/Cargo.toml")
if [[ ${workspace_manifest_source} != *'rust-version = "1.94.0"'* ]]; then
    printf '%s\n' "gate contract: Suprnova workspace MSRV is not 1.94.0" >&2
    exit 1
fi
for fixture_manifest in \
    tests/fixtures/compile/1-component/Cargo.toml \
    tests/fixtures/compile/10-component/Cargo.toml \
    tests/fixtures/compile/100-component/Cargo.toml
do
    fixture_source=$(<"${live_root}/${fixture_manifest}")
    if [[ ${fixture_source} != *'rust-version = "1.94.0"'* ]]; then
        printf 'gate contract: %s does not use the Suprnova workspace MSRV\n' \
            "${fixture_manifest}" >&2
        exit 1
    fi
done

rtk node "${live_root}/scripts/generate-license-inventory.mjs" --check

if contains_blanket_warning_denial "${gate_source}"; then
    printf '%s\n' "gate contract: blanket -D warnings is forbidden" >&2
    exit 1
fi

for warning_denial_mutation in \
    'rtk cargo clippy -- -D warnings' \
    'rtk cargo clippy -- -Dwarnings' \
    'rtk cargo clippy -- --deny warnings' \
    $'rtk cargo clippy -- --deny\twarnings' \
    'rtk cargo clippy -- --deny=warnings' \
    'RUSTFLAGS=-Dwarnings rtk cargo clippy' \
    'RUSTFLAGS="-D warnings" rtk cargo clippy' \
    "RUSTFLAGS='--deny warnings' rtk cargo clippy" \
    "RUSTFLAGS='--deny=warnings' rtk cargo clippy" \
    'CARGO_INCREMENTAL=0 RUSTFLAGS=-Dwarnings rtk cargo clippy' \
    'BUILD_SENTINEL=present CARGO_INCREMENTAL=0 RUSTFLAGS="--deny warnings" rtk cargo clippy' \
    '/usr/bin/env CARGO_INCREMENTAL=0 CARGO_ENCODED_RUSTFLAGS=-Dwarnings /opt/bin/rtk cargo clippy' \
    "PATH_SENTINEL=/opt/tools CARGO_INCREMENTAL=0 RUSTFLAGS=\$'-Dwarnings' ./bin/rtk cargo clippy"; do
    if ! contains_blanket_warning_denial "${warning_denial_mutation}"; then
        printf 'gate contract: warning-denial mutation survived (%s)\n' \
            "${warning_denial_mutation}" >&2
        exit 1
    fi
done
if contains_blanket_warning_denial \
    'BUILD_SENTINEL=present CARGO_INCREMENTAL=0 RUSTFLAGS="-C target-cpu=native -D dead_code" rtk cargo clippy'; then
    printf '%s\n' "gate contract: narrow lint denial was mistaken for blanket warning denial" >&2
    exit 1
fi
if contains_blanket_warning_denial \
    "BUILD_SENTINEL=present CARGO_ENCODED_RUSTFLAGS=\$'-Ddead_code\\x1f-Copt-level=2' ./bin/rtk cargo clippy"; then
    printf '%s\n' \
        "gate contract: narrow encoded lint denial was mistaken for blanket warning denial" >&2
    exit 1
fi

require_order "browser lockfile install" "npm ci" \
    "deterministic browser build" "npm run build:check"
require_order "deterministic browser build" "npm run build:check" \
    "reference host" 'phase "iteration 004 reference host"'
require_order "gate contract" 'phase "gate contract"' \
    "correctness-delay scanner" 'phase "correctness-delay scanner"'
require_order "browser lockfile install" "npm ci" \
    "correctness-delay scanner" 'phase "correctness-delay scanner"'
require_order "correctness-delay scanner" 'phase "correctness-delay scanner"' \
    "iteration 004 browser unit boundaries" 'phase "iteration 004 browser unit boundaries"'
require_order "deterministic browser build" "npm run build:check" \
    "browser matrix" 'phase "iteration 004 browser matrix"'
require_order "iteration 004 browser unit boundaries" \
    'phase "iteration 004 browser unit boundaries"' \
    "broad browser unit suite" 'phase "browser broad unit suite"'
require_order "iteration 004 browser matrix" 'phase "iteration 004 browser matrix"' \
    "real BFCache browser lifecycle" 'phase "real BFCache browser lifecycle"'
require_order "real BFCache browser lifecycle" 'phase "real BFCache browser lifecycle"' \
    "broad browser matrix" 'phase "browser broad matrix"'
require_order "iteration 004 Rust boundaries" 'phase "iteration 004 Rust boundaries"' \
    "broad Rust suite" 'phase "Rust all-target and documentation tests"'
require_order "iteration 004 reference host" 'phase "iteration 004 reference host"' \
    "broad Rust suite" 'phase "Rust all-target and documentation tests"'

probe_root=$(mktemp -d)
mutant_runners=()
cleanup_probes() {
    local mutant_runner
    rm -rf -- "${probe_root}"
    for mutant_runner in "${mutant_runners[@]}"; do
        rm -f -- "${mutant_runner}"
    done
}
trap cleanup_probes EXIT
printf '%s\n' \
    '#!/usr/bin/env bash' \
    'set -euo pipefail' \
    'printf '\''profile=%s'\'' "${SUPRNOVA_LIVE_BUDGET_PROFILE-}" >>"${SUPRNOVA_LIVE_GATE_TRACE:?}"' \
    'for argument in "$@"; do' \
    '    printf '\''\t%s'\'' "${argument}" >>"${SUPRNOVA_LIVE_GATE_TRACE:?}"' \
    'done' \
    'printf '\''\n'\'' >>"${SUPRNOVA_LIVE_GATE_TRACE:?}"' \
    'if [[ -n ${SUPRNOVA_LIVE_GATE_FAIL_MATCH-} && "$*" == *"${SUPRNOVA_LIVE_GATE_FAIL_MATCH}"* ]]; then' \
    '    exit 97' \
    'fi' \
    >"${probe_root}/rtk"
chmod +x "${probe_root}/rtk"

ordinary_trace=${probe_root}/ordinary.trace
ordinary_output=${probe_root}/ordinary.output
release_trace=${probe_root}/release.trace
release_output=${probe_root}/release.output
run_gate_probe "${gate_path}" 0 "${ordinary_trace}" "${ordinary_output}"
run_gate_probe "${gate_path}" 1 "${release_trace}" "${release_output}"

alternate_cwd_trace=${probe_root}/alternate-cwd.trace
alternate_cwd_output=${probe_root}/alternate-cwd.output
(
    cd "${probe_root}"
    run_gate_probe \
        "${gate_path}" 0 "${alternate_cwd_trace}" "${alternate_cwd_output}"
)

outside_repository=${probe_root}/outside-repository
mkdir -p "${outside_repository}/scripts"
cp "${gate_path}" "${outside_repository}/scripts/gate.sh"
git -C "${outside_repository}" init --quiet
if PATH="${probe_root}:${PATH}" \
    SUPRNOVA_LIVE_GATE_TRACE="${probe_root}/outside.trace" \
    SUPRNOVA_LIVE_RELEASE=0 \
    bash "${outside_repository}/scripts/gate.sh" \
        >"${probe_root}/outside.output" 2>&1; then
    printf '%s\n' "gate contract: a standalone crate root was accepted" >&2
    exit 1
fi

require_gate_execution_trace \
    "ordinary gate" 0 "${ordinary_trace}" "${ordinary_output}" "ordinary"
require_gate_execution_trace \
    "release gate" 1 "${release_trace}" "${release_output}" "release"
require_gate_execution_trace \
    "alternate-current-directory gate" 0 \
    "${alternate_cwd_trace}" "${alternate_cwd_output}" "alternate-cwd"

if ! gate_stops_at_clippy_failure \
    "${gate_path}" \
    "${probe_root}/strict-original.trace" \
    "${probe_root}/strict-original.output" \
    "${probe_root}/strict-original.normalized"; then
    printf '%s\n' "gate contract: command failure did not stop the real gate" >&2
    exit 1
fi

strict_mutant=$(mktemp "${live_root}/scripts/.gate-contract-strict-mutant.XXXXXX")
mutant_runners+=("${strict_mutant}")
write_replacement_mutant \
    "${gate_path}" "${strict_mutant}" \
    "set -euo pipefail" "set -uo pipefail"
if gate_stops_at_clippy_failure \
    "${strict_mutant}" \
    "${probe_root}/strict-mutant.trace" \
    "${probe_root}/strict-mutant.output" \
    "${probe_root}/strict-mutant.normalized"; then
    printf '%s\n' "gate contract: strict-mode removal mutation survived" >&2
    exit 1
fi

conditional_mutant=$(mktemp \
    "${live_root}/scripts/.gate-contract-conditional-mutant.XXXXXX")
mutant_runners+=("${conditional_mutant}")
write_replacement_mutant \
    "${gate_path}" "${conditional_mutant}" \
    $'rtk env CARGO_INCREMENTAL=0 cargo clippy \\\n    --manifest-path "${workspace_manifest}" \\\n    "${live_packages[@]}" \\\n    --all-targets \\\n    --all-features' \
    $'if false; then\n    rtk env CARGO_INCREMENTAL=0 cargo clippy \\\n        --manifest-path "${workspace_manifest}" \\\n        "${live_packages[@]}" \\\n        --all-targets \\\n        --all-features\nfi'
run_gate_probe \
    "${conditional_mutant}" 0 \
    "${probe_root}/conditional-mutant.trace" \
    "${probe_root}/conditional-mutant.output"
if gate_execution_trace_is_valid \
    0 \
    "${probe_root}/conditional-mutant.trace" \
    "${probe_root}/conditional-mutant.output" \
    "conditional-mutant"; then
    printf '%s\n' "gate contract: conditional command-skip mutation survived" >&2
    exit 1
fi

bfcache_mutant=$(mktemp \
    "${live_root}/scripts/.gate-contract-bfcache-mutant.XXXXXX")
mutant_runners+=("${bfcache_mutant}")
write_replacement_mutant \
    "${gate_path}" "${bfcache_mutant}" \
    "--project=chrome-bfcache" "--project=chromium"
run_gate_probe \
    "${bfcache_mutant}" 0 \
    "${probe_root}/bfcache-mutant.trace" \
    "${probe_root}/bfcache-mutant.output"
if gate_execution_trace_is_valid \
    0 \
    "${probe_root}/bfcache-mutant.trace" \
    "${probe_root}/bfcache-mutant.output" \
    "bfcache-mutant"; then
    printf '%s\n' "gate contract: BFCache project-substitution mutation survived" >&2
    exit 1
fi

printf '%s\n' "gate contract ok"
