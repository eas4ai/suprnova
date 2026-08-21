#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# FAIL_CLOSED_FEATURE_GATE: derive the named-feature matrix from Cargo metadata
# so newly declared features cannot silently avoid validation.
METADATA="$(cargo metadata --no-deps --format-version 1)"
PACKAGE_JSON="$(jq -ce '
  [ .packages[] | select(.name == "suprnova-magnetar") ] |
  if length == 1 then .[0] else error("expected exactly one suprnova-magnetar package") end
' <<<"$METADATA")" || {
  printf 'Cargo metadata did not contain exactly one suprnova-magnetar package.\n' >&2
  exit 1
}
FEATURE_NAMES="$(jq -r '.features | keys[] | select(. != "default")' <<<"$PACKAGE_JSON")"
if [[ -z "$FEATURE_NAMES" ]]; then
  printf 'Cargo metadata declared no named features; refusing an empty matrix.\n' >&2
  exit 1
fi
mapfile -t FEATURES <<<"$FEATURE_NAMES"

printf '> cargo check --all-targets --no-default-features\n'
cargo check --all-targets --no-default-features

printf '> cargo check --all-targets\n'
cargo check --all-targets

printf '> cargo check --all-targets --all-features\n'
cargo check --all-targets --all-features

for feature in "${FEATURES[@]}"; do
  if [[ -z "$feature" ]]; then
    printf 'Cargo metadata returned an empty feature name; refusing a partial matrix.\n' >&2
    exit 1
  fi
  printf '> cargo check --all-targets --no-default-features --features %s\n' "$feature"
  cargo check --all-targets --no-default-features --features "$feature"
done

# Keep forbidden provider dependencies disabled in the foundation crate. Match
# complete package-name tokens rather than rejecting similarly named packages.
# This covers torii-core, torii-storage-seaorm, torii-migration, torii-axum,
# suprnova-core, and oauth2-broker-core.
readonly DISABLED_PROVIDER_NAMES=(
  "suprnova"
  "torii"
  "oauth2-broker"
)

validate_tree() {
  local tree_output="$1"
  local tree_package_names
  tree_package_names="$(
    awk '
      {
        if ($0 ~ /^[^[:alnum:]]*\[(build|dev)-dependencies\][[:space:]]*$/) {
          next
        }
        line = $0
        sub(/^[^[:alnum:]]+/, "", line)
        if (line ~ /^[[:space:]]*$/) {
          next
        }
        split(line, fields, /[[:space:]]+/)
        if (fields[1] !~ /^[[:alnum:]][[:alnum:]_.-]*$/ ||
            fields[2] !~ /^v[0-9]/) {
          printf "Unable to extract a package name from cargo tree line: %s\n", $0 > "/dev/stderr"
          exit 1
        }
        print fields[1]
      }
    ' <<<"$tree_output"
  )" || {
    printf 'Could not parse cargo tree package names; refusing to skip dependency validation.\n' >&2
    return 1
  }
  if [[ -z "$tree_package_names" ]]; then
    printf 'Cargo tree did not contain any package names; refusing to skip dependency validation.\n' >&2
    return 1
  fi

  local root_seen=false
  local package_name provider
  mapfile -t tree_packages <<<"$tree_package_names"
  for package_name in "${tree_packages[@]}"; do
    if [[ "$package_name" == "suprnova-magnetar" ]]; then
      root_seen=true
      continue
    fi
    for provider in "${DISABLED_PROVIDER_NAMES[@]}"; do
      case "$package_name" in
        "$provider"|"$provider"-*)
          printf 'Disabled provider dependency found in cargo tree: %s\n' "$package_name" >&2
          return 1
          ;;
      esac
    done
  done
  if [[ "$root_seen" != true ]]; then
    printf 'Cargo tree did not contain the suprnova-magnetar root package.\n' >&2
    return 1
  fi
}

for tree_args in "--no-default-features" "--all-features"; do
  printf '> cargo tree %s --quiet\n' "$tree_args"
  if [[ "$tree_args" == "--no-default-features" ]]; then
    TREE_OUTPUT="$(cargo tree --no-default-features --quiet)"
  else
    TREE_OUTPUT="$(cargo tree --all-features --quiet)"
  fi
  validate_tree "$TREE_OUTPUT"
done
