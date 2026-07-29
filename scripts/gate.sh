#!/usr/bin/env bash
# Canonical local CI gate for Suprnova.
#
# GitHub Actions auto-runs are disabled during development (see
# .github/workflows/ci.yml); this script IS the gate until the CI flip at
# launch. It mirrors ci.yml's jobs exactly so flipping CI back on later
# changes nothing about what "green" means.
#
# Usage:
#   scripts/gate.sh           # default gate (pre-push enforced)
#   scripts/gate.sh --full    # + MSRV, feature sets, and security audits
#
# On success with a clean working tree, the tree hash is stamped to
# git's suprnova-gate-pass path; the pre-push hook (.githooks/pre-push) skips
# re-running the gate when the stamp matches HEAD's tree, so the usual
# flow — commit, gate, push — runs the suite once, not twice.
#
# Emergency escape is `git push --no-verify`. Don't.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

FULL=0
if [[ "${1:-}" == "--full" ]]; then
    FULL=1
elif [[ -n "${1:-}" ]]; then
    echo "usage: scripts/gate.sh [--full]" >&2
    exit 2
fi

GATE_START=$SECONDS

step() {
    local name=$1
    shift
    local start=$SECONDS
    echo
    echo "==> ${name}"
    if ! "$@"; then
        echo
        echo "GATE FAILED at: ${name}" >&2
        echo "    (command: $*)" >&2
        exit 1
    fi
    echo "    ok (${name}, $((SECONDS - start))s)"
}

# Style + lint first: they return in seconds and nothing else is worth
# checking on a tree that doesn't pass them. RUSTFLAGS is NOT set here
# (ci.yml sets it for belt-and-braces); clippy's -D warnings already
# covers rustc warnings on every target, and leaving RUSTFLAGS alone
# preserves the local incremental cache.
step "cargo fmt --all --check" \
    cargo fmt --all --check

step "cargo clippy --workspace --all-targets (default features)" \
    cargo clippy --workspace --all-targets -- -D warnings

# Lint the opt-in feature sets so feature-gated code can't silently rot.
step "cargo clippy -p suprnova --features vector-pinecone" \
    cargo clippy -p suprnova --all-targets --features vector-pinecone -- -D warnings

step "cargo clippy -p suprnova --features broadcasting-fanout" \
    cargo clippy -p suprnova --all-targets --features broadcasting-fanout -- -D warnings

# `framework/src/lib.rs` denies `rustdoc::broken_intra_doc_links` and
# `rustdoc::private_intra_doc_links`, but rustdoc lints only fire under
# rustdoc — neither `cargo check` nor clippy runs it. Without this step
# those two denies were decorative: a doc link naming a private item, or
# one that resolves to nothing, compiled and shipped clean. (`missing_docs`
# is a rustc lint, so clippy already covers that one.)
#
# `--no-deps` keeps it to our own crate; dependency docs aren't ours to gate.
step "cargo doc -p suprnova (intra-doc links)" \
    cargo doc -p suprnova --no-deps --lib

# Functional gate on the default feature set.
step "cargo test --workspace" \
    cargo test --workspace --no-fail-fast

# Generated-project gate: scaffold a project and `cargo check` it against
# the in-tree framework. These are #[ignore]d in the normal suite (each
# compiles a whole generated project); the gate is where they run. This
# is the guard for the entire "scaffold templates drift from the
# framework API" bug class.
step "scaffold_snapshot compile checks" \
    cargo test -p suprnova-cli --test scaffold_snapshot -- --ignored

if [[ $FULL -eq 1 ]]; then
    step "Rust 1.91.1 MSRV" \
        scripts/check-msrv.sh

    step "framework feature matrix" \
        scripts/check-feature-matrix.sh

    step "cargo test -p suprnova --features vector-pinecone" \
        cargo test -p suprnova --features vector-pinecone --no-fail-fast

    step "cargo test -p suprnova --features broadcasting-fanout" \
        cargo test -p suprnova --features broadcasting-fanout --no-fail-fast

    step "cargo audit" scripts/check-audit.sh

    step "isolated downstream dependency security" \
        scripts/check-downstream-dependencies.sh

    step "normal release temp-remote smoke" \
        scripts/tests/release-normal-smoke.sh
fi

echo
echo "GATE GREEN ($((SECONDS - GATE_START))s total)"

# Stamp the exact tree that passed, but only when the working tree is
# clean — a dirty tree means the gate validated state that no commit
# pins, so the pre-push hook must re-run it.
if [[ -z "$(git status --porcelain)" ]]; then
    stamp="$(git rev-parse --git-path suprnova-gate-pass)"
    git rev-parse 'HEAD^{tree}' > "$stamp"
    echo "stamped $stamp for $(git rev-parse --short HEAD)"
else
    echo "working tree dirty — no gate stamp written (commit first, then gate, to skip the pre-push re-run)"
fi
