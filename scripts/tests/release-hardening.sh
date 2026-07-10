#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

fail() {
    echo "release hardening test failed: $*" >&2
    exit 1
}

assert_log_line() {
    local expected=$1
    local log=$2

    grep -Fqx "$expected" "$log" || fail "missing command: $expected"
}

test_audit_is_required_and_fail_closed() {
    local output="$TMP_DIR/audit-missing.log"

    if env PATH=/usr/bin:/bin scripts/check-audit.sh >"$output" 2>&1; then
        fail "audit check succeeded without cargo-audit"
    fi
    grep -Fq "cargo-audit is required" "$output" \
        || fail "missing cargo-audit error was not actionable"

    mkdir -p "$TMP_DIR/audit-bin"
    cat >"$TMP_DIR/audit-bin/cargo-audit" <<'EOF'
#!/usr/bin/env bash
echo "simulated audit vulnerability" >&2
exit 42
EOF
    cat >"$TMP_DIR/audit-bin/cargo" <<'EOF'
#!/usr/bin/env bash
exec "$(dirname "$0")/cargo-audit" "$@"
EOF
    chmod +x "$TMP_DIR/audit-bin/cargo"
    chmod +x "$TMP_DIR/audit-bin/cargo-audit"

    output="$TMP_DIR/audit-failed.log"
    if env PATH="$TMP_DIR/audit-bin:/usr/bin:/bin" \
        scripts/check-audit.sh >"$output" 2>&1; then
        fail "audit check swallowed cargo-audit failure"
    fi
    grep -Fq "simulated audit vulnerability" "$output" \
        || fail "cargo-audit failure output was not preserved"
}

write_fake_cargo() {
    mkdir -p "$TMP_DIR/cargo-bin"
    cat >"$TMP_DIR/cargo-bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

printf '%s\n' "$*" >>"$CARGO_LOG"

case "${1:-}" in
    tree)
        features=""
        while [[ $# -gt 0 ]]; do
            if [[ "$1" == "--features" ]]; then
                features=$2
                break
            fi
            shift
        done
        case "$features" in
            database-sqlite)
                echo "sqlx-sqlite v0.8.0"
                ;;
            database-postgres)
                echo "sqlx-postgres v0.8.0"
                ;;
            database-mysql)
                echo "sqlx-mysql v0.8.0"
                ;;
            database-sqlite,database-postgres,broadcasting-fanout)
                echo "sqlx-sqlite v0.8.0"
                echo "sqlx-postgres v0.8.0"
                ;;
        esac
        ;;
    test)
        target=""
        while [[ $# -gt 0 ]]; do
            if [[ "$1" == "--test" ]]; then
                target=$2
                break
            fi
            shift
        done
        case "$target" in
            eloquent_casts_encrypted)
                echo "as_hashed_writes_bcrypt_and_does_not_decrypt: test"
                echo "as_hashed_is_idempotent_across_re_saves: test"
                ;;
            encryption)
                echo "appears_encrypted_rejects_plaintext_and_short_payloads: test"
                ;;
            remember_me)
                echo "forget_remember_cookie_clears_the_cookie: test"
                ;;
        esac
        ;;
esac
EOF
    chmod +x "$TMP_DIR/cargo-bin/cargo"
}

test_feature_matrix_runs_all_rustdoc_profiles() {
    local log="$TMP_DIR/cargo.log"
    : >"$log"
    write_fake_cargo

    env CARGO_LOG="$log" PATH="$TMP_DIR/cargo-bin:/usr/bin:/bin" \
        scripts/check-feature-matrix.sh >"$TMP_DIR/matrix.log" 2>&1

    assert_log_line "doc -p suprnova --no-default-features --no-deps" "$log"
    assert_log_line "doc -p suprnova --no-deps" "$log"
    assert_log_line "doc -p suprnova --all-features --no-deps" "$log"
}

write_release_stub() {
    local path=$1
    local marker=$2
    local status=$3

    cat >"$path" <<EOF
#!/usr/bin/env bash
echo "\$*" >>"$marker"
exit $status
EOF
    chmod +x "$path"
}

test_release_dry_run_stops_before_bump_when_gate_fails() {
    local gate="$TMP_DIR/failing-gate"
    local smoke="$TMP_DIR/bump-smoke"
    local gate_marker="$TMP_DIR/gate.marker"
    local smoke_marker="$TMP_DIR/smoke.marker"

    write_release_stub "$gate" "$gate_marker" 23
    write_release_stub "$smoke" "$smoke_marker" 0

    set +e
    SUPRNOVA_RELEASE_GATE="$gate" \
        SUPRNOVA_RELEASE_BUMP_SMOKE="$smoke" \
        scripts/release.sh --dry-run 0.6.0 >"$TMP_DIR/release.log" 2>&1
    local status=$?
    set -e

    [[ $status -eq 23 ]] || fail "dry-run did not return the failing gate status"
    [[ -f "$gate_marker" ]] || fail "dry-run did not invoke the canonical gate"
    [[ ! -e "$smoke_marker" ]] || fail "dry-run invoked bump smoke after gate failure"
    grep -Fqx -- "--full" "$gate_marker" \
        || fail "release did not request the canonical full gate"
}

test_strict_semver_validation() {
    local version

    for version in \
        0.0.0 \
        1.2.3 \
        1.2.3-0 \
        1.2.3-alpha.1 \
        1.2.3-alpha-beta+build.001; do
        python3 scripts/bump-workspace-version.py \
            --validate-only "$version" \
            || fail "valid SemVer was rejected: $version"
    done

    for version in \
        01.2.3 \
        1.02.3 \
        1.2.03 \
        1.2.3-01 \
        1.2.3-alpha.01 \
        1.2.3- \
        1.2.3-alpha_1; do
        if python3 scripts/bump-workspace-version.py \
            --validate-only "$version" >/dev/null 2>&1; then
            fail "invalid SemVer was accepted: $version"
        fi
    done
}

assert_release_order() {
    local repo=$1
    local current=$2
    local proposed=$3
    local expectation=$4
    local gate_marker=$5
    local output="$TMP_DIR/order-${current//[^A-Za-z0-9]/_}-${proposed//[^A-Za-z0-9]/_}.log"
    local status

    python3 - "$repo/Cargo.toml" "$current" <<'PY'
import sys
from pathlib import Path

Path(sys.argv[1]).write_text(
    "[workspace]\nmembers = []\n\n[workspace.package]\n"
    f'version = "{sys.argv[2]}"\n'
)
PY
    : >"$gate_marker"
    set +e
    (
        cd "$repo"
        RELEASE_TEST_GATE_LOG="$gate_marker" scripts/release.sh --dry-run "$proposed"
    ) >"$output" 2>&1
    status=$?
    set -e

    if [[ "$expectation" == "accept" ]]; then
        [[ $status -eq 91 ]] \
            || fail "release rejected increasing SemVer $current -> $proposed"
        grep -Fqx 'gate --full' "$gate_marker" \
            || fail "increasing SemVer did not reach the release gate"
    else
        [[ $status -eq 64 ]] \
            || fail "release accepted non-increasing SemVer $current -> $proposed"
        [[ ! -s "$gate_marker" ]] \
            || fail "non-increasing SemVer reached the release gate"
        grep -Fq 'must be greater than current workspace version' "$output" \
            || fail "non-increasing SemVer rejection was not actionable"
    fi
}

test_release_requires_strictly_increasing_semver() {
    local repo="$TMP_DIR/version-order"
    local gate_marker="$TMP_DIR/version-order-gate.marker"

    mkdir -p "$repo/scripts"
    cp scripts/release.sh scripts/bump-workspace-version.py "$repo/scripts/"
    cat >"$repo/scripts/gate.sh" <<'EOF'
#!/usr/bin/env bash
printf 'gate %s\n' "$*" >>"$RELEASE_TEST_GATE_LOG"
exit 91
EOF
    chmod +x "$repo/scripts/release.sh" "$repo/scripts/gate.sh" \
        "$repo/scripts/bump-workspace-version.py"
    git -C "$repo" init --initial-branch=main >/dev/null

    assert_release_order "$repo" 1.2.3 1.2.2 reject "$gate_marker"
    assert_release_order "$repo" 1.2.3 1.2.3 reject "$gate_marker"
    assert_release_order "$repo" 1.2.3+build.1 1.2.3+build.2 reject "$gate_marker"
    assert_release_order "$repo" 1.2.3-alpha 1.2.3-alpha.1 accept "$gate_marker"
    assert_release_order "$repo" 1.2.3-alpha.2 1.2.3-alpha.1 reject "$gate_marker"
    assert_release_order "$repo" 1.2.3-alpha.1 1.2.3-alpha.beta accept "$gate_marker"
    assert_release_order "$repo" 1.2.3-beta.2 1.2.3-beta.11 accept "$gate_marker"
    assert_release_order "$repo" 1.2.3-rc.1 1.2.3 accept "$gate_marker"
    assert_release_order "$repo" 1.2.3 1.2.3-rc.1 reject "$gate_marker"
    assert_release_order "$repo" 1.2.3 1.2.4-alpha.1 accept "$gate_marker"
}

test_full_gate_includes_release_security_profiles() {
    grep -Fq 'scripts/check-msrv.sh' scripts/gate.sh \
        || fail "full gate does not invoke the Rust MSRV check"
    grep -Fq 'scripts/check-downstream-dependencies.sh' scripts/gate.sh \
        || fail "full gate does not invoke the downstream dependency check"
    grep -Fq 'scripts/tests/release-normal-smoke.sh' scripts/gate.sh \
        || fail "full gate does not invoke the normal release smoke"
}

test_release_smokes_are_version_agnostic_and_isolated() {
    if grep -F 'scripts/tests/release-normal-smoke.sh 0.6.0' scripts/gate.sh >/dev/null; then
        fail "full gate hard-codes the normal release smoke version"
    fi
    if grep -F '0.5.10' scripts/tests/release-normal-smoke.sh >/dev/null; then
        fail "normal release smoke assumes the current source version"
    fi
    grep -Fq 'export CARGO_TARGET_DIR="$TMP_DIR/target"' scripts/tests/release-normal-smoke.sh \
        || fail "normal release smoke does not force its target beneath TMP_DIR"
    grep -Fq 'export CARGO_TARGET_DIR="$TMP_DIR/target"' scripts/tests/release-bump-smoke.sh \
        || fail "release bump smoke does not force its target beneath TMP_DIR"
    if grep -Fq 'CARGO_TARGET_DIR:-' scripts/tests/release-normal-smoke.sh \
        || grep -Fq 'CARGO_TARGET_DIR:-' scripts/tests/release-bump-smoke.sh; then
        fail "release smoke permits a caller target directory override"
    fi
    grep -Fq 'git push --atomic origin main "v$NEW_VERSION"' scripts/release.sh \
        || fail "release does not atomically push main and its tag"
}

test_normal_release_rejects_untracked_files_before_side_effects() {
    local repo="$TMP_DIR/untracked-release"
    local remote="$TMP_DIR/untracked-origin.git"
    local gate_marker="$TMP_DIR/untracked-gate.marker"
    local output="$TMP_DIR/untracked-release.log"
    local baseline

    mkdir -p "$repo/scripts" "$repo/src/bin"
    cp scripts/release.sh scripts/bump-workspace-version.py "$repo/scripts/"
    cat >"$repo/Cargo.toml" <<'EOF'
[workspace]
members = []

[workspace.package]
version = "1.2.3"
edition = "2024"
rust-version = "1.91.1"
license = "MIT"
EOF
    printf '/ignored-release-artifact\n' >"$repo/.gitignore"
    cat >"$repo/scripts/gate.sh" <<'EOF'
#!/usr/bin/env bash
printf 'gate %s\n' "$*" >>"$RELEASE_TEST_GATE_LOG"
exit 91
EOF
    chmod +x "$repo/scripts/gate.sh" "$repo/scripts/release.sh" \
        "$repo/scripts/bump-workspace-version.py"

    git -C "$repo" init --initial-branch=main >/dev/null
    git -C "$repo" config user.name "Suprnova Release Test"
    git -C "$repo" config user.email "release-test@suprnova.invalid"
    git -C "$repo" add .
    git -C "$repo" commit -m "test: clean release source" >/dev/null
    git init --bare "$remote" >/dev/null
    git -C "$repo" remote add origin "$remote"
    git -C "$repo" push -u origin main >/dev/null
    baseline="$(git --git-dir="$remote" rev-parse refs/heads/main)"

    printf 'fn main() {}\n' >"$repo/src/bin/release_probe.rs"
    set +e
    (
        cd "$repo"
        RELEASE_TEST_GATE_LOG="$gate_marker" scripts/release.sh 1.2.4
    ) >"$output" 2>&1
    local status=$?
    set -e

    [[ $status -ne 0 ]] || fail "release accepted an untracked auto-discovered binary"
    grep -Fq 'working tree is dirty' "$output" \
        || fail "untracked-file rejection did not report a dirty worktree"
    [[ ! -e "$gate_marker" ]] || fail "release gate ran after untracked-file rejection"
    [[ "$(git -C "$repo" rev-parse HEAD)" == "$baseline" ]] \
        || fail "release committed after untracked-file rejection"
    [[ -z "$(git -C "$repo" tag)" ]] \
        || fail "release tagged after untracked-file rejection"
    [[ "$(git --git-dir="$remote" rev-parse refs/heads/main)" == "$baseline" ]] \
        || fail "release pushed after untracked-file rejection"
    grep -Fq 'version = "1.2.3"' "$repo/Cargo.toml" \
        || fail "release edited the version after untracked-file rejection"

    rm "$repo/src/bin/release_probe.rs"
    : >"$repo/ignored-release-artifact"
    set +e
    (
        cd "$repo"
        RELEASE_TEST_GATE_LOG="$gate_marker" scripts/release.sh 1.2.4
    ) >>"$output" 2>&1
    status=$?
    set -e

    [[ $status -eq 91 ]] || fail "ignored file prevented the release from reaching its gate"
    grep -Fqx 'gate --full' "$gate_marker" \
        || fail "ignored file did not remain excluded from release cleanliness"
}

test_audit_is_required_and_fail_closed
test_feature_matrix_runs_all_rustdoc_profiles
test_release_dry_run_stops_before_bump_when_gate_fails
test_strict_semver_validation
test_release_requires_strictly_increasing_semver
test_full_gate_includes_release_security_profiles
test_release_smokes_are_version_agnostic_and_isolated
test_normal_release_rejects_untracked_files_before_side_effects

echo "Release hardening tests passed."
