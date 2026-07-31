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
# crates.io — the tag IS the release. See README.md → "Distribution
# model".

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

# ---------- 0. GitHub release preflight ------------------------------------
#
# Resolved before the gate so a missing `gh` fails in seconds rather than after
# a full matrix run. Getting this wrong late is how v0.5.10 and v0.6.1..v0.6.3
# ended up tag-only: the tag was pushed, the Release was a manual "next step",
# and the Releases page sat on a stale version while the tags were correct.
#
# Publishing is skipped automatically unless origin is GitHub. That is what
# keeps scripts/tests/release-normal-smoke.sh — which runs this script against
# a disposable bare origin — from ever publishing a fake version.

# Prints the CHANGELOG body for a version, header excluded, leading blanks
# trimmed. Stops at the next `## <version>` heading.
extract_changelog_section() {
  awk -v want="$1" '
    $1 == "##" { if (found) exit; found = ($2 == want); next }
    found { print }
  ' "$REPO_ROOT/CHANGELOG.md" | sed '/./,$!d'
}

PUBLISH_GITHUB_RELEASE=1
GITHUB_REMOTE_RE='(^git@github\.com:|^(https|ssh)://([^@/]+@)?github\.com/)'
if [[ "${SUPRNOVA_SKIP_GITHUB_RELEASE:-0}" == "1" ]]; then
  PUBLISH_GITHUB_RELEASE=0
  echo "==> GitHub release step disabled via SUPRNOVA_SKIP_GITHUB_RELEASE"
elif ! git remote get-url origin 2>/dev/null | grep -qE "$GITHUB_REMOTE_RE"; then
  PUBLISH_GITHUB_RELEASE=0
  echo "==> origin is not a GitHub remote; skipping the GitHub release step"
fi

RELEASE_NOTES_FILE=""
if [[ $PUBLISH_GITHUB_RELEASE -eq 1 ]]; then
  if ! command -v gh >/dev/null 2>&1; then
    echo "error: gh is required to publish the GitHub release for v$NEW_VERSION" >&2
    echo "       install it (https://cli.github.com), or re-run with" >&2
    echo "       SUPRNOVA_SKIP_GITHUB_RELEASE=1 to tag without publishing" >&2
    exit 1
  fi
  if ! gh auth status >/dev/null 2>&1; then
    echo "error: gh is installed but not authenticated — run 'gh auth login'" >&2
    exit 1
  fi

  RELEASE_NOTES_FILE="$(mktemp)"
  trap 'rm -f "$RELEASE_NOTES_FILE"' EXIT
  extract_changelog_section "$NEW_VERSION" >"$RELEASE_NOTES_FILE"
  if [[ ! -s "$RELEASE_NOTES_FILE" ]]; then
    echo "error: CHANGELOG.md has no section for $NEW_VERSION" >&2
    echo "       add a '## $NEW_VERSION — <date>' section before releasing;" >&2
    echo "       its body becomes the GitHub release notes" >&2
    exit 1
  fi
  echo "==> GitHub release notes: $(wc -l <"$RELEASE_NOTES_FILE") lines from CHANGELOG.md"
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

# ---------- 7. Publish the GitHub release ----------------------------------
#
# The tag is the release for consumers (`tag = "vX.Y.Z"` resolves the moment
# the push lands), so a failure here does not break downstream — but it does
# leave the Releases page claiming an older version is Latest. Report it as a
# failure with the exact retry, rather than exiting 0 on a half-done release.

RELEASE_PUBLISHED=0
if [[ $PUBLISH_GITHUB_RELEASE -eq 1 ]]; then
  if gh release view "v$NEW_VERSION" >/dev/null 2>&1; then
    echo "==> GitHub release v$NEW_VERSION already exists; leaving it untouched"
    RELEASE_PUBLISHED=1
  else
    echo "==> publishing GitHub release v$NEW_VERSION"
    if gh release create "v$NEW_VERSION" \
      --title "Suprnova v$NEW_VERSION" \
      --notes-file "$RELEASE_NOTES_FILE" \
      --latest \
      --verify-tag; then
      RELEASE_PUBLISHED=1
    fi
  fi
fi

echo
echo "released v$NEW_VERSION"
echo "  commit: $(git rev-parse HEAD)"
echo "  tag:    v$NEW_VERSION"
if [[ $PUBLISH_GITHUB_RELEASE -eq 1 && $RELEASE_PUBLISHED -eq 1 ]]; then
  echo "  release: $(gh release view "v$NEW_VERSION" --json url -q .url 2>/dev/null || echo published)"
fi

if [[ $PUBLISH_GITHUB_RELEASE -eq 1 && $RELEASE_PUBLISHED -eq 0 ]]; then
  echo
  echo "error: commit and tag are pushed, but the GitHub release was NOT created" >&2
  echo "       downstream consumers are unaffected — the tag is what they resolve" >&2
  echo "       retry with:" >&2
  echo "         awk -v want=$NEW_VERSION '\$1==\"##\"{if(f)exit;f=(\$2==want);next} f' CHANGELOG.md \\" >&2
  echo "           | gh release create v$NEW_VERSION --title \"Suprnova v$NEW_VERSION\" \\" >&2
  echo "               --notes-file - --latest --verify-tag" >&2
  exit 1
fi
