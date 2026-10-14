#!/usr/bin/env bash
# Create the Python venv manual_check.py runs in.
#
# Usage: feature-map/tools/manual_check_env.sh [venv-dir]   (default: target/feature-map/venv)
# Then:  <venv-dir>/bin/python feature-map/tools/manual_check.py . target/feature-map/manual-check.jsonl
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
VENV=${1:-$REPO/target/feature-map/venv}
python3 -m venv "$VENV"
"$VENV/bin/pip" install -q markdown-it-py==4.2.0 tree-sitter==0.26.0 tree-sitter-rust==0.24.2
echo "venv ready at $VENV"
