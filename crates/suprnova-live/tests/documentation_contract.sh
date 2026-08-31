#!/usr/bin/env bash
set -euo pipefail

live_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
workspace_root=$(git -C "${live_root}" rev-parse --show-toplevel)
case ${live_root} in
    "${workspace_root}"/*) ;;
    *)
        printf 'documentation contract: Live root is outside the Suprnova workspace (%s)\n' \
            "${live_root}" >&2
        exit 1
        ;;
esac

require_file() {
    local relative_path=$1
    if [[ ! -f ${live_root}/${relative_path} ]]; then
        printf 'documentation contract: missing %s\n' "${relative_path}" >&2
        exit 1
    fi
}

require_heading() {
    local relative_path=$1
    local heading=$2
    local contents
    contents=$(<"${live_root}/${relative_path}")
    if [[ ${contents} != *$'\n## '"${heading}"$'\n'* ]]; then
        printf 'documentation contract: %s is missing heading "## %s"\n' \
            "${relative_path}" "${heading}" >&2
        exit 1
    fi
}

require_text() {
    local relative_path=$1
    local description=$2
    local needle=$3
    local contents
    contents=$(<"${live_root}/${relative_path}")
    contents=${contents//$'\n'/ }
    if [[ ${contents} != *"${needle}"* ]]; then
        printf 'documentation contract: %s is missing %s (%s)\n' \
            "${relative_path}" "${description}" "${needle}" >&2
        exit 1
    fi
}

for required_file in \
    README.md \
    docs/implementation/component-authoring.md \
    docs/implementation/views-and-checker.md \
    docs/implementation/lifecycle-and-state.md \
    docs/implementation/actions-and-validation.md \
    docs/implementation/host-adapter-contract.md \
    docs/implementation/protocol-v2.md \
    docs/implementation/component-harness.md \
    docs/implementation/fixtures.md \
    docs/implementation/benchmarking.md \
    docs/implementation/threat-model-v1.md \
    docs/implementation/browser-runtime.md \
    docs/implementation/browser-assets.md \
    docs/implementation/live-directives.md \
    docs/implementation/local-reactivity.md \
    docs/implementation/scheduling-and-feedback.md \
    docs/implementation/morphing-and-continuity.md \
    docs/implementation/document-navigation.md \
    docs/implementation/browser-testing.md \
    docs/implementation/uploads.md \
    docs/implementation/async-updates.md \
    docs/implementation/iteration-004-operations.md
do
    require_file "${required_file}"
done

require_heading docs/implementation/component-authoring.md "Application-facing authoring"
require_heading docs/implementation/component-authoring.md "Generated metadata and registration"
require_heading docs/implementation/component-authoring.md "Internal standalone machinery"

require_heading docs/implementation/views-and-checker.md "Rendering contract"
require_heading docs/implementation/views-and-checker.md "Askama checker"
require_heading docs/implementation/views-and-checker.md "Trusted markup and escaping"
require_heading docs/implementation/views-and-checker.md "Failure and recovery"

require_heading docs/implementation/lifecycle-and-state.md "Lifecycle and mount authority"
require_heading docs/implementation/lifecycle-and-state.md "State categories"
require_heading docs/implementation/lifecycle-and-state.md "Model binding"
require_heading docs/implementation/lifecycle-and-state.md "Child composition"
require_heading docs/implementation/lifecycle-and-state.md "Failure and recovery"

require_heading docs/implementation/actions-and-validation.md "Actions and validation"
require_heading docs/implementation/actions-and-validation.md "Transactions and idempotency"
require_heading docs/implementation/actions-and-validation.md "Outcomes and recovery"

require_heading docs/implementation/host-adapter-contract.md "Trusted request context"
require_heading docs/implementation/host-adapter-contract.md "Host adapter contract"
require_heading docs/implementation/host-adapter-contract.md "Endpoint service"
require_heading docs/implementation/host-adapter-contract.md "Security boundary"
require_heading docs/implementation/host-adapter-contract.md "Failure mapping"

require_heading docs/implementation/protocol-v2.md "Protocol v2 request"
require_heading docs/implementation/protocol-v2.md "Child parameter envelope"
require_heading docs/implementation/protocol-v2.md "Response ordering"
require_heading docs/implementation/protocol-v2.md "Failure and recovery"

require_heading docs/implementation/component-harness.md "Browserless component harness"
require_heading docs/implementation/component-harness.md "Host controls and fault injection"
require_heading docs/implementation/component-harness.md "Assertions and redaction"

require_heading docs/implementation/fixtures.md "v1 and v2 conformance fixtures"
require_heading docs/implementation/fixtures.md "Parser, property, and fuzz regressions"

require_heading docs/implementation/benchmarking.md "Snapshot-processing benchmark"
require_heading docs/implementation/benchmarking.md "Action-framework benchmark"
require_heading docs/implementation/benchmarking.md "Macro expansion and compile budget"

require_heading docs/implementation/threat-model-v1.md "Host and endpoint threats"
require_heading docs/implementation/threat-model-v1.md "Component-kernel threats"

require_heading docs/implementation/browser-runtime.md "Boot and configuration"
require_heading docs/implementation/browser-runtime.md "Island lifecycle"
require_heading docs/implementation/browser-runtime.md "Extensions and diagnostics"

require_heading docs/implementation/browser-assets.md "Artifact contract"
require_heading docs/implementation/browser-assets.md "Serving and CSP"
require_heading docs/implementation/browser-assets.md "Dependency notices"

require_heading docs/implementation/live-directives.md "Closed directive grammar"
require_heading docs/implementation/live-directives.md "Models and server actions"
require_heading docs/implementation/live-directives.md "Effects and public calls"

require_heading docs/implementation/local-reactivity.md "Local signals"
require_heading docs/implementation/local-reactivity.md "Optional Stimulus"
require_heading docs/implementation/local-reactivity.md "Local and server boundaries"

require_heading docs/implementation/scheduling-and-feedback.md "Island scheduler"
require_heading docs/implementation/scheduling-and-feedback.md "Feedback and validation"
require_heading docs/implementation/scheduling-and-feedback.md "Failure and recovery"

require_heading docs/implementation/morphing-and-continuity.md "Morph preflight"
require_heading docs/implementation/morphing-and-continuity.md "Identity and controls"
require_heading docs/implementation/morphing-and-continuity.md "Focus, forms, and IME"

require_heading docs/implementation/document-navigation.md "Native navigation"
require_heading docs/implementation/document-navigation.md "Prefetch and View Transitions"
require_heading docs/implementation/document-navigation.md "Page lifecycle and bfcache"

require_heading docs/implementation/browser-testing.md "Test layers"
require_heading docs/implementation/browser-testing.md "Actual browser qualification"
require_heading docs/implementation/browser-testing.md "Budgets and diagnostics"

require_heading docs/implementation/uploads.md "Handle and grant"
require_heading docs/implementation/uploads.md "Provider modes"
require_heading docs/implementation/uploads.md "Quarantine and scanning"
require_heading docs/implementation/uploads.md "Finalization and compensation"
require_heading docs/implementation/uploads.md "Current-document resume"
require_heading docs/implementation/uploads.md "Cleanup"

require_heading docs/implementation/async-updates.md "Event schemas"
require_heading docs/implementation/async-updates.md "Subscription authorization"
require_heading docs/implementation/async-updates.md "Polling and push modes"
require_heading docs/implementation/async-updates.md "Continuity"
require_heading docs/implementation/async-updates.md "Degraded freshness"
require_heading docs/implementation/async-updates.md "Backpressure"

require_heading docs/implementation/iteration-004-operations.md "Artifacts"
require_heading docs/implementation/iteration-004-operations.md "Limits"
require_heading docs/implementation/iteration-004-operations.md "Observability"
require_heading docs/implementation/iteration-004-operations.md "Benchmarks"
require_heading docs/implementation/iteration-004-operations.md "Reference-host boundary"
require_heading docs/implementation/iteration-004-operations.md "Suprnova integration boundary"

require_text docs/implementation/component-authoring.md \
    "final application facade" 'suprnova::live'
require_text docs/implementation/component-authoring.md \
    "standalone integration disclaimer" 'does not claim registered Suprnova integration'
require_text docs/implementation/lifecycle-and-state.md \
    "mount authority operation" 'mount_instance'
require_text docs/implementation/protocol-v2.md \
    "signed child authority" 'child-params-v1'
require_text docs/implementation/actions-and-validation.md \
    "accepted-outcome guarantee" \
    'at most one accepted committed outcome per base revision'
require_text docs/implementation/actions-and-validation.md \
    "exactly-once limitation" \
    'not exactly-once method invocation or external effects'
require_text docs/implementation/host-adapter-contract.md \
    "endpoint-owned response authority" \
    'The endpoint owns status, headers, cache policy, cookies, and media type.'
require_text docs/implementation/browser-runtime.md \
    "standalone runtime disclaimer" 'standalone development machinery'
require_text docs/implementation/browser-assets.md \
    "immutable asset policy" 'public, max-age=31536000, immutable'
require_text docs/implementation/browser-assets.md \
    "module preload policy" 'modulepreload'
require_text docs/implementation/live-directives.md \
    "closed grammar authority" 'generated from the Rust directive catalog'
require_text docs/implementation/local-reactivity.md \
    "Stimulus and bridge exclusion" \
    "Neither Stimulus nor Suprnova's bridge/continuity implementation is bundled"
require_text docs/implementation/scheduling-and-feedback.md \
    "one scheduler invariant" 'one bounded scheduler per island'
require_text docs/implementation/morphing-and-continuity.md \
    "private morph implementation" 'Idiomorph 0.7.4'
require_text docs/implementation/document-navigation.md \
    "real-route invariant" 'real HTTP navigation'
require_text docs/implementation/browser-testing.md \
    "qualification distinction" 'Playwright is not actual-browser floor evidence'
require_text THIRD_PARTY_LICENSES.md \
    "npm usage classification" '| Usage |'
require_text THIRD_PARTY_LICENSES.md \
    "Idiomorph production dependency" '| npm | idiomorph | 0.7.4 | Production runtime |'
require_text THIRD_PARTY_LICENSES.md \
    "Terser build dependency" '| npm | terser | 5.50.0 | Production build |'
require_text THIRD_PARTY_LICENSES.md \
    "Playwright test dependency" '| npm | @playwright/test | 1.62.1 | Test only |'

# The shared data-driven semantic contract also runs in this shell gate. Its
# in-memory mutation cases prove that one critical inversion in each Iteration
# 004 guide and one broken README link are detected.
rtk node "${live_root}/scripts/check-implementation-docs.mjs" --semantic-only

printf '%s\n' "documentation contract ok"
