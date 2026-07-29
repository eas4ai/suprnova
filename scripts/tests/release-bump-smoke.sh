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

# Coverage, not just execution. `--verify` asks the bumper whether it is
# happy, using the same discovery the bump used — so a file class the
# discovery does not scan passes both steps while staying stale. That is
# exactly how `manual/*.md` and a public doc comment in
# `broadcasting/fanout/mod.rs` sat at older versions through several
# releases: the bump ran, verify agreed, and neither looked at them.
#
# So grep the bumped tree independently: no shipped file may still pin a
# version other than the one just released.
echo "==> asserting no shipped file still pins an older tag"
stale="$(
    grep -rIl --exclude-dir=target --exclude-dir=node_modules \
        --exclude-dir=reference --exclude-dir=templates \
        --exclude-dir=.git --exclude-dir=docs --exclude-dir=.superpowers \
        -E 'tag = "v[0-9]+\.[0-9]+\.[0-9]+' \
        --include='*.md' --include='*.rs' "$TMP_DIR" 2>/dev/null \
    | while IFS= read -r file; do
        relative="${file#"$TMP_DIR"/}"
        # A parser fixture pins a historical manifest on purpose.
        [[ "$relative" == "suprnova-cli/src/commands/cargo_meta.rs" ]] && continue
        if grep -qE "tag = \"v[0-9]+\.[0-9]+\.[0-9]+" "$file" \
            && grep -vE "tag = \"v${NEW_VERSION}\"" "$file" \
                | grep -qE 'tag = "v[0-9]+\.[0-9]+\.[0-9]+'; then
            echo "$relative"
        fi
    done
)"
if [[ -n "$stale" ]]; then
    echo "these files still pin a version other than $NEW_VERSION after the bump:" >&2
    echo "$stale" >&2
    exit 1
fi

echo "==> cargo check --workspace in the temporary copy"
cargo check --manifest-path "$TMP_DIR/Cargo.toml" --workspace

echo "Release bump smoke passed for $NEW_VERSION."
