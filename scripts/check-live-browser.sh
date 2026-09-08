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

# `git diff --exit-code` compares the worktree to the index, so it stays silent
# on a drift that was already staged, and it cannot see a bundle filename the
# build has just invented. Porcelain status reports staged and unstaged entries
# alike, which closes the first hole. `--ignored=matching` closes the second:
# `.gitignore` carries a `**/dist/` rule and the shipped bundles are tracked
# only because they were force-added, so a genuinely new file under `dist/` is
# ignored rather than untracked and would otherwise be invisible here. It
# surfaces as a `!!` entry, and an unexplained artifact is exactly the drift
# this check exists to refuse.
artifacts=$(git status --porcelain --ignored=matching -- dist)
if [[ -n ${artifacts} ]]; then
    printf '%s\n' "${artifacts}" >&2
    git diff --stat -- dist >&2
    printf '%s\n' "" >&2
    printf '%s\n' "tracked browser artifacts drifted from the rebuild above." >&2
    printf '%s\n' "Commit the rebuilt dist/ or fix the source that changed it." >&2
    exit 1
fi

echo "live browser: generated contracts, lint, unit suites, and tracked artifacts hold"
