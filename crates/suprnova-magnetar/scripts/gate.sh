#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

: "${MAGNETAR_POSTGRES_TEST_URL:?MAGNETAR_POSTGRES_TEST_URL must be set}"
: "${MAGNETAR_MYSQL_TEST_URL:?MAGNETAR_MYSQL_TEST_URL must be set}"

if ! command -v jq >/dev/null 2>&1; then
    printf 'Required command not found: jq. Install it before running the gate.\n' >&2
    exit 1
fi
if ! command -v cargo-nextest >/dev/null 2>&1; then
    printf 'Required command not found: cargo-nextest. Install it before running the gate.\n' >&2
    exit 1
fi

METADATA="$(cargo metadata --no-deps --format-version 1)"
if jq -e '
    [
      .packages[]?.targets[]?
      | select(.name == "concurrency" and (.kind | index("test")))
    ] | length > 0
  ' >/dev/null <<<"$METADATA"; then
    RUN_CONCURRENCY_TEST=true
else
    RUN_CONCURRENCY_TEST=false
fi

printf 'Running all-target checks...\n'
cargo check --all-targets --all-features

printf 'Running formatting check...\n'
cargo fmt --all -- --check

printf 'Running lint check...\n'
cargo clippy --all-targets --all-features

printf 'Running the CI test profile...\n'
cargo nextest run --profile ci --all-features

printf 'Running documentation tests...\n'
cargo test --doc --all-features

if [[ "$RUN_CONCURRENCY_TEST" == true ]]; then
    printf 'Running the concurrency test target...\n'
    cargo nextest run --profile ci --test concurrency --all-features
else
    printf 'Concurrency test target not yet configured; continuing.\n'
fi

printf 'Checking example host formatting...\n'
cargo fmt --manifest-path examples/host/Cargo.toml -- --check
printf 'Checking example host...\n'
cargo check --manifest-path examples/host/Cargo.toml
printf 'Linting example host...\n'
cargo clippy --manifest-path examples/host/Cargo.toml
printf 'Running credential-only host smoke...\n'
cargo run --manifest-path examples/host/Cargo.toml -- --smoke-credentials
printf 'Running the fail-closed feature matrix...\n'
exec "$ROOT_DIR/scripts/check-feature-matrix.sh"
