#!/usr/bin/env bash

set -euo pipefail

if ! command -v cargo-audit >/dev/null 2>&1; then
    echo "error: cargo-audit is required for the full release gate" >&2
    echo "       install it with: cargo install cargo-audit --locked" >&2
    exit 1
fi

cargo audit
