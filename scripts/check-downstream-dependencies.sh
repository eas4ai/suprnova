#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$TMP_DIR/.cargo" "$TMP_DIR/src"
cp "$REPO_ROOT/.cargo/audit.toml" "$TMP_DIR/.cargo/audit.toml"

cat >"$TMP_DIR/Cargo.toml" <<EOF
[package]
name = "suprnova-downstream-security"
version = "0.0.0"
edition = "2024"
rust-version = "1.94.0"
publish = false

[workspace]

[dependencies]
suprnova = { path = "$REPO_ROOT/framework", default-features = false, features = ["filesystem"] }
EOF

cat >"$TMP_DIR/src/main.rs" <<'EOF'
fn main() {}
EOF

echo "==> resolving isolated downstream consumer"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/downstream-security}" \
    cargo generate-lockfile --manifest-path "$TMP_DIR/Cargo.toml"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/downstream-security}" \
    cargo check --manifest-path "$TMP_DIR/Cargo.toml"
cargo metadata \
    --manifest-path "$TMP_DIR/Cargo.toml" \
    --locked \
    --format-version 1 >"$TMP_DIR/metadata.json"

python3 "$REPO_ROOT/scripts/check-downstream-dependencies.py" \
    "$TMP_DIR/metadata.json"

echo "==> auditing isolated downstream consumer"
(cd "$TMP_DIR" && cargo audit)

echo "Downstream dependency check passed."
