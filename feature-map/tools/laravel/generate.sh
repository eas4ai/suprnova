#!/usr/bin/env bash
# Regenerate the Laravel surface from laravel/framework source.
#
# Usage: feature-map/tools/laravel/generate.sh [laravel-version] [work-dir]
#
# Clones laravel/framework at v<version> and laravel/docs (13.x) pinned to
# the last commit on or before that release's date, installs the framework's
# Composer dependencies (dev included, so testing classes load), and reads
# the surface through PHP reflection. Needs PHP 8.3+, Composer and git.
# Writes feature-map/laravel/{surface.jsonl,meta.json,exclusions.json};
# never touches feature-map/laravel/parity.jsonl.
set -euo pipefail
export PYTHONDONTWRITEBYTECODE=1
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../../.." && pwd)
OUT=$REPO/feature-map/laravel
VERSION=${1:-$(python3 -c "import json;print(json.load(open('$OUT/meta.json'))['laravel_version'])" 2>/dev/null || echo 13.33.0)}
WORK=${2:-$REPO/target/feature-map/laravel}
FW=$WORK/framework-$VERSION
DOCS=$WORK/docs
mkdir -p "$WORK"

if [ ! -d "$FW/.git" ]; then
  git clone -q --depth 1 --branch "v$VERSION" https://github.com/laravel/framework.git "$FW"
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

php "$HERE/extract.php" "$FW" "$WORK/raw.jsonl" "$WORK/extract-exclusions.json"
python3 "$HERE/build.py" "$WORK/raw.jsonl" "$WORK/extract-exclusions.json" "$DOCS" \
  "$(git -C "$DOCS" rev-parse --short "$DOCS_REV")" "$VERSION" "$OUT"
echo "Laravel $VERSION surface regenerated"
