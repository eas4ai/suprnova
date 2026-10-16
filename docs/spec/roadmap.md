# Roadmap

Status: Draft. Not normative.

Order lives here. Filenames carry meaning, never sequence.

Current: qdrant-native-bm25

The developer named the work on 2026-09-13 08:52: the Live component
library, as scoped on 2026-09-12 and reconciled against Live specs 20-25.
The order below follows the developer's sequencing rulings: platform-first
components in waves, the advanced components last, the datatable closing.

0. live-and-render-cache-hardening - DONE 2026-09-13 (`fe65703c`, receipts
   `c9471a80`; one clause of LIVE-016 left open for the owner, see the
   review) - the thirteen defects of the
   2026-09-13 adversarial audit, in the audit's order: the four that
   change browser execution or replay bytes past the handler's HTTP
   contract first (CACHE-003, CACHE-004, CACHE-001, CACHE-002), then
   write invalidation (CACHE-009), database routing (CACHE-008), the
   Live authorization and transaction contracts (LIVE-016, LIVE-017),
   then CACHE-006, CACHE-005, LIVE-018, CACHE-010, CACHE-007. The owner
   ruled at 11:13 that this precedes the library.
1. live-session-revocation - DONE 2026-09-14 (`cf7332d1` and `3700c117`,
   receipts on `3700c117`; reopened once for LIVE-021 after the developer's
   `ok` on escalation `live-019`, plain `Auth::logout` keeping the session
   row) - the session and revocation-state clauses
   of LIVE-016 that the hardening left open: a membership whose session
   is destroyed on this node or another stops receiving events
   (LIVE-019, LIVE-020). The developer answered `ok` on escalation
   `live-016` at 21:21 on 2026-09-13: this precedes the library.
2. live-issuance-credentials - DONE 2026-09-14 (`6546bb31` and `a62136ab`,
   receipts on `a62136ab`; widened once for LIVE-023 after the developer's
   `ok` on escalation `live-022`, the issuance context race the baseline
   exposed) - the backlog observation from the LIVE-018 probe: concurrent
   issuances of one scope in the same millisecond mint identical
   descriptors and the later one's credential replaced the earlier one's,
   so 39 of 512 admitted requests answered 403 (LIVE-022). The developer
   named it the next commitment at 09:05 on 2026-09-14, ahead of the
   library.
3. component-library-foundations - DONE 2026-09-14 (built between
   `781b63c1` and `44e7bed7`; receipts on the closing tree; resumed after
   the developer's `ok` on escalation `loop-035`, keeping the v2.0.2
   release history as the base) - tokens, base layer, state-attribute
   styling, asset delivery, explicit registration and reserved
   namespaces, the checker and baseline mechanisms, and the form
   family's presentational tier (`component-library-foundations.md`,
   `component-library-forms.md`).
4. component-library-overlays - DONE 2026-09-14 (built between
   `e692a480` and `1efccb1d` after the developer's `ok` on escalation
   `ovl-001`, which agreed OVL-001 to OVL-006 and raised the browser
   baseline to the `popover` floor; receipts and review on `0ed49968`) -
   dialog, sheet, drawer,
   single-level dropdown menu, popover, tooltip, collapsible and
   accordion; opened with the anchor-positioning and popover-continuity
   spike in the qualification host (`component-library-overlays.md`).
   Tabs, listed here at first, was never in OVL-001 to OVL-006 and moves
   to item 5, where NAV-002 specifies it.
5. component-library-feedback-and-navigation - DONE 2026-09-15 (built
   between `7aef377a` and `cdc648b5` after the developer's `ok` on
   escalation `fdb-001`, which agreed FDB-001 to FDB-004, FDB-006,
   NAV-001 to NAV-004 and NAV-006; receipts and review on `344ff655`) -
   alert, toast and flash region, skeleton, progress, spinner, empty
   state; header bar, footer, sidebar, breadcrumbs, tabs, pagination,
   load more; the `live_key` view filter landed with them
   (`component-library-feedback.md`, `component-library-navigation.md`).
6. component-library-data-display - DONE 2026-09-15 (built between
   `2d9bec74` and `41e6a04b`; DATA-001 to DATA-005 agreed on escalation
   `data-001` under the developer's 2026-09-14 ruling that every
   remaining commitment is agreed; receipts and review on `cea49611`) -
   card, image and aspect ratio, scroll area, separator, badge, avatar,
   list group, description list, stat card; the chart through
   `charts-rs`; the datatable last (`component-library-data-display.md`).
7. component-library-live-native - DONE 2026-09-15 (built between
   `9c9883c0` and `dfeb91bb`; FORM-005 to FORM-008, FDB-005 and NAV-005
   agreed on escalation `form-005` under the developer's 2026-09-14 ruling;
   the Live gate failed four times and each cause was fixed on the tree,
   `e1a3492a`, `824c3919`, `3478e659` with `14efccaa`, and `2dc59f5a`;
   the review's two open findings were routed on escalation `form-008`;
   receipts `e81fce64`, review `9300e61a`) - upload widget, live feed,
   notification bell, account menu, input OTP, date picker, combobox; the
   custom-element enhancement layer landed here (`component-library-forms.md`
   custom-element tier, `-feedback.md`, `-navigation.md` behavioral tiers);
   the server-rendered chart had landed under DATA-004 in item 6.
8. session-request-serialization - DONE 2026-09-15 (built at `36287efb`
   after the race was pinned at `1a6c1dda`; the combined check after the
   kernel change exposed a webkit race in the input OTP's mirror and a
   Cairn lint on LIVE-020, fixed at `fa3e572e` and `ee60c838`; receipts
   `04be1af0`, `e5607e60`, `5c561a51`, review `7103d526`) - session
   blocking through the cache lock driver, the session write race the
   live-native review recorded as an open finding and the developer routed
   here on escalation `form-008` (`sessions.md`); `SessionBlock`,
   `SESSION_BLOCK`, `block_session` on every route and group builder, the
   dogfood application blocking on every route.
9. live-key-vocabulary - DONE 2026-09-16 (built at `159dd219`, the
   dogfood needles and one unformatted file fixed at `3fc2a965`
   after the first stale check; receipts `0d0d3ce8`, review
   `9c4ffc68`) - the runtime reads `live:key`, the attribute the checker
   validates and the manual names, for morph identity, controls and
   preservation scopes, and the library writes the key once; promoted
   from the backlog on escalation `ovl-006` (`live.md` LIVE-024). The form gallery's
   `.prevent` rode along, and its first browser case found a runtime
   defect that refused any Live submit from a form holding an empty
   number input or an unselected select, fixed in the same build.
10. checker-proves-runtime-accepts - DONE 2026-09-16 (built at
   `f7ca27ca`; the first Live gate found a session lock held past an
   abandoned request, kept on escalation `loop-035-sess-001` at `0e51a13e`,
   and a save form over the eight-proposal cap, fixed at `e88a9596`;
   receipts `fc6946b5`, review `75e095a2`) - `live:check` proves a component only
   after checking every element its view renders, a declared debounce is
   one the grammar and the runtime accept, and error feedback may target
   an action; promoted on escalation `live-024` and widened on `live-008`
   after the build found an empty call block hiding 24 errors in the form
   gallery (`live.md` LIVE-025 to LIVE-027).
11. live-protocol-bounds - DONE 2026-09-16 (built at `dc031353`, receipts
   `7b4918d7`, review `46bbc3da`; the browser caps turned out to be
   unspecified and below the server's, so the browser was aligned to 128)
   - the browser admits the message counts the
   framework's protocol limits admit, `live:check` refuses a submit form
   one request cannot carry, and a request refused for size fails visibly;
   promoted on escalation `live-027` after the dogfood save form's ten
   fields never submitted (`live.md` LIVE-028 to LIVE-030).

12. live-library-review-remediation - DONE 2026-09-17 (built between
   `5fc3d651` and `c7c12fdd`, receipts `92e2703b`, review `53309c54`; an
   independent review of the build found three more defects, fixed in the
   same build, and `live:check` learned to expand a control's rendered state
   once so a library form stays under its branch limit) - the adversarial
   review of Live and
   the component library before the 2.1.0 release found eighteen defects,
   among them a combobox that freezes the page, form macros that render no
   island state, undecodable proposals dropped with an accepted outcome, and
   keys the checker proves and the runtime refuses; promoted on the
   developer's direction of 2026-09-17 (`component-library-forms.md` FORM-008
   to FORM-012, `component-library-feedback.md` FDB-007,
   `component-library-navigation.md` NAV-007, `component-library-overlays.md`
   OVL-007, `component-library-data-display.md` DATA-006,
   `component-library-foundations.md` UI-020 to UI-024, `live.md` LIVE-031
   to LIVE-036).

13. live-model-render-baseline - DONE 2026-09-17 (built at `661e7310`,
   formatted at `3c868f88` after the first check, receipts `3166159b`, review
   `4c1ba597`; the settle step also moved `dirty` onto the last applied render,
   which nothing had updated since mount) - after a refused value and a render
   that replaces it,
   the browser sent nothing when the same value was typed again, leaving a
   control and its island out of step with no error; found while proving the
   remediation's reset, promoted on the developer's `ok` to escalation
   `live-031` (`live.md` LIVE-037).

14. tooltip-dismissal - DONE 2026-09-17 (built at `2c137bf4`, documented
   at `1a706450`, receipts `78803f43`, review `abcbe761`; the Live gate needed
   five runs, four of them red on intermittent cases outside this work, now in
   the backlog with a measured rate) - a tooltip bubble that covers content
   cannot be dismissed
   where it is shown, which WCAG 2.2 success criterion 1.4.13 requires;
   specified 2026-09-17 in the phase between loops, which revised OVL-002
   to admit a dismissal enhancement and agreed OVL-008
   (`component-library-overlays.md` OVL-002 revised and OVL-008).

15. upload-store-write-completion - DONE 2026-09-17 (fixed at `12a7df56`,
   receipts `b5dcccb0`, review `89c54fd4`; six failures in sixty runs without
   the flush, none in a hundred and twenty with it, and the Live gate green on
   the first run after it) - the store the Live gate runs the upload provider
   against completed a write while the bytes were still in a tokio file's
   buffer, so the provider's own tests failed about one whole-file run in
   fifteen and four gate runs went red on 2026-09-17; promoted on the
   developer's ruling answering escalation `live-010` (`live.md` LIVE-038).

## laravel-port-requests

Requirements: PAR-001, PAR-002, PAR-003, PAR-004, PAR-005, PAR-006, PAR-007, PAR-008, PAR-009

16. laravel-port-requests - the requests an application team porting a
   Laravel 13 application filed as issues #125 to #129, and the queue fake
   gaps found while recording the parity rulings: joins and the remaining
   `where` helpers on both query builders, file and download responses,
   `#[authorize]` on handlers, a model's changes after a save,
   `sync_without_detaching`, `DB::after_commit`, a configurable Inertia
   page lookup, and the queue fake's `except`, raw pushes and
   `assert_pushed_without_chain` (`laravel-parity.md` PAR-001 to PAR-009).
   The developer asked for issues #125 to #129 to be done on 2026-10-01;
   the queue fake gaps are among the recommendations the developer accepted
   on 2026-09-30. This is the first commitment cut from the parity map.

## chart-tokens-and-test-diagnostics

Requirements: DATA-007, PAR-010, PAR-011

17. chart-tokens-and-test-diagnostics - the chart takes its colors and
   font from `--sn-` tokens through a six-color chart palette defined for
   light and dark (`component-library-data-display.md` DATA-007, backlog
   item `chart-follows-the-tokens`); and a failed request carries its own
   error report, which `TestResponse` shows in a failing assertion
   (`laravel-parity.md` PAR-010 and PAR-011, the developer's first priority
   from the third review). Done when the chart renders in a dark document
   with no literal color in its SVG, and a test's failing status assertion
   names the error that caused the 500, with both mechanisms passing on a
   committed tree.

## debug-error-page

Requirements: PAR-010, PAR-012, PAR-013, PAR-014, PAR-015

18. debug-error-page - with debug on, a 5xx that carries an error report
   renders a self-contained development error page for a browser or
   Inertia request: the error chain or panic, the stack frames where the
   error began, and the request with its secrets redacted
   (`laravel-parity.md` PAR-012 to PAR-015, next-feature item
   `debug-error-page`, the developer's second priority from the third
   review); and PAR-010's last sentence holds with debug off (next-feature
   item `par-010-names-debug-off`). Done when, with debug on, a browser
   request to a failing handler gets the page with the error, the
   handler's frame and the redacted request, even with no frontend build;
   with debug off every response is unchanged; and both mechanisms pass on
   a committed tree.

## default-disk

Requirements: PAR-016, PAR-017

19. default-disk - `Storage::default_disk()` returns the disk
   `Storage::set_default_disk` or `FILESYSTEM_DISK` names, and startup
   fails when that disk is not registered (`laravel-parity.md` PAR-016
   and PAR-017, next-feature item `default-disk`, the developer's third
   priority from the third review). Done when the mechanism passes on a
   committed tree.

## sqs-queue-driver

Requirements: PAR-018, PAR-019, PAR-020

20. sqs-queue-driver - `QUEUE_DRIVER=sqs` queues jobs on Amazon SQS
   standard queues, configured from the environment as Laravel's
   connection is, with delays past 15 minutes, attempt counts that match
   the other drivers, and overflow of large payloads to a disk
   (`laravel-parity.md` PAR-018 to PAR-020, next-feature item
   `sqs-queue-driver`). Done when the mechanism passes on a committed
   tree.

## process-facade

Requirements: PAR-021, PAR-022, PAR-023, PAR-024, PAR-025

21. process-facade - `Process::command(args)` runs a program with its
   arguments and no shell, captures its output, kills it with its children
   on a timeout, an idle timeout or a drop, runs pools with bounded
   concurrency and pipes, and fakes every process in tests
   (`laravel-parity.md` PAR-021 to PAR-025, next-feature item
   `process-facade`). Done when the mechanism passes on a committed tree.

## log-channels

Requirements: PAR-026, PAR-027, PAR-028, PAR-029, PAR-030

22. log-channels - `LOG_CHANNEL` picks the channel log events go to,
   stdout unless set, among stdout, stderr, single and rotating files,
   syslog, null and stacks, with a `Log` facade for channels, stacks and
   on-demand channels, retention, and flushing on shutdown
   (`laravel-parity.md` PAR-026 to PAR-030, next-feature item
   `log-channels`). Done when the mechanism passes on a committed tree.

## redis-facade

Requirements: PAR-031, PAR-032, PAR-033, PAR-034

23. redis-facade - `Redis::connection(name)` over named connections the
   environment and the bootstrap define, with typed common commands, any
   command, the client as an escape hatch, events, pipelines, transactions,
   and dedicated connections for subscriptions and blocking commands
   (`laravel-parity.md` PAR-031 to PAR-034, next-feature item
   `redis-facade`). Done when the mechanism passes on a committed tree.

## str-number-subset

Requirements: PAR-035, PAR-036, PAR-037

24. str-number-subset - `Str::slug`, `mask`, `limit`, `limit_words` and
   `excerpt`; `Str::plural` and `singular` by the language of the current
   locale, with doctrine/inflector's six rule sets; `Lang::percentage` and
   `Lang::abbreviate` (`laravel-parity.md` PAR-035 to PAR-037, next-feature
   item `str-number-subset`). Done when the mechanism passes on a
   committed tree.

## schema-dump

Requirements: PAR-038, PAR-039, PAR-040

25. schema-dump - `schema:dump` writes the schema and the migration ledger
   with each engine's own tool, `migrate`, `migrate:fresh`, `serve` and
   `TestDatabase::fresh` load it into a database with no applied
   migration before newer migrations run, and `--prune` replaces the
   dumped migrations with `PrunedMigration` names
   (`laravel-parity.md` PAR-038 to PAR-040, next-feature item
   `schema-dump`). Done when the mechanism passes on a committed tree,
   against SQLite, Postgres, MySQL and MariaDB.

## memory-footprint

Requirements: MEM-001, MEM-002, MEM-003, MEM-004, MEM-005, MEM-006, MEM-007

26. memory-footprint - every validated item of the 2026-10-02 allocation
   audit addressed: memory a long-running process kept for finished work,
   capacity reserved and never used, full copies on request and job paths,
   the browser runtime's buffers and the tooling's; the defects validation
   found beside them; and heap profiling with dhat behind the
   `heap-profiling` feature, with a `profiling` Cargo profile
   (`memory.md` MEM-001 to MEM-007). Done when the mechanism passes on a
   committed tree, and a dhat profile of the dogfood app on one fixed
   workload, before and after, is in the delivery report.

## application-port-second-round

Requirements: PAR-041, PAR-042, PAR-043, PAR-044, PAR-045, LIVE-039

27. application-port-second-round - issues #137, #139 and #140 from the
   application port, which the developer asked for on 2026-10-04: the
   provider avatar on every OAuth identity, with the Facebook profile
   request naming its fields (PAR-041); the multipart request's
   asynchronous hook, stage order and field-level validation errors that
   reach an Inertia form (PAR-042, PAR-043); the model key type taken
   from its field, `u64` keys on every database, and the app-wide
   date-time cast and unsigned-key settings (PAR-044, PAR-045); and the
   Live gate's browser suite reset that cancels a stalled test's uploads
   (LIVE-039). Delivery includes a release of the GitHub provider crate
   that fills `avatar_url`. Done when the mechanisms pass on a committed
   tree, the schema ones against SQLite, Postgres and MySQL.

## live-components-sdk

Requirements: REG-001, REG-002, REG-003, REG-004, REG-005, REG-006, REG-007, REG-008, REG-009, REG-010, REG-011, REG-012, REG-013, REG-014, REG-015, REG-016, REG-017, REG-018, REG-019, REG-020, REG-021, REG-022, REG-023, REG-024, REG-025, REG-026, REG-027, REG-028, REG-029, REG-030, REG-031, REG-032, REG-033

28. live-components-sdk - the Suprnova Live components SDK the developer
   asked for on 2026-10-03 and made the primary focus of the next release
   on 2026-10-05: third-party component libraries published as signed,
   tagged repositories (REG-001 to REG-005, REG-023 to REG-026, REG-033);
   `live:add` by repository address with a plan the developer confirms,
   trust on first use, capabilities approved per install, and provenance
   in the lowercase `suprnova.toml` (REG-006 to REG-013, REG-027, REG-028);
   validation inside the pull that compiles and runs nothing: the hash,
   the signature, and the AST scans of Rust, views and scripts against the
   Suprnova and Live API, with no crates and no overrides (REG-014 to
   REG-022, REG-029 to REG-032); each library's assets served under its
   own root (REG-017); and the authoring commands, scaffold, checks and
   manual chapters (REG-018 to REG-021). Delivery includes the shipped
   library moved to the one manifest format. Done when the four
   mechanisms pass on a committed tree: `registries`, `registries-scan`
   (its bypass corpus refused), `registries-compile` (an installed
   component compiles and renders in a scaffolded application) and
   `registries-serve`.

## application-port-third-round

Requirements: BIND-001, BIND-002, BIND-003, BIND-004, BIND-005, BIND-006, BIND-007, BIND-008, BIND-009, BIND-010, BIND-011, BIND-012, BIND-013, BIND-014, BIND-015, PFX-001, PFX-002, PFX-003, PFX-004, PFX-005, PFX-006, PFX-007, PFX-008, PFX-009, PFX-010, PFX-011, PFX-012, PFX-013, IMG-001, IMG-002, IMG-003, IMG-004, IMG-005, IMG-006, IMG-007, IMG-008, LDB-001, LDB-002, LDB-003, LDB-004, LDB-005, LDB-006, LDB-007, LDB-008, LDB-009, LDB-010, LDB-011, LDB-012, MEM-003, REG-031

29. application-port-third-round - the next four issues from the
   application port (#141, #142, #144, #145), which the developer ruled on
   2026-10-05: route model binding with custom keys and columns, scoped
   child bindings, custom resolution, soft deletes, missing rows, enums,
   resources, binding by type and URL generation (BIND-001 to BIND-015);
   serving under a trusted path prefix with one public root for every
   browser URL, cookie path and cache key, and `url::root()` for views
   (PFX-001 to PFX-013, with REG-031 revised so a component view can write
   the root); upright images with their metadata stripped and the rest of
   Laravel's Image API (IMG-001 to IMG-008, with MEM-003's byte-for-byte
   rule given its narrow exceptions); and running on a database Laravel
   created, with Laravel's layouts, an in-place upgrade, and live sharing
   with a Laravel application (LDB-001 to LDB-012). Delivery includes the
   manual chapters and corrections each spec names, the scaffold's
   templates, the regenerated feature map and component allowlist, and the
   CHANGELOG's breaking-change notes. Done when the six mechanisms pass on
   a committed tree: `route-binding`, `path-prefix`, `images`,
   `laravel-database`, `mem-footprint` and `registries-scan`.

## application-port-third-round-revised

Requirements: BIND-001, BIND-002, BIND-003, BIND-004, BIND-005, BIND-006, BIND-007, BIND-008, BIND-009, BIND-010, BIND-011, BIND-012, BIND-013, BIND-014, BIND-015, PFX-001, PFX-002, PFX-003, PFX-004, PFX-005, PFX-006, PFX-007, PFX-008, PFX-009, PFX-010, PFX-011, PFX-012, PFX-013, IMG-001, IMG-002, IMG-003, IMG-004, IMG-005, IMG-006, IMG-007, IMG-008, LDB-001, LDB-002, LDB-003, LDB-004, LDB-005, LDB-006, LDB-007, LDB-008, LDB-009, LDB-010, LDB-011, LDB-012, MEM-003, REG-031

30. application-port-third-round-revised - application-port-third-round
   carried over unchanged, except that MEM-003's exception also names the
   JPEGs the built-in oxideav driver writes, corrected from RGB samples to
   JFIF YCbCr (the developer's answer to escalation 81c49927 on
   2026-10-06). Same requirements, delivery and mechanisms: done when
   `route-binding`, `path-prefix`, `images`, `laravel-database`,
   `mem-footprint` and `registries-scan` pass on a committed tree.

## application-port-third-round-bind

Requirements: BIND-001, BIND-002, BIND-003, BIND-004, BIND-005, BIND-006, BIND-007, BIND-008, BIND-009, BIND-010, BIND-011, BIND-012, BIND-013, BIND-014, BIND-015, PFX-001, PFX-002, PFX-003, PFX-004, PFX-005, PFX-006, PFX-007, PFX-008, PFX-009, PFX-010, PFX-011, PFX-012, PFX-013, IMG-001, IMG-002, IMG-003, IMG-004, IMG-005, IMG-006, IMG-007, IMG-008, LDB-001, LDB-002, LDB-003, LDB-004, LDB-005, LDB-006, LDB-007, LDB-008, LDB-009, LDB-010, LDB-011, LDB-012, MEM-003, REG-031

31. application-port-third-round-bind - application-port-third-round-revised
   carried over unchanged, except that BIND-003 adds a fifth change (a
   `#[handler]` inside an `impl` block names its type with
   `#[handler(Self = Type)]`) and BIND-012 defines a bound value as a
   `RouteValue` and lists the four string-pair forms that compiled only
   through the old slice parameter (the developer's answer to escalation
   0c8024de on 2026-10-06). Same requirements, delivery and mechanisms:
   done when `route-binding`, `path-prefix`, `images`, `laravel-database`,
   `mem-footprint` and `registries-scan` pass on a committed tree.

## application-port-third-round-final

Requirements: BIND-001, BIND-002, BIND-003, BIND-004, BIND-005, BIND-006, BIND-007, BIND-008, BIND-009, BIND-010, BIND-011, BIND-012, BIND-013, BIND-014, BIND-015, PFX-001, PFX-002, PFX-003, PFX-004, PFX-005, PFX-006, PFX-007, PFX-008, PFX-009, PFX-010, PFX-011, PFX-012, PFX-013, IMG-001, IMG-002, IMG-003, IMG-004, IMG-005, IMG-006, IMG-007, IMG-008, LDB-001, LDB-002, LDB-003, LDB-004, LDB-005, LDB-006, LDB-007, LDB-008, LDB-009, LDB-010, LDB-011, LDB-012, MEM-003, REG-031

32. application-port-third-round-final - application-port-third-round-bind
   carried over unchanged, except that IMG-001 has `magick` apply the
   orientation the framework reads, as fixed rotations and flips, with
   `-auto-orient` only for a format the framework cannot read, and PFX-010
   says `url::to` and `url::secure` turn a relative path into an absolute
   URL under the root (the developer's answer to escalation 5ef445fa on
   2026-10-06). Same requirements, delivery and mechanisms: done when
   `route-binding`, `path-prefix`, `images`, `laravel-database`,
   `mem-footprint` and `registries-scan` pass on a committed tree.

## application-port-third-round-closing

Requirements: BIND-001, BIND-002, BIND-003, BIND-004, BIND-005, BIND-006, BIND-007, BIND-008, BIND-009, BIND-010, BIND-011, BIND-012, BIND-013, BIND-014, BIND-015, PFX-001, PFX-002, PFX-003, PFX-004, PFX-005, PFX-006, PFX-007, PFX-008, PFX-009, PFX-010, PFX-011, PFX-012, PFX-013, IMG-001, IMG-002, IMG-003, IMG-004, IMG-005, IMG-006, IMG-007, IMG-008, LDB-001, LDB-002, LDB-003, LDB-004, LDB-005, LDB-006, LDB-007, LDB-008, LDB-009, LDB-010, LDB-011, LDB-012, MEM-003, REG-031

33. application-port-third-round-closing - application-port-third-round-final
   carried over unchanged, except that BIND-004 says a handler the route
   did not plan at startup binds under the route's settings at the
   request, and BIND-012 lists every string-pair form a generic parameter
   cannot accept (the developer's answer to escalation 39578e39 on
   2026-10-06). Same requirements, delivery and mechanisms: done when
   `route-binding`, `path-prefix`, `images`, `laravel-database`,
   `mem-footprint` and `registries-scan` pass on a committed tree.

## release-foundations

Requirements: RTC-001, RTC-002, RTC-003, RTC-004, MEM-003, REG-025, REG-032, REG-033

34. release-foundations - the runtime contention issues #146 to #149
   (RTC-001 to RTC-004); the image codecs pinned to the eas4ai forks of
   oxideav-png, oxideav-bmp and oxideav-webp under MEM-003, with the
   memory tests' copy allowances dropped; `library.json`'s one key set
   (REG-025 with REG-033) and relative script imports (REG-032); and four
   backlog items delivered beside them: Magnetar's rustdoc link that
   needs the two-factor feature, the render-cache and route-binding tests
   that assert a bare hex-safe name, the Live public API test that
   rendered HTML rustdoc, and the six en dashes in source comments with
   the dash check extended to U+2013. Delivery: `HASH_MAX_CONCURRENCY` in
   the configuration and hashing chapters, the responses and rate
   limiting chapters where the behaviour shows, and a changelog entry.
   Done when `runtime-contention`, `mem-footprint`, `registries-scan` and
   `registries` pass on a committed tree.

## release-foundations-revised

Requirements: RTC-001, RTC-002, RTC-003, RTC-004, MEM-003, REG-025, REG-032, REG-033

35. release-foundations-revised - release-foundations carried over
   unchanged, except that MEM-003 names the output changes the pinned
   forks bring from upstream master: the corrected lossy WebP decode, PNG
   above 1 MiB of filtered data deflated in segments, and lossless WebP
   with alpha as a simple `VP8L` file (the developer's words of 2026-10-07
   08:30 and 17:35). The forks also fix what upstream master regressed
   against IMG-002 and MEM-003: the PNG decoder inflating every `iCCP` and
   XMP chunk before the budget counts it, the BMP decoder copying an
   embedded profile, and the BMP encoder's borrowed-profile entry point
   left deprecated. Same delivery, mechanisms and done-when as 34.

## inertia-protocol

Requirements: PAR-046, PAR-047, PAR-048, PAR-049, PAR-050, PAR-051, PAR-052, PAR-053, PAR-054, PAR-055, PAR-056, RDOC-001, RDOC-002, RDOC-003, RDOC-004, RDOC-006

36. inertia-protocol - the first of the parity rounds the developer ordered
   on 2026-10-07 (17:35: "then the sixteen parity commitments in
   docs/superpowers/notes/parity-rulings-2026-10-07.md in the order of
   that log's table"): the Inertia server adapter's protocol, 61 of the 62
   rows of the first group (inertia-laravel 3.5.1 response factory, props
   resolution, prop types, headers, middleware, JSON encoding, response,
   helpers and blade; Inertia.js 3.8.0 html shell, request body, response
   header and server behaviour), the client nonce row H03 left to the
   security-headers commitment with RDOC-005.
   Delivery: PAR-046 to PAR-056 built in `framework/src/inertia/` and the
   request's form and query decoding (`framework/src/http/`); the
   application-owned root template of RDOC-001 to RDOC-004 and RDOC-006,
   with the scaffold's root template, default title and Dockerfile copy
   (`suprnova-cli/src/templates/`); the dogfood application exercising a
   root template and the new facade surface; `manual/frontend-inertia-responses.md`
   and `manual/frontend.md` updated, the narrowing divergence entry and the
   always-409 `location` claim removed, a passage on working with a coding
   assistant on Inertia pages (HM-05); `feature-map/laravel/parity.jsonl`
   rows of the group linked to the shipped items; the changelog.
   Mechanisms: `par-inertia-protocol` (the Inertia test binary, the `inp_`
   tests of the http binary and the inertia unit tests), `inertia-root-template`
   (the `rdoc_` tests of the inertia, memory and suprnova-cli binaries).
   Done when both pass on a committed tree, the manual passages are in
   place, and the review is recorded.

## inertia-ssr-errors-commands

Requirements: PAR-057, PAR-058, PAR-059, PAR-060, PAR-061, PAR-062, RDOC-002, REG-032, REG-033

37. inertia-ssr-errors-commands - the second of the parity rounds the
   developer ordered on 2026-10-07 (17:35): inertia-laravel 3.5.1's SSR
   gateway, exception handling and commands, the fifteen build rows of the
   second group (SS-02, SS-03, SS-04, SS-05, SS-08, SS-10, SS-13, SS-14,
   EX-01, EX-02, EX-03, EX-06, EX-07, CM-01, CM-02), with the three
   next-feature items the first round left: RDOC-002's baseline reworded,
   REG-033's `--yes` clarified and REG-032 extended to the replacement of a
   built-in function.
   Delivery: PAR-057 to PAR-060 in `framework/src/inertia/ssr.rs` and the
   gateway binding; PAR-061 in the application CLI (`framework/src/app/`
   and `framework/src/console/`) with the `suprnova` CLI's `ssr:*`
   commands on the shared implementation (`suprnova-cli/src/commands/`);
   PAR-062 in `framework/src/inertia/` with `InertiaConfig::error_page` as
   sugar; the REG-032 refusal in the registry script scan
   (`suprnova-cli/src/registry/scan/`) with fixtures; the dogfood
   application on the defaults; the manual's SSR, error-page and console
   passages and the registries chapter's words for `--yes` and built-in
   replacement; the changelog.
   Mechanisms: `par-inertia-ssr` (the `inssr_` tests of the inertia, error
   and console binaries and the unit tests, and the `inssr_` tests of the
   CLI's `ssr_e2e` binary), `inertia-root-template` rebound on RDOC-002's
   revised text, `registries` rebound on REG-033's and `registries-scan`
   on REG-032's.
   Done when the four pass on a committed tree, the manual passages are in
   place, and the review is recorded.

## inertia-testing-and-types

Requirements: PAR-063, PAR-064, PAR-065, PAR-066, PAR-067, PAR-068, PAR-069, PAR-070, PAR-058

38. inertia-testing-and-types - the third of the parity rounds the
   developer ordered on 2026-10-07 (17:35): inertia-laravel 3.5.1's
   testing surface and the Inertia.js 3.8.0 types the generator writes,
   the sixteen build rows of the third group (TS-01, TS-03, TS-06, TS-07,
   TS-08, TS-10, TS-12, TS-13, TS-14, TS-15, TS-16, T01, T02, T03, T05,
   T06), with the next-feature item the second round left: PAR-058
   reworded to name the hot file and the plugin, as the code does.
   Delivery: PAR-063 to PAR-067 in `framework/src/testing/` (the in-process
   `TestClient`, `TestResponse` and `AssertableInertia`) with
   `InertiaConfig::testing_ensure_pages_exist` in `framework/src/inertia/`;
   PAR-068 to PAR-070 in the `suprnova` CLI's generators
   (`suprnova-cli/src/commands/generate_types.rs` and `generate_routes.rs`),
   the `#[inertia_props(shared)]` and `#[inertia_props(flash)]` markers on
   the derive (`suprnova-macros/`), and the starter kits' `package.json`,
   generated types file and pages (`suprnova-cli/src/templates/`); the
   dogfood application's tests on the client and its generated file
   regenerated; the manual's `http-tests.md` Inertia passages rewritten
   around the client, `frontend-typescript-types.md` with the page types,
   the augmentation, the integer mapping and the route component, and
   `frontend-inertia-responses.md` where SSR hot mode is described; the
   changelog.
   Mechanisms: `par-inertia-testing` (the `intt_` tests of the testing and
   inertia binaries and the unit tests, and the `intt_` tests of the CLI's
   generator and template drift binaries), `par-inertia-ssr` rebound on
   PAR-058's revised text.
   Done when both pass on a committed tree, the manual passages are in
   place, and the review is recorded.

## inertia-devtools

Requirements: PAR-071, PAR-072, PAR-073, PAR-074, PAR-075, PAR-061, PAR-068

39. inertia-devtools - the fourth of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build at 17:23: the server side of
   the Inertia DevTools extension, the eleven build rows of the fourth
   group (DT-01 to DT-10 and the Precognition group's row 057), with the three next-feature items
   the third round left: PAR-061 revised so the `suprnova` CLI's `ssr:*`
   commands run the application binary and the CLI drops the framework
   crate (the developer, 2026-10-08 10:58), and PAR-068 revised to name
   `Inertia::share_data`, to type the errors under `with_all_errors(true)`
   and to skip the augmentation beside a hand-written one (the developer's
   heads-up of 11:10).
   Delivery: PAR-071 to PAR-075 in `framework/src/inertia/devtools/` (the
   gate, the recorder and collector, the prop classifier, the file
   repository, redaction, the entries endpoints as middleware the stack
   carries) with `InertiaConfig::devtools` and `DevToolsConfig`, the
   `X-Inertia-Devtools-*` headers and the first-visit tag in the Inertia
   middleware and response, `#[track_caller]` share and render sources;
   the PAR-061 delegation in `suprnova-cli/src/commands/ssr_*.rs` with the
   framework dependency removed from `suprnova-cli/Cargo.toml` and the
   `ssr_e2e` tests rewritten against a fake application binary; the
   PAR-068 revisions in `suprnova-cli/src/commands/generate_types.rs`; the
   dogfood application recording in its tests; a new manual chapter
   `manual/frontend-inertia-devtools.md` linked from
   `manual/documentation.md` and `manual/frontend.md`, the console and
   types chapters updated; the changelog.
   Mechanisms: `par-inertia-devtools` (the `indt_` tests of the inertia
   binary and the unit tests), `par-inertia-ssr` rebound on PAR-061's
   revised text, `par-inertia-testing` rebound on PAR-068's.
   Done when the three pass on a committed tree, the manual passages are
   in place, and the review is recorded.

## inertia-starter-kits

Requirements: PAR-076, PAR-077, PAR-078, PAR-079, PAR-080, PAR-071, PAR-074

40. inertia-starter-kits - the fifth of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build at 17:23: the three starter
   kits on Inertia.js 3.8.0, the fifteen build rows of the starter-kit
   group (K01, K05, K06, K07, K10, K11, K12, K13, K14, K15, K16, K17, K18,
   K19 and K21), K10 met by PAR-068 already; with PAR-071 and PAR-074 reworded to name
   `APP_ENV=local` under the developer's ruling of 2026-10-08 14:25
   (decision 01M4EC8TAKX6XXKXBTT7K5S60M).
   Delivery: PAR-076 to PAR-079 in `suprnova-cli/src/templates/files/frontend/{svelte,react,vue}/`
   (manifests at `^3.8.0` with `@inertiajs/vite`, the Vite configs and
   entries on the plugin, layouts, the flash toast, `Link`, `Head`, `Form`,
   the dashboard and notes pages with `Deferred`, `WhenVisible`,
   `InfiniteScroll`, `usePoll`, `usePrefetch`, `useRemember`, `useHttp`,
   optimistic updates and instant visits) and PAR-080 in
   `suprnova-cli/src/templates/files/backend/` (the dashboard, notes and
   profile handlers, the flash struct, the routes); `suprnova new` writing
   the new files; the 3.8.0 citations in `framework/src/inertia/`, the
   scaffold comments and `manual/csrf.md`; the dogfood `app/frontend` on
   the same packages with `Link`, `Head` and `InfiniteScroll` on its users
   page; the mechanism's tests in `suprnova-cli/tests/kit_pages.rs`,
   `kit_typecheck.rs` and `kit_browser.rs` with its Playwright suite;
   `manual/starter-kits.md`, `manual/frontend-pages.md` and
   `manual/frontend.md` updated; the changelog.
   Mechanisms: `par-starter-kits` (the `kit_` text tests over fresh
   scaffolds, the type-check and build of each kit, and the browser suite
   over a built scaffold).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.

## precognition

Requirements: PAR-081, PAR-082, PAR-083, PAR-084, PAR-085, PAR-086, PAR-087, PAR-088

41. precognition - the sixth of the parity rounds the developer ordered on
   2026-10-07 (17:35), ruled build by the parity default: the thirty-one
   build rows of the Precognition group (rows 002, 003, 006,
   009, 012, 013, 016, 018, 019, 021, 022,
   026, 027, 028, 029, 030, 031, 032, 033,
   034, 035, 036, 037, 039, 040, 043, 047,
   049, 061 and 063; row 057 met by PAR-072 already), with
   rows 001 and 017 kept as the developer listed them.
   Delivery: PAR-081 to PAR-084 in `framework/src/http/precognition.rs`
   (the middleware, the request marks and predicates, the helper, the
   validate-only match), `framework/src/http/form_request.rs` and
   `suprnova-macros/src/request.rs` (rules narrowed before validation,
   query and multipart data, the inline `Request::validate`), the handler
   dispatch answering `204`; PAR-085 in `framework/src/session/middleware.rs`
   and the Inertia middleware's previous-URL store; PAR-086 in
   `framework/src/http/response.rs`; PAR-087 in `framework/src/testing/`;
   PAR-088 as `manual/precognition.md` linked from
   `manual/documentation.md`, with `manual/validation.md` and
   `manual/requests.md` updated; the scaffold's auth routes carrying the
   middleware where a kit page validates live; the changelog.
   Mechanisms: `par-precognition` (the `precognition` tests of the
   `http`, `session` and `testing` suites).
   Done when it passes on a committed tree, the manual chapter is in
   place, and the review is recorded.

## laravel-parity-round-4

Requirements: PAR-001, PAR-002, PAR-003, PAR-004, PAR-005, PAR-006, PAR-009, PAR-012, PAR-013, PAR-016, PAR-017, PAR-018, PAR-019, PAR-020, PAR-021, PAR-022, PAR-025, PAR-026, PAR-027, PAR-030, PAR-031, PAR-032, PAR-035, PAR-036, PAR-037, PAR-038, PAR-042, PAR-043, PAR-045

42. laravel-parity-round-4 - the seventh of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build by the parity default: the
   PAR rows of the log where Suprnova differs from Laravel 13.35.0 (rows
   001, 002, 003, 004, 005, 006, 009, 012, 013, 016, 017, 018, 019, 020,
   021, 022, 025 with 025.2, 026, 027, 030, 031, 032, 035, 036, 037 with
   037.1, 038, 042, 043 and 045), revised in place under the same
   identifiers; row 024 and the keep rows stay as they are.
   Delivery: the flat `or`, `where_column`'s operator, the cross-join
   closure and the joined `update` and `delete` in
   `framework/src/database/` and `framework/src/eloquent/`; ranges,
   `Last-Modified`, headers, disposition and the streaming download in
   `framework/src/http/response.rs` and the filesystem responses; guest
   and enum abilities in the `#[authorize]` attribute; clean saves,
   `get_previous`, `sync`'s lists, pivot columns, filters and parent touch
   in `framework/src/eloquent/`; the fake's closure filter in
   `framework/src/queue/`; the production HTML error view, source
   snippets and per-error traces in `framework/src/error/`; the `local`
   default and the `AWS_*` names in `framework/src/filesystem/`; the
   per-queue sizes, FIFO messages and the overflow store in the SQS
   driver; `env` removal, the `tty` check, quoted command lines, `stop`'s
   grace and signal, the fake's closure handlers, line order and
   defaults in `framework/src/process/`; the `custom` channel, the
   seven-day retention and the `APP_NAME` ident in `framework/src/logging/`;
   the `REDIS_*` names, the `cache` connection, the key prefix,
   `evalsha`, `execute_raw` and the dispatcher events in
   `framework/src/redis/`; `slug_in`, `words`, the invisible characters,
   Esperanto and Italian, `plural_studly`, `plural_pascal`,
   `prepend_count`, `percentage_in`, `max_precision`, `use_locale`,
   `with_locale` and the infinite and NaN output in
   `framework/src/support/`; `--without-migration-data` and `--database`
   on `schema:dump`; `prepare_for_validation`, hooks alongside rule
   errors and the redirect back for plain forms in
   `framework/src/http/form_request.rs` and the multipart extractor; the
   `type/*` wildcard; Laravel's column defaults with the opt-out setting
   in the schema builder, the model macro and the scaffold; the manual
   chapters each area owns and the changelog.
   Mechanisms: the seventeen the blocks already name (`par-joins`,
   `par-file-responses`, `par-authorize`, `par-model-changes`,
   `par-sync-without-detaching`, `par-query-helpers`, `par-queue-fake`,
   `par-debug-error-page`, `par-default-disk`, `par-sqs-queue`,
   `par-process`, `par-log-channels`, `par-redis`, `par-strings`,
   `par-schema-dump`, `par-multipart-validation`, `par-laravel-defaults`),
   each rebound on a fail receipt for the revised text.
   Done when they pass on a committed tree, the manual passages are in
   place, and the review is recorded.

## laravel-delta

Requirements: PAR-089, PAR-090, PAR-091, PAR-092, PAR-093, PAR-094, PAR-095, PAR-096, PAR-097, PAR-098

43. laravel-delta - the eighth of the parity rounds the developer ordered
   on 2026-10-07 (17:35), ruled build by the parity default: the
   thirty-seven delta rows of the parity log, the members Laravel 13.35.0
   added or changed that Suprnova lacks, grouped by area into PAR-089 to
   PAR-098.
   Delivery: the schedule-wide one-server switch, `schedule:interrupt` and
   the interrupt check in `framework/src/schedule/`; the raw chunk cursor
   and the task-scoped default connection in `framework/src/database/` and
   `framework/src/eloquent/`; model defaults in the `#[model]` macro;
   `chunk_map`, `first_or_create`, `increment_or_create`, `find_or_new` and
   `is` on the relations; the worker controls on `queue:work`, the queue
   fake's negative and filtered assertions, `dispatch_after_response` and
   the bus fake's accessors in `framework/src/queue/`, `framework/src/bus/`
   and the app's command set; the markdown response; the `QUERY` method and
   `query!` in `framework/src/routing/` and `framework/src/http/`;
   `logout_other_devices` in `framework/src/auth/`; the typed notification
   fake in `framework/src/notifications/`; the disk-handle copy and move,
   the list `forget`, the named `purge` and `set` in
   `framework/src/filesystem/`; a `laravel_delta` test module in each of
   the schedule, database, eloquent, queue, bus, http, routing, auth,
   notifications and filesystem suites; the manual chapters each area owns
   and the changelog.
   Mechanisms: `par-laravel-delta` (the `laravel_delta` modules of the ten
   suites).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.

## laravel-gaps-data

Requirements: PAR-099, PAR-100, PAR-101, PAR-102, PAR-103, PAR-104, PAR-105, PAR-106, PAR-107, PAR-108, PAR-109, PAR-110

44. laravel-gaps-data - the ninth of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build by the parity default: the
   forty-five rows of the log's database, Eloquent, migration, pagination,
   factory and seeding areas where Suprnova lacks or differs from Laravel
   13.35.0, grouped into PAR-099 to PAR-110; the four rows that have an
   equivalent under another name stay as they are.
   Delivery: transactions, raw expressions and `first_or_fail` in
   `framework/src/database/`; scopes, morph existence, force destroy,
   collections, casts, appends, hidden lists and serialization in
   `framework/src/eloquent/` and the `#[model]` macro; factories in
   `framework/src/factory/`; pivot deletion, per-parent limits and morph
   writes in `framework/src/eloquent/relations/`; resources in
   `framework/src/resources/`; the column types in `framework/src/schema/`;
   cursors and pages in `framework/src/pagination/`; seeders and `db:seed`
   in `framework/src/seed/` and the console; a `laravel_gaps` test module
   in each of the database, eloquent, factory, pagination, resources,
   schema and seed suites; the manual chapters each area owns and the
   changelog.
   Mechanisms: `par-laravel-gaps-data` (the `laravel_gaps` modules of the
   seven suites).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.

## laravel-gaps-http

Requirements: PAR-111, PAR-112, PAR-113, PAR-114, PAR-115, PAR-116, PAR-117, PAR-118, PAR-119, PAR-120, PAR-121, PAR-122

45. laravel-gaps-http - the tenth of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build by the parity default: the
   thirty-two rows of the log's routing, controller, request, response,
   URL, middleware, CSRF, validation, error, view and session areas where
   Suprnova lacks or differs from Laravel 13.35.0, grouped into PAR-111 to
   PAR-120, with PAR-121, the polymorphic owner relation under the
   existence filters, and PAR-122, the seeded SQLite order on a
   text-keyed model, both surfaced from the ninth round's reviews; `TrustProxies::at`, which the log keeps, and `View::with`,
   which chained `with` and `prop` calls already cover, stay as they are.
   Delivery: reportable callbacks, `Exceptions::report`, the stopped
   retries, the error's type name and the editor links in
   `framework/src/error/`, `framework/src/queue/` and the console; the
   CSRF methods, the JSON `_token` and the token a regenerated session
   gets in `framework/src/csrf/` and `framework/src/session/`; controller
   middleware, `without_middleware`, the `405` and `OPTIONS` answers and
   `route_has` in `framework/src/routing/`, `framework/src/middleware/`,
   the server and the `resource!` macro; the signature middleware and the
   URL helpers in `framework/src/routing/`; the guest redirect, the request
   host, `TrustHosts`, `ajax` and cookie deletion in
   `framework/src/http/`; the IP and numeric messages, `RequiredIf::when`,
   `ImageFile` and `Dimensions` in `framework/src/validation/`, the English
   catalog and the upload validators; `make:middleware`, `make:view` and
   `make:inertia` in `suprnova-cli`; the `MorphTo` dispatch of `has`,
   `doesnt_have`, `has_count` and `where_has` in
   `framework/src/eloquent/builder.rs` and the rowid carried through a
   union's projection for the seeded SQLite order, with their tests in
   the eloquent suite's `laravel_gaps` module; a `laravel_http_gaps` test module in
   each of the error, queue, session, routing, http and validation suites
   and in the CLI's `make_scaffolds` test; the manual chapters each area
   owns and the changelog.
   Mechanisms: `par-laravel-gaps-http` (the `laravel_http_gaps` modules of
   the six suites and the CLI test) and `par-laravel-gaps-data` (the
   `laravel_gaps` modules of the data suites, for PAR-121 and PAR-122).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.

## laravel-gaps-auth

Requirements: PAR-123, PAR-124, PAR-125, PAR-126, PAR-127, PAR-128, PAR-129, PAR-130, PAR-131, PAR-132

46. laravel-gaps-auth - the eleventh of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build by the parity default: the
   build rows of the log's authorization, verification, password reset,
   rate-limiting, encryption, hashing and request-guard areas where
   Suprnova lacks or differs from Laravel 13.35.0, grouped into PAR-123 to
   PAR-130 and PAR-132, with PAR-131, the relation existence probe on a
   self-referential relation, surfaced on 2026-10-09; the Laravel-format
   encrypted payload (two rows) and the request guard's `validate`
   through its callback (one clause) stay with the developer, and the two
   rows that Suprnova already answers the way they ask stay as they are.
   Delivery: the gate methods, the `GateEvaluated` event, the `Response`
   code and the after-callback arguments in `framework/src/authorization/`
   and `framework/src/error.rs`; the verified-email middleware, the
   verification and reset notifications, the verification answers and
   the reset timebox in `framework/src/auth_flows/` and
   `framework/src/auth/`; the rehash moved from `validate_credentials` to
   the sign-in step in `framework/src/auth/`; the rate-limiter key
   cleaning, the wrapped `limiter`, `hit_for_minute`, `hit_until`, the
   throttle headers, the refused-request count, the inline user key and
   the `guest|user` limit in `framework/src/rate_limit/`; the encrypter
   and the key generator in `framework/src/crypto/`; the Argon time
   setting and named hasher drivers in `framework/src/hashing/`; the
   request guard resolver in `framework/src/auth/`; a `laravel_auth_gaps`
   test module in each of the auth, auth_flows, authorization, crypto,
   hashing and rate_limit suites; the aliased existence probe in
   `framework/src/eloquent/builder.rs`, with its tests in the eloquent
   suite's `laravel_gaps` module; the manual chapters each area owns and
   the changelog.
   Mechanisms: `par-laravel-gaps-auth` (the `laravel_auth_gaps` modules of
   the six suites, for PAR-123 to PAR-130 and PAR-132) and
   `par-laravel-gaps-data` (the `laravel_gaps` modules of the data suites,
   for PAR-131).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.
## laravel-gaps-infra

Requirements: PAR-133, PAR-134, PAR-135, PAR-136, PAR-137, PAR-138, PAR-139, PAR-140, PAR-141, PAR-142, PAR-143, PAR-144, PAR-145, PAR-146, PAR-147, PAR-148, PAR-149, PAR-150, PAR-151, PAR-152, PAR-153, PAR-154, PAR-155, PAR-156, PAR-157, PAR-158, PAR-159, PAR-160, PAR-161, PAR-162, PAR-163, PAR-164, PAR-165, PAR-166, PAR-167, PAR-168, PAR-169, PAR-170, PAR-171, PAR-172, PAR-173

47. laravel-gaps-infra - the twelfth of the parity rounds the developer
   ordered on 2026-10-07 (17:35), ruled build by the parity default: the
   build rows of the log's cache, configuration, logging, console,
   scheduling, migration, mail, notification, event, queue,
   broadcasting, filesystem, process, image, HTTP client, Vite, document
   head, Precognition, cookie, view, string, collection, date,
   middleware, pipeline, translation, container and coding-assistant
   areas where Suprnova lacks or differs from Laravel 13.35.0, grouped
   into PAR-133 to PAR-173; the personal access tokens of `docs:sanctum`
   and the exact Precognition header match stay with the developer, as
   each would weaken a property Suprnova keeps, the support policy of
   `docs:releases` waits for the window the developer names, and the
   five rows Suprnova already answers the way they ask stay as they are.
   Delivery: the Redis cache connection, the cache prefix, the cache
   events and the computed lifetime in `framework/src/cache/` and
   `framework/src/redis_facade/`; the MySQL character set and collation
   in `framework/src/database/` and `framework/src/schema/`; the
   production default for an unset `APP_ENV` and the merged
   configuration in `framework/src/config/` and the CLI's
   `migrate:fresh`, with an explicit environment for the framework's
   and the dogfood application's tests that relied on the old default; the log level methods, the carried and shared
   context, `MessageLogged`, the channel levels and the placeholder
   switch in `framework/src/logging/`; the styled console lines, the
   verbosity flags and the prompts in `framework/src/console/`; `down`'s
   event, messages, retry date and rendered page and the registered
   migrations in `framework/src/app/` and `framework/src/database/`; the
   one-server refusal, the day and time fields and the environment and
   maintenance filters in `framework/src/schedule/`; the attachment
   comparison, the mailable's recipients, the one-call sends, the sent
   check and the mail delay in `framework/src/mail/`; the broadcast
   notification event and job, the attachments by path, the subject
   fallback and the notification delay in `framework/src/notifications/`;
   the unique and delayed queued listeners in `framework/src/events/`;
   the per-connection after-commit setting in `framework/src/queue/`; the
   Pusher client in `framework/src/broadcasting/`; the bare directory
   path and the upload-URL callback in `framework/src/filesystem/`; the
   fake's argument lists and gradual output in `framework/src/process/`;
   the clamped and one-side resizes and the transformation settings in
   `framework/src/media/`; `HEAD`, the global middleware, the request
   builder additions and the fake's patterns, callbacks, sequences and
   stray allowlist in `framework/src/http_client/`; the Vite tags and
   the hot file in `framework/src/inertia/`; the `Head` facade in
   `framework/src/view/`, `framework/src/inertia/` and the route
   builders of `framework/src/routing/`; the Precognition hook and the rule selection in
   `framework/src/http/` and `framework/src/validation/`; the CSRF field
   in `framework/src/csrf/`; raw cookies in `framework/src/http/`; view
   sharing in `framework/src/view/` and the Live view renderer;
   `Stringable` and inline Markdown in `framework/src/strings/` and
   `framework/src/content/`; the ordered collection helpers in
   `framework/src/eloquent/`; `Date` in a new `framework/src/date.rs`
   that reads `framework/src/clock.rs`; the middleware priority defaults and the
   value pipeline in `framework/src/middleware/`; the translation
   sources in `framework/src/localization/`; the environment bindings,
   the renamed route parameter and lazy singletons in
   `framework/src/container/` and `suprnova-macros`; a
   `laravel_infra_gaps` test module in each of the broadcasting, cache,
   config, console, container, database, eloquent, events, filesystem,
   http, http_client, inertia, live, localization, logging, mail, media,
   middleware, notifications, process, queue, routing, schedule, schema,
   session, strings and testing suites and in the CLI's
   `migrate_fresh_gate` test; the manual chapter on AI-assisted
   development, the manual chapters each area owns and the changelog.
   Mechanisms: `par-laravel-gaps-infra` (the `laravel_infra_gaps`
   modules of the twenty-seven suites, the cache suite's Redis tests
   against the local Redis, and the CLI test).
   Done when it passes on a committed tree, the manual passages are in
   place, and the review is recorded.
## laravel-gaps-testing

Requirements: PAR-174, PAR-175, PAR-176, PAR-177, PAR-178, PAR-179, PAR-180, PAR-181

48. laravel-gaps-testing - the thirteenth of the parity rounds the
   developer ordered on 2026-10-07 (17:35), ruled build by the parity
   default: the build rows of the log's testing, HTTP test, database
   test, mocking and console test areas where Suprnova lacks or differs
   from Laravel 13.35.0, with the full-text search row the developer
   added on 2026-10-09 ("Yes add that"), grouped into PAR-174 to
   PAR-181; the browser-test row of `docs:dusk` stays kept by his word
   of 2026-10-07 17:17. Delivery: the test client's default headers,
   `QUERY` helper and signed-in user in `framework/src/testing/`; the
   configured-connection, transaction-per-test, lazy and seeded test
   databases in `framework/src/database/testing.rs` and the
   `#[suprnova_test]` options in `suprnova-macros/`; the console test
   expectations, confirmations and exit codes in `framework/src/console/`;
   the typed queued reads of the mail fake in `framework/src/mail/`; the
   queue fake's decode failures and reserved jobs in
   `framework/src/queue/`; `assert_no_content` in
   `framework/src/testing/response.rs`; `where_full_text`,
   `or_where_full_text`, `Blueprint::full_text` and `drop_full_text` in
   `framework/src/database/`, `framework/src/eloquent/` and
   `framework/src/schema/`, with engine tests named for the gate's
   Postgres and MySQL passes; a `laravel_testing_gaps` test module in
   each of the testing, auth, console, database, eloquent, schema, mail
   and queue suites; the manual chapters each area owns (testing, HTTP
   tests, database testing, mocking, console, database, migrations) and
   the changelog. Mechanisms: `par-laravel-gaps-testing` (the
   `laravel_testing_gaps` modules of the eight suites). Done when it
   passes on a committed tree, the manual passages are in place, and the
   review is recorded.

## laravel-gaps-mongodb

Requirements: PAR-182, PAR-183, PAR-184, PAR-185, PAR-186, PAR-187, PAR-188

49. laravel-gaps-mongodb - the fifteenth of the parity rounds the
   developer ordered on 2026-10-07 (17:35), the row he ruled build
   himself at 17:12 ("You will add MongoDB"): a MongoDB backend on the
   official `mongodb` crate behind the `database-mongodb` feature, off
   by default, grouped into PAR-182 to PAR-188 in the scope the rulings
   log names (models, queries, relations to and from SQL models, and the
   queue, cache, session and batch stores); vector search, GridFS and
   the Scout engine stay out (the search layer is the next version's).
   Delivery: the connection, configuration and facade in
   `framework/src/mongodb/`; the `#[suprnova::document]` macro in
   `suprnova-macros/`; the document query builder and the relations,
   document to document and across stores, in `framework/src/mongodb/`
   and `framework/src/eloquent/`; the queue driver, batch repository and
   failed-job store in `framework/src/queue/`; the cache store in
   `framework/src/cache/`; the session store in `framework/src/session/`;
   a `mongodb` test binary (`framework/tests/mongodb/`, built with the
   feature) whose unignored tests assert the rendered BSON, the
   configuration and the macro and whose `mongodb_` tests run against
   the gate's standing MongoDB container; the manual chapter
   `manual/mongodb.md` linked from `manual/documentation.md`, the
   chapters each store owns, and the changelog. Mechanisms:
   `par-laravel-gaps-mongodb` (the `mongodb` test binary under the
   feature). Done when it passes on a committed tree, the manual
   passages are in place, and the review is recorded.

## laravel-gaps-concurrency

Requirements: PAR-189, PAR-190

50. laravel-gaps-concurrency - the sixteenth of the parity rounds the
   developer ordered on 2026-10-07 (17:35), the `docs:concurrency` row he
   confirmed at 17:17: a `Concurrency` facade with `run`, `run_named`,
   `with_timeout`, `driver` and `defer` over a `ConcurrencyDriver` trait
   with the `tokio` (default), `process` and `sync` drivers, tasks being
   registered `#[suprnova::concurrent_task]` functions with serde input
   and output, grouped into PAR-189 and PAR-190. Delivery: the facade,
   trait, drivers and the hidden `concurrency:run` console command in
   `framework/src/concurrency/`; the task macro in `suprnova-macros/`; a
   `concurrency` test binary under `framework/tests/concurrency/`; the
   manual chapter `manual/concurrency.md` linked from
   `manual/documentation.md`; the changelog. Mechanisms:
   `par-laravel-gaps-concurrency` (the `concurrency` test binary). Done
   when it passes on a committed tree, the manual passages are in place,
   and the review is recorded.

## backlog-defects-round

Requirements: MAN-002, PAR-017, PAR-048, PAR-049, PAR-052, PAR-055, PAR-056, PAR-068, PAR-080, PAR-089, PAR-111, PAR-121, PAR-123, PAR-183, PAR-188, REG-032

51. backlog-defects-round - the eighteen backlog items captured during the
   parity rounds, taken up together before the upgrade guide and the
   release by the developer's words of 2026-10-09 20:58 ("ok you know how
   to fix them"): each item names the Agreed requirement it violates or
   falls short of, and each is fixed with its regression test under that
   requirement's existing mechanism. The items:
   - live-testing-types-not-nameable-from-the-crate-root (from MAN-002, item 50c8a6d88b7d): suprnova::live::testing::request_cancellation_for_test returns Option<suprnova_live::resource::CancellationFlag>, and AdjustableTestClock implements suprnova_live::clock::Clock with suprnova_live::clock::ClockError; neit
   - script-scan-written-globals-depend-on-walk-order (from REG-032, item 6dc6ff1cfbcd): The script scan's first walk collects written_globals, the names a computed write to the global object (window[k] = v) traces to, while it is still walking: a name the walk has not reached yet resolves where the walk sta
   - multipart-request-derive-matches-names-literally (from PAR-055, item 53356ae40499): #[derive(MultipartRequest)] (the streaming upload request, suprnova-macros) matches part names literally: Inertia 3.8 sends photos[0], photos[1] (forceIndicesArrayFormatInFormData defaults to true) and user[name], which
   - request-full-url-does-not-normalise-its-query (from PAR-056, item 257d7489ac50): Request::full_url() returns the query as it arrived, while Laravel's fullUrl() (Symfony Request::getQueryString, normalizeQueryString) sorts the pairs by key and re-encodes them per RFC 3986; PAR-056 normalised the Inert
   - paginator-builders-miss-the-cursor-and-the-page-name (from PAR-052, item 8c1588571984): Builder::cursor_paginate does not record the cursor the page was fetched with (current_cursor), so a cursor page's current page in scroll metadata falls back to the request's cursor parameter, and Builder::simple_paginat
   - inertia-rows-have-no-feature-map-surface (from PAR-049, item 3ba43cfbf890): The roadmap's inertia-protocol delivery names feature-map/laravel/parity.jsonl rows of the group linked to the shipped items, but the Laravel surface (feature-map/laravel/surface.jsonl) holds laravel/framework alone: it
   - inertia-fallback-visits-record-no-previous-url (from PAR-048, item 12301b4d3f1d): A fallback route's request carries no route pattern (server.rs builds it with Request::new(req).with_params(..) and no with_route_pattern), so an Inertia GET answered by the fallback handler records no previous URL, wher
   - cursor-paginate-ignores-the-query-order (from PAR-080, item bf363cd70ba1): Builder::cursor_paginate (framework/src/eloquent/builder.rs, from the cursor branch near line 5555) drops any order_by on the query and always walks the primary key ascending, so a list that asks for newest first (the sc
   - generate-types-pairs-no-page-built-with-the-builder (from PAR-068, item c05ddf6552e4): suprnova generate-types pairs a page component with its props struct only through inertia_response!, Inertia::data and InertiaResponse::new(..).with_data(struct); a page whose handler assembles its props with the builder
   - laravel-database-notes-test-flakes-with-no-such-table (from PAR-080, item 79eaa210ec84): framework/tests/laravel_database/columns.rs kit_scaffold_notes_belong_to_laravels_users_sqlite (kit backend lane of inertia-starter-kits, commit daea7e145) failed three runs in a row under nextest at 16:58 to 17:05 on 20
   - untagged-cli-build-pins-the-last-tag (from PAR-080, item 33499ff96fc6): templates::framework_tag (suprnova-cli/src/templates/mod.rs:28) pins a new project's framework dependency at the tag of the CLI's own version, so a CLI built from main between two releases (workspace 3.2.1 while the temp
   - where-relation-on-a-morph-to-matches-nothing (from PAR-121, item 310cae7a17b2): where_relation(relation, column, op, value) on a MorphTo relation still builds the generic probe with no target table and matches nothing; has, doesnt_have, has_count and where_has route a MorphTo through the morph engin
   - console-bootstrap-failure-is-not-reported (from PAR-111, item e5f5cdbb1655): A console command's bootstrap failure (Config::init, the container boot, the database connection before the handler runs) exits without reaching Exceptions::report; a command's own Err, every framework-built 5xx, timed-o
   - devtools-gate-event-names-the-erased-user (from PAR-123, item c75c64459e22): The Inertia DevTools gate (framework/src/inertia/devtools/endpoints.rs around line 93) still names the event's user through the registry, so a user type that registered no gate or hook appears as the erased trait object
   - queue-set-driver-replaced-by-bootstrap-from-env (from PAR-017, item a6167c90f674): manual/queues.md's failover example calls Queue::set_driver(..) in the application's bootstrap, but Server::run calls queue::bootstrap_from_env afterwards and that replaces the installed driver unconditionally, so the do
   - database-session-expired-read-deletes-a-refreshed-row (from PAR-188, item 9153cc0aaa91): The SQL session driver (framework/src/session/driver/database.rs, the read path near line 426) decides a session expired from the row it fetched and then calls destroy(id), whose delete filters only by id. A concurrent w
   - model-explicit-timestamps-true-without-columns-is-silent (from PAR-183, item bf5320b0e838): suprnova-macros/src/model/parse.rs lines 575 to 582: #[model(timestamps = true)] on a struct with neither created_at nor updated_at is accepted and timestamps are silently off, the same as the default. An explicit timest
   - schedule-interrupt-boots-without-a-cache-store (from PAR-089, item a34d5f8d1163): schedule:interrupt boots through ProcessBoot::Core (framework/src/app/mod.rs, the Commands::ScheduleInterrupt arm), which binds no cache store, yet Schedule::interrupt (framework/src/schedule/mod.rs line 256) writes its 
   Delivery: one fix per item in its module, each with the test that
   failed before it, merged under a lease on the item's requirement; the
   manual passages the fixes touch and the changelog. Mechanisms: no new
   mechanism; each regression test lives where its requirement's
   mechanism already selects: the Inertia items (PAR-048, PAR-049,
   PAR-052, PAR-055, PAR-056) in the `inertia` binary or as `inp_` tests
   of the `http` binary for `par-inertia-protocol`; PAR-068 as `intt_`
   tests of the CLI's `generate_types_data` for `par-inertia-testing`;
   the untagged pin (PAR-080) as a `kit_` test of `kit_pages` for
   `par-starter-kits`, the cursor order (PAR-080) beside PAR-121 in the
   `eloquent` binary's `laravel_gaps` module for `par-laravel-gaps-data`,
   and the notes flake (PAR-080) made deterministic in `laravel_database`,
   where the workspace run observes it; PAR-089 and PAR-017 in the
   `laravel_delta` modules of `schedule` and `queue` for
   `par-laravel-delta`; PAR-111 and PAR-188 in the `laravel_http_gaps`
   modules of `error` and `session` for `par-laravel-gaps-http`; PAR-183
   as a trybuild case in the `eloquent` binary's `laravel_gaps` module for
   `par-laravel-gaps-data`; PAR-123 in the `authorization` binary's
   `laravel_auth_gaps` module for `par-laravel-gaps-auth`; REG-032 in the
   CLI's `live_registry_scan` for `registries-scan`; and the Live
   re-exports (MAN-002) in the `live` binary's `laravel_infra_gaps` module
   for `par-laravel-gaps-infra`, beside the public API test. Done when
   every requirement above has a current pass on a committed tree and the
   review is recorded.

## release-4-0-0

Requirements: MAN-002, MAN-108

52. release-4-0-0 - the 4.0.0 release, after the backlog defects round,
   under the developer's words of 2026-10-09 13:06 and 13:07 ("next
   release is 4.0.0", "an upgrade guide (humans and agents) for 4.0.0 is
   the LAST task before the release") and of 2026-10-10 16:09 ("ok" to
   rewording PAR-042 and PAR-083 to the typed request, applied at this
   spec phase). Delivery: the upgrade chapter `manual/upgrade-4.0.md`,
   linked from the documentation index, with a section for every
   `### Changed` and `### Removed` entry of the 4.0.0 changelog section and
   for the `### Fixed` entries the release notes name as behaviour
   changes, each with what an application observed before, what it
   observes now and the edit to make, checked by
   `feature-map/tools/upgrade_guide_check.py` under the `upgrade-guide`
   mechanism (MAN-108), which the release gate runs; the backlog item
   magnetar-engine-types-not-nameable-from-the-crate-root (item
   01a98725846b, from MAN-002): the Magnetar types public signatures name
   re-exported through `suprnova` with the live binary's public-signature
   walk covering them; the feature map filed completely (the ten records
   with no chapter rule, `app ssr:check`, `app ssr:start`, `app ssr:stop`,
   `console concurrency:run`, `suprnova::AsBsonDateTime`,
   `suprnova::AsDecimal128`, `suprnova::BelongsToDocument`,
   `suprnova::BelongsToManyDocuments`, `suprnova::BelongsToModel` and
   `suprnova::Concurrency`, get their rules in
   `feature-map/tools/build_surface.py`) and regenerated at the final
   source commit with the component allowlist, so the manual check runs at
   the gate (MAN-002); the release build `cargo build --release --locked
   -p app --bins`. Mechanisms: manual-check (MAN-002), upgrade-guide
   (MAN-108, declared in this phase with a violating example), and the
   live binary under par-laravel-gaps-infra through PAR-133 for the
   Magnetar walk. Done when both requirements have a current pass on a
   committed tree and the review is recorded; the tag v4.0.0 then follows
   through `scripts/release.sh`, which runs the whole gate, on a pushed
   main.

## cli-generators-and-live-kit

Requirements: PAR-191, PAR-192

53. cli-generators-and-live-kit - the twenty-one artisan generators the
   parity map marks not built and a Live starter kit beside the Svelte,
   React and Vue kits, in 4.0.0 before the tag, under the developer's
   words of 2026-10-11 18:00 ("Yes add them") and 18:01 ("You also need
   to add a Live scaffold just like the react, vue and svelte").
   Delivery: the twenty-one `make:*` commands in `suprnova-cli/src/commands/`
   with their templates, each run against a fresh scaffold by
   `suprnova-cli/tests/make_generators.rs`, and a manual chapter for the
   generators linked from the documentation index and the CLI chapter; the
   `live` frontend in `suprnova-cli/src/templates/` (the kit's documents,
   components, views and layouts) behind `suprnova new --frontend live`,
   checked by `suprnova-cli/tests/kit_live.rs` and the scaffold compile
   check, documented in `manual/cli-new.md` and `manual/starter-kits.md`;
   changelog entries under 4.0.0 `### Added`. Mechanisms:
   par-cli-generators (PAR-191) and par-live-kit (PAR-192), both declared
   in this phase with a violating example. Done when both requirements
   have a current pass on a committed tree and the review is recorded; the
   tag v4.0.0 then follows through `scripts/release.sh` on a pushed main.

## daybreak-defects-round

Requirements: HRD-001, HRD-002, HRD-003, HRD-004, HRD-005, HRD-006, PAR-080

54. daybreak-defects-round - the six findings of the Daybreak defensive
   audit of 2026-10-11 and one defect the Live kit lane found, fixed
   before the 4.0.0 tag by the developer's word of 2026-10-11 19:38
   ("confirmed"). The items:
   - slow-request-bodies-occupy-generated-applications (HRD-001, item cc0b71be)
   - magick-driver-autodetects-unrecognised-uploads (HRD-002, item ed74e9fb)
   - web-push-strict-endpoint-is-dns-blind (HRD-003, item 8e4a3937)
   - dogfood-reset-reports-success-after-revocation-failure (HRD-004, item c8dd575c)
   - asset-serving-canonicalize-then-open-race (HRD-005, item 84080b9c)
   - websocket-same-origin-ignores-the-scheme (HRD-006, item fda6f83a)
   - full-stack-dockerfile-omits-lang (PAR-080, item 3e112824): the
     Dockerfile `docker:init` writes for an Inertia kit never copies
     `lang/` into the runtime image.
   Delivery: one fix per item in its module with the test that failed
   before it, merged under a lease on the item's requirement; the manual
   passages the fixes touch; changelog entries under 4.0.0 (`### Fixed`,
   each with its upgrade-chapter section or listing). Mechanisms: the
   `hardening` mechanism, declared in this phase with a violating example
   (no `hardening_` test exists on the tree before the fixes), runs the
   `hardening_` tests of the framework's http, server, media,
   static_files, live and ws binaries, the Web Push crate, the CLI's
   kit_pages and the dogfood application; the Dockerfile fix lands under
   `par-starter-kits`. Done when every requirement has a current pass on
   a committed tree and the review is recorded; the tag v4.0.0 then
   follows through `scripts/release.sh` on a pushed main.

## daybreak-remedies-complete

Requirements: HRD-007, HRD-008, HRD-009, HRD-010, HRD-011

55. daybreak-remedies-complete - the remedies of the Daybreak audit of
   2026-10-11 that the defect round did not build, ruled in by the
   developer on 2026-10-12 ("I want all defects identified by Daybreak to
   be remediaited for 4.0.0"), before the 4.0.0 tag:
   - the authentication epoch on every provider-backed session and
     remember credential, checked on every request (HRD-007, DBK-04)
   - the durable revocation retry a failed reset queues (HRD-008, DBK-04)
   - a finite connection cap by default (HRD-009, DBK-01)
   - a framework-owned ImageMagick policy under the Magick driver
     (HRD-010, DBK-02)
   - a Web Push endpoint allowlist (HRD-011, DBK-03)
   Delivery: one lane per requirement in its module with the test that
   failed before it, merged under a lease on the requirement; the manual
   passages they touch (authentication, session, env-vars, images,
   web-push); changelog entries under 4.0.0 with their upgrade-chapter
   sections. Mechanism: `hardening`, re-declared to run the auth,
   auth_flows and notifications binaries' `hardening_` tests as well, with
   a violating example bound before the start. Done when every requirement
   has a current pass on a committed tree and the review is recorded; the
   tag v4.0.0 then follows through `scripts/release.sh --resume` on a
   pushed main, whose gate repeats only the steps these changes touch.

## search-and-starter-recommender

Requirements: SEARCH-001, SEARCH-002, SEARCH-003, SEARCH-004, SEARCH-005, SEARCH-006, SEARCH-007, SEARCH-008, SEARCH-009, SEARCH-010, SEARCH-011, SEARCH-012, SEARCH-013, SEARCH-014, REC-001, REC-002, REC-003, REC-004, REC-008

56. search-and-starter-recommender - Suprnova's own search, the feature the
   developer chose for this version over WebMCP ("search first",
   2026-10-14), designed with him on 2026-10-14 (`search.md`), reviewed
   adversarially by gpt-6.1-sol the same morning and redrafted twice, the
   second time on his ruling that each engine's native full-text ranking
   does the ranking; and the starter recommender borrowed from his
   nation-x_com application (`recommendations.md`, REC-001 to REC-004 and
   REC-008), shipped now because "the idea is to build data" for the graph
   recommender, which is the commitment after this one (REC-005 to
   REC-007). In the order the work depends:
   - the document, the derive and the two settings classes (SEARCH-001,
     SEARCH-002), then the `SearchEngine` trait with the `collection`,
     `null` and `database` engines on SQLite, MariaDB and Postgres and the
     `qdrant` engine (SEARCH-003), the builder with its filter grammar,
     facets, grouping, geo and federated search (SEARCH-004, SEARCH-009)
   - indexing with versions, tombstones, after-commit on every transaction
     form, generations and the commands (SEARCH-005)
   - the `Embedder` trait with the `http` provider, qualified against the
     developer's OpenRouter test target by an ignored hand-run test, and
     the `fastembed` provider behind `search-fastembed` with the
     EmbeddingGemma 2 default profile and its provenance record
     (SEARCH-006); expansion, hybrid fusion and rerank (SEARCH-007),
     highlights (SEARCH-008), scopes (SEARCH-010)
   - the Live `search-box`, `Search::routes()` and the kits' notes search
     (SEARCH-011); analytics, suggestions and the prune command
     (SEARCH-012); the relevance and latency baseline (SEARCH-013); the
     manual chapter and env-vars rows (SEARCH-014)
   - the event stream, the starter engine with its bounds, run records and
     projected explanations, the versioned data contract and the manual
     chapter (REC-001 to REC-004, REC-008)
   Delivery: the `search` and `recommendations` modules under
   `framework/src/`, crate-root re-exports, the derive in
   `suprnova-macros`, integration test binaries `search` and
   `recommendations` under `framework/tests/`, engine-named ignored tests
   (`_postgres`, `_mariadb`, `_qdrant`) the gate's engine passes select by
   name, the BEIR fixture corpus with its judgments under
   `framework/tests/search/fixtures/`, the `search:baseline` calibration
   command run by hand against Meilisearch and Typesense in Docker on the
   developer's dedicated server (Agent 1), whose nDCG@10 and versions it
   commits as the per-engine floors with the tolerance and p95 budgets, the gate's
   `suprnova-gate-qdrant` container (`qdrant/qdrant:v1.19.2`) created once
   by `scripts/setup-gate-databases.sh`, a `pgvector` driver for the
   `VectorDriver` trait (ruled in by the developer on 2026-10-14 06:41, "yes
   add": a Postgres application gets hybrid search with nothing extra, as a
   MariaDB one does) (whose Mongo block moves above its
   exit) with a `scripts/check-qdrant.sh` engine pass, the migrations for
   the analytics and recommendation tables, the kit template changes
   mirrored in `scaffold_snapshot`, `manual/search.md`,
   `manual/recommendations.md`, `manual/env-vars.md` rows, changelog entries
   under the open version. Mechanisms: `search` (the `search` binary, then
   its `_qdrant` tests against the gate container) and `recommendations`
   (the `recommendations` binary), each bound to a violating example before
   the start. The `search-fastembed` feature is off by default and the kits
   search lexically until `SEARCH_EMBEDDER` is set, as presented and
   confirmed on 2026-10-14. Done when every requirement has a current pass on
   a committed tree, the engine passes are green on the host MariaDB and
   Postgres and the Qdrant container, the hand-run http embedder test has
   passed once against the OpenRouter target, and the review is recorded.

Out of every commitment above, by the developer's 2026-09-12 ruling:
complete blocks (auth flows, account settings, CTAs, pricing, dashboard
layouts), tag input, nested menus, command palette, drag-reorder,
auto-infinite-scroll, carousel, image cropper, rich text editor, rating
input. They are a separate project and enter this roadmap only if the
developer writes them in.

Every commitment inherits the Live gate and the repository gate; a
commitment is Done only when its mechanisms pass on a committed tree and
its review is recorded (`sudus review`).

## password-change-keeps-the-changing-session

Requirements: HRD-007

Delivers backlog item password-change-keeps-the-changing-session: A signed-in user who changes their own password is signed out on their next request: the session that made the change carries the old auth epoch, so the epoch check ends it with every other session. Laravel's AuthenticateSession stores the new password hash in the current session at the end of the request, so the session that changed the password stays signed in. suprnova.app hit it on 4.0.0 (its profile password form) and now calls Auth::login after saving the password. The framework refreshes the recorded epoch of the current session's guard when the request that resolved the user changed that user's epoch, and a test shows the changing session stays signed in while another session of the same user is ended.

## scaffold-files-not-rustfmt-clean

Requirements: PAR-080

Delivers backlog item scaffold-files-not-rustfmt-clean: Found building the Live starter kit (lane k, 2026-10-11): in a fresh 2024-edition scaffold, cargo fmt rewrites the shared templates example_action.rs, logging.rs, two migrations and models/user.rs, so a developer's first cargo fmt produces a diff in files they did not touch. Fix: format those templates to rustfmt's 2024 layout and keep a test that runs cargo fmt --check on a scaffold.

## magick-driver-keeps-the-accounts-user-policy

Requirements: HRD-010

Delivers backlog item magick-driver-keeps-the-accounts-user-policy: The magick driver runs every child with HOME set to its policy directory, so ImageMagick no longer reads the account's own $XDG_CONFIG_HOME/ImageMagick or $HOME/.config/ImageMagick. A policy or config the operator kept there stopped applying in 4.0.0. The driver appends the real user dir of the process ($XDG_CONFIG_HOME/ImageMagick, else $HOME/.config/ImageMagick) to MAGICK_CONFIGURE_PATH after the operator's value, so the account's policy and config apply as before while the framework file under the child's HOME stays the last Path. Real-binary tests: a user-dir denial still applies, and a user-dir grant is still outranked by the framework file.

## framework-frames-under-a-live-action

Requirements: PAR-192

Delivers backlog item framework-frames-under-a-live-action: The deeper issue behind defect 122c1056 (Shawn 2026-10-11 19:55: make sure the bigger issue is notated): in a debug build the framework's own frames below a Live action use about 1.75 to 1.8 MiB of stack before the action runs (measured by lane k under gdb: about 930 KB in the hyper server, after-response and task-local frames, about 360 KB in the Live endpoint and executor, most of the rest in the session and RenderCache middleware; registration overflows at RUST_MIN_STACK=1.75 MiB inside its first email-regex compile). The 8 MiB runtime thread stack the main macro now sets gives actions room; it does not make the frames smaller. Fix: box the large futures and shrink the frames in the server, the middleware chain and the Live engine so a debug build runs a Live action within Tokio's 2 MiB default, with a measurement in the test suite that fails when the frames below an action grow past a budget.

## security-headers

Requirements: SEC-001, SEC-002, SEC-003, SEC-004, SEC-005, SEC-006, RDOC-005

57. security-headers - the last item of issue #143, which the developer put
   ahead of the search work on 2026-10-15 ("143 should have already been
   done before search"; the search commitment was superseded the same
   morning). In the order the work depends:
   - the `SecurityHeaders` middleware on the framework's own `Middleware`
     trait, with strict, balanced and relaxed presets, configured from the
     environment or a builder, outermost of the global middleware (SEC-001),
     and every header SEC-002 names, HSTS decided by `Request::secure`
   - a fresh nonce for every HTML request under a nonce policy, readable by
     application code and views, and stamped on every script and script or
     style link the framework writes: the Inertia document with its
     `csp-nonce` meta element, the development preamble, `Vite::tags`, the
     `Head` facade's links and Live's bootstrap (SEC-003, RDOC-005);
     RenderCache's stitched hits take that nonce instead of minting one
   - route and group overrides, a response's own header winning (SEC-004);
     nonce pages rendered on every request (SEC-005); the manual (SEC-006)
   The data cache the 2026-10-05 draft tied to nonce pages moved to the
   search work ("ok let's move it", 2026-10-15).
   Delivery: `framework/src/security_headers/` with crate-root re-exports;
   the nonce read by the Inertia writer, the root template's `nonce` part,
   `Vite::tags`, the `Head` facade, `LiveBootstrapOptions` and RenderCache's
   stitch; every kit's scaffold registering the balanced preset with nonces
   as its outermost global middleware, mirrored in `scaffold_snapshot`; the
   dogfood application; the integration test binary `security_headers`
   under `framework/tests/`; `manual/security-headers.md` linked from
   `manual/documentation.md`, `manual/env-vars.md` rows, the upgrade guide
   and changelog entries under the open version. Mechanisms:
   `security-headers` (the `security_headers` binary and the scaffold tests
   named `sec_`) and `inertia-root-template` extended to RDOC-005, each
   bound to a violating example before the start. Done when every
   requirement has a current pass on a committed tree and the review is
   recorded.

## error-reporting

Requirements: REPORT-001, REPORT-002, REPORT-003, REPORT-004, REPORT-005, REPORT-006, REPORT-007, REPORT-008, REPORT-009, REPORT-010

58. error-reporting - Rustrak support, which the developer named on
   2026-10-15 ("https://github.com/rustrak/rustrak add this next before
   finishing search and include it with the security headers fix in the
   next release"), with logs, traces and browser reporting included
   ("yes"). In the order the work depends:
   - the `Reporter` trait with the `sentry`, `log` and `null` drivers, the
     `sentry` driver on the official `sentry` crate behind the default
     `reporting-sentry` feature and inert without a DSN (REPORT-001)
   - reporting every unhandled error the framework catches, Laravel's
     helpers and suppression settings (REPORT-002), the event's metadata
     and breadcrumbs (REPORT-003), one redaction shared with the debug page
     (REPORT-004), a bounded background queue with dedupe and a shutdown
     flush (REPORT-005), opt-in logs and transactions (REPORT-006)
   - browser reporting in the kits, `SecurityHeaders` allowing the
     tracker's origin (REPORT-007); the `docker:compose` Rustrak service and
     the env rows (REPORT-008); the manual chapter (REPORT-009); the
     standing gate container and the real-delivery tests (REPORT-010)
   Delivery: `framework/src/reporting/` with crate-root re-exports, the
   shared redaction in `framework/src/error/`, Live's panic observer and
   browser reporter, the three Inertia kits' lazy SDK loading, the
   `docker:compose` option, the integration test binary `error_reporting`
   under `framework/tests/`, `manual/error-reporting.md` linked from
   `manual/documentation.md`, `manual/env-vars.md` rows and changelog
   entries under the open version. Mechanism: `error-reporting` (the
   `error_reporting` binary, the scaffold tests named `report_`, and the
   ignored tests ending in `_rustrak` against the standing Rustrak), bound
   to a violating example before the start. Done when every requirement
   has a current pass on a committed tree and the review is recorded.

## tests-share-process-global-state

Requirements: PAR-096

Delivers backlog item tests-share-process-global-state: Shawn 2026-10-14 04:16 'ok' to a backlog item (side-agent note on the fixes round): two framework tests fail only when their binary runs under plain cargo test with several threads, from process-wide state, and pass alone and under nextest, which the gate and every mechanism use. framework/tests/auth/laravel_delta.rs: configured_store_without_selective_revocation_returns_an_error and its siblings bind the session store with App::bind, the process-global container, instead of TestContainer::bind inside their TestContainer::scope, so a parallel test's DatabaseSessionDriver can win the lookup and Auth::logout_other_devices succeeds where the test expects the UnsupportedStore refusal (PAR-096). framework/tests/eloquent/model_changes.rs: dirty_save_syncs_memory_so_a_second_save_preserves_other_writers counts saved events through the global observer registry that ChangeUser::observe(AuditObserver) fills from several parallel tests, so the count comes out high (10 for 7). Fix: bind the store through the test container and count the test's own observer's events (or run each alone with own_process), so a plain cargo test of the auth and eloquent binaries is trustworthy. No production code is involved; low priority.

## decision-01m4zjm8qzc5n3qjx3c9ayha7f

Requirements: SEARCH-004

Delivers backlog item decision-01m4zjm8qzc5n3qjx3c9ayha7f: A search fetches the page it shows and loads later pages on request. Developer, 2026-10-15 06:44: "The search should be paged according to paging on the results page and lazy loaded on subsequent page loads". A lexical search asks the engine for offset + limit + 1 candidates bounded by the hits cap (the extra one decides has_more), refills only when distinct, group_by or hydration drops leave the page short, and each later page is fetched when it is requested; the baseline measures the first page of 10 hits, the page the rivals return.

## decision-01m4zmzch6hckq8nd8ehy25bj8

Requirements: SEARCH-004

Delivers backlog item decision-01m4zmzch6hckq8nd8ehy25bj8 (4482b84f14c2); its body is in the promotion decision.

## qdrant-native-bm25

Requirements: SEARCH-003

Delivers backlog item qdrant-native-bm25: Decision 01M527GRBJM0J80F7Q099WMTCM (developer, 2026-10-16 07:26: "yes"): Qdrant's lexical leg moves from the framework's BM25 encoder to Qdrant's own qdrant/bm25 model, one named sparse vector per weighted searchable attribute with the IDF modifier, options from the index language (default word tokenizer, ASCII folding, no stemmer, no stop words), the scores summed by attribute weight in a formula query; a configured sparse model keeps one named sparse vector; the word payload stays for prefix filters and the typo vocabulary; the framework encoder and its hashed __sparse_indices payload go for the default path; the Qdrant baseline is rerun against the SEARCH-013 floor.
