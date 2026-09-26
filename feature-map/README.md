# Suprnova feature map

Every piece of Suprnova's public surface, extracted from the source at one
commit, with a checkbox per item. A box is checked when the manual's
documentation for that item has been remediated against the source. The
map is the checklist; the manual is what gets fixed.

Nothing here comes from the manual, the changelog, or the specs. Each
section is produced by a tool reading the code (or the built binaries), and
each item cites the file and line it came from.

## Size

10,785 checkboxes at commit d03b4f1:

| File | Boxes |
|---|---|
| `suprnova.md` | 5,993 (2,237 items, 3,756 methods and associated items) |
| `suprnova-live.md` | 2,858 |
| `suprnova-magnetar.md` | 1,170 |
| `live-templates.md` | 216 |
| `configuration.md` | 190 (156 env vars, 34 Cargo features) |
| `suprnova-macros.md` | 183 (32 macros, 151 arguments) |
| `endpoints-and-tables.md` | 61 (14 endpoints, 47 tables) |
| `cli.md` | 53 (33 `suprnova`, 15 app runner, 5 console) |
| `suprnova-web-push.md` | 34 |
| payment adapters | 25 |

## Sections

| File | What it covers | Extracted from |
|---|---|---|
| [`suprnova.md`](suprnova.md) | Framework Rust API: every public module, type, function, macro, constant, and their methods, grouped by feature area | rustdoc JSON, all features, cross-checked against a default-features build |
| [`suprnova-live.md`](suprnova-live.md) | Live engine Rust API | rustdoc JSON |
| [`suprnova-magnetar.md`](suprnova-magnetar.md) | Magnetar auth engine Rust API | rustdoc JSON |
| [`suprnova-payments-stripe.md`](suprnova-payments-stripe.md), [`-paddle`](suprnova-payments-paddle.md), [`-nowpayments`](suprnova-payments-nowpayments.md) | Payment adapters | rustdoc JSON |
| [`suprnova-web-push.md`](suprnova-web-push.md) | Web Push crate | rustdoc JSON |
| [`suprnova-macros.md`](suprnova-macros.md) | All 32 proc-macros, their helper attributes, and every argument their parsers accept | rustdoc JSON, plus each macro's parser |
| [`live-templates.md`](live-templates.md) | Live directives, modifier vocabularies, runtime feature modules, and the component library with each component's macros | the directive grammar contract, runtime-features contract, component manifests and templates |
| [`cli.md`](cli.md) | Every command and flag of `suprnova`, the app runner, and the console binary | the binaries' own `--help` output |
| [`configuration.md`](configuration.md) | Every environment variable read at runtime, and every Cargo feature | source reads traced to their literals; crate manifests |
| [`endpoints-and-tables.md`](endpoints-and-tables.md) | HTTP endpoints the framework owns, and database tables by who creates them | route registration and migration code (curated; line numbers looked up from code excerpts) |

## Reading an entry

```
- [ ] struct `suprnova::Router` · framework/src/routing/router.rs:564 (also `suprnova::routing::Router`)
  - [ ] fn `suprnova::Router::try_live` · framework/src/live/routes.rs:83
```

- The path is the shortest one a user can write; `also` lists other public paths to the same item.
- `feature: ...` names the Cargo feature an item needs, and says when that feature is off by default.
- Sections in the API maps follow the defining module, so each crate reads by feature area.

## What is deliberately not listed

- Private and `pub(crate)` items, and `#[doc(hidden)]` items. rustdoc removes them before the map is built.
- Trait implementations of external traits (`Debug`, `Clone`, `Serialize`, ...). Implementations of Suprnova's own traits are listed on each type.
- Demo application code (`app/`), test-support crates, and fixtures.
- Build-time variables (`CARGO_*`, `OUT_DIR`) and uppercase strings that are not env vars. `configuration.md` lists these so the exclusion is visible.
- Keywords the macro parsers match only internally. `suprnova-macros.md`'s tool keeps that list with reasons.

## Regenerating

```
feature-map/tools/generate.sh
```

It needs a nightly toolchain for rustdoc JSON (`rustup toolchain install
nightly --profile minimal`); the project's pinned toolchain is not changed.
It builds into `target/feature-map`, rewrites every map file, and keeps
checked boxes: an item keeps its check while its identifying path is
unchanged. Items removed from the source disappear, and new items appear
unchecked. A feature-gate label that disagrees with a default-features
build stops the run.

## How the extraction was checked

- **Rust APIs.** Every free function and every inherent public method in
  rustdoc's index appears in the map. The only indexed items that don't
  are listed under "Public but unnameable": four sealed traits, and
  `suprnova_live::validation::error_bag::ValidationBagError`, which public
  functions return but no public path names.
- **Feature gates.** Each label is evaluated against the crate's default
  features and compared with a default-features rustdoc build; they agree
  for every item in every crate.
- **Macro arguments.** Every keyword the macro parsers match is either an
  argument in the map or listed as internal, with a reason, in
  `tools/macro_vocab.py`. Each argument's line is looked up from the parser
  itself, so an entry fails the run if its code moves or goes.
- **Environment variables.** Every read was traced to its literal or
  constant, including reads through wrapper functions and reader closures.
  All 156 were checked against the read site's shape; the two rejected
  candidates are SQL function names.
- **CLI.** Commands come from each binary's clap definitions and are
  confirmed by the built binary answering `--help`. Console commands hidden
  from `help` are included and marked. The `suprnova` banner help matches
  its parser exactly.
- **Live components.** Every file in each component directory is declared
  in its manifest, and every `{% macro %}` in them is mapped.
- **Regeneration.** Running the driver from scratch reproduces every file
  byte for byte.
