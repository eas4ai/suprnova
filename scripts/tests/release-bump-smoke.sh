#!/usr/bin/env bash

set -euo pipefail

NEW_VERSION="${1:-0.6.0}"
REPO_ROOT="$(git rev-parse --show-toplevel)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
export CARGO_TARGET_DIR="$TMP_DIR/target"

echo "==> copying the workspace into $TMP_DIR"
git -C "$REPO_ROOT" ls-files --cached --others --exclude-standard -z \
    | tar --null --files-from=- --create \
    | tar --extract --directory "$TMP_DIR"

echo "==> bumping the temporary workspace to $NEW_VERSION"
python3 "$TMP_DIR/scripts/bump-workspace-version.py" \
    --root "$TMP_DIR" "$NEW_VERSION"

while IFS= read -r manifest; do
    source_mode="$(stat --format='%a' "$REPO_ROOT/$manifest")"
    bumped_mode="$(stat --format='%a' "$TMP_DIR/$manifest")"
    if [[ "$source_mode" != "$bumped_mode" ]]; then
        echo "manifest mode changed for $manifest: $source_mode -> $bumped_mode" >&2
        exit 1
    fi
done < <(git -C "$REPO_ROOT" ls-files '*Cargo.toml')

echo "==> verifying workspace and internal path-dependency versions"
python3 "$TMP_DIR/scripts/bump-workspace-version.py" \
    --root "$TMP_DIR" --verify "$NEW_VERSION"

echo "==> cargo check --workspace in the temporary copy"
cargo check --manifest-path "$TMP_DIR/Cargo.toml" --workspace

echo "Release bump smoke passed for $NEW_VERSION."
