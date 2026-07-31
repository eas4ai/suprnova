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
#
# **Every pin syntax, not just the dependency one.** `cargo install --tag
# vX.Y.Z` is not a stylistic variant of `tag = "vX.Y.Z"` — it is a second
# form, and checking only the first repeated the very failure this block
# was added to catch, one level down. `manual/installation.md` carried both
# and the release bumped only the dependency snippet; `manual/cli.md`,
# `manual/cli-new.md` and `suprnova-cli/README.md` carry *only* the install
# form, so discovery never picked them up at all and the CLI's own README
# sat three releases stale. Scanning for one shape while the rewrite fixes
# another is how a file passes the bump, passes `--verify`, and ships
# wrong — so this list and TAG_PIN_PATTERNS must gain a form together.
SEMVER_RE='[0-9]+\.[0-9]+\.[0-9]+'
# Dots are literal here: an unescaped `v0.6.0` would also match `v0X6Y0`.
NEW_VERSION_RE="${NEW_VERSION//./\\.}"
# Three spellings, matching TAG_PIN_PATTERNS in bump-workspace-version.py:
# the dependency snippet, the install command, and a documented
# `suprnova --version` output line. The last is anchored to a whole line so
# it matches example output and not prose like "Suprnova 0.7.2 introduced",
# which is a historical statement a rewrite would falsify.
ANY_VERSION_PIN="tag = \"v${SEMVER_RE}|--tag v${SEMVER_RE}|^# suprnova ${SEMVER_RE}\$"
# The install form ends at a word boundary rather than a quote, so bound it
# explicitly or `--tag v0.6.0` would also accept `--tag v0.6.01`.
CURRENT_VERSION_PIN="tag = \"v${NEW_VERSION_RE}\"|--tag v${NEW_VERSION_RE}([^0-9.]|\$)|^# suprnova ${NEW_VERSION_RE}\$"

echo "==> asserting no shipped file still pins an older tag (all pin syntaxes)"
stale="$(
    grep -rIl --exclude-dir=target --exclude-dir=node_modules \
        --exclude-dir=reference --exclude-dir=templates \
        --exclude-dir=.git --exclude-dir=docs --exclude-dir=.superpowers \
        -E "$ANY_VERSION_PIN" \
        --include='*.md' --include='*.rs' "$TMP_DIR" 2>/dev/null \
    | while IFS= read -r file; do
        relative="${file#"$TMP_DIR"/}"
        # Kept in step with TAG_PINS_FROZEN in bump-workspace-version.py: a
        # parser fixture pinning a historical manifest, and the changelog,
        # whose every tag names a past release *because* it is past.
        [[ "$relative" == "suprnova-cli/src/commands/cargo_meta.rs" ]] && continue
        [[ "$relative" == "CHANGELOG.md" ]] && continue
        if grep -vE "$CURRENT_VERSION_PIN" "$file" | grep -qE "$ANY_VERSION_PIN"; then
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
