#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
export CARGO_TARGET_DIR="$TMP_DIR/target"

BIN_DIR="$TMP_DIR/bin"
mkdir -p "$BIN_DIR"
export PATH="$BIN_DIR:$PATH"

# The release smoke exercises real shell/Git behavior but must not compile the
# disposable workspaces. This shim implements only the metadata contract used
# by the version bumper and records the release's Cargo.lock refresh check.
cat >"$BIN_DIR/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ -n "${RELEASE_TEST_CARGO_LOG:-}" ]]; then
    printf '%s\n' "$*" >>"$RELEASE_TEST_CARGO_LOG"
fi
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
        [[ "$*" == "check --workspace" ]]
        if [[ "${RELEASE_TEST_CARGO_CHECK_FAIL:-0}" == "1" ]]; then
            echo "synthetic cargo check failure" >&2
            exit 43
        fi
        ;;
    *)
        echo "unexpected cargo invocation in release smoke: $*" >&2
        exit 97
        ;;
esac
EOF
chmod +x "$BIN_DIR/cargo"
cat >"$BIN_DIR/rustc" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "--version" ]]
printf 'rustc 1.94.0 (release-smoke)\n'
EOF
chmod +x "$BIN_DIR/rustc"

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
    done < <(git -C "$REPO_ROOT" worktree list --porcelain)
    return 1
}

PUBLIC_ROOT="${SUPRNOVA_RELEASE_PUBLIC_ROOT:-}"
if [[ -z "$PUBLIC_ROOT" ]]; then
    PUBLIC_ROOT="$(find_main_worktree)"
fi
if [[ -z "$PUBLIC_ROOT" || ! -d "$PUBLIC_ROOT/.git" && ! -f "$PUBLIC_ROOT/.git" ]]; then
    echo "release smoke could not locate the public main worktree" >&2
    exit 2
fi

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
    local tooling_root="${3:-$REPO_ROOT}"
    local manifest="$TMP_DIR/source-files"

    mkdir -p "$destination"
    # Release fixtures exercise the reviewed working tree, including newly
    # added source, while omitting tracked paths deleted by a clean cutover.
    git -C "$source_root" ls-files --cached --others --exclude-standard -z \
        | while IFS= read -r -d '' path; do
            [[ -e "$source_root/$path" ]] && printf '%s\0' "$path"
        done >"$manifest"
    tar --directory "$source_root" --null --files-from="$manifest" --create \
        | tar --extract --directory "$destination"
    # Local gate assets are ignored on public main. Overlay the reviewed
    # tooling worktree so the disposable fixture installs this exact revision.
    for asset_root in scripts .githooks .cargo; do
        if [[ -d "$tooling_root/$asset_root" ]]; then
            tar --directory "$tooling_root" \
                --exclude='scripts/.git' \
                --exclude='scripts/__pycache__' \
                --create "$asset_root" \
                | tar --extract --directory "$destination"
        fi
    done
}


initialize_source_fixture() {
    local source_root=$1

    git -C "$source_root" init --initial-branch=main >/dev/null
    git -C "$source_root" config user.name "Suprnova Release Test"
    git -C "$source_root" config user.email "release-test@suprnova.invalid"
    git -C "$source_root" add -f .
    git -C "$source_root" commit -m "test: release source fixture" >/dev/null
}

install_disposable_gate() {
    local worktree=$1
    local tooling_source=$2
    local tooling_commit

    git clone --quiet "$worktree" "$tooling_source"
    tooling_commit="$(git -C "$worktree" rev-parse HEAD)"
    python3 "$tooling_source/scripts/install-gate.py" \
        --source "$tooling_source" \
        --repo "$worktree" \
        --commit "$tooling_commit" >/dev/null
    python3 "$tooling_source/scripts/install-gate.py" \
        --source "$tooling_source" \
        --repo "$worktree" \
        --verify-only >/dev/null
}

install_gate_stub() {
    local worktree=$1

    cat >"$worktree/scripts/gate.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "--full" && $# -eq 1 ]]
python3 - "$RELEASE_TEST_TARGET_VERSION" "$RELEASE_TEST_GATE_LOG" <<'PY'
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tomllib

target, gate_log = sys.argv[1:]
with open("Cargo.toml", "rb") as handle:
    version = tomllib.load(handle)["workspace"]["package"]["version"]
if version != target:
    raise SystemExit(f"gate did not observe bumped version: {version}")
if subprocess.check_output(
    ["git", "status", "--porcelain=v1", "--untracked-files=all"], text=True
):
    raise SystemExit("gate did not observe a clean release commit")
commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
subject = subprocess.check_output(
    ["git", "log", "-1", "--format=%s"], text=True
).strip()
if subject != f"release: v{target}":
    raise SystemExit(f"gate did not observe release commit: {subject}")

module_path = Path("scripts/gate-runner.py")
spec = importlib.util.spec_from_file_location("release_smoke_gate_runner", module_path)
if spec is None or spec.loader is None:
    raise SystemExit("cannot load gate runner")
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)
stamp = runner.build_stamp(
    Path.cwd(),
    tier="full",
    run_id="release-smoke",
    code_provenance=None,
    env=os.environ,
)
runner.write_stamp(Path.cwd(), stamp)
with open(gate_log, "a", encoding="utf-8") as handle:
    handle.write(
        f"gate --full version={version} commit={commit} clean=1 stamp={stamp.commit}\n"
    )
PY
EOF
    chmod +x "$worktree/scripts/gate.sh"
}

install_failing_gate_stub() {
    local worktree=$1

    cat >"$worktree/scripts/gate.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "--full" && $# -eq 1 ]]
python3 - "$RELEASE_TEST_TARGET_VERSION" "$RELEASE_TEST_GATE_LOG" <<'PY'
import subprocess
import sys
import tomllib

target, gate_log = sys.argv[1:]
with open("Cargo.toml", "rb") as handle:
    version = tomllib.load(handle)["workspace"]["package"]["version"]
if version != target:
    raise SystemExit(f"gate did not observe bumped version: {version}")
if subprocess.check_output(
    ["git", "status", "--porcelain=v1", "--untracked-files=all"], text=True
):
    raise SystemExit("gate did not observe a clean release commit")
commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
with open(gate_log, "a", encoding="utf-8") as handle:
    handle.write(f"gate --full version={version} commit={commit} clean=1\n")
PY
echo "step=magnetar-live tier=full outcome=fail classification=fail"
exit 1
EOF
    chmod +x "$worktree/scripts/gate.sh"
}

install_partially_failing_bump_stub() {
    local worktree=$1

    cat >"$worktree/scripts/bump-workspace-version.py" <<'PY'
#!/usr/bin/env python3
from pathlib import Path
import re
import sys

if "--validate-only" in sys.argv:
    raise SystemExit(0)

new_version = sys.argv[1]
manifest = Path("Cargo.toml")
source = manifest.read_text()
updated, count = re.subn(
    r'(?m)^(version = ")[^"]+("$)',
    rf"\g<1>{new_version}\g<2>",
    source,
    count=1,
)
if count != 1:
    raise SystemExit("synthetic bump could not update the workspace version")
manifest.write_text(updated)
print("Cargo.toml")
print("synthetic partial bump failure", file=sys.stderr)
raise SystemExit(42)
PY
}

install_normal_override_probe() {
    local probe=$1

    cat >"$probe" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'override %s\n' "$*" >>"$RELEASE_TEST_OVERRIDE_LOG"
exit 97
EOF
    chmod +x "$probe"
}

plant_release_stamp() {
    local worktree=$1
    local git_dir commit tree

    git_dir="$(git -C "$worktree" rev-parse --absolute-git-dir)"
    commit="$(git -C "$worktree" rev-parse HEAD)"
    tree="$(git -C "$worktree" rev-parse 'HEAD^{tree}')"
    cat >"$git_dir/suprnova-gate-pass" <<EOF
{"schema":2,"tier":"full","tree":"$tree","commit":"$commit","toolchain":"release-smoke","steps_hash":"$(printf '1%.0s' {1..64})","finished_at":"2026-08-24T00:00:00Z","run_id":"release-smoke","code_provenance":null,"local_tooling_commit":"$(printf '2%.0s' {1..40})"}
EOF
}

assert_exact_log() {
    local path=$1
    local expected=$2
    local -a lines

    mapfile -t lines <"$path"
    if [[ ${#lines[@]} -ne 1 || "${lines[0]}" != "$expected" ]]; then
        printf 'expected exactly one log line %q in %s; observed:\n' "$expected" "$path" >&2
        printf '  %s\n' "${lines[@]}" >&2
        exit 1
    fi
}

assert_gate_observed_release_commit() {
    local gate_log=$1
    local worktree=$2
    local target_version=$3
    local baseline=$4

    python3 - "$gate_log" "$worktree" "$target_version" "$baseline" <<'PY'
from pathlib import Path
import re
import subprocess
import sys

log_path, worktree, target, baseline = sys.argv[1:]
lines = Path(log_path).read_text(encoding="utf-8").splitlines()
if len(lines) != 1:
    raise SystemExit(f"expected one gate observation, got {lines!r}")
match = re.fullmatch(
    r"gate --full version=([^ ]+) commit=([0-9a-f]{40,64}) clean=1"
    r"(?: stamp=([0-9a-f]{40,64}))?",
    lines[0],
)
if match is None:
    raise SystemExit(f"invalid gate observation: {lines[0]!r}")
version, commit, stamp_commit = match.groups()
if version != target:
    raise SystemExit(f"gate observed version {version}, expected {target}")
if stamp_commit is not None and stamp_commit != commit:
    raise SystemExit("fake full-gate stamp does not name the release commit")
parent = subprocess.check_output(
    ["git", "-C", worktree, "rev-parse", f"{commit}^"], text=True
).strip()
if parent != baseline:
    raise SystemExit(f"release commit parent {parent} != baseline {baseline}")
subject = subprocess.check_output(
    ["git", "-C", worktree, "log", "-1", "--format=%s", commit], text=True
).strip()
if subject != f"release: v{target}":
    raise SystemExit(f"unexpected release commit subject: {subject}")
PY
}

assert_no_gate_stamp() {
    local worktree=$1
    local git_dir
    git_dir="$(git -C "$worktree" rev-parse --absolute-git-dir)"
    [[ ! -e "$git_dir/suprnova-gate-pass" ]]
}

prove_dry_run_uses_override_without_trusting_stamp() {
    local source_root=$1
    local source_version new_version
    local worktree="$TMP_DIR/dry-run-worktree"
    local gate_override="$TMP_DIR/dry-run-gate"
    local bump_override="$TMP_DIR/dry-run-bump"
    local gate_log="$TMP_DIR/dry-run-gate.log"
    local bump_log="$TMP_DIR/dry-run-bump.log"
    local cargo_log="$TMP_DIR/dry-run-cargo.log"
    local baseline

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    copy_tracked_source "$source_root" "$worktree"
    initialize_source_fixture "$worktree"
    plant_release_stamp "$worktree"
    baseline="$(git -C "$worktree" rev-parse HEAD)"

    cat >"$gate_override" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ "${1:-}" == "--full" && $# -eq 1 ]]
printf 'gate %s\n' "$*" >>"$RELEASE_TEST_GATE_LOG"
EOF
    chmod +x "$gate_override"
    cat >"$bump_override" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ $# -eq 1 ]]
printf 'bump %s\n' "$1" >>"$RELEASE_TEST_BUMP_LOG"
EOF
    chmod +x "$bump_override"

    echo "==> proving dry-run override still runs T2 despite a planted full stamp"
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_BUMP_LOG="$bump_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        SUPRNOVA_RELEASE_GATE="$gate_override" \
        SUPRNOVA_RELEASE_BUMP_SMOKE="$bump_override" \
            scripts/release.sh --dry-run "$new_version"
    )

    assert_exact_log "$gate_log" "gate --full"
    assert_exact_log "$bump_log" "bump $new_version"
    [[ "$(git -C "$worktree" rev-parse HEAD)" == "$baseline" ]]
    [[ -z "$(git -C "$worktree" tag)" ]]
    [[ "$(workspace_version "$worktree")" == "$source_version" ]]
    [[ -z "$(git -C "$worktree" status --porcelain=v1 --untracked-files=all)" ]]
}

prove_bump_failure_does_not_create_release_commit() {
    local source_root=$1
    local source_version new_version
    local worktree="$TMP_DIR/bump-failure-worktree"
    local remote="$TMP_DIR/bump-failure-origin.git"
    local gate_log="$TMP_DIR/bump-failure-gate.log"
    local cargo_log="$TMP_DIR/bump-failure-cargo.log"
    local receive_log="$TMP_DIR/bump-failure-receive.log"
    local override_log="$TMP_DIR/bump-failure-override.log"
    local override_probe="$TMP_DIR/bump-failure-override"
    local output="$TMP_DIR/bump-failure-output.log"
    local baseline status

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    copy_tracked_source "$source_root" "$worktree"
    install_gate_stub "$worktree"
    install_partially_failing_bump_stub "$worktree"
    initialize_source_fixture "$worktree"
    install_disposable_gate "$worktree" "$TMP_DIR/bump-failure-tooling"
    install_normal_override_probe "$override_probe"
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
        git -C "$worktree" push --no-verify -u origin main >/dev/null
    baseline="$(git -C "$worktree" rev-parse HEAD)"
    plant_release_stamp "$worktree"
    : >"$receive_log"
    : >"$gate_log"
    : >"$cargo_log"

    echo "==> proving a partial bump failure cannot create a release commit"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_OVERRIDE_LOG="$override_log" \
        RELEASE_TEST_TARGET_VERSION="$new_version" \
        SUPRNOVA_RELEASE_GATE="$override_probe" \
            scripts/release.sh "$new_version"
    ) >"$output" 2>&1
    status=$?
    set -e

    [[ $status -ne 0 ]]
    grep -Fq "synthetic partial bump failure" "$output"
    [[ ! -s "$gate_log" ]]
    [[ ! -s "$override_log" ]]
    ! grep -Fq "check --workspace" "$cargo_log"
    [[ "$(git -C "$worktree" rev-parse HEAD)" == "$baseline" ]]
    [[ "$(git --git-dir="$remote" rev-parse refs/heads/main)" == "$baseline" ]]
    [[ ! -s "$receive_log" ]]
    [[ -z "$(git -C "$worktree" tag)" ]]
    [[ "$(workspace_version "$worktree")" == "$source_version" ]]
    [[ -z "$(git -C "$worktree" status --porcelain=v1 --untracked-files=all)" ]]
    assert_no_gate_stamp "$worktree"
}

prove_cargo_failure_rolls_back_release_files() {
    local source_root=$1
    local source_version new_version
    local worktree="$TMP_DIR/cargo-failure-worktree"
    local remote="$TMP_DIR/cargo-failure-origin.git"
    local gate_log="$TMP_DIR/cargo-failure-gate.log"
    local cargo_log="$TMP_DIR/cargo-failure-cargo.log"
    local override_log="$TMP_DIR/cargo-failure-override.log"
    local override_probe="$TMP_DIR/cargo-failure-override"
    local output="$TMP_DIR/cargo-failure-output.log"
    local baseline status

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    copy_tracked_source "$source_root" "$worktree"
    install_gate_stub "$worktree"
    initialize_source_fixture "$worktree"
    install_disposable_gate "$worktree" "$TMP_DIR/cargo-failure-tooling"
    install_normal_override_probe "$override_probe"
    git init --bare "$remote" >/dev/null
    git -C "$worktree" remote add origin "$remote"
    git -C "$worktree" push --no-verify -u origin main >/dev/null
    baseline="$(git -C "$worktree" rev-parse HEAD)"
    plant_release_stamp "$worktree"
    : >"$gate_log"
    : >"$cargo_log"

    echo "==> proving Cargo.lock refresh failure rolls back release-owned files"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        RELEASE_TEST_OVERRIDE_LOG="$override_log" \
        RELEASE_TEST_TARGET_VERSION="$new_version" \
        RELEASE_TEST_CARGO_CHECK_FAIL=1 \
        SUPRNOVA_RELEASE_GATE="$override_probe" \
            scripts/release.sh "$new_version"
    ) >"$output" 2>&1
    status=$?
    set -e

    [[ $status -ne 0 ]]
    grep -Fq "synthetic cargo check failure" "$output"
    [[ "$(grep -Fc "check --workspace" "$cargo_log")" -eq 1 ]]
    [[ ! -s "$gate_log" ]]
    [[ ! -s "$override_log" ]]
    [[ "$(git -C "$worktree" rev-parse HEAD)" == "$baseline" ]]
    [[ "$(git --git-dir="$remote" rev-parse refs/heads/main)" == "$baseline" ]]
    [[ -z "$(git -C "$worktree" tag)" ]]
    [[ "$(workspace_version "$worktree")" == "$source_version" ]]
    [[ -z "$(git -C "$worktree" status --porcelain=v1 --untracked-files=all)" ]]
    assert_no_gate_stamp "$worktree"
}

prove_classified_t2_failure_rolls_back_release_commit() {
    local source_root=$1
    local source_version new_version
    local worktree="$TMP_DIR/t2-failure-worktree"
    local remote="$TMP_DIR/t2-failure-origin.git"
    local gate_log="$TMP_DIR/t2-failure-gate.log"
    local cargo_log="$TMP_DIR/t2-failure-cargo.log"
    local receive_log="$TMP_DIR/t2-failure-receive.log"
    local override_log="$TMP_DIR/t2-failure-override.log"
    local override_probe="$TMP_DIR/t2-failure-override"
    local output="$TMP_DIR/t2-failure-output.log"
    local baseline status

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    copy_tracked_source "$source_root" "$worktree"
    install_failing_gate_stub "$worktree"
    initialize_source_fixture "$worktree"
    install_disposable_gate "$worktree" "$TMP_DIR/t2-failure-tooling"
    install_normal_override_probe "$override_probe"
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
        git -C "$worktree" push --no-verify -u origin main >/dev/null
    baseline="$(git -C "$worktree" rev-parse HEAD)"
    plant_release_stamp "$worktree"
    : >"$receive_log"
    : >"$gate_log"
    : >"$cargo_log"

    echo "==> proving a classified T2 failure rolls back the clean release commit"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_OVERRIDE_LOG="$override_log" \
        RELEASE_TEST_TARGET_VERSION="$new_version" \
        SUPRNOVA_RELEASE_GATE="$override_probe" \
            scripts/release.sh "$new_version"
    ) >"$output" 2>&1
    status=$?
    set -e

    [[ $status -ne 0 ]]
    grep -Fq "tier=full outcome=fail classification=fail" "$output"
    assert_gate_observed_release_commit "$gate_log" "$worktree" "$new_version" "$baseline"
    [[ ! -s "$override_log" ]]
    [[ "$(grep -Fc "check --workspace" "$cargo_log")" -eq 1 ]]
    [[ "$(git -C "$worktree" rev-parse HEAD)" == "$baseline" ]]
    [[ "$(git --git-dir="$remote" rev-parse refs/heads/main)" == "$baseline" ]]
    [[ ! -s "$receive_log" ]]
    [[ -z "$(git -C "$worktree" tag)" ]]
    [[ "$(workspace_version "$worktree")" == "$source_version" ]]
    [[ -z "$(git -C "$worktree" status --porcelain=v1 --untracked-files=all)" ]]
    assert_no_gate_stamp "$worktree"
}

prove_atomic_tag_rejection() {
    local source_root=$1
    local source_version new_version
    local worktree="$TMP_DIR/atomic-rejection-worktree"
    local remote="$TMP_DIR/atomic-rejection-origin.git"
    local gate_log="$TMP_DIR/atomic-rejection-gate.log"
    local cargo_log="$TMP_DIR/atomic-rejection-cargo.log"
    local receive_log="$TMP_DIR/atomic-rejection-receive.log"
    local override_log="$TMP_DIR/atomic-rejection-override.log"
    local override_probe="$TMP_DIR/atomic-rejection-override"
    local output="$TMP_DIR/atomic-rejection.log"
    local baseline status

    source_version="$(workspace_version "$source_root")"
    new_version="$(next_patch_version "$source_version")"
    copy_tracked_source "$source_root" "$worktree"
    install_gate_stub "$worktree"
    install_normal_override_probe "$override_probe"
    initialize_source_fixture "$worktree"
    install_disposable_gate "$worktree" "$TMP_DIR/atomic-rejection-tooling"

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
        git -C "$worktree" push --no-verify -u origin main >/dev/null
    baseline="$(git --git-dir="$remote" rev-parse refs/heads/main)"
    plant_release_stamp "$worktree"
    : >"$receive_log"
    : >"$gate_log"

    echo "==> proving atomic rollback when tag v$new_version is rejected"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_OVERRIDE_LOG="$override_log" \
        RELEASE_TEST_REJECT_TAG="refs/tags/v$new_version" \
        RELEASE_TEST_TARGET_VERSION="$new_version" \
        SUPRNOVA_RELEASE_GATE="$override_probe" \
            scripts/release.sh "$new_version"
    ) >"$output" 2>&1
    status=$?
    set -e

    [[ $status -ne 0 ]]
    assert_gate_observed_release_commit "$gate_log" "$worktree" "$new_version" "$baseline"
    grep -Fq "pre-push: gate stamp authorizes 2 pushed tip(s)" "$output"
    [[ "$(grep -Fc "check --workspace" "$cargo_log")" -eq 1 ]]
    [[ ! -s "$override_log" ]]
    [[ "$(git --git-dir="$remote" rev-parse refs/heads/main)" == "$baseline" ]]
    if git --git-dir="$remote" rev-parse --verify "refs/tags/v$new_version" >/dev/null 2>&1; then
        echo "rejected tag unexpectedly reached the remote" >&2
        exit 1
    fi
    mapfile -t updates <"$receive_log"
    if [[ ${#updates[@]} -ne 2 ]]; then
        cat "$output" >&2
        echo "expected branch and tag updates; received ${#updates[@]}" >&2
        exit 1
    fi
    grep -Fq ' refs/heads/main' "$receive_log"
    grep -Fq " refs/tags/v$new_version" "$receive_log"
    [[ "$(workspace_version "$source_root")" == "$source_version" ]]
    [[ "$(workspace_version "$worktree")" == "$new_version" ]]
    [[ "$(git -C "$worktree" log -1 --format=%s)" == "release: v$new_version" ]]
    [[ "$(git -C "$worktree" rev-parse "v$new_version^{}")" == "$(git -C "$worktree" rev-parse HEAD)" ]]
}

run_release_case() {
    local source_root=$1
    local case_name=$2
    local source_version new_version
    local worktree="$TMP_DIR/$case_name-worktree"
    local remote="$TMP_DIR/$case_name-origin.git"
    local gate_log="$TMP_DIR/$case_name-gate.log"
    local cargo_log="$TMP_DIR/$case_name-cargo.log"
    local receive_log="$TMP_DIR/$case_name-receive.log"
    local override_log="$TMP_DIR/$case_name-override.log"
    local override_probe="$TMP_DIR/$case_name-override"
    local rejection_log="$TMP_DIR/$case_name-untracked.log"
    local output="$TMP_DIR/$case_name-release.log"
    local probe baseline

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
    install_normal_override_probe "$override_probe"

    initialize_source_fixture "$worktree"
    install_disposable_gate "$worktree" "$TMP_DIR/$case_name-tooling"
    baseline="$(git -C "$worktree" rev-parse HEAD)"
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
        git -C "$worktree" push --no-verify -u origin main >/dev/null
    plant_release_stamp "$worktree"
    : >"$receive_log"
    : >"$gate_log"

    echo "==> proving untracked-file rejection for $source_version"
    probe="$worktree/framework/tests/release_untracked_probe.rs"
    printf '#[test]\nfn release_untracked_probe() {}\n' >"$probe"
    set +e
    (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_OVERRIDE_LOG="$override_log" \
        RELEASE_TEST_TARGET_VERSION="$new_version" \
        SUPRNOVA_RELEASE_GATE="$override_probe" \
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
    [[ ! -s "$override_log" ]]

    : >"$cargo_log"
    echo "==> running normal release $source_version -> $new_version against disposable bare origin"
    if ! (
        cd "$worktree"
        RELEASE_TEST_GATE_LOG="$gate_log" \
        RELEASE_TEST_CARGO_LOG="$cargo_log" \
        RELEASE_TEST_RECEIVE_LOG="$receive_log" \
        RELEASE_TEST_OVERRIDE_LOG="$override_log" \
        RELEASE_TEST_TARGET_VERSION="$new_version" \
        SUPRNOVA_RELEASE_GATE="$override_probe" \
            scripts/release.sh "$new_version"
    ) >"$output" 2>&1; then
        cat "$output" >&2
        return 1
    fi

    assert_gate_observed_release_commit "$gate_log" "$worktree" "$new_version" "$baseline"
    grep -Fq "pre-push: gate stamp authorizes 2 pushed tip(s)" "$output"
    [[ ! -s "$override_log" ]]
    [[ "$(grep -Fc "check --workspace" "$cargo_log")" -eq 1 ]]
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

current_version="$(workspace_version "$PUBLIC_ROOT")"
prove_dry_run_uses_override_without_trusting_stamp "$PUBLIC_ROOT"
prove_bump_failure_does_not_create_release_commit "$PUBLIC_ROOT"
prove_cargo_failure_rolls_back_release_files "$PUBLIC_ROOT"
prove_classified_t2_failure_rolls_back_release_commit "$PUBLIC_ROOT"
prove_atomic_tag_rejection "$PUBLIC_ROOT"
run_release_case "$PUBLIC_ROOT" "current"

# Keep an explicit post-0.6 fixture until the source itself reaches 0.6.0.
# This proves the smoke derives a later release instead of repeatedly targeting
# the semantic release currently being prepared.
if [[ "$current_version" != "0.6.0" ]]; then
    fixture="$TMP_DIR/source-0.6.0"
    copy_tracked_source "$PUBLIC_ROOT" "$fixture"
    python3 "$fixture/scripts/bump-workspace-version.py" \
        --root "$fixture" 0.6.0 >/dev/null
    initialize_source_fixture "$fixture"
    run_release_case "$fixture" "already-0.6.0"
fi
