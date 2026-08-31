#!/usr/bin/env bash

set -euo pipefail

NEW_VERSION="${1:-0.6.0}"
TOOLING_ROOT="$(git rev-parse --show-toplevel)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
export CARGO_TARGET_DIR="$TMP_DIR/target"

BIN_DIR="$TMP_DIR/bin"
mkdir -p "$BIN_DIR"
export PATH="$BIN_DIR:$PATH"

# Exercise the bumper without compiling the disposable workspace. This shim
# implements only the cargo metadata contract the helper reads and the final
# cargo-check invocation this smoke records.
cat >"$BIN_DIR/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
    metadata)
        manifest_path=""
        while [[ $# -gt 0 ]]; do
            if [[ "$1" == "--manifest-path" ]]; then
                manifest_path=$2
                break
            fi
            shift
        done
        [[ -n "$manifest_path" ]]
        python3 - "$manifest_path" <<'PY'
import json
from pathlib import Path
import sys
import tomllib

root_manifest = Path(sys.argv[1]).resolve()
root = root_manifest.parent
workspace = tomllib.loads(root_manifest.read_text())["workspace"]
workspace_version = workspace["package"]["version"]
packages = []
for member in workspace["members"]:
    manifest = root / member / "Cargo.toml"
    document = tomllib.loads(manifest.read_text())
    package = document["package"]
    version = package["version"]
    if isinstance(version, dict) and version.get("workspace") is True:
        version = workspace_version
    packages.append(
        {
            "name": package["name"],
            "version": version,
            "manifest_path": str(manifest.resolve()),
        }
    )
print(json.dumps({"packages": packages}))
PY
        ;;
    check)
        [[ "$*" == check\ --manifest-path\ *\ --workspace ]]
        ;;
    *)
        echo "unexpected cargo invocation in release bump smoke: $*" >&2
        exit 97
        ;;
esac
EOF
chmod +x "$BIN_DIR/cargo"

find_main_worktree() {
    local candidate=""
    local line

    while IFS= read -r line; do
        case "$line" in
            "worktree "*)
                candidate="${line#worktree }"
                ;;
            "branch refs/heads/main")
                printf '%s\n' "$candidate"
                return 0
                ;;
        esac
    done < <(git -C "$TOOLING_ROOT" worktree list --porcelain)
    return 1
}

PUBLIC_ROOT="${SUPRNOVA_RELEASE_PUBLIC_ROOT:-}"
if [[ -z "$PUBLIC_ROOT" ]]; then
    PUBLIC_ROOT="$(find_main_worktree)"
fi
if [[ -z "$PUBLIC_ROOT" || ! -d "$PUBLIC_ROOT/.git" && ! -f "$PUBLIC_ROOT/.git" ]]; then
    echo "release bump smoke could not locate the public main worktree" >&2
    exit 2
fi

assert_tagged_release_wording_contract() {
    python3 - "$TOOLING_ROOT/scripts/bump-workspace-version.py" <<'PY'
import importlib.util
from pathlib import Path
import sys

path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("bump_workspace_version", path)
if spec is None or spec.loader is None:
    raise SystemExit(f"could not load {path}")
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)

def canonical(version: str, wording: str) -> str:
    return (
        "Current `main` requires Rust 1.94.0 or newer. "
        f"The tagged v{version} release {wording}\n"
        "the same Rust 1.94.0 floor.\n"
    )


def expect_update(label: str, source: str, expected: str) -> None:
    observed = module.replace_readme_versions(
        source,
        "1.3.1",
        (module.RULE_TAGGED_RELEASE,),
        label,
    )
    if observed != expected:
        raise SystemExit(f"{label} produced {observed!r}; expected {expected!r}")


def expect_failure(label: str, source: str) -> None:
    try:
        module.replace_readme_versions(
            source,
            "1.3.1",
            (module.RULE_TAGGED_RELEASE,),
            label,
        )
    except ValueError:
        return
    raise SystemExit(f"{label} did not fail closed")


expect_update(
    "tagged-release-canonical-has",
    canonical("1.3.0", "has"),
    canonical("1.3.1", "has"),
)
expect_update(
    "tagged-release-canonical-retains",
    canonical("1.3.0", "retains"),
    canonical("1.3.1", "retains"),
)

expect_failure(
    "tagged-release-historical-has-only",
    "Historical: The tagged v0.8.0 release has the old Rust floor.\n",
)
expect_failure(
    "tagged-release-historical-retains-only",
    "Historical: The tagged v0.8.0 release retains the old Rust floor.\n",
)

historical_allowed = (
    "Historical: The tagged v0.8.0 release retains the old Rust floor.\n"
)
expect_update(
    "tagged-release-canonical-plus-historical-allowed-wording",
    canonical("1.3.0", "has") + historical_allowed,
    canonical("1.3.1", "has") + historical_allowed,
)

expect_failure(
    "tagged-release-duplicate-canonical",
    canonical("1.3.0", "has") + canonical("1.3.0", "retains"),
)
expect_failure(
    "tagged-release-malformed-verb",
    canonical("1.3.0", "keeps"),
)

historical_changed = (
    "Historical: The tagged v0.8.0 release changed the public API.\n"
)
expect_update(
    "tagged-release-unrelated-historical-changed-prose",
    canonical("1.3.0", "retains") + historical_changed,
    canonical("1.3.1", "retains") + historical_changed,
)
PY
}

echo "==> checking the tagged-release wording contract"
assert_tagged_release_wording_contract

echo "==> copying the public workspace into $TMP_DIR"
SOURCE_FILES="$TMP_DIR/source-files"
git -C "$PUBLIC_ROOT" ls-files --cached --others --exclude-standard -z \
    | while IFS= read -r -d '' path; do
        [[ -e "$PUBLIC_ROOT/$path" ]] && printf '%s\0' "$path"
    done >"$SOURCE_FILES"
tar --directory "$PUBLIC_ROOT" --null --files-from="$SOURCE_FILES" --create \
    | tar --extract --directory "$TMP_DIR"
tar --directory "$TOOLING_ROOT" --create scripts/bump-workspace-version.py \
    | tar --extract --directory "$TMP_DIR"

echo "==> bumping the temporary workspace to $NEW_VERSION"
python3 "$TMP_DIR/scripts/bump-workspace-version.py" \
    --root "$TMP_DIR" "$NEW_VERSION"

while IFS= read -r manifest; do
    source_mode="$(stat --format='%a' "$PUBLIC_ROOT/$manifest")"
    bumped_mode="$(stat --format='%a' "$TMP_DIR/$manifest")"
    if [[ "$source_mode" != "$bumped_mode" ]]; then
        echo "manifest mode changed for $manifest: $source_mode -> $bumped_mode" >&2
        exit 1
    fi
done < <(git -C "$PUBLIC_ROOT" ls-files '*Cargo.toml')

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

echo "==> cargo check --workspace in the temporary copy (fake Cargo)"
cargo check --manifest-path "$TMP_DIR/Cargo.toml" --workspace

echo "Release bump smoke passed for $NEW_VERSION."
