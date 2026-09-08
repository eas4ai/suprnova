#!/usr/bin/env bash
#
# The Live crate's own document and gate contracts, from the repository gate.
#
# `crates/suprnova-live` arrived with a spec set and a gate of its own, and
# those checks stayed reachable only through `crates/suprnova-live/scripts/
# gate.sh`. That gate compiles Rust, installs npm dependencies and drives three
# browser engines, so it belongs in the full tier - which means a stale Live
# specification, a missing implementation-document heading, or a Live gate
# script that no longer satisfies its own contract could sit on the branch
# through every default-tier run.
#
# These five checks are pure text: four read the tracked specification and
# implementation documents and the Live gate script, and the fifth asks Git
# whether the working tree introduces whitespace errors. They cost seconds,
# they need no toolchain beyond Node and Git, and they fail on exactly the
# kind of drift a documentation-shaped change introduces. Running them in the
# default tier keeps the Live subtree's documents honest between full runs.
#
# The two shell contracts (`documentation_contract.sh`, `gate_contract.sh`)
# resolve the Live root from their own location, so they are safe to invoke
# from the workspace root; the two Node checkers (`check-specs.mjs`,
# `check-implementation-docs.mjs`) take the workspace-relative path the Live
# conventions document specifies; and `git diff --check` reads the whole
# working tree from wherever it is run.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

node crates/suprnova-live/scripts/check-specs.mjs
node crates/suprnova-live/scripts/check-implementation-docs.mjs
crates/suprnova-live/tests/documentation_contract.sh
crates/suprnova-live/tests/gate_contract.sh
git diff --check

echo "live contracts: specifications, implementation documents, and the Live gate contract hold"
