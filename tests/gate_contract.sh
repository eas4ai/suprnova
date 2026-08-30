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

budget_trace_is_valid() {
    local release_mode=$1
    local trace_path=$2
    local reduced_upload=0
    local reduced_async=0
    local qualified_upload=0
    local qualified_async=0
    local invalid=0
    local inherited_profile
    local profile
    local runner
    local runner_index
    local first
    local first_base
    local token
    local token_base
    local executable
    local index
    local -a fields

    while IFS=$'\t' read -r -a fields; do
        if (( ${#fields[@]} < 2 )); then
            continue
        fi

        inherited_profile=${fields[0]#profile=}
        profile=${inherited_profile}
        runner=
        runner_index=-1
        for ((index = 1; index < ${#fields[@]}; index += 1)); do
            token=${fields[index]}
            token_base=${token##*/}
            case ${token_base} in
                run-upload-budget.sh)
                    runner=upload
                    runner_index=${index}
                    ;;
                run-async-budget.sh)
                    runner=async
                    runner_index=${index}
                    ;;
            esac
        done

        if (( runner_index < 0 )); then
            continue
        fi

        profile=${inherited_profile}
        for ((index = 1; index < runner_index; index += 1)); do
            token=${fields[index]}
            if [[ ${token} == SUPRNOVA_LIVE_BUDGET_PROFILE=* ]]; then
                profile=${token#*=}
            fi
        done

        executable=0
        first=${fields[1]}
        first_base=${first##*/}
        if (( runner_index == 1 )); then
            executable=1
        elif [[ ${first_base} == env ]]; then
            executable=1
            for ((index = 2; index < runner_index; index += 1)); do
                token=${fields[index]}
                token_base=${token##*/}
                if [[ ${token} == *=* || ${token} == -- || ${token} == -* ]]; then
                    continue
                fi
                if [[ (${token_base} == bash || ${token_base} == sh) &&
                      ${index} == $((runner_index - 1)) ]]; then
                    continue
                fi
                executable=0
                break
            done
        elif [[ (${first_base} == bash || ${first_base} == sh || ${first_base} == proxy) &&
                ${runner_index} == 2 ]]; then
            executable=1
        fi

        if (( executable == 0 )); then
            invalid=$((invalid + 1))
            continue
        fi

        case ${runner}:${profile} in
            upload:reduced) reduced_upload=$((reduced_upload + 1)) ;;
            async:reduced) reduced_async=$((reduced_async + 1)) ;;
            upload:qualified) qualified_upload=$((qualified_upload + 1)) ;;
            async:qualified) qualified_async=$((qualified_async + 1)) ;;
            *) invalid=$((invalid + 1)) ;;
        esac
    done <"${trace_path}"

    if [[ ${release_mode} == 0 ]]; then
        (( invalid == 0 && reduced_upload == 1 && reduced_async == 1 &&
            qualified_upload == 0 && qualified_async == 0 ))
    else
        (( invalid == 0 && reduced_upload == 1 && reduced_async == 1 &&
            qualified_upload == 1 && qualified_async == 1 ))
    fi
}

require_budget_trace_contract() {
    local description=$1
    local release_mode=$2
    local trace_path=$3
    if ! budget_trace_is_valid "${release_mode}" "${trace_path}"; then
        printf 'gate contract: %s has invalid executable budget invocations\n' \
            "${description}" >&2
        exit 1
    fi
}

qualified_runner_fails_closed() {
    local runner_path=$1
    local trace_path=$2
    local output_path=$3
    local status

    : >"${trace_path}"
    if env \
        -u SUPRNOVA_LIVE_S1_DEDICATED \
        -u SUPRNOVA_LIVE_B1_DEDICATED \
        PATH="${probe_root}:${PATH}" \
        SUPRNOVA_LIVE_GATE_TRACE="${trace_path}" \
        SUPRNOVA_LIVE_BUDGET_PROFILE=qualified \
        bash "${runner_path}" >"${output_path}" 2>&1; then
        status=0
    else
        status=$?
    fi

    (( status != 0 )) && [[ ! -s ${trace_path} ]]
}

write_exit_bypass_mutant() {
    local source_path=$1
    local destination_path=$2
    local source
    local mutated

    source=$(<"${source_path}")
    mutated=${source/"exit 1"/":"}
    if [[ ${mutated} == "${source}" ]]; then
        printf 'gate contract: could not create exit-bypass mutation for %s\n' \
            "${source_path}" >&2
        exit 1
    fi
    printf '%s\n' "${mutated}" >"${destination_path}"
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
            field=${fields[index]//${repository_root}/<repo>}
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
        $'node\tscripts/generate-license-inventory.mjs\t--check' \
        $'cargo\tfmt\t--all\t--\t--check' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--workspace\t--all-targets\t--all-features' \
        $'node\ttests/correctness_delay_clippy.mjs' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--workspace\t--all-targets\t--all-features\t--\t-D\tclippy::disallowed_methods' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--manifest-path\tfuzz/Cargo.toml\t--all-targets\t--\t-D\tclippy::disallowed_methods' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--test\tgolden_fixtures\t--test\tbrowser_contract_properties' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--test\tchecker_positive\t--test\tchecker_negative\t--test\tchecker_regressions' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--test\tcompatibility\t--test\tprotocol_v2' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--test\tsecurity_boundaries' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--test\tsecurity_hostile_context' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t-p\tsuprnova-live-macros\t--test\tui' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--test\titeration_004_conformance\t--test\titeration_004_adversarial\t--test\titeration_004_exhaustion' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t-p\tsuprnova-live-test-support\t--test\treference_host\t--\t--test-threads=1' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--workspace\t--all-targets\t--all-features\t--no-fail-fast' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\ttest\t--workspace\t--doc\t--all-features' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\t+1.91.1\tcheck\t--workspace\t--all-targets\t--all-features' \
        $'env\tCARGO_INCREMENTAL=0\tcargo\t+1.91.1\tcheck\t--manifest-path\ttests/fixtures/compile/Cargo.toml\t--workspace\t--all-targets' \
        $'cargo\t+nightly\tfuzz\tbuild' \
        $'npm\tci' \
        $'node\ttests/correctness_delay_scanner.mjs' \
        $'node\tscripts/check-correctness-delays.mjs' \
        $'npm\trun\tgenerate:check' \
        $'npm\trun\tformat:check' \
        $'npm\trun\tlint' \
        $'npm\trun\ttypecheck' \
        $'npm\trun\ttest:unit\t--\ttests/golden-fixtures.test.ts\ttests/upload-protocol.test.ts\ttests/upload-manager.test.ts\ttests/async-envelope.test.ts\ttests/async-feature.test.ts\ttests/async-dispatch.test.ts\ttests/bounded-resources.test.ts' \
        $'npm\trun\ttest:unit\t--\ttests/feature-host.test.ts\ttests/document-lifecycle.test.ts\ttests/optional-artifacts.test.ts\ttests/build-contract.test.ts\ttests/budget-contract.test.ts' \
        $'npm\trun\ttest:unit' \
        $'npm\trun\tbuild' \
        $'npm\trun\tbuild:check' \
        $'npm\trun\ttest:browser\t--\te2e/iteration-004-integration.spec.ts\te2e/iteration-004-adversarial.spec.ts\te2e/iteration-004-lifecycle.spec.ts\te2e/iteration-004-accessibility.spec.ts\t--project=chromium\t--project=firefox\t--project=webkit' \
        $'npm\trun\ttest:browser\t--\te2e/csp.spec.ts\t--project=chromium' \
        $'npm\trun\ttest:browser\t--\te2e/async-lifecycle.spec.ts\te2e/iteration-004-lifecycle.spec.ts\t--project=chrome-bfcache' \
        $'npm\trun\ttest:browser\t--\t--project=chromium\t--project=firefox\t--project=webkit' \
        >"${expected_path}"

    if [[ ${release_mode} == 1 ]]; then
        printf '%s\n' \
            $'npm\trun\tcompatibility:check' \
            $'npm\trun\tbudget:browser\t--\t--release\t--dedicated' \
            $'npm\trun\tbudget\t--\t--release' \
            >>"${expected_path}"
    else
        printf '%s\n' \
            $'npm\trun\tcompatibility:check\t--\t--allow-unqualified' \
            $'npm\trun\tbudget' \
            >>"${expected_path}"
    fi

    printf '%s\n' \
        $'env\tCARGO_INCREMENTAL=0\tSUPRNOVA_LIVE_BENCH_RESULT=<repo>/benchmarks/local/gate-snapshot-budget-v1.json\tscripts/run-snapshot-budget.sh' \
        $'env\tCARGO_INCREMENTAL=0\tSUPRNOVA_LIVE_BENCH_RESULT=<repo>/benchmarks/local/gate-action-budget-v1.json\tscripts/run-action-budget.sh' \
        $'env\tSUPRNOVA_LIVE_BUDGET_PROFILE=reduced\tscripts/run-upload-budget.sh' \
        $'env\tSUPRNOVA_LIVE_BUDGET_PROFILE=reduced\tscripts/run-async-budget.sh' \
        >>"${expected_path}"

    if [[ ${release_mode} == 1 ]]; then
        printf '%s\n' \
            $'env\tSUPRNOVA_LIVE_BUDGET_PROFILE=qualified\tscripts/run-upload-budget.sh' \
            $'env\tSUPRNOVA_LIVE_BUDGET_PROFILE=qualified\tscripts/run-async-budget.sh' \
            >>"${expected_path}"
    fi

    printf '%s\n' \
        $'node\tscripts/check-expansion-budget.mjs' \
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
        "iteration 004 reference host" \
        "Rust all-target and documentation tests" \
        "Rust MSRV" \
        "nightly fuzz build" \
        "browser dependency and conformance gates" \
        "correctness-delay scanner" \
        "iteration 004 browser unit boundaries" \
        "browser broad unit suite" \
        "iteration 004 browser matrix" \
        "real BFCache browser lifecycle" \
        "browser broad matrix" \
        "A8/16 snapshot budget" \
        "A8/16 action framework budget" \
        "iteration 004 reduced deterministic budgets" \
        "U4/16 upload framework and browser budget" \
        "E100/1K and R100 async continuity budgets" \
        >"${expected_path}"

    if [[ ${release_mode} == 1 ]]; then
        printf '%s\n' \
            "U4/16 qualified upload budget" \
            "E100/1K and R100 qualified async budgets" \
            >>"${expected_path}"
    fi

    printf '%s\n' \
        "macro expansion and isolated compile budget" \
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
    local expected_last=$'env\tCARGO_INCREMENTAL=0\tcargo\tclippy\t--workspace\t--all-targets\t--all-features'
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
require_text "correctness-delay scanner phase" 'phase "correctness-delay scanner"'
require_text "correctness-delay scanner self-tests" \
    "node tests/correctness_delay_scanner.mjs"
require_text "compiler-resolved correctness-delay self-test" \
    "node tests/correctness_delay_clippy.mjs"
require_text "compiler-resolved correctness-delay lint denial" \
    "-D clippy::disallowed_methods"
require_text "compiler-resolved fuzz correctness-delay lint" \
    "--manifest-path fuzz/Cargo.toml"
require_text "correctness-delay repository scan" \
    "node scripts/check-correctness-delays.mjs"
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
require_text "real BFCache browser lifecycle phase" 'phase "real BFCache browser lifecycle"'
require_text "iteration 004 browser unit phase" 'phase "iteration 004 browser unit boundaries"'
require_text "broad browser unit phase" 'phase "browser broad unit suite"'
require_text "broad browser matrix phase" 'phase "browser broad matrix"'
require_text "real BFCache Chromium project" "--project=chrome-bfcache"
require_text "iteration 004 browser integration matrix" "e2e/iteration-004-integration.spec.ts"
require_text "iteration 004 browser adversarial matrix" "e2e/iteration-004-adversarial.spec.ts"
require_text "iteration 004 browser lifecycle matrix" "e2e/iteration-004-lifecycle.spec.ts"
require_text "iteration 004 browser accessibility matrix" "e2e/iteration-004-accessibility.spec.ts"
require_text "qualified U4/16 release phase" 'phase "U4/16 qualified upload budget"'
require_text "qualified E100/1K and R100 release phase" \
    'phase "E100/1K and R100 qualified async budgets"'
require_text "final worktree diff phase" 'phase "final worktree diff check"'

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
require_order "deterministic browser build" "npm run build:check" \
    "reduced deterministic budgets" 'phase "iteration 004 reduced deterministic budgets"'

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

require_budget_trace_contract "ordinary gate" 0 "${ordinary_trace}"
require_budget_trace_contract "release gate" 1 "${release_trace}"
require_gate_execution_trace \
    "ordinary gate" 0 "${ordinary_trace}" "${ordinary_output}" "ordinary"
require_gate_execution_trace \
    "release gate" 1 "${release_trace}" "${release_output}" "release"

if ! gate_stops_at_clippy_failure \
    "${gate_path}" \
    "${probe_root}/strict-original.trace" \
    "${probe_root}/strict-original.output" \
    "${probe_root}/strict-original.normalized"; then
    printf '%s\n' "gate contract: command failure did not stop the real gate" >&2
    exit 1
fi

strict_mutant=$(mktemp "${repository_root}/scripts/.gate-contract-strict-mutant.XXXXXX")
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
    "${repository_root}/scripts/.gate-contract-conditional-mutant.XXXXXX")
mutant_runners+=("${conditional_mutant}")
write_replacement_mutant \
    "${gate_path}" "${conditional_mutant}" \
    "rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features" \
    $'if false; then\n    rtk env CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --all-features\nfi'
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
    "${repository_root}/scripts/.gate-contract-bfcache-mutant.XXXXXX")
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

alternate_path_ordinary_trace=${probe_root}/alternate-path-ordinary.trace
printf '%s\n' \
    $'profile=\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=reduced\t./scripts/run-upload-budget.sh' \
    $'profile=\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=reduced\t../suprnova-live/scripts/run-async-budget.sh' \
    $'profile=\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=qualified\t./scripts/run-upload-budget.sh' \
    >"${alternate_path_ordinary_trace}"
if budget_trace_is_valid 0 "${alternate_path_ordinary_trace}"; then
    printf '%s\n' \
        "gate contract: alternate-path qualified invocation survived ordinary mode" >&2
    exit 1
fi

alternate_path_release_trace=${probe_root}/alternate-path-release.trace
printf '%s\n' \
    $'profile=\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=reduced\t./scripts/run-upload-budget.sh' \
    $'profile=\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=reduced\t../suprnova-live/scripts/run-async-budget.sh' \
    "profile="$'\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=qualified\t'"${repository_root}/scripts/run-upload-budget.sh" \
    "profile="$'\tenv\tSUPRNOVA_LIVE_BUDGET_PROFILE=qualified\t'"${repository_root}/scripts/run-async-budget.sh" \
    >"${alternate_path_release_trace}"
if ! budget_trace_is_valid 1 "${alternate_path_release_trace}"; then
    printf '%s\n' "gate contract: harmless alternate runner paths were not normalized" >&2
    exit 1
fi

upload_fail_trace=${probe_root}/upload-qualified-fail.trace
upload_fail_output=${probe_root}/upload-qualified-fail.output
async_fail_trace=${probe_root}/async-qualified-fail.trace
async_fail_output=${probe_root}/async-qualified-fail.output
if ! qualified_runner_fails_closed \
    "${upload_runner_path}" "${upload_fail_trace}" "${upload_fail_output}"; then
    printf '%s\n' "gate contract: qualified upload runner did not fail before workload" >&2
    exit 1
fi
if ! qualified_runner_fails_closed \
    "${async_runner_path}" "${async_fail_trace}" "${async_fail_output}"; then
    printf '%s\n' "gate contract: qualified async runner did not fail before workload" >&2
    exit 1
fi

upload_mutant=$(mktemp "${repository_root}/scripts/.gate-contract-upload-mutant.XXXXXX")
mutant_runners+=("${upload_mutant}")
async_mutant=$(mktemp "${repository_root}/scripts/.gate-contract-async-mutant.XXXXXX")
mutant_runners+=("${async_mutant}")
write_exit_bypass_mutant "${upload_runner_path}" "${upload_mutant}"
write_exit_bypass_mutant "${async_runner_path}" "${async_mutant}"
if qualified_runner_fails_closed \
    "${upload_mutant}" "${probe_root}/upload-mutant.trace" "${probe_root}/upload-mutant.output"; then
    printf '%s\n' "gate contract: upload qualification exit-bypass mutation survived" >&2
    exit 1
fi
if qualified_runner_fails_closed \
    "${async_mutant}" "${probe_root}/async-mutant.trace" "${probe_root}/async-mutant.output"; then
    printf '%s\n' "gate contract: async qualification exit-bypass mutation survived" >&2
    exit 1
fi

ordinary_phase_output=$(<"${ordinary_output}")
release_phase_output=$(<"${release_output}")
if [[ ${ordinary_phase_output} == *"U4/16 qualified upload budget"* ||
      ${ordinary_phase_output} == *"E100/1K and R100 qualified async budgets"* ]]; then
    printf '%s\n' "gate contract: ordinary mode executed a qualified phase" >&2
    exit 1
fi
if [[ ${release_phase_output} != *"U4/16 qualified upload budget"* ||
      ${release_phase_output} != *"E100/1K and R100 qualified async budgets"* ]]; then
    printf '%s\n' "gate contract: release mode omitted a qualified phase" >&2
    exit 1
fi

printf '%s\n' "gate contract ok"
