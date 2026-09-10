# Iteration 006 implementation ledger

This ledger records implementation checkpoints for iteration 006, the sweep
of everything iteration 005 staged rather than built. It is evidence about
the current implementation state, not a replacement for the normative
Iteration 006 contract at
`docs/specs/suprnova-live/iterations/006.md`.

## 2026-09-08 -- RenderCache correctness and coherence

Plan E, the first of the sweep's five plans, closed definition-of-done items
1 to 6 and the parts of 15 those six touch. Six observations that were
either conservative to the point of uselessness or missing became exact.

### The engine surface

Four additions, and nothing else. `AuthorizationConsult` (`None`,
`TenantOnly`, `Principal`, with a `Principal`-absorbing `join`) replaced the
`authorization_read` boolean on `ObservedContext`, and
`ClassificationReason::AuthorizationTenantRead` is what a tenant-only
consult narrows through. `DependencyIdentity::UnkeyedWrite` took digest tag
10. `CoherenceCheck::Rewound { stamped, authority }` is reported before any
dependency comparison. `GenerationLedger::lift_epoch_above` is a required
method with no default body, because a ledger that cannot lift cannot claim
rewind safety.

### What the framework does with them

An authorization decision is bracketed by a consult window and judged by
what it recorded: principal material, or nothing resolvable, requires
`Principal`; tenant material alone requires `Tenant`. The framework's RBAC
statements were named and given their table lists, and three crate-private
observing helpers on `DB` run them without marking the render unobservable,
so an RBAC-gated route caches and a permission grant rebuilds it.

A `GlobalScope` declares `ScopeDependency::Constant` or the conservative
default `PerRequest`, and the registry brackets each evaluation with
`collector::resolvable_reads`: a per-request scope that read nothing the
collector can name is recorded as an undeclared read under
`global_scope:<type>`, which narrows the render to `Uncacheable`.
`suprnova::live::current_tenant()` is the instrumented accessor a gate body
or a scope reaches for, scoped by `LiveTenantMiddleware` around the rest of
the chain.

A flag read observes a `Feature` generation whenever the snapshot holds the
flag at any scope key; `set_flag` advances it after the snapshot swap, and
`reload` advances it for every flag its diff found changed and forwards
those names to caches alone through `on_snapshot_reloaded`, which
`DatabaseEvaluator` deliberately does not override.

The write side is a process-wide tri-state, probed at most once and never
inside a caller's transaction, so a queue worker, a scheduled task, or a
console command advances the same generations the server does while an
application with RenderCache disabled still issues no RenderCache SQL at
all.

A primary-key point read that returns a row observes that record and the
table's unkeyed-write identity; one that returns nothing observes the table.
Bulk, table-builder, and named-table raw writes advance both `Table` and
`UnkeyedWrite`.

An entry or a lease stamped above the authority is refused at any age, and
the detecting node lifts the ledger epoch past the stamp, drops its lease,
clears its L0, and increments `suprnova.render_cache.epoch_rewinds`.

### Evidence

Every rule above is proven by a named test; the iteration 006 checkpoint
list names them. The workloads bench gained
`point_read_invalidation_ratio` and now measures
`every_write_invalidates_every_key` as `false`; the checked-in result was
regenerated under its existing `exploratory` label, which is a result-shape
change and not a qualification refresh. No benchmark budget, wire format,
storage codec, or tier semantic changed.

## 2026-09-09 -- RenderCache observability and production build shape

Plan F, the second of the sweep's five plans, closed definition-of-done
items 7 to 9 and the parts of 15 those three touch.

### Declined lookups carry a closed reason

`LookupOutcome::Declined` carries a `LookupDeclineReason`
(`framework/src/render_cache/decline.rs`, 32 variants) rather than an
untyped flag. Every one of the ten decline branches in `lead_render`
(`framework/src/render_cache/middleware.rs`) obtains its reason from a
typed value computed at that branch: `run_render`'s success tuple now
carries a `Result<GenerationSet, RenderObservationFailure>`
(`Overflowed`, `LedgerRead`, `HandlerNotBegun`) in place of an `Option`,
nested inside the `Result<_, RenderRequestLost>` the safe-failure fix
below adds around the whole function; the existing
`Eligibility::Decline` payload converts directly; the first of
`SessionValueRead`/`SecretContextRead`/`UndeclaredContext` present in a
narrowed classification maps one to one; `live::document_declines` returns
an `Option<LiveDocumentDecline>` instead of a `bool`;
`key_used_different_values_than_the_render_saw` returns an
`Option<KeyMismatch>` naming the dimension and whether it was undeclared or
divergent; and `stitch::build_composite_entry` returns a
`Result<_, CompositeBuildError>` with seven variants. Adding a decline
branch without a reason, or a reason without a `snake_case` label, fails to
compile because `LookupDeclineReason::as_str` is an exhaustive match.
`framework/src/render_cache/telemetry.rs` emits the `reason` attribute only
beside `outcome="declined"` and exposes a bounded test recorder
(`recorded_lookups_for_test`, `reset_recorded_lookups_for_test`) plus
`decline_reason_labels_for_test`, which lets
`every_decline_reason_is_documented_in_the_operations_chapter`
(`framework/tests/render_cache/operations.rs`) assert every label appears
in the manual rather than only in prose. Four named tests
(`a_session_value_read_declines_with_reason_session_value_read`,
`a_principal_gate_without_principal_variance_declines_with_reason_principal_undeclared`,
`an_undeclared_locale_declines_with_reason_locale_undeclared`,
`an_ineligible_status_declines_with_reason_status`) prove the session,
principal, locale, and ineligibility declines are four distinct reasons,
and `declined_is_the_only_outcome_that_carries_a_reason` proves a hit and a
miss carry none.

A follow-up fix (`c00d7916`) closed a correctness gap the reason work
surfaced: six sites in the middleware answered a violated invariant with a
panic (three `unreachable!()` arms, one `.expect()` on the classification
decline reason, and two `.expect()` calls taking the request out of its
slot). A panic there fails a request the cache exists to make faster,
unwinding through the request task and the mutex holding the render slot.
The four decline-reason sites now `debug_assert!` and decline in release
rather than panic in every build. The two slot-take sites had no error
channel to answer with, so `run_render` gained one, `RenderRequestLost`,
for the one failure that cannot degrade to an uncached render because the
request itself is already gone; `lead_render` answers it with a controlled
500 and releases the lease so the route is not left fenced. This also
covers a case the prior code did not: a transaction failing at COMMIT,
after its closure had already taken the request, used to panic and now
takes the same controlled-500 path.

### A production build shape by construction

`app/Cargo.toml` and both CLI templates
(`suprnova-cli/src/templates/files/backend/Cargo.toml.tpl` and
`suprnova-cli/src/templates/files/api/Cargo.toml.tpl`) declare `suprnova`
with `default-features = false`
plus the nine non-`testing` defaults (`filesystem`, `database-sqlite`,
`database-postgres`, `database-mysql`, `vector-mariadb`, `web-push`,
`localization`, `magnetar-oauth`, `media`) in `[dependencies]`, and re-add
`features = ["testing"]` in `[dev-dependencies]`; Cargo's feature resolver
only pulls a dev-dependency's features into `cargo test` and other
`--tests` builds, so `cargo build --bin app` (or `--bin console`) never
sees `testing`. `suprnova-cli/tests/new_scaffolds_console.rs` pins both
template lines through `assert_production_build_shape`.

`framework/tests/fixtures/testing-off-probe/` is a separate-workspace probe
crate (its own `Cargo.toml` opens a `[workspace]` table so the repository
workspace never absorbs it) whose `src/main.rs` references, by path, every
`pub` item the framework gates behind `cfg(any(test, feature = "testing"))`.
`scripts/check-production-build.sh` runs the probe without `testing`
(asserting the failure is only `E0425`, `E0433`, or `E0599`), runs it again
with its own `with-testing` feature (asserting success), runs
`cargo build -p app --bin app --bin console` in the production shape,
runs the built binary's `migrate` against a fresh SQLite file, then runs `serve
--no-migrate` and polls `/_suprnova/health/live` until it answers.
`scripts/check-feature-matrix.sh` gained a matching "production shape"
profile (`cargo check -p suprnova --no-default-features --features
"<the nine>"`, and the same with `--tests`), and `scripts/gate-steps.json`
gained the `production-build` step. A follow-up fix (`6331fda3`) added
`#![cfg(feature = "testing")]` to `framework/tests/render_cache/main.rs`,
the render-cache integration test binary's crate root, because gating the
four `_for_test` seams left several `framework/tests/support/*` helper
modules this binary pulls in referencing items that no longer exist
without `testing`, which had stopped the feature matrix's "production
build shape test targets" profile (`--tests`) from compiling; its own
commit message states this closes that gap. This task did not rerun that
profile to confirm.

### Build identity from the application

`framework/src/boot.rs` gained a process-wide `OnceLock<&'static str>`
behind `set_default_build_id`/`default_build_id` (first call wins).
`#[suprnova::main]`'s expansion
(`suprnova-macros/src/main_macro.rs`) calls
`::suprnova::boot::set_default_build_id(::core::env!("CARGO_PKG_VERSION"))`
immediately after `::suprnova::boot::load_env_or_exit()`, so the
application crate's own package version is what gets recorded; a macro
test (`the_expansion_records_the_application_build_id_after_loading_the_environment`)
pins the ordering. `RenderCacheConfig::from_env`'s `build_id` now resolves
through three sources in order: an explicit `APP_BUILD_ID`, the recorded
application version, then this framework crate's own version only for a
binary that never expanded `#[suprnova::main]`
(`build_id_prefers_app_build_id_then_the_recorded_default_then_the_framework_version`
in `framework/src/render_cache/config.rs`). The public, ungated
`RenderCacheConfig::with_build_id` builder overrides whatever `from_env`
chose, for a programmatic install; `RenderCache::install`'s rustdoc names
all three sources. `two_installs_with_different_application_build_ids_never_share_an_entry`
(`framework/tests/render_cache/middleware.rs`) proves two builds with
different application build ids never share a stored entry.

### Manual

`manual/render-cache-operations.md`'s Telemetry section documents the
`reason` attribute and all 32 labels, grouped by the contract each names,
and its troubleshooting bullet sends an operator to the `reason` label
first. `manual/deployment.md` gained a "Production build shape" section
naming the two-entry dependency shape; `manual/render-cache-deployment.md`
links to it and rewrote its `APP_BUILD_ID` row and paragraph for the
three-source chain and the `with_build_id` programmatic path. All six
mirrors (`de`, `es`, `fr`, `ja`, `pt-BR`, `zh-Hans`) carry the same
byte-identical code spans as English, and `.manual-translations.lock` is
restamped for the three chapters.

### Evidence

Tasks 1 to 3 report `cargo test -p suprnova --test render_cache` green at
335 to 336 passed with 0 failed (32 ignored) across their respective
changes, `cargo fmt --all --check` and `cargo clippy` clean of new
warnings, and `scripts/check-production-build.sh` passing end to end; this
final task did not rerun the Rust suite (its own changes are manual and
specification prose, verified instead by `python3
scripts/check-manual-structure.py`, `scripts/check-manual-translations.sh`,
`scripts/check-prose-dashes.sh`, `node
crates/suprnova-live/scripts/check-specs.mjs`, `node
crates/suprnova-live/scripts/check-implementation-docs.mjs`,
`crates/suprnova-live/tests/documentation_contract.sh`, and `git diff
--check`). Task 3's report separately notes that
`scripts/check-feature-matrix.sh`'s production-shape "test targets" profile
failed with `E0599` for `with_clock_for_test` in five
`framework/tests/support/*` helper crates, predating `6331fda3`'s
whole-binary `#![cfg(feature = "testing")]` gate on the render-cache test
binary; this task did not rerun that profile to confirm the later commit
resolved it. No benchmark budget, wire format, storage codec, or default
feature set changed.

## 2026-09-10 -- Manual inline-code-span parity, the whole corpus

Plan I, the manual-parity plan, closed definition-of-done item 14 and the
part of 15 it touches.

### The ratchet is gone

`SPAN_CHECKED_SOURCES` in `scripts/check-manual-structure.py`, the
seven-chapter allowlist that scoped the inline-code-span comparison, is
deleted along with its explanatory comment; `_compare_shapes` drops its
`compare_spans` parameter and compares spans for every chapter and all six
mirrors unconditionally. `scripts/tests/test_manual_structure.py` drops the
ratchet-membership test, renames
`test_span_defect_is_reported_only_for_a_listed_chapter` to
`test_a_span_defect_is_reported_for_every_chapter` (now planting the same
defect in two chapters and asserting both are reported), and adds
`test_the_real_manual_tree_reports_no_problems`, which runs the checker over
this repository's actual `manual/` tree rather than a fixture, so a future
regression fails the unit suite and not only the gate. One fixture in
`scripts/tests/test_gate_scoping.py`
(`test_escaped_and_code_pipes_do_not_add_table_columns`) had relied on the
ratchet excluding its synthetic chapter from span comparison to isolate a
table column-counting check; its English and mirror code spans now carry the
same identifiers, the way a real translation must, leaving the
pipe-escaping and column-counting behavior it tests unchanged.

### The corpus reached zero

The whole manual started this plan at 1,591 span problems across 68 of 111
chapters. 958 of them traced to one malformed construct in
`manual/seeding.md`: a backslash-escaped backtick inside a single-backtick
span, which CommonMark does not honor. Fixing that source construct and
adding paragraph-bounded span extraction
(`test_a_mis_nested_span_in_one_paragraph_does_not_taint_later_ones`) left
633 genuine translation drifts. Paragraph-bounding itself cleared none of
that count: on this corpus every amplified problem came through the
seeding.md construct, so bounding is containment against a future
mis-nesting, not a fix for anything this corpus had. A checker fix for
block-quote markers
(`test_a_span_wrapped_across_two_quoted_lines_drops_the_quote_marker`)
cleared 8 more problems the corpus never really had, because the checker
was folding a quoted line's `> ` marker into the span it compared. The
remaining 625 were fixed by translation across five sequential batches
(tasks 4a through 4e, each restamping `.manual-translations.lock`). The
whole manual and all six mirrors (`de`, `es`, `fr`, `ja`, `pt-BR`,
`zh-Hans`) now report zero span problems.

### Open, not a defect here

Several translation batches restored or retranslated sentences the mirrors
had dropped or invented, across `de`, `es`, `fr`, `ja`, `pt-BR`, and
`zh-Hans`. Every automated check passes, but the writer is not a native or
fluent speaker of any of the six languages; the retranslated passages reused
established vocabulary from the same chapters rather than inventing new
wording, but have not had native-speaker naturalness review. This is a
fluency question the automated checks cannot settle, not a correctness
defect, and stays flagged for review before this documentation ships as
final.

### Evidence

`python3 .superpowers/sdd/plan-i/census.py` (a throwaway, gitignored harness)
and `python3 scripts/check-manual-structure.py` both report zero span
problems over the real tree; both now call the same unconditional
`_compare_shapes`, so this is two invocations of one code path rather than
two independent checks. `python3 -m
unittest discover -s scripts/tests -p 'test_*.py'` passes at 144 of 144.
`scripts/check-manual-translations.sh`, `scripts/check-prose-dashes.sh`, and
`scripts/check-live-contracts.sh` (which runs the spec, implementation-doc,
and gate-contract checks together) all pass, and `git diff --check` reports
nothing. No Cargo build ran; none was needed for this change.
