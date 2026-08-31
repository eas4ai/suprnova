#!/usr/bin/env bash
#
# `cargo audit`, plus enforcement of the exception policy in
# `.cargo/audit.toml`.
#
# `cargo audit` has no notion of an expiring ignore: an entry added
# "temporarily" stays until somebody re-reads the file, which is to say
# forever. Every ignore therefore carries an OWNER and an EXPIRES date,
# and this script fails once one lapses — which turns the ignore list into
# something that has to be renewed on purpose rather than inherited.

set -euo pipefail

AUDIT_TOML="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/.cargo/audit.toml"

if ! command -v cargo-audit >/dev/null 2>&1; then
    echo "error: cargo-audit is required for the full release gate" >&2
    echo "       install it with: cargo install cargo-audit --locked" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Exception policy
# ---------------------------------------------------------------------------
#
# Walks the ignore list and requires an OWNER and EXPIRES comment
# immediately before each advisory id. Missing metadata fails as hard as a
# lapsed date: an unowned exception is precisely what this exists to stop.

check_exception_policy() {
    local today
    today=$(date +%F)
    local failures=0
    local checked=0

    local owner="" expires=""
    while IFS= read -r line; do
        # Leading whitespace stripped so an entry is distinguishable from
        # the commented format example in this file's header — which the
        # first version of this parser dutifully tried to validate.
        local trimmed="${line#"${line%%[![:space:]]*}"}"

        case "$trimmed" in
            "# OWNER:"*)
                owner="${trimmed#\# OWNER: }"
                ;;
            "# EXPIRES:"*)
                expires="${trimmed#\# EXPIRES: }"
                expires="${expires%% *}"
                ;;
            '"RUSTSEC-'*)
                local id="${trimmed#\"}"
                id="${id%%\"*}"
                checked=$((checked + 1))

                if [ -z "$owner" ]; then
                    echo "error: $id has no OWNER in .cargo/audit.toml" >&2
                    failures=$((failures + 1))
                elif [ -z "$expires" ]; then
                    echo "error: $id has no EXPIRES in .cargo/audit.toml" >&2
                    failures=$((failures + 1))
                elif ! date -d "$expires" >/dev/null 2>&1; then
                    echo "error: $id has an unparseable EXPIRES ($expires)" >&2
                    failures=$((failures + 1))
                elif [[ "$expires" < "$today" ]]; then
                    echo "error: the exception for $id expired on $expires" >&2
                    echo "       owner: $owner" >&2
                    echo "       Re-check whether a fix has shipped, then either drop the" >&2
                    echo "       ignore or renew it with a new date and a stated reason." >&2
                    failures=$((failures + 1))
                fi

                # Metadata applies to a single id; the next needs its own.
                owner=""
                expires=""
                ;;
        esac
    done < "$AUDIT_TOML"

    if [ "$failures" -gt 0 ]; then
        echo "" >&2
        echo "$failures audit exception(s) failed policy. See .cargo/audit.toml." >&2
        return 1
    fi

    echo "audit exception policy: $checked ignore(s), all owned and unexpired."
}

# ---------------------------------------------------------------------------
# Dependencies eliminated on purpose
# ---------------------------------------------------------------------------
#
# `cargo audit` reports unsoundness as a *warning* and still exits 0, so
# nothing here would fail if one came back. These are the ones removed by
# choosing a different dependency rather than by waiting for a fix, and
# this is what stops them returning quietly.
#
# Asserted against Cargo.lock rather than a `cargo tree` profile because
# that is what `cargo audit` actually reads, and because these are
# dev-dependencies — `check-feature-matrix.sh` builds its trees with
# `--edges normal,build`, where a dev-dependency is invisible by
# construction.
#
# `scc`: RUSTSEC-2026-0205, unsound — `Array::insert` violates exception
# safety if the comparison function panics, so a panicking compare can
# double-free. It reached us only through `serial_test 3.x`, which pinned
# `scc = "^2"` and so could never resolve to the patched 3.8.4.
# `serial_test 4.0.1` dropped `scc` for `parking_lot` + `once_cell`; the
# bump needed no source changes across 537 `#[serial]` sites. A drift back
# to `serial_test 3.x` would silently restore the unsoundness.
assert_lock_absent() {
    local package=$1
    local why=$2
    local lock
    lock="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/Cargo.lock"

    if grep -qE "^name = \"${package}\"$" "$lock"; then
        echo "error: $package is back in Cargo.lock" >&2
        echo "       $why" >&2
        return 1
    fi
}

assert_lock_absent scc \
    "removed with the serial_test 4.x bump; RUSTSEC-2026-0205 is unsound and cargo audit only warns"
echo "eliminated-dependency assertions: ok"

check_exception_policy
cargo audit
