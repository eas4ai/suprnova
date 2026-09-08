#!/usr/bin/env bash
#
# The whole Suprnova Live gate, as one full-tier repository step.
#
# The Live crate keeps its own gate because it verifies surfaces this
# repository's step table does not otherwise reach: the fixture corpora and
# the checker, the macro trybuild suite, the nightly fuzz build, the MSRV
# check over the compile fixtures, the correctness-delay scanners, the
# Playwright matrix across Chromium, Firefox and WebKit including a real
# BFCache lifecycle, and the browser compatibility evidence. Re-expressing
# those phases as repository steps would fork the contract; the Live
# `tests/gate_contract.sh` asserts the phases of that script, not of a copy.
#
# So the full tier calls the script itself. `SUPRNOVA_LIVE_RELEASE` stays
# whatever the caller set, defaulting to 0: the release value adds the
# qualified budgets and drops the unqualified compatibility allowance, and it
# is honest only on a dedicated runner. `CARGO_INCREMENTAL=0` matches every
# other Cargo invocation in this repository.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

export SUPRNOVA_LIVE_RELEASE="${SUPRNOVA_LIVE_RELEASE:-0}"
export CARGO_INCREMENTAL=0

exec crates/suprnova-live/scripts/gate.sh
