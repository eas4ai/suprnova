#!/usr/bin/env bash
# Verify Suprnova's public feature boundaries with Cargo's resolver.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

MINIMAL_FEATURES="database-sqlite,database-postgres,broadcasting-fanout"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

run() {
    printf '\n==> %s\n' "$1"
    shift
    printf '    +'
    printf ' %q' "$@"
    printf '\n'
    "$@"
}

write_tree() {
    local name=$1
    local features=$2

    cargo tree \
        -p suprnova \
        --no-default-features \
        --features "$features" \
        --edges normal,build \
        --prefix none > "$TMP_DIR/$name.tree"
}

# The tree a plain `cargo build` resolves — no --no-default-features.
#
# Every other tree here deliberately strips defaults to isolate one
# feature. The RUSTSEC exception in `.cargo/audit.toml` rests on a claim
# about the *default* build specifically, so it needs its own tree.
write_default_tree() {
    cargo tree \
        -p suprnova \
        --edges normal,build \
        --prefix none > "$TMP_DIR/default.tree"
}

# Every Suprnova feature at once. This is the widest resolution a direct
# framework consumer can ask for, but it does not activate optional features
# declared by transitive dependencies.
write_all_features_tree() {
    cargo tree \
        -p suprnova \
        --all-features \
        --edges normal,build \
        --prefix none > "$TMP_DIR/all-features.tree"
}

# The broadest tree this workspace can compile: every member, workspace
# feature, target-specific edge, and dev/build/normal dependency. Cargo.lock
# can still contain dormant optional dependencies that are absent here; this
# tree is the reachability proof for exceptions based on that distinction.
write_workspace_all_targets_tree() {
    cargo tree \
        --workspace \
        --all-features \
        --target all \
        --edges all \
        --prefix none > "$TMP_DIR/workspace-all-targets.tree"
}

write_test_list() {
    local target=$1

    cargo test \
        -p suprnova \
        --no-default-features \
        --features "$MINIMAL_FEATURES" \
        --test "$target" \
        -- \
        --list \
        --format terse > "$TMP_DIR/$target.tests"
}

tree_has_package() {
    local tree=$1
    local package=$2

    awk -v package="$package" '$1 == package { found = 1 } END { exit !found }' "$tree"
}

tree_has_package_prefix() {
    local tree=$1
    local prefix=$2

    awk -v prefix="$prefix" \
        'index($1, prefix) == 1 { found = 1 } END { exit !found }' "$tree"
}

assert_present() {
    local tree=$1
    local package=$2

    if ! tree_has_package "$tree" "$package"; then
        echo "expected $package in $(basename "$tree")" >&2
        exit 1
    fi
}

assert_absent() {
    local tree=$1
    local package=$2

    if tree_has_package "$tree" "$package"; then
        echo "unexpected $package in $(basename "$tree")" >&2
        exit 1
    fi
}

assert_prefix_absent() {
    local tree=$1
    local prefix=$2

    if tree_has_package_prefix "$tree" "$prefix"; then
        echo "unexpected package prefix $prefix in $(basename "$tree")" >&2
        exit 1
    fi
}

tree_has_package_version() {
    local tree=$1
    local package=$2
    local version_prefix=$3

    awk -v package="$package" -v prefix="$version_prefix" \
        '$1 == package && index($2, prefix) == 1 { found = 1 } END { exit !found }' "$tree"
}

assert_version_absent() {
    local tree=$1
    local package=$2
    local version_prefix=$3

    if tree_has_package_version "$tree" "$package" "$version_prefix"; then
        echo "unexpected $package $version_prefix in $(basename "$tree")" >&2
        exit 1
    fi
}

assert_version_present() {
    local tree=$1
    local package=$2
    local version_prefix=$3

    if ! tree_has_package_version "$tree" "$package" "$version_prefix"; then
        echo "expected $package $version_prefix in $(basename "$tree")" >&2
        exit 1
    fi
}

assert_test_listed() {
    local target=$1
    local test_name=$2

    if ! awk -v test_name="$test_name" \
        '$0 == test_name ": test" { found = 1 } END { exit !found }' \
        "$TMP_DIR/$target.tests"; then
        echo "expected $test_name in the minimal $target test target" >&2
        exit 1
    fi
}

assert_test_not_listed() {
    local target=$1
    local test_name=$2

    if awk -v test_name="$test_name" \
        '$0 == test_name ": test" { found = 1 } END { exit !found }' \
        "$TMP_DIR/$target.tests"; then
        echo "unexpected $test_name in the minimal $target test target" >&2
        exit 1
    fi
}

run "zero-driver profile" \
    cargo check -p suprnova --no-default-features
run "zero-driver rustdoc" \
    cargo doc -p suprnova --no-default-features --no-deps
run "testing-only profile" \
    cargo check -p suprnova --no-default-features --features testing
run "SQLite-only profile" \
    cargo check -p suprnova --no-default-features --features database-sqlite
run "Postgres-only profile" \
    cargo check -p suprnova --no-default-features --features database-postgres
run "MySQL-only profile" \
    cargo check -p suprnova --no-default-features --features database-mysql
run "filesystem-only profile" \
    cargo check -p suprnova --no-default-features --features filesystem
run "filesystem+azure profile" \
    cargo check -p suprnova --no-default-features --features filesystem-azure
run "filesystem+gcs profile" \
    cargo check -p suprnova --no-default-features --features filesystem-gcs
# `lib.rs` denies `rustdoc::broken_intra_doc_links`, and the Azure/GCS
# constructors are `#[cfg]`-gated. Doc links that resolve only when a
# feature is on — or only when it is off — break exactly one of these two
# builds, so both have to run.
#
# The filesystem-only run is not redundant with the zero-driver and
# default runs above: it is the first configuration to document the
# storage module *without* `testing`, and it was failing on seven
# unresolved `Storage::fake` links until this step went in. `testing` is a
# default feature, so nothing else here ever built that combination.
run "filesystem-only rustdoc" \
    cargo doc -p suprnova --no-default-features --features filesystem --no-deps
run "filesystem+azure+gcs rustdoc" \
    cargo doc -p suprnova --no-default-features \
        --features filesystem-azure,filesystem-gcs --no-deps
run "web-push-only profile" \
    cargo check -p suprnova --no-default-features --features web-push
run "vector-mariadb-only profile" \
    cargo check -p suprnova --no-default-features --features vector-mariadb
run "localization-only profile" \
    cargo check -p suprnova --no-default-features --features localization
# Same doc-link trap as the filesystem runs above: `localization` is
# default-on, so "default rustdoc" and "all-features rustdoc" only ever
# document it alongside `testing` and the rest of the default set. This is
# the first configuration to document the localization module with nothing
# else on — a doc link from `Lang` or the middleware to a `testing`-gated
# or driver-gated item resolves in every other build and breaks only here.
run "localization-only rustdoc" \
    cargo doc -p suprnova --no-default-features --features localization --no-deps
run "Nation X minimal profile" \
    cargo check -p suprnova --no-default-features --features "$MINIMAL_FEATURES"
run "Nation X minimal test targets" \
    cargo check -p suprnova --no-default-features --features "$MINIMAL_FEATURES" --tests
run "filesystem-off doctests" \
    cargo test -p suprnova --no-default-features --features "$MINIMAL_FEATURES" --doc
run "enumerate minimal encrypted-cast tests" \
    write_test_list eloquent_casts_encrypted
run "enumerate minimal encryption tests" \
    write_test_list encryption
run "enumerate minimal remember-me tests" \
    write_test_list remember_me

assert_test_listed eloquent_casts_encrypted as_hashed_writes_bcrypt_and_does_not_decrypt
assert_test_listed eloquent_casts_encrypted as_hashed_is_idempotent_across_re_saves
assert_test_not_listed eloquent_casts_encrypted as_encrypted_round_trips_and_storage_is_ciphertext
assert_test_listed encryption appears_encrypted_rejects_plaintext_and_short_payloads
assert_test_not_listed encryption round_trip_string
assert_test_listed remember_me forget_remember_cookie_clears_the_cookie
assert_test_not_listed remember_me login_remember_issues_cookie_and_persists_token

run "default profile" \
    cargo check -p suprnova
run "default rustdoc" \
    cargo doc -p suprnova --no-deps
run "all-features profile" \
    cargo check -p suprnova --all-features
run "all-features rustdoc" \
    cargo doc -p suprnova --all-features --no-deps

run "resolve SQLite-only dependency tree" \
    write_tree sqlite database-sqlite
run "resolve Postgres-only dependency tree" \
    write_tree postgres database-postgres
run "resolve MySQL-only dependency tree" \
    write_tree mysql database-mysql
run "resolve Nation X minimal dependency tree" \
    write_tree minimal "$MINIMAL_FEATURES"
run "resolve default-features dependency tree" \
    write_default_tree
run "resolve Pinecone opt-in dependency tree" \
    write_tree pinecone vector-pinecone
run "resolve filesystem-only dependency tree" \
    write_tree filesystem filesystem
run "resolve filesystem+azure dependency tree" \
    write_tree filesystem-azure filesystem-azure
run "resolve filesystem+gcs dependency tree" \
    write_tree filesystem-gcs filesystem-gcs
run "resolve localization-only dependency tree" \
    write_tree localization localization
run "resolve all-features dependency tree" \
    write_all_features_tree
run "resolve workspace all-features/all-targets dependency tree" \
    write_workspace_all_targets_tree

assert_present "$TMP_DIR/sqlite.tree" sqlx-sqlite
assert_absent "$TMP_DIR/sqlite.tree" sqlx-postgres
assert_absent "$TMP_DIR/sqlite.tree" sqlx-mysql
assert_present "$TMP_DIR/postgres.tree" sqlx-postgres
assert_absent "$TMP_DIR/postgres.tree" sqlx-sqlite
assert_absent "$TMP_DIR/postgres.tree" sqlx-mysql
assert_present "$TMP_DIR/mysql.tree" sqlx-mysql
assert_absent "$TMP_DIR/mysql.tree" sqlx-sqlite
assert_absent "$TMP_DIR/mysql.tree" sqlx-postgres

assert_present "$TMP_DIR/minimal.tree" sqlx-sqlite
assert_present "$TMP_DIR/minimal.tree" sqlx-postgres
assert_absent "$TMP_DIR/minimal.tree" opendal
assert_prefix_absent "$TMP_DIR/minimal.tree" reqsign
assert_absent "$TMP_DIR/minimal.tree" suprnova-web-push
assert_absent "$TMP_DIR/minimal.tree" sqlx-mysql
assert_absent "$TMP_DIR/minimal.tree" rsa

# ---------------------------------------------------------------------------
# Localization gating (`localization`)
# ---------------------------------------------------------------------------
#
# `localization` is default-on, so what these assertions protect is the
# opt-OUT: a build without the feature must actually shed the Fluent and
# ICU4X formatting stack, or the gate is theatre. The assertions name
# specific crates rather than an `icu_` prefix on purpose — `url -> idna ->
# idna_adapter` already puts `icu_normalizer`/`icu_properties` in every
# tree that carries reqwest, and that arrival has nothing to do with this
# feature. The crates asserted absent below reach the graph only through
# the `localization` feature's `dep:` list.
assert_absent "$TMP_DIR/minimal.tree" fluent-bundle
assert_absent "$TMP_DIR/minimal.tree" fluent-langneg
assert_absent "$TMP_DIR/minimal.tree" icu_datetime
assert_absent "$TMP_DIR/minimal.tree" icu_decimal
assert_absent "$TMP_DIR/minimal.tree" icu_experimental
assert_absent "$TMP_DIR/minimal.tree" intl-memoizer

# Opting in delivers the full formatting surface.
assert_present "$TMP_DIR/localization.tree" fluent-bundle
assert_present "$TMP_DIR/localization.tree" fluent-langneg
assert_present "$TMP_DIR/localization.tree" icu_datetime
assert_present "$TMP_DIR/localization.tree" icu_decimal
assert_present "$TMP_DIR/localization.tree" icu_experimental
assert_present "$TMP_DIR/localization.tree" fixed_decimal

# ---------------------------------------------------------------------------
# RUSTSEC exception scope (.cargo/audit.toml)
# ---------------------------------------------------------------------------
#
# RUSTSEC-2026-0235 applies to rkyv 0.7.46. Cargo.lock records rkyv only
# because rust_decimal declares an optional compatibility feature pinned to
# rkyv 0.7; no workspace feature activates it. Check the broadest resolvable
# tree so a future normal, build, dev, target-specific, or workspace-member
# edge cannot make the exception's reachability claim stale.
assert_absent "$TMP_DIR/workspace-all-targets.tree" rkyv
assert_absent "$TMP_DIR/workspace-all-targets.tree" rkyv_derive

# `.cargo/audit.toml` used to ignore four rustls-webpki advisories —
# RUSTSEC-2026-0049 / -0098 / -0099 / -0104 — on the claim that the
# vulnerable `rustls-webpki 0.102.x` reached the graph only through
# `pinecone-sdk`, itself behind the off-by-default `vector-pinecone`
# feature. All four ignores are gone: the Pinecone driver was rewritten
# against Pinecone's REST API and `pinecone-sdk` left the tree entirely,
# taking `tonic 0.11 -> rustls 0.22 -> rustls-webpki 0.102` with it.
#
# These assertions are what stop that from silently regressing. Re-adding
# the SDK — or any dependency dragging `tonic 0.11` back in — fails here
# first, while it is still one revert away.
assert_absent "$TMP_DIR/all-features.tree" pinecone-sdk
assert_version_absent "$TMP_DIR/all-features.tree" rustls-webpki v0.102
assert_version_absent "$TMP_DIR/all-features.tree" tonic v0.11

# The Pinecone feature must stay dependency-free: it gates compilation of
# a driver that talks REST over the `reqwest` client the framework
# already carries. If opting in ever starts adding crates again, that is
# the moment to re-examine what they drag with them.
assert_absent "$TMP_DIR/pinecone.tree" pinecone-sdk
assert_absent "$TMP_DIR/default.tree" pinecone-sdk
assert_present "$TMP_DIR/pinecone.tree" reqwest

# The workspace resolves one rustls-webpki, and it is the patched line.
assert_version_present "$TMP_DIR/all-features.tree" rustls-webpki v0.103

# ---------------------------------------------------------------------------
# Azure / GCS gating (`filesystem-azure`, `filesystem-gcs`)
# ---------------------------------------------------------------------------
#
# The whole point of splitting these out of `filesystem` is that `rsa` —
# RUSTSEC-2023-0071, the Marvin timing attack, no fixed release upstream —
# becomes avoidable. It was not before: `filesystem` meant opendal, and
# opendal was configured with `services-azblob` and `services-gcs`
# unconditionally, so the only way to shed `rsa` was to give up storage.
#
# The two service crates reach `rsa` by taking `reqsign-core` with its
# `jwt` feature (`reqsign-core`'s `rsa` is optional behind exactly that),
# and `reqsign-azure-storage` also depends on `rsa` directly. Asserting on
# `rsa` rather than on the reqsign crates is deliberate — a future opendal
# could reshuffle its signers, and it is `rsa`'s presence that the
# advisory is about.
assert_present "$TMP_DIR/filesystem.tree" opendal
assert_absent "$TMP_DIR/filesystem.tree" rsa
assert_prefix_absent "$TMP_DIR/filesystem.tree" reqsign-azure
assert_prefix_absent "$TMP_DIR/filesystem.tree" reqsign-google

# S3 is NOT gated, and must not drift into being gated by accident:
# `reqsign-aws-v4` takes `reqsign-core` without `jwt`, so S3 never cost an
# `rsa`. Gating it would break the most-used cloud backend and remove no
# dependency. If this assertion ever fails, someone has moved S3 behind a
# feature — check whether they had a reason better than symmetry.
assert_present "$TMP_DIR/filesystem.tree" opendal-service-s3

# Opting in re-admits the advisory. That is the trade, made knowingly by
# an app that actually stores objects there — so assert it happens, or the
# features are gating nothing and the split is theatre.
assert_present "$TMP_DIR/filesystem-azure.tree" rsa
assert_present "$TMP_DIR/filesystem-azure.tree" opendal-service-azblob
assert_present "$TMP_DIR/filesystem-gcs.tree" rsa
assert_present "$TMP_DIR/filesystem-gcs.tree" opendal-service-gcs

# Neither feature drags the other in.
assert_absent "$TMP_DIR/filesystem-azure.tree" opendal-service-gcs
assert_absent "$TMP_DIR/filesystem-gcs.tree" opendal-service-azblob

echo
echo "Feature matrix passed."
