#!/usr/bin/env bash
#
# The Live browser runtime: generated contracts, lint, units, and artifacts.
#
# `crates/suprnova-live/browser` is a second source tree in this repository -
# strict TypeScript, its own lockfile, and a `dist/` that is tracked. Two of
# its contracts are cross-language: `generate:check` proves the generated
# directive contract in Rust and in TypeScript still matches the fixture
# corpus that produced them, and `build:check` plus the tracked-artifact diff
# prove the checked-in bundles are exactly what the pinned lockfile and the
# current source rebuild. Either can break from a change made entirely on the
# Rust side, and neither is visible to `cargo`.
#
# This is the browser work that needs no browser: no Playwright engines, no
# reference hosts, no dedicated runner. `compatibility:check` therefore runs
# with `--allow-unqualified`, which reports the honest local state instead of
# demanding release-qualified evidence; the release path drops that flag and
# runs inside the Live gate.
#
# `npm ci` rather than `npm install`: the lockfile is ground truth and the
# artifact-parity check below is only meaningful against pinned dependencies.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
cd crates/suprnova-live/browser

npm ci
npm run generate:check
npm run format:check
npm run lint
npm run typecheck
npm run test:unit
npm run build
npm run build:check
npm run compatibility:check -- --allow-unqualified

git diff --exit-code --stat -- dist

echo "live browser: generated contracts, lint, unit suites, and tracked artifacts hold"
