# Suprnova feature map

Every piece of Suprnova's public surface, extracted from the source at one
commit, filed under the manual chapter that owns its domain, with a
checkbox per item. A box is checked when that chapter's documentation of
the item has been remediated against the source. The map is the checklist;
the manual is what gets fixed.

Nothing here comes from the manual, the changelog, or the specs. Every
entry is produced by a tool reading the code or the built binaries, and
cites the file and line it came from. The chapter an entry is filed under
decides only where it is tracked, never what it says.

## Layout

- [`INDEX.md`](INDEX.md): every chapter with its item count and progress.
- `domains/<chapter>.md`: the checklist for one manual chapter, grouped by
  where each entry came from (Rust API per crate, command line,
  configuration, Live templates, endpoints and tables), then by source module.
- `domains/_no-chapter-*.md`: surface no manual chapter covers today.
- [`EXCLUSIONS.md`](EXCLUSIONS.md): what the extractors saw and deliberately
  did not list, with the reason.
- `tools/`: the extractors, the filing rules, and the driver.

## Where entries come from

| Family | Extracted from |
|---|---|
| Rust API of all eight crates: every public module, type, function, macro, constant, and their methods | rustdoc JSON with all features, cross-checked against a default-features build |
| Proc-macros, their helper attributes, and every argument each parser accepts | rustdoc JSON plus each macro's parser |
| Live directives, modifier vocabularies, runtime feature modules, component library | the directive grammar contract, runtime-features contract, component manifests and templates |
| Commands and flags of `suprnova`, the app runner, and the console binary | each binary's clap definitions, confirmed by the built binary's `--help` |
| Environment variables read at runtime; Cargo features | reads traced to their literal or constant; crate manifests |
| HTTP endpoints the framework owns; database tables by who creates them | route registration and migration code, curated, with each line looked up from a code excerpt |

## How entries are filed

`tools/domains.py` holds the rules, in one place, for review:

- Rust items by the source file that defines them, longest prefix first
  (`framework/src/eloquent/relations/` goes to `eloquent-relationships`).
  Methods stay with their type, wherever they are defined.
- Proc-macros, CLI commands, endpoints and tables by an explicit rule each.
- Every environment variable goes to `env-vars`; Cargo features to `installation`.
- Where `suprnova` re-exports an item from a sibling crate, the item is
  listed once, under its home crate, marked "re-exported as `suprnova::...`".
- An entry no rule claims goes to a "no chapter" group, never to a guess.
  A rule naming a chapter that does not exist stops the run.

## Reading an entry

```
- [ ] struct `suprnova::Router` · framework/src/routing/router.rs:564 (also `suprnova::routing::Router`)
  - [ ] fn `suprnova::Router::try_live_document` · framework/src/live/document.rs:606
- [ ] command `suprnova make:controller` · suprnova-cli/src/main.rs:160
  - argument `<NAME>`: Name of the controller (e.g., users, user_profile)
```

- The path is the shortest one a user can write; `also` lists the other public paths.
- `feature: ...` names the Cargo feature an item needs, and says when it is off by default.

## Regenerating

```
feature-map/tools/generate.sh
```

It needs a nightly toolchain for rustdoc JSON (`rustup toolchain install
nightly --profile minimal`); the project's pinned toolchain is not changed.
It builds into `target/feature-map`, rewrites `domains/`, `INDEX.md` and
`EXCLUSIONS.md`, and keeps checked boxes: an item keeps its check while its
identifying path is unchanged, even if it moves to another chapter. Items
removed from the source disappear; new items appear unchecked. The source
commit is the last one that touched the crates, not the map's own commits.

## How the extraction was checked

- **Rust APIs.** Every free function and every inherent public method in
  rustdoc's index appears in the map. The only indexed items that don't
  are filed as "Public but unnameable": four sealed traits, and
  `suprnova_live::validation::error_bag::ValidationBagError`, which public
  functions return but no public path names.
- **Feature gates.** Each label is evaluated against the crate's default
  features and compared with a default-features rustdoc build; they agree
  for every item in every crate. A disagreement stops the run.
- **Macro arguments.** Every keyword the macro parsers match is either an
  argument in the map or listed as internal in `EXCLUSIONS.md`. Each
  argument's line is looked up from the parser, so an entry fails the run
  if its code moves or goes.
- **Environment variables.** Every read was traced to its literal or
  constant, including reads through wrapper functions and reader closures.
- **CLI.** Commands hidden from `help` are included and marked. The
  `suprnova` banner help matches its parser exactly.
- **Live components.** Every file in each component directory is declared
  in its manifest, and every `{% macro %}` in them is mapped.
- **Filing.** Raw entries minus deduplicated sibling re-exports equals
  filed entries exactly, every re-export resolves to the item it names, and
  checked boxes survive regeneration.
