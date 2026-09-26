#!/usr/bin/env bash
# Regenerate the whole feature map from the source.
#
# Usage: feature-map/tools/generate.sh [work-dir]
#
# Needs a nightly toolchain for rustdoc JSON (`rustup toolchain install
# nightly --profile minimal`); it does not change the project's pinned
# toolchain. Builds go to <work-dir> (default: target/feature-map), so the
# project's own target directory is untouched. Checked boxes are preserved.
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
MAP=$(cd "$HERE/.." && pwd)
WORK=${1:-$REPO/target/feature-map}
J=$WORK/rustdoc
mkdir -p "$J"
REV=$(git -C "$REPO" rev-parse --short HEAD)
cd "$REPO"

# 1. rustdoc JSON per crate: all features, and default features for the gate cross-check.
export CARGO_TARGET_DIR=$WORK/target-nightly
RD=(-Z unstable-options --output-format json)
for spec in suprnova:suprnova suprnova-live:suprnova_live suprnova-magnetar:magnetar \
            suprnova-payments-stripe:suprnova_payments_stripe \
            suprnova-payments-paddle:suprnova_payments_paddle \
            suprnova-payments-nowpayments:suprnova_payments_nowpayments \
            suprnova-web-push:suprnova_web_push; do
  pkg=${spec%%:*}; lib=${spec##*:}
  cargo +nightly rustdoc -q -p "$pkg" --all-features -- "${RD[@]}"
  mv "$CARGO_TARGET_DIR/doc/$lib.json" "$J/$lib.all.json"
  cargo +nightly rustdoc -q -p "$pkg" -- "${RD[@]}"
  mv "$CARGO_TARGET_DIR/doc/$lib.json" "$J/$lib.default.json"
done
cargo +nightly rustdoc -q -p suprnova-macros -- "${RD[@]}"
mv "$CARGO_TARGET_DIR/doc/suprnova_macros.json" "$J/suprnova_macros.json"
unset CARGO_TARGET_DIR

# 2. Evidence the maps are built from.
python3 "$HERE/macro_keywords.py" "$REPO" > "$WORK/kw.json"
python3 "$HERE/macro_vocab.py" "$REPO" "$WORK/kw.json" > "$WORK/vocab.json"
python3 "$HERE/env_vars.py" "$REPO" > "$WORK/env.json"

# 3. Maps.
VOCAB="$WORK/vocab.json" "$HERE/generate_rust.sh" "$REPO" "$J" "$MAP"
python3 "$HERE/live_surface.py" "$REPO" "$REV" "$MAP/live-templates.md"
python3 "$HERE/config_surface.py" "$REPO" "$REV" "$WORK/env.json" "$MAP/configuration.md"
python3 "$HERE/runtime_surface.py" "$REPO" "$REV" "$MAP/endpoints-and-tables.md"

# 4. CLI, read from the built binaries.
CARGO_TARGET_DIR=$WORK/target-stable cargo build -q -p suprnova-cli -p app --bins
python3 "$HERE/cli_surface.py" "$REPO" "$REV" "$WORK/target-stable/debug" "$MAP/cli.md"

rm -f "$MAP"/.*.report.json
echo "feature map regenerated at $REV"
