#!/usr/bin/env bash
# Release-tagging script for Suprnova.
#
# Usage:
#   scripts/release.sh [--dry-run] <new-version>
#
# Example:
#   scripts/release.sh 0.1.0
#
# What it does (in order):
#   1. Refuses to run with a dirty working tree.
#   2. Runs the canonical full local gate, including cargo audit.
#   3. Bumps `workspace.package.version` and every versioned internal path
#      dependency requirement as one verified manifest operation.
#   4. Commits the bump.
#   5. Tags `v<new-version>`.
#   6. Pushes the commit and tag to `origin`.
#
# Under the current git-distribution model nothing is published to
# crates.io — the tag IS the release. See release-prep.md "Distribution
# model (corrected 2026-05-30)" and project_distribution_model.md.

set -euo pipefail

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=1
  shift
fi

if [ $# -ne 1 ]; then
  echo "usage: $0 [--dry-run] <new-version>" >&2
  echo "example: $0 0.1.0" >&2
  exit 64
fi

NEW_VERSION="$1"

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

if ! python3 scripts/bump-workspace-version.py --validate-only "$NEW_VERSION"; then
  exit 64
fi

CURRENT_VERSION="$(python3 - <<'PY'
import tomllib

with open("Cargo.toml", "rb") as handle:
    print(tomllib.load(handle)["workspace"]["package"]["version"])
PY
)"

if ! python3 - "$CURRENT_VERSION" "$NEW_VERSION" <<'PY'
import re
import sys

pattern = re.compile(
    r"^(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)\."
    r"(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


def parse(version):
    match = pattern.fullmatch(version)
    if match is None:
        raise ValueError(f"invalid workspace semantic version: {version}")
    core = tuple(int(part) for part in match.group(1, 2, 3))
    prerelease = match.group(4)
    return core, None if prerelease is None else prerelease.split(".")


def compare_identifiers(left, right):
    left_numeric = left.isdigit()
    right_numeric = right.isdigit()
    if left_numeric and right_numeric:
        return (int(left) > int(right)) - (int(left) < int(right))
    if left_numeric != right_numeric:
        return -1 if left_numeric else 1
    return (left > right) - (left < right)


def compare(left, right):
    left_core, left_pre = parse(left)
    right_core, right_pre = parse(right)
    if left_core != right_core:
        return (left_core > right_core) - (left_core < right_core)
    if left_pre is None or right_pre is None:
        if left_pre is None and right_pre is None:
            return 0
        return 1 if left_pre is None else -1
    for left_id, right_id in zip(left_pre, right_pre):
        compared = compare_identifiers(left_id, right_id)
        if compared:
            return compared
    return (len(left_pre) > len(right_pre)) - (len(left_pre) < len(right_pre))


current, proposed = sys.argv[1:]
raise SystemExit(0 if compare(proposed, current) > 0 else 1)
PY
then
  echo "error: release version $NEW_VERSION must be greater than current workspace version $CURRENT_VERSION" >&2
  exit 64
fi

if [[ $DRY_RUN -eq 1 ]]; then
  RELEASE_GATE="${SUPRNOVA_RELEASE_GATE:-scripts/gate.sh}"
  RELEASE_BUMP_SMOKE="${SUPRNOVA_RELEASE_BUMP_SMOKE:-scripts/tests/release-bump-smoke.sh}"

  echo "==> release dry-run: canonical full gate"
  "$RELEASE_GATE" --full
  echo "==> release dry-run: isolated version-bump smoke"
  "$RELEASE_BUMP_SMOKE" "$NEW_VERSION"
  echo
  echo "release dry-run passed; no manifests, commits, tags, or remotes were changed"
  exit 0
fi

# ---------- 1. Clean tree --------------------------------------------------

RELEASE_STATUS="$(git status --porcelain=v1 --untracked-files=all)"
if [[ -n "$RELEASE_STATUS" ]]; then
  echo "error: working tree is dirty — commit or stash first" >&2
  printf '%s\n' "$RELEASE_STATUS" >&2
  exit 1
fi

# Verify we're on `main` so the tag points where it should.
CURRENT_BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [ "$CURRENT_BRANCH" != "main" ]; then
  echo "error: release must be cut from main (currently on '$CURRENT_BRANCH')" >&2
  exit 1
fi

# Make sure the tag doesn't already exist.
if git rev-parse "v$NEW_VERSION" >/dev/null 2>&1; then
  echo "error: tag v$NEW_VERSION already exists" >&2
  exit 1
fi

# ---------- 2. Canonical full gate -----------------------------------------

scripts/gate.sh --full

# ---------- 3. Bump workspace version metadata -----------------------------

echo "==> bumping workspace version metadata to $NEW_VERSION"
mapfile -t BUMPED_MANIFESTS < <(
  python3 scripts/bump-workspace-version.py "$NEW_VERSION"
)

if [[ ${#BUMPED_MANIFESTS[@]} -eq 0 ]]; then
  echo "error: version helper did not report any changed manifests" >&2
  exit 1
fi

# Refresh Cargo.lock so the commit is self-contained.
echo "==> cargo check --workspace (refresh Cargo.lock for the bumped version)"
cargo check --workspace

# ---------- 4 + 5 + 6. Commit, tag, push -----------------------------------

echo "==> committing release: v$NEW_VERSION"
git add Cargo.lock "${BUMPED_MANIFESTS[@]}"
git commit -m "release: v$NEW_VERSION"

echo "==> tagging v$NEW_VERSION"
git tag -a "v$NEW_VERSION" -m "Suprnova v$NEW_VERSION"

echo "==> atomically pushing main + tag"
git push --atomic origin main "v$NEW_VERSION"

echo
echo "released v$NEW_VERSION"
echo "  commit: $(git rev-parse HEAD)"
echo "  tag:    v$NEW_VERSION"
echo
echo "next steps:"
echo "  - draft GitHub release notes from CHANGELOG.md section [$NEW_VERSION]"
echo "  - update manual/releases.md per its 'When v0.1.0 ships' plan"
