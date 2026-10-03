#!/usr/bin/env bash
# Regenerate the Rust API maps from rustdoc JSON.
#
# Usage: generate_rust.sh <repo> <rustdoc-json-dir> <map-dir>
# The JSON dir holds <lib>.all.json and <lib>.default.json per crate and
# suprnova_macros.json, produced by `cargo +nightly rustdoc -p <crate>
# [--all-features] -- -Z unstable-options --output-format json --document-hidden-items`.
set -euo pipefail
REPO=$1
J=$2
OUT=$3
HERE=$(cd "$(dirname "$0")" && pwd)
REV=${REV:-$(git -C "$REPO" rev-parse --short HEAD)}
mkdir -p "$OUT"

# crate-dir  lib-name  map-file  siblings (lib names whose public paths resolve trait names)
while read -r dir lib file sibs; do
  args=()
  for s in $sibs; do
    [ "$s" = "-" ] && continue
    if [ "$s" = suprnova_macros ]; then args+=(--sibling "$J/$s.json"); else args+=(--sibling "$J/$s.all.json"); fi
  done
  if [ "$lib" = suprnova_macros ]; then
    json="$J/$lib.json"; def=(--vocab "${VOCAB:?set VOCAB to the macro vocabulary JSON}"); note="rustdoc JSON; macro arguments read from each macro's parser."
  else
    json="$J/$lib.all.json"; def=(--default "$J/$lib.default.json")
    note="rustdoc JSON (all features), cross-checked against a default-features build."
  fi
  (cd "$REPO" && python3 "$HERE/rust_api.py" "$json" "${def[@]}" "${args[@]}" \
    --manifest "$dir/Cargo.toml" --out "$OUT/$file.jsonl") > "$OUT/.$file.report.json"
done <<'EOF'
framework suprnova suprnova suprnova_live magnetar suprnova_web_push suprnova_macros
crates/suprnova-live suprnova_live suprnova-live -
crates/suprnova-magnetar magnetar suprnova-magnetar -
crates/suprnova-payments-stripe suprnova_payments_stripe suprnova-payments-stripe suprnova
crates/suprnova-payments-paddle suprnova_payments_paddle suprnova-payments-paddle suprnova
crates/suprnova-payments-nowpayments suprnova_payments_nowpayments suprnova-payments-nowpayments suprnova
crates/suprnova-web-push suprnova_web_push suprnova-web-push suprnova
suprnova-macros suprnova_macros suprnova-macros -
EOF

# Any feature-gate disagreement between the gate labels and a default build is a hard error.
python3 - "$OUT" <<'PY'
import glob, json, sys
bad = []
for f in glob.glob(sys.argv[1] + "/.*.report.json"):
    bad += json.load(open(f))["gate_mismatches"]
if bad:
    sys.exit("feature gate mismatches:\n" + "\n".join(bad))
PY

