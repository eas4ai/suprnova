#!/usr/bin/env bash
# Installed compatibility entry point for the classified local gate.
#
# Usage:
#   scripts/gate.sh
#   scripts/gate.sh --full

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

case "${1-}" in
    "")
        exec python3 scripts/gate-runner.py
        ;;
    --full)
        if [[ $# -ne 1 ]]; then
            printf 'usage: scripts/gate.sh [--full]\n' >&2
            exit 2
        fi
        exec python3 scripts/gate-runner.py --full
        ;;
    *)
        printf 'usage: scripts/gate.sh [--full]\n' >&2
        exit 2
        ;;
esac
