#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

run_live_test() {
    local target_type=$1
    local target=$2
    local test_name=$3

    printf 'Running live test: %s %s %s\n' "$target_type" "$target" "$test_name"

    case "$target_type" in
        test)
            cargo test --test "$target" --all-features "$test_name" -- --ignored --exact
            ;;
        lib)
            cargo test --lib --all-features "$test_name" -- --ignored --exact
            ;;
        *)
            printf 'Invalid live test target type: %s\n' "$target_type" >&2
            exit 1
            ;;
    esac
}

if [[ "${1-}" == "--live" ]]; then
    shift || true
    : "${MAGNETAR_POSTGRES_TEST_URL:?MAGNETAR_POSTGRES_TEST_URL is required for --live gate}"
    : "${MAGNETAR_MYSQL_TEST_URL:?MAGNETAR_MYSQL_TEST_URL is required for --live gate}"

    run_live_test test default_schema_backends postgres_default_schema_is_replay_safe
    run_live_test test default_schema_backends postgres_api_import_advances_the_default_user_sequence
    run_live_test test default_schema_backends mysql_default_schema_is_replay_safe
    run_live_test test foundation_gate postgres_backend_is_reachable
    run_live_test test foundation_gate mysql_backend_is_reachable
    run_live_test test seaorm_upgrade_compat postgres_upgrade_from_seaorm_1_1_is_replay_safe
    run_live_test test seaorm_upgrade_compat mysql_upgrade_from_seaorm_1_1_is_replay_safe
    run_live_test test storage_tokens configured_postgres_target_is_required
    run_live_test test storage_tokens configured_mysql_target_is_required
    run_live_test test token_broker_concurrency two_pod_convergence_postgres
    run_live_test test token_broker_concurrency two_pod_convergence_mysql
    run_live_test lib _ migration::mysql_swap_tests::plan_bound_coordinator_revalidates_imports_swaps_cleans_and_releases_barrier
    run_live_test lib _ migration::seaorm_upgrade_tests::postgres_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1
    run_live_test lib _ migration::seaorm_upgrade_tests::mysql_source_catalog_is_idempotent_when_upgrading_from_seaorm_1_1

    exit 0
fi

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

# PostgreSQL and MySQL suites are manual `#[ignore]`d qualification tests.
# Run their individual test targets while changing a backend-specific boundary;
# the permanent gate stays self-contained and never requires live services.

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
