# Suprnova feature map

A record of every piece of Suprnova's public surface, extracted from the
source, filed under the manual chapter that owns it, with the remediation
status of the manual's documentation for each one.

**The source is the only truth.** Nothing here comes from the manual, the
changelog, or the specs. Every record is produced by a tool reading the code
or the built binaries, and cites the file and line it came from. The chapter
a record is filed under decides only where it is tracked, never what it says.

## Files

| File | What it is | Who writes it |
|---|---|---|
| `surface.jsonl` | 10,561 records, one per line, sorted by `id` | `tools/generate.sh` (never edit by hand) |
| `state.jsonl` | remediation status per record | `fmap mark` only |
| `meta.json` | the source commit the surface was built from | `tools/generate.sh` |
| `exclusions.json` | what the extractors saw and deliberately left out, with the reason | `tools/generate.sh` |
| `fmap` | the query and update tool | |

## Using it (agents)

`fmap` loads both JSONL files into an in-memory SQLite database and prints
JSON. It needs only Python 3.

```
feature-map/fmap summary                            progress per chapter
feature-map/fmap next --chapter queues --limit 20   top-level items still to do
feature-map/fmap show suprnova::Queue               one item, its members, its state
feature-map/fmap chapter queues --status stale      a chapter's items by status
feature-map/fmap search retry_failed                ids containing a string
feature-map/fmap query "SELECT id, file, line FROM items WHERE chapter='queues' AND status='todo'"
feature-map/fmap mark suprnova::Queue --children --status done --note "<where in queues.md>"
feature-map/fmap stale --body                       done items that changed since
feature-map/fmap orphans                            state for items the source no longer has
```

Remediating a chapter:

1. `fmap next --chapter <chapter>` for the items still to do.
2. `fmap show <id>` for each: its source location, signature details and members.
3. Read the source at that location and fix the chapter until it matches.
4. `fmap mark <id> --status done --note "<where in the chapter>"`. Add
   `--children` to cover the item's methods too. For an item that should not
   be documented, use `--status skip` with a note giving the reason.

Statuses: `todo` (nothing recorded), `in_progress`, `done`, `skip`, and
`stale`. A record is `stale` when it was marked done and its signature has
changed in the source since. `body_changed` flags a done record whose
implementation changed while its signature did not, which may mean its
documented behaviour changed. `fmap` refuses to mark anything done while the
source has moved past the commit in `meta.json`; regenerate first.

## The `items` view

`fmap query` runs read-only SQL against one view:

| Column | Meaning |
|---|---|
| `id` | the path a user writes (`suprnova::Queue::retry_failed`), or `binary command`, env var name, `crate/feature`, directive, endpoint path, `table (creator)` |
| `kind` | `struct`, `enum`, `trait`, `fn`, `const`, `type`, `macro`, `proc macro`, `argument`, `reexport`, `command`, `env`, `cargo-feature`, `directive`, `vocabulary`, `runtime-feature`, `component`, `template-macro`, `endpoint`, `table` |
| `family` | `rust-api`, `cli`, `config`, `live-templates`, `endpoints-tables` |
| `chapter` | the manual chapter (`manual/<chapter>.md`), or a `(no chapter) ...` group |
| `parent` | the owning item for methods, macro arguments and component macros |
| `file`, `line` | where it is defined |
| `feature` | the Cargo feature it needs, noting when that feature is off by default |
| `status`, `body_changed`, `note`, `verified_rev`, `updated_at` | remediation state |

The full record (variants, fields, implemented traits, flags, read sites,
aliases, and so on) is in `fmap show <id>`.

## Where records come from

| Family | Extracted from |
|---|---|
| Rust API of all eight crates | rustdoc JSON with all features, cross-checked against a default-features build |
| Proc-macro arguments | each macro's own parser |
| Live directives, vocabularies, runtime features, components | the directive grammar contract, runtime-features contract, component manifests and templates |
| Commands and flags | each binary's clap definitions, confirmed by the built binary's `--help` |
| Environment variables, Cargo features | reads traced to their literal or constant; crate manifests |
| Endpoints, tables | route registration and migration code, curated, each line looked up from a code excerpt |

Filing rules live in `tools/build_surface.py`: Rust items by the source file
that defines them, members with their parent, everything else by an explicit
rule. A record no rule claims goes to a `(no chapter)` group, never to a
guess. `suprnova` re-exports of sibling crates are folded into the `aliases`
of the item they name.

## Regenerating

```
feature-map/tools/generate.sh
```

It needs a nightly toolchain for rustdoc JSON (`rustup toolchain install
nightly --profile minimal`); the project's pinned toolchain is unchanged. It
builds into `target/feature-map` and rewrites `surface.jsonl`, `meta.json`
and `exclusions.json`. It never touches `state.jsonl`, so no remediation
work is lost; records whose signatures changed show up as `stale`, and state
for removed items shows up in `fmap orphans`.

## How the extraction was checked

- Every free function and every inherent public method in rustdoc's index is
  a record. The only indexed items that aren't nameable by a public path are
  recorded as such: four sealed traits, and
  `suprnova_live::validation::error_bag::ValidationBagError`, which public
  functions return but no public path names.
- Every feature-gate label agrees with a default-features rustdoc build; a
  disagreement stops the run.
- Every keyword the macro parsers match is either an argument record or
  listed as internal in `exclusions.json`.
- Every environment variable read was traced to its literal or constant,
  including reads through wrappers and reader closures.
- Console commands hidden from `help` are included. The `suprnova` banner
  help matches its parser exactly.
- Raw records minus folded sibling re-exports equals filed records, every
  re-export resolves to the item it names, and ids are unique.
- A signature edit turns a done record `stale`, a body edit sets
  `body_changed`, and regeneration reproduces `surface.jsonl` byte for byte.
