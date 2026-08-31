#!/usr/bin/env bash
#
# Hyphens, never em dashes - everywhere.
#
# House style for every rendered surface - the manual, the changelog,
# the crate READMEs, the Fluent catalogs - is the ASCII hyphen. The
# manual's translations were authored hyphen-only from day one, and
# suprnova.app (which renders this manual as its /docs) keys its
# translation-review ledger to content hashes of the English text it
# serves - text that is the hyphen form. Keeping the source hyphen-native
# means the synced bytes ARE the source bytes; one new em dash here would
# come back from the site's sync as a diff against reviewed text.
#
# When a dash lands at a wrap boundary, keep it attached to the previous
# line (`word -` at line end, never `- word` at line start): a bare "- "
# opening a line parses as a Markdown list item and splits the paragraph.
#
# Scope is every tracked text file - prose, Rust sources and doc comments,
# templates, scripts, config. Owner ruling 2026-08-25: there is no exemption
# for source code; the hyphen is the only dash anywhere in this repo.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

EM=$'\xe2\x80\x94'

matches="$(git ls-files -z | xargs -0 grep -nHI -- "$EM" || true)"

if [[ -n "$matches" ]]; then
    printf '%s\n' "$matches" >&2
    echo "" >&2
    echo "em dash (U+2014) in a tracked file; write ' - ' instead (no exemption for source)." >&2
    exit 1
fi

count=$(git ls-files | wc -l)
echo "dashes: no em dash in $count tracked files"
