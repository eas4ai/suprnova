#!/usr/bin/env bash
# Regenerate the Laravel surface from laravel/framework, inertiajs/inertia-laravel
# and inertiajs/inertia source.
#
# Usage: feature-map/tools/laravel/generate.sh [laravel-version] [work-dir]
#
# Clones laravel/framework at v<version> and laravel/docs (13.x) pinned to
# the last commit on or before that release's date, installs the framework's
# Composer dependencies (dev included, so testing classes load), and reads
# the surface through PHP reflection. Clones inertia-laravel and Inertia.js
# at the versions meta.json records (INERTIA_LARAVEL_VERSION and
# INERTIA_VERSION override them): inertia-laravel is read through the same
# reflection, loaded beside the framework it extends, and Inertia.js through
# TypeScript's parser, installed at the version Inertia's own lockfile pins.
# Needs PHP 8.3+, Composer, git and Node with npm.
# Writes feature-map/laravel/{surface.jsonl,meta.json,exclusions.json};
# never touches feature-map/laravel/parity.jsonl.
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
OUT=$REPO/feature-map/laravel
VERSION=${1:-$(python3 -c "import json;print(json.load(open('$OUT/meta.json'))['laravel_version'])" 2>/dev/null || echo 13.33.0)}
WORK=${2:-$REPO/target/feature-map/laravel}
package_version() {  # <package> <fallback>: the version meta.json records for a package
  python3 -c "import json,sys;print(json.load(open('$OUT/meta.json'))['packages'][sys.argv[1]]['version'])" "$1" \
    2>/dev/null || echo "$2"
}
INERTIA_LARAVEL_VERSION=${INERTIA_LARAVEL_VERSION:-$(package_version inertiajs/inertia-laravel 3.5.1)}
INERTIA_VERSION=${INERTIA_VERSION:-$(package_version inertiajs/inertia 3.8.0)}
# The TypeScript Inertia 3.8.0 builds with; the run stops if its lockfile names another.
TYPESCRIPT_VERSION=6.0.3
FW=$WORK/framework-$VERSION
DOCS=$WORK/docs
INERTIA_LARAVEL=$WORK/inertia-laravel-$INERTIA_LARAVEL_VERSION
INERTIA=$WORK/inertia-$INERTIA_VERSION
TYPESCRIPT=$WORK/typescript-$TYPESCRIPT_VERSION
mkdir -p "$WORK"

if [ ! -d "$FW/.git" ]; then
  git clone -q --depth 1 --branch "v$VERSION" https://github.com/laravel/framework.git "$FW"
fi
if [ ! -d "$INERTIA_LARAVEL/.git" ]; then
  git clone -q --depth 1 --branch "v$INERTIA_LARAVEL_VERSION" https://github.com/inertiajs/inertia-laravel.git "$INERTIA_LARAVEL"
fi
if [ ! -d "$INERTIA/.git" ]; then
  git clone -q --depth 1 --branch "v$INERTIA_VERSION" https://github.com/inertiajs/inertia.git "$INERTIA"
fi
if ! grep -q "typescript@$TYPESCRIPT_VERSION" "$INERTIA/pnpm-lock.yaml"; then
  echo "Inertia $INERTIA_VERSION's lockfile does not pin typescript@$TYPESCRIPT_VERSION; update TYPESCRIPT_VERSION" >&2
  exit 1
fi
if [ ! -f "$TYPESCRIPT/node_modules/typescript/package.json" ]; then
  npm install -q --prefix "$TYPESCRIPT" --no-save --no-audit --no-fund "typescript@$TYPESCRIPT_VERSION"
fi
RELEASED=$(git -C "$FW" log -1 --format=%cI "v$VERSION")

if [ ! -d "$DOCS/.git" ]; then
  git clone -q --branch 13.x https://github.com/laravel/docs.git "$DOCS"
fi
git -C "$DOCS" fetch -q origin 13.x
DOCS_REV=$(git -C "$DOCS" rev-list -1 --before="$RELEASED" origin/13.x)
git -C "$DOCS" checkout -q "$DOCS_REV"

# Composer reaches GitHub through its API by default; plain git works everywhere.
export COMPOSER_HOME=$WORK/composer-home COMPOSER_ALLOW_SUPERUSER=1 COMPOSER_PROCESS_TIMEOUT=900
# Source installs clone every package; a cache would hold a second copy of each.
export COMPOSER_CACHE_DIR=$WORK/composer-cache
mkdir -p "$COMPOSER_HOME"
composer config -g use-github-api false
# PHPStan, Rector and Pint ship only as archives, not git sources, and none of them is
# needed to reflect Laravel's classes, so they are dropped from this throwaway clone.
(cd "$FW" && composer remove --dev --no-update -q phpstan/phpstan rector/rector laravel/pint \
  && composer update -q --no-scripts --no-interaction --prefer-source --ignore-platform-reqs \
  && composer dump-autoload -q -o)
rm -rf "$COMPOSER_CACHE_DIR"

php "$HERE/extract.php" "$FW" "$WORK/raw.jsonl" "$WORK/extract-exclusions.json" "$INERTIA_LARAVEL"
node "$HERE/inertia_js.cjs" "$TYPESCRIPT/node_modules/typescript" "$INERTIA" "$WORK/inertia-js.jsonl"
python3 "$HERE/build.py" "$WORK/raw.jsonl" "$WORK/extract-exclusions.json" "$DOCS" \
  "$(git -C "$DOCS" rev-parse --short "$DOCS_REV")" "$VERSION" "$OUT" \
  "$WORK/inertia-js.jsonl" "$INERTIA_LARAVEL_VERSION" "$INERTIA_VERSION"
echo "Laravel $VERSION, inertia-laravel $INERTIA_LARAVEL_VERSION and Inertia.js $INERTIA_VERSION surface regenerated"
