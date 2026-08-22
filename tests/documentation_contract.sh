#!/usr/bin/env bash
set -euo pipefail

repository_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

require_file() {
    local relative_path=$1
    if [[ ! -f ${repository_root}/${relative_path} ]]; then
        printf 'documentation contract: missing %s\n' "${relative_path}" >&2
        exit 1
    fi
}

require_heading() {
    local relative_path=$1
    local heading=$2
    local contents
    contents=$(<"${repository_root}/${relative_path}")
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
    contents=$(<"${repository_root}/${relative_path}")
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
    docs/implementation/threat-model-v1.md
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

printf '%s\n' "documentation contract ok"
