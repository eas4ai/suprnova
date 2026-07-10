#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
export CARGO_TARGET_DIR="$TMP_DIR/target"

workspace_version() {
    python3 - "$1/Cargo.toml" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as handle:
    print(tomllib.load(handle)["workspace"]["package"]["version"])
PY
}

next_patch_version() {
    python3 - "$1" <<'PY'
import sys

version = sys.argv[1]
core = version.split("+", 1)[0].split("-", 1)[0]
major, minor, patch = (int(part) for part in core.split("."))
print(f"{major}.{minor}.{patch + 1}")
PY
}

copy_tracked_source() {
    local source_root=$1
    local destination=$2

    mkdir -p "$destination"
    git -C "$source_root" ls-files --cached -z \
        | tar --directory "$source_root" --null --files-from=- --create \
        | tar --extract --directory "$destination"
}

initialize_source_fixture() {
    local source_root=$1

    git -C "$source_root" init --initial-branch=main >/dev/null
    git -C "$source_root" config user.name "Suprnova Release Test"
    git -C "$source_root" config user.email "release-test@suprnova.invalid"
    git -C "$source_root" add -f .
    git -C "$source_root" commit -m "test: release source fixture" >/dev/null
}

install_gate_stub() {
    local worktree=$1

    cat >"$worktree/scripts/gate.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "--full" ]]
python3 - "$RELEASE_TEST_SOURCE_VERSION" <<'PY'
import sys
import tomllib

with open("Cargo.toml", "rb") as handle:
    version = tomllib.load(handle)["workspace"]["package"]["version"]
if version != sys.argv[1]:
    raise SystemExit(f"gate ran after version edit: {version}")
PY
[[ -z "$(git status --porcelain=v1 --untracked-files=all)" ]]
printf 'gate %s\n' "$*" >>"$RELEASE_TEST_GATE_LOG"
EOF
    chmod +x "$worktree/scripts/gate.sh"
}

prove_atomic_tag_rejection() {
    local source_root=$1
    local source_version new_version
    local worktree="$TMP_DIR/atomic-rejection-worktree"
    local remote="$TMP_DIR/atomic-rejection-origin.git"
    local gate_log="$TMP_DIR/atomic-rejection-gate.log"
    local receive_log="$TMP_DIR/atomic-rejection-receive.log"
    local output="$TMP_DIR/atomic-rejection.log"
    local baseline status

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    copy_tracked_source "$source_root" "$worktree"
    install_gate_stub "$worktree"
    initialize_source_fixture "$worktree"

    git init --bare "$remote" >/dev/null
    cat >"$remote/hooks/update" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
ref=$1
old=$2
new=$3
printf '%s %s %s\n' "$old" "$new" "$ref" >>"$RELEASE_TEST_RECEIVE_LOG"
if [[ "$ref" == "${RELEASE_TEST_REJECT_TAG:-}" ]]; then
    echo "rejecting test tag $ref" >&2
    exit 1
fi
EOF
    chmod +x "$remote/hooks/update"
    git -C "$worktree" remote add origin "$remote"
    RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        git -C "$worktree" push -u origin main >/dev/null
    baseline="$(git --git-dir="$remote" rev-parse refs/heads/main)"
    : >"$receive_log"
    : >"$gate_log"

    echo "==> proving atomic rollback when tag v$new_version is rejected"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_REJECT_TAG="refs/tags/v$new_version" \
        RELEASE_TEST_SOURCE_VERSION="$source_version" \
            scripts/release.sh "$new_version"
    ) >"$output" 2>&1
    status=$?
    set -e

    [[ $status -ne 0 ]]
    grep -Fqx "gate --full" "$gate_log"
    [[ "$(git --git-dir="$remote" rev-parse refs/heads/main)" == "$baseline" ]]
    if git --git-dir="$remote" rev-parse --verify "refs/tags/v$new_version" >/dev/null 2>&1; then
        echo "rejected tag unexpectedly reached the remote" >&2
        exit 1
    fi
    mapfile -t updates <"$receive_log"
    [[ ${#updates[@]} -eq 2 ]]
    grep -Fq ' refs/heads/main' "$receive_log"
    grep -Fq " refs/tags/v$new_version" "$receive_log"
    [[ "$(workspace_version "$source_root")" == "$source_version" ]]
}

run_release_case() {
    local source_root=$1
    local case_name=$2
    local source_version new_version
    local worktree="$TMP_DIR/$case_name-worktree"
    local remote="$TMP_DIR/$case_name-origin.git"
    local gate_log="$TMP_DIR/$case_name-gate.log"
    local receive_log="$TMP_DIR/$case_name-receive.log"
    local rejection_log="$TMP_DIR/$case_name-untracked.log"
    local probe

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    [[ "$new_version" != "$source_version" ]]
    python3 "$source_root/scripts/bump-workspace-version.py" \
        --validate-only "$new_version"

    copy_tracked_source "$source_root" "$worktree"

    # Keep release.sh unmodified. Replace only the copied canonical gate so
    # this test exercises normal commit/tag/push behavior without replaying
    # the expensive matrix or auditing a disposable lockfile.
    install_gate_stub "$worktree"

    initialize_source_fixture "$worktree"
    git init --bare "$remote" >/dev/null
    cat >"$remote/hooks/pre-receive" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
while read -r old new ref; do
    printf '%s %s %s\n' "$old" "$new" "$ref" >>"$RELEASE_TEST_RECEIVE_LOG"
done
EOF
    chmod +x "$remote/hooks/pre-receive"

    git -C "$worktree" remote add origin "$remote"
    RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        git -C "$worktree" push -u origin main >/dev/null
    : >"$receive_log"
    : >"$gate_log"

    echo "==> proving untracked-file rejection for $source_version"
    probe="$worktree/framework/tests/release_untracked_probe.rs"
    printf '#[test]\nfn release_untracked_probe() {}\n' >"$probe"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_SOURCE_VERSION="$source_version" \
            scripts/release.sh "$new_version"
    ) >"$rejection_log" 2>&1
    local rejection_status=$?
    set -e

    [[ $rejection_status -ne 0 ]]
    grep -Fq 'working tree is dirty' "$rejection_log"
    [[ ! -s "$gate_log" ]]
    [[ ! -s "$receive_log" ]]
    [[ -z "$(git -C "$worktree" tag)" ]]
    [[ "$(workspace_version "$worktree")" == "$source_version" ]]
    rm "$probe"
    [[ -z "$(git -C "$worktree" status --porcelain=v1 --untracked-files=all)" ]]

    echo "==> running normal release $source_version -> $new_version against disposable bare origin"
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_SOURCE_VERSION="$source_version" \
            scripts/release.sh "$new_version"
    )

    grep -Fqx "gate --full" "$gate_log"
    mapfile -t updates <"$receive_log"
    [[ ${#updates[@]} -eq 2 ]]
    [[ "${updates[0]}" == *" refs/heads/main" ]]
    [[ "${updates[1]}" == *" refs/tags/v$new_version" ]]

    local remote_main remote_tag
    remote_main="$(git --git-dir="$remote" rev-parse refs/heads/main)"
    remote_tag="$(git --git-dir="$remote" rev-parse "refs/tags/v$new_version^{}")"
    [[ "$remote_main" == "$remote_tag" ]]
    [[ "$(git --git-dir="$remote" log -1 --format=%s refs/heads/main)" == \
        "release: v$new_version" ]]

    python3 "$worktree/scripts/bump-workspace-version.py" \
        --root "$worktree" --verify "$new_version"
    [[ "$(workspace_version "$source_root")" == "$source_version" ]]

    echo "Normal release smoke passed: $source_version -> $new_version."
}

current_version="$(workspace_version "$REPO_ROOT")"
prove_atomic_tag_rejection "$REPO_ROOT"
run_release_case "$REPO_ROOT" "current"

# Keep an explicit post-0.6 fixture until the source itself reaches 0.6.0.
# This proves the smoke derives a later release instead of repeatedly targeting
# the semantic release currently being prepared.
if [[ "$current_version" != "0.6.0" ]]; then
    fixture="$TMP_DIR/source-0.6.0"
    copy_tracked_source "$REPO_ROOT" "$fixture"
    python3 "$fixture/scripts/bump-workspace-version.py" \
        --root "$fixture" 0.6.0 >/dev/null
    initialize_source_fixture "$fixture"
    run_release_case "$fixture" "already-0.6.0"
fi
