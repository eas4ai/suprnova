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

## 2026-09-09 -- Contracts and negotiation

Plan G, the third of the sweep's five plans, closed definition-of-done items
10 and 11 in full.

Item 10's closing clause, that iteration 004 browser evidence is regenerated
against the new descriptor under its existing unqualified label rather than
relabelled, is met in the only way it can be. The iteration 004 conformance
bundle `browser/generated/iteration-004-conformance.mjs` was regenerated from
its TypeScript source in the rename commit and `npm run generate:check` proves
it byte-exact. The compatibility receipt set is empty and stays empty:
`npm run compatibility:check -- --allow-unqualified` reports
`compatibility qualification: unqualified (0/8)`, so there is no receipt that
could have been carried forward, and the label is the same `unqualified` it
was before. Nothing was relabelled because there was nothing to relabel.

### The registered-event descriptor rename

`DESCRIPTOR_SCHEMA_VERSION` moved 1 to 2
(`crates/suprnova-live/src/async_updates/subscription.rs`), and the issued
subscription descriptor's registered-event fields are now `maximum_hops`,
`maximum_fanout`, and `payload_contract` everywhere the field names are
written by hand: the framework's single projection site, `IssuedView::new`
(`framework/src/live/async_updates.rs`); the browser runtime's hand-written
sources (`src/async-updates/types.ts`, `browser-host.ts`, `envelope.ts`,
`feature.ts`, `src/islands/registered-events.ts`, `src/islands/discovery.ts`,
and the embedded declaration template in `scripts/build.mjs`); and the
async reference host, whose `authorize` path and multi-subscription
`transport_response` path both used to hardcode an empty `"events": []`.
`authorize` now builds its `events` field from the real signed
`claims.events()` the same call already produced.
`transport_response` holds only the descriptor string, not a fresh claims
value, so it takes the descriptor it carries and reads its events back out
through a new `AsyncReferenceAuthority::events_from_descriptor`
(`crates/suprnova-live/crates/suprnova-live-test-support/src/async_reference_host.rs`),
which parses and verifies that exact descriptor before projecting its
claims, so what it advertises can never disagree with the descriptor beside
it. This replaced a first fix, `registered_events_json`, that instead
projected the reference host's own separately-defined contract (still
present in the source but no longer called by either path): fine as long as
that contract matched what a given response's descriptor was signed with,
which the multi-subscription path's own fixture registration does not
always do, so it was replaced with the verify-the-actual-bytes approach
before this plan closed.

Conformance fixtures for the descriptor now exist where there were none:
`fixtures/v4/async-envelope.json` gained a `descriptor_cases` collection
(`event_naming`: `well-formed-registered-event`,
`legacy-camel-case-registered-event`; `schema_version`:
`schema-version-two-accepted`, `schema-version-one-refused`) and a top-level
`descriptor_schema_version: 2` field. `browser/src/generated/descriptor-contract.ts`
is generated from the well-formed fixture case by
`scripts/generate-browser-contracts.mjs` rather than hand-copied, and
`npm run generate:check` proves it byte-exact, so Rust and TypeScript cannot
drift silently.

### Media and Encoding negotiation

Declaring `VarianceDimension::Media` or `::Encoding` used to partition
nothing: the middleware resolved both to the constants `text/html` and
`identity` regardless of what a request sent. `NegotiatedPolicy`
(`crates/suprnova-live/src/render_cache/policy.rs`) now types a route's
closed, bounded, canonically-lower-case accepted set and its default;
`RenderCachePolicyBuilder::vary_media`/`vary_encoding` declare the dimension
and its set together, and `RenderCachePolicy::validate` refuses a policy
that varies one of these two dimensions without also declaring its set, so
the bare `.vary(VarianceDimension::Media)` every other dimension accepts is
rejected at build for these two alone. `variance_descriptor`
(`framework/src/render_cache/middleware.rs`) negotiates the request's
`Accept` or `Accept-Encoding` header against the declared set and returns
the resolved value alongside the descriptor; `key_input` and
`key_input_for_test` take their `media`/`encoding` fields from that one
resolution instead of a second, separately hardcoded value, so the render
key and the descriptor are structurally incapable of disagreeing.

The negotiation rule is `q`-weighted per RFC 9110: the declared-set member
with the highest quality wins, equal quality keeps the header's own
left-to-right order, a wildcard is compared as a literal token and never
expanded against the set, and a `q=0`, an out-of-range, or an unparsable
quality excludes that candidate rather than defaulting it to `1.0`. An
absent header, one naming nothing in the declared set, or one negotiation
cannot make sense of resolves to the declared default, and the resolution
never panics.

### Manual

`manual/render-cache.md` replaced its "resolves to a constant" statement
with the `vary_media`/`vary_encoding` API and the negotiation rule, and all
six mirrors (`de`, `es`, `fr`, `ja`, `pt-BR`, `zh-Hans`) carry a translated
version of the same change with every inline code span byte-identical to
English; `.manual-translations.lock` is restamped for `render-cache.md`.

### Evidence

Descriptor rename: `descriptor_fixtures_prove_snake_case_naming_and_the_schema_version_gate`
(`crates/suprnova-live/tests/golden_fixtures.rs`) proves the real
engine-signed event carries every renamed field, that the fixture's
well-formed and camel-case cases respectively match and diverge from it,
and that a descriptor signed at schema version 1 is refused with
`SubscriptionErrorKind::InvalidDescriptor`; Task 2's report records
`cargo test -p suprnova-live --test golden_fixtures` passing this test
among 12 passed, 0 failed. `issuance_returns_a_document_scoped_bearer_subscription`
(`framework/tests/live/async_routes.rs`) compares a real live-issued
subscription's event key set against the same fixture, proving
`IssuedView::new`'s own projection against it; Task 2's report records
running that one test filtered, once, passing, and did not rerun the
whole `tests/live` binary (see the gap noted below). The transport-path
projection is proven non-empty and exactly snake_case by an assertion
added to `async_routes_authorize_poll_sse_and_one_bounded_websocket`
(`crates/suprnova-live/crates/suprnova-live-test-support/tests/reference_host.rs`);
the controller's own record of that change reports it passing at 28
passed, 0 failed. The TypeScript side is proven by
`browser/tests/golden-fixtures.test.ts`'s "proves the registered-event
descriptor's snake_case naming and schema-version gate", and
`npm run generate:check` proves the generated TypeScript contract
byte-exact against the fixture.

Negotiation: six new tests in `crates/suprnova-live/tests/render_cache_policy.rs`
(`declared_rejects_an_empty_or_oversized_set_and_a_default_outside_it`,
`negotiate_is_q_weighted_and_ties_keep_header_order`,
`negotiate_falls_back_to_the_default_and_never_widens_the_set`,
`negotiate_never_panics_on_hostile_or_malformed_input`,
`vary_media_and_vary_encoding_require_pairing_with_a_declared_set`,
`a_vary_patch_dropping_media_also_drops_its_stale_declared_set`), plus one
existing test, `private_cached_with_no_identity_bearing_dimension_is_rejected`,
updated to build with `vary_media` so it still fails for the reason its own
comment states -- all pass among the file's 15 passed, 0 failed. Six
end-to-end tests in `framework/tests/render_cache/middleware.rs`
(`two_media_negotiations_on_a_declared_media_route_are_both_cached_and_never_cross`,
`a_media_declaring_route_falls_back_to_its_default_for_absent_or_unmatched_accept`,
`a_hostile_accept_header_degrades_to_the_media_default_without_panicking`,
`a_higher_quality_media_candidate_wins_even_when_listed_second`,
`two_encoding_negotiations_on_a_declared_encoding_route_are_both_cached_and_never_cross`,
`an_encoding_declaring_route_falls_back_to_its_default_for_absent_or_hostile_headers`)
pass among `cargo test -p suprnova --test render_cache`'s reported 342
passed, 0 failed, 32 ignored (the ignored cases are live MySQL, Postgres,
and Redis tiers, correctly skipped locally). The two "never cross" tests
also assert the `Vary` header names `Accept` or `Accept-Encoding`
respectively, so the key, the header, and the served representation are
proven to agree rather than merely asserted to. Task 3's report also records
`cargo fmt --all --check` and `cargo clippy` on the touched crates clean of
new warnings. Task 2's report records `npm run test:unit` at 861 of 861
passing on its final clean run (after `dist/` and the reviewed integrity
digests were refreshed to match the descriptor rename).

This recording task ran no Cargo build and no npm command itself; every
count above is quoted from the delivering tasks' own reports and the
plan's progress ledger, not re-verified here. Three gaps neither this task
nor the delivering tasks closed, recorded rather than hidden: the
`cargo test -p suprnova --test live` binary reported four to five failures
when Task 1 ran it, which Task 1's report attributes to in-process
`LiveRuntime`/mount-catalog state carried over from the 2026-09-05
test-folding work (none of the failing modules sit in a file this plan's
descriptor or negotiation commits touch); no run of that binary against
this branch's own base (`3776cdb0`) is recorded to confirm that reading, so
"pre-existing" is the delivering task's attribution rather than an
established baseline, and that binary's own health stays unconfirmed by
this plan. The bare `npm run lint` invocation resolves to a stale machine-wide
`eslint` on this workstation rather than the project's own, so the local
binary (`./node_modules/.bin/eslint . --max-warnings 0`) is what was
actually run and found clean; and definition-of-done item 10's closing
clause, regenerating the iteration 004 browser compatibility evidence, was
not exercised by any command in this plan.

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

## 2026-09-10 -- Nested cached segments and credible generation hints

Plan H, the last of the sweep's five plans, closed definition-of-done items
12 and 13 and the parts of 15 those two touch.

### Nested cached segments

`Segment::Nested { key, version, assembled_len, on_failure }`
(`crates/suprnova-live/src/render_cache/composite.rs`) lets a stored
composite name another stored entry as one of its segments. The inner entry
keeps its own key and its own version, so it is invalidated, republished, and
fenced on its own terms rather than the includer's, and `on_failure` is a
`SlotFailurePolicy` - the same three answers a stitch slot declares - because
the including graph is the only place a per-segment failure decision can
live. Two constants bound the shape: `MAX_NESTING_DEPTH` (3) and
`MAX_NESTED_SEGMENTS` (16).

Three engine entry points carry it. `descend_nested(chain, key)` refuses a
key already in the ancestor chain as `Cycle` and a chain at the depth bound
as `DepthExceeded`, in that order, so a self-reference that also happens to
overrun the depth is always reported as the cycle it is.
`verify_nested(named_version, named_len, actual_version, actual_len)` refuses
a fetched inner entry whose version or assembled length is not what the
includer named, and distinguishes the two.
`assemble_nested(entry, input, nested, ancestors, max_body_bytes)` assembles
a graph containing nested segments; flat graphs keep using the unchanged
`assemble`. The assembled length is known before any byte is copied, because
`assembled_len` is the length the graph named rather than the length of a
body that has been fetched: the total is summed and checked against
`max_body_bytes` first, and only then copied.

The framework refuses a bad composition where it is built.
`refuse_unsafe_nesting`, through `check_nested_composition`
(`framework/src/render_cache/stitch.rs`), declines publication of a
composite that names an inner segment of a wider
representation class, an inner segment with a longer freshness window, a
transitive cycle, a graph past the depth bound, or a `PrivateCached` inner
segment. The last is this plan's own correction: a `PrivateCached` inner can
never resolve, because it always fails closed as unauthorized at hit time, so
a composition that can never resolve is refused at publish rather than
discovered at runtime as an always-omitted segment.

On the hit path, `resolve_nested_segment` fetches each inner entry, runs
`verify_nested` against what the includer named, and reauthorizes an
identity-bound inner segment for this request. An inner segment declared
identity-free skips that reauthorization, which is the specification's
"per-request reauthorization or proven identity freedom" rather than a
weakening of it. Every failure - a fetch that missed, a version or length
mismatch, a depth overrun, a cycle, an unauthorized inner - resolves through
that segment's own declared `fail_document`, `omit`, or `fallback`.

`LiveNestedSegment` (`framework/src/live/document.rs`) is the one typed
declaration. `identity_bound` requires the includer's *literal* route pattern
and refuses any `{parameter}` pattern; `Router::try_live_nested_segment`
refuses at router construction, before a request is served, when two
declarations for the same inner route disagree on binding; and
`into_segment` re-checks the resolved path length on every conversion. Those
three layers exist for the length-stability constraint recorded in spec 16:
a nested `Composite`'s re-mount binds the *outer* document's path, which is
embedded in the signed snapshot, so an identity-bound inner segment's actual
assembled length varies with the includer's path length, and one stored
`assembled_len` cannot match two includers whose paths differ in length. The
system is safe without the layers, because `verify_nested` sees the mismatch
and resolves it through the declared policy; what is lost is sharing,
silently, as a performance cliff rather than a fault, so the declaration
surface refuses the shape rather than letting an author meet it as a bug.

Telemetry gained `suprnova.render_cache.stitch.nested`, which distinguishes
an inner segment's outcomes from an island slot's under a closed pair:
`outcome` from `resolved`, `omitted`, `fallback`, `failed`, and `cause` from
`none`, `fetch_failed`, `version_mismatch`, `length_mismatch`,
`depth_exceeded`, `cycle`, `unauthorized`, with `none` used exactly when
`outcome` is `resolved`.

The conformance case is two scenarios added to
`render_store_conformance::run_all`
(`crates/suprnova-live/crates/suprnova-live-test-support/src/render_store_conformance.rs`),
so no `RenderStore` provider can skip them: a well-formed nested graph
resolves and an excess-depth one is refused from the same two stored
entries, and a composite naming a segment kind this build does not
recognize is refused whole rather than assembled around, which is the shape
an older build meeting a newer build's entry actually has. The suite's
production consumer,
`crates/suprnova-live/tests/render_cache_store_conformance.rs`,
already ran `run_all` before this plan; what it lacked was a self-test under
`cargo test -p suprnova-live-test-support`, which `ledger_conformance.rs`
already had, so `render_store_conformance.rs` gained the same one.

### Credible generation hints

`framework/src/render_cache/hints.rs` holds the publisher, the subscriber,
the applier, and the lease table a hint reaches through. A hint names
dependency digests that just advanced somewhere; its only power is to make
this node revalidate earlier than its own lease would have, and the engine's
`ValidationLease::hint_invalidate` takes the minimum of the held expiry and
the instant offered, so nothing here can extend a lease, create one, prove an
entry current, bypass the ledger read a hit still makes, or touch the
authority epoch. That is why the channel is unauthenticated by design: the
worst a hostile publisher buys is one extra ledger read that returns the
truth.

Publication happens in `advance_through`
(`framework/src/render_cache/ledger.rs`) through a new `announce`, after
every statement for every identity has succeeded. All five advance entry
points reach that one function, so publishing anywhere else would announce
one path's writes and silently miss the others. `announce` hands digests to
a bounded `tokio::sync::mpsc` (`MAX_PENDING_HINTS`, 256) with `try_send` and
returns; the writing task never touches Redis. An advance wider than
`MAX_HINT_DIGESTS` (64) is split into several messages, each within the
bound, which is not truncation because no digest is discarded.

The wire is `srh1` followed by comma-separated lowercase-hex digests.
Decoding decides the digest count by counting separators before it decides
the byte length, and both run before any digest is parsed, so an over-bound
message is reported as over-bound rather than as merely long. A message over
the bound is dropped whole, never truncated, because a truncated hint is a
silently wrong hint.

A hint carries no instant. The publisher's clock is not comparable with the
receiver's, so the applier shortens a matched lease to *its own* `now_ms`,
which expires it; a node whose clock runs fast or slow therefore cannot move
a peer's lease in either direction, and no inter-node skew assumption is
needed. The staleness bound stays the lease's own `max_age_ms`, exactly as it
is with hints off.

Routing a digest to a lease is `LeaseTable`. The runtime's lease map became
`RenderKey -> { lease, observed }`, capturing the entry's observed digests at
the one moment both are in hand: `coherence`'s grant site, where
`header.observed` is exactly what the entry depends on. One structure, one
lock, one sweep - the `retain` that already bounded the lease map now evicts
each lease's digests with it, and `apply_hint` sweeps the same way, so the
bound holds on a node that receives hints and serves nothing. Applying a hint
walks the leases rather than indexing them, at most one entry per lease-mode
key requested within the last `max_age_ms`, each tested by binary search
against at most 64 digests, and it never runs on a request's task.

Two defects in that first delivery were found and fixed on the same branch,
in the same plan.

`hold_subscription` returned `Ok(true)` both when a subscription had carried
traffic and when this node dropped it because its bounded inbound queue was
full, and `subscribe_loop` read `true` as healthy and reset the reconnect
pause to 50 ms. A node whose applier was saturated therefore subscribed,
received, dropped, resubscribed, and fell behind again on a fifty-millisecond
cycle for as long as the load lasted, under exactly the traffic this channel
exists to carry, reporting one `subscriber_dropped` per cycle - a storm where
the truth is one node that cannot keep up. Three endings need three answers,
so the boolean became a closed `SubscriptionEnding`: `CarriedTraffic` resets,
`CarriedNothing` and `FellBehind` double to the same
`MAX_RESUBSCRIBE_BACKOFF` ceiling. The decision is `next_backoff`, a pure
function of the ending and the last pause, and `offer_to_applier` is the one
step that turns a full queue into that ending, so both halves are provable
without a Redis and without waiting. Establishing a subscription can only
fail above the message loop, so an error is an ending that carried nothing.
Every ending still counts exactly one `subscriber_dropped`.

`HintChannel::publish` dropped a message when its outbound queue was full
and counted nothing. Spec 18 requires a hint-channel failure to degrade
silently to today's behavior and to be visible only in telemetry - silent in
the request path, visible in telemetry - and that drop was invisible in both.
The closed four-value outcome set, written earlier in this same plan, had no
publish-side value; that omission was a defect in the specification rather
than a licence to leave the drop unobservable, so spec 18 widened to five and
`dropped_publish_queue_full` is the fifth. It is deliberately not
`dropped_over_bound`, which names a received message carrying more digests
than the bound allows: that is a peer sending something malformed, this is a
local publisher outrunning its own queue, and an operator answers the two
differently. It is counted once per abandoned message, not once per call: the
loop's `return` abandons the rest of a multi-chunk advance and not only the
chunk that did not fit, the message is the unit every other value on this
metric uses, and the number an operator reads is how much announcement was
lost.

The metric is `suprnova.render_cache.hints`, the ninth counter, whose
`outcome` attribute takes exactly one of `applied`, `ignored_unknown_key`,
`dropped_over_bound`, `subscriber_dropped`, and
`dropped_publish_queue_full`, and which never names a route, a key, a digest,
or a dependency identity.

`HintsConfig { Disabled, Redis { url, prefix } }` on `RenderCacheConfig` is
parsed from `RENDER_CACHE_HINTS`, defaulting to on for the Redis profile and
off for Embedded and Database-coordinated. It has no endpoint of its own: it
reuses `RENDER_CACHE_REDIS_URL` and `RENDER_CACHE_REDIS_PREFIX`, and its
hand-written `Debug` redacts the URL through the same constant every other
Redis configuration uses. The hint endpoint is deliberately absent from
`redis_endpoints()`, the boot-time reachability probe: spec 18 requires that
no new external daemon become required at any tier and that losing the
channel leave behavior identical, and a boot refusal over an unreachable
accelerator would break both.

### Manual

`manual/render-cache-operations.md` states that the render cache has nine
counters, documents `hints` and all five of its `outcome` values, and
documents `stitch.nested` and its `outcome`/`cause` pair;
`manual/render-cache-deployment.md` documents `RENDER_CACHE_HINTS`. All six
mirrors (`de`, `es`, `fr`, `ja`, `pt-BR`, `zh-Hans`) carry the same changes
with every inline code span byte-identical to English, and
`.manual-translations.lock` is restamped.

### Evidence

Nested segments, engine
(`crates/suprnova-live/src/render_cache/composite.rs`):
`descend_nested_permits_exactly_three_levels_and_rejects_a_fourth`,
`a_cycle_is_reported_even_when_it_would_also_exceed_the_depth_bound`,
`a_two_level_cycle_and_a_self_reference_both_fail_as_cycle_not_depth`,
`verify_nested_distinguishes_version_and_length_mismatch`,
`too_many_nested_segments_is_invalid`,
`a_composite_naming_itself_directly_is_refused_at_construction`,
`assembled_len_uses_the_named_nested_length_not_the_resolved_bodys_actual_length`,
`assemble_nested_rejects_an_over_bound_total_length_but_accepts_it_exactly_at_the_bound`,
`assemble_nested_fails_closed_on_a_version_or_a_length_mismatch`,
`assembling_past_the_depth_bound_fails_closed`,
`assembling_a_two_level_cycle_fails_closed`, and
`a_depth_three_nested_chain_assembles_and_its_length_is_the_named_sum`.
`cargo test -p suprnova-live render_cache` reports 57 passed, 0 failed.

Nested segments, framework (`framework/tests/render_cache/stitch.rs`):
`a_two_level_nested_document_assembles_and_serves`,
`a_version_mismatch_resolves_through_each_declared_policy`,
`a_nested_identity_bound_segment_is_reauthorized_per_request_and_never_leaks_across_identities`,
`an_identity_free_nested_segment_skips_reauthorization`,
`publishing_a_composite_naming_a_wider_inner_segment_is_refused`,
`publishing_a_composite_naming_a_longer_freshness_inner_segment_is_refused`,
`publishing_a_transitive_cycle_is_refused_at_publish`,
`publishing_beyond_the_nesting_depth_bound_is_refused_at_publish`, and
`publishing_a_composite_naming_a_private_cached_inner_segment_is_refused`.
The declaration surface is proven in `framework/tests/render_cache/live.rs`
by `identity_free_and_identity_bound_nested_segments_report_their_own_binding`,
`identity_bound_nested_segment_refuses_a_dynamic_includer_pattern`,
`a_nested_segment_fallback_is_bounded_at_declaration_and_never_printed`,
`into_segment_accepts_a_matching_includer_path_and_refuses_a_mismatched_one`,
`identity_free_nested_segments_accept_any_includer_path_length`, and
`router_refuses_a_conflicting_identity_binding_for_the_same_inner_route`.
The two conformance scenarios,
`a_well_formed_nested_graph_resolves_and_excess_depth_is_refused` and
`a_composite_naming_an_unknown_segment_kind_is_refused_not_ignored`, are not
themselves `#[test]` functions: they are cases inside
`render_store_conformance::run_all`, executed by
`render_store_conformance::tests::the_suite_passes_over_the_embedded_provider`
and by every provider that runs the suite.

Hints, without a Redis (`framework/tests/render_cache/hints.rs`, driving the
production decode, bound, apply, and telemetry path through
`RenderCache::deliver_hint_for_test`):
`a_hint_naming_an_observed_digest_makes_the_next_lookup_revalidate_earlier`,
`no_hint_makes_a_refused_entry_serve`,
`a_message_over_the_digest_bound_is_dropped_whole`,
`a_hint_naming_nothing_this_node_holds_is_ignored`,
`a_hint_never_extends_or_creates_a_lease`,
`a_dead_hint_channel_behaves_exactly_like_hints_switched_off` (the hints-off
equivalence property: the same six-step sequence compared whole - status,
body, renders, and statements at every step - between `Disabled` and a
channel pointed at a closed port), and
`an_unreachable_hint_channel_degrades_in_telemetry_and_not_in_the_response`.

Hints, unit level (`framework/src/render_cache/hints.rs`, and `config.rs` for
the two configuration tests): `a_message_round_trips`,
`an_empty_message_decodes_to_no_digests`,
`a_message_at_the_bound_is_read_and_one_past_it_is_dropped_whole`,
`a_foreign_message_is_malformed_rather_than_read`,
`a_hint_only_ever_shortens_a_lease`, `a_hint_never_creates_a_lease`,
`only_a_subscription_that_carried_traffic_resets_the_backoff`,
`a_message_meeting_a_full_inbound_queue_ends_the_subscription_as_fell_behind`,
`a_full_publish_queue_counts_every_message_it_abandons`,
`hints_default_to_the_profile_and_ride_the_shared_redis_endpoint`, and
`hints_can_be_turned_on_and_off_against_the_profile`. These run only under
`cargo test -p suprnova --lib --all-features render_cache`, which reports 95
passed, 0 failed, 2 ignored; `--all-targets` compiles them without executing
them, so that invocation is the one that proves them.

`cargo test -p suprnova --test render_cache --all-features` reports 358
passed, 0 failed, 34 ignored. Two of the ignored cases are the live-Redis
hint tests, `live_redis_a_published_hint_shortens_a_subscribing_nodes_lease`
and `live_redis_a_subscriber_that_falls_behind_is_dropped_and_resubscribes`
(`framework/tests/render_cache/tiers/redis.rs`); they need a reachable Redis
and were run against one, with
`REDIS_TEST_URL=redis://127.0.0.1:6379/15 cargo test -p suprnova --test
render_cache --all-features -- --ignored live_redis` reporting 19 passed, 0
failed. The unfiltered `-- --ignored` form does not pass in this environment:
it exits 101 with 19 passed and 15 failed, and every one of the 15 is a
`live_mysql_*` or `live_postgres_*` test reporting a missing `MYSQL_TEST_URL`
or `PG_TEST_URL`. Those need a MySQL and a Postgres this environment does not
run, and they fail identically without this plan's changes; no run of this
plan's own work is missing from the counts above because of them.

No sleep and no timing wait was added to any test in this plan; the live
tests synchronize on the process-global subscription count and on the
recorded telemetry outcomes, and the clock-based tests move the injectable
clock. The one `tokio::time::sleep` in `hints.rs` is the production
reconnect backoff, which nothing waits on and no test observes. No benchmark
budget, wire format, storage codec, or default feature set changed; the
stored entry format is version-governed by `ENTRY_FORMAT_VERSION`, written at
encode and refused at decode, and `Segment::Nested` was added under it.
