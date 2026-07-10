#!/usr/bin/env bash

set -euo pipefail

MSRV="1.91.1"
REPO_ROOT="$(git rev-parse --show-toplevel)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
cd "$REPO_ROOT"

if ! rustup run "$MSRV" rustc --version >/dev/null 2>&1; then
    echo "error: Rust $MSRV is required for the MSRV gate" >&2
    echo "       install it with: rustup toolchain install $MSRV --profile minimal" >&2
    exit 1
fi

cargo metadata --no-deps --format-version 1 >"$TMP_DIR/metadata.json"
python3 - "$TMP_DIR/metadata.json" "$MSRV" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    packages = json.load(handle)["packages"]

expected = sys.argv[2]
mismatches = [
    f"{package['name']}={package.get('rust_version')!r}"
    for package in packages
    if package.get("rust_version") != expected
]
if mismatches:
    raise SystemExit(
        "workspace packages must declare rust-version "
        f"{expected}: {', '.join(mismatches)}"
    )
print(f"workspace rust-version={expected} ({len(packages)} packages)")
PY

echo "==> cargo +$MSRV check: Suprnova filesystem profile"
cargo +"$MSRV" check \
    -p suprnova \
    --locked \
    --no-default-features \
    --features filesystem

echo "Rust $MSRV MSRV check passed."
