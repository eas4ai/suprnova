#!/usr/bin/env bash
#
# Shipped files must not point at development artifacts.
#
# `.gitignore` lists the working documents this project keeps locally —
# audit reports, remediation plans, review notes, per-directory assistant
# guidance. None of them exist in a user's clone, on docs.rs, or in a
# scaffolded project. A comment, doc comment, or script message citing
# one is a dangling reference the reader cannot follow, and it leaks the
# shape of an internal process into published surface.
#
# This caught seven at once: a `#[cfg(test)]` module citing an audit file
# by path, two test/source comments deferring to assistant guidance for a
# rule instead of stating it, `release.sh` pointing at two planning
# documents and printing a "next step" naming a manual chapter that does
# not exist, and two references added while writing the fix for the rest.
#
# Note this file is checked like any other: naming an ignored document
# here, even to explain the rule, is the thing the rule forbids.
#
# The rule is not "never mention them" — it is that published files must
# stand alone. Say the rule, don't cite where it is written down.
#
# Only literal names are checked. A glob entry like `/GROK-*.md` names no
# specific file, so there is nothing for a reference to dangle against.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

failures=0

# Basenames from .gitignore that are literal (no glob metacharacter), so
# a reference to one is unambiguously a reference to an ignored file.
mapfile -t ignored_docs < <(
    grep -oE '[A-Za-z0-9_.-]+\.md' .gitignore | sort -u
)

for doc in "${ignored_docs[@]}"; do
    # `git ls-files` rather than a directory walk: only tracked files are
    # published, and `target/` would otherwise dominate the scan.
    while IFS= read -r offender; do
        # .gitignore naming them is the point of .gitignore.
        [ "$offender" = ".gitignore" ] && continue

        echo "error: $offender references the gitignored $doc" >&2
        grep -nF "$doc" "$offender" | sed 's/^/       /' >&2
        failures=$((failures + 1))
    done < <(git ls-files -z | xargs -0 grep -lF "$doc" 2>/dev/null || true)
done

if [ "$failures" -gt 0 ]; then
    echo "" >&2
    echo "$failures published file(s) cite a development artifact." >&2
    echo "State the rule or the reason inline instead of citing where it" >&2
    echo "is written down — the reader has no such file." >&2
    exit 1
fi

echo "doc references: no published file cites a development artifact (${#ignored_docs[@]} checked)"
