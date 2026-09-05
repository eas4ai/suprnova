# Iteration 005 implementation ledger

This ledger records implementation checkpoints for the integrated Suprnova Live
authority. It is evidence about the current implementation state, not a
replacement for the normative Iteration 005 contract.

## 2026-09-04 -- RenderCache Tier 0 foundation

Suprnova now carries a render cache for Complete representations. The plan ran
as nineteen tasks on `iteration-005-live-integration` from `c2aac7a7`, each one
reviewed and fixed before the next started. Their commit subjects, oldest
first, are the record of what was built:

```
feat(render-cache): add route policy, patches, and response eligibility
fix(render-cache): enforce patch bounds through apply as well as build
feat(render-cache): add variance dimensions and privacy classification
fix(render-cache): reject duplicate variance declarations before mutating, redact private material debug output, and drop the test-only key-ring constructor
feat(render-cache): derive canonical bounded lookup keys
fix(render-cache): key digest identity independent of dimensions
feat(render-cache): add the versioned Complete entry codec with integrity
fix(render-cache): serialize variance dimensions by their canonical name
fix(render-cache): hex-encode generation keys and revalidate decoded headers
fix(render-cache): reject uppercase and mixed-case generation keys
test(render-cache): cover the wrong-length and non-hex generation key paths
feat(render-cache): add the RenderStore contract and immutable L0 store
fix(render-cache): fail closed on a zero-bounded L0 store
feat(render-cache): add dependency identities, generation sets, and the ledger contract
fix(render-cache): align the observation bound and add bounded config/feature identities
test(render-cache): cover epoch mismatch, record key bounds, and query class bounds
feat(render-cache): add coherence intervals, leases, and HTTP cache metadata
test(render-cache): pin freshness transition boundaries and document shell-stitched default
feat(render-cache): add fenced in-process rebuild coordination
fix(render-cache): box the leader lease and cover the waiter wakeup
feat(render-cache): register route and group policies on the router
fix(render-cache): drop the test suppression and pin full-policy override
feat(render-cache): add the request-scoped dependency collector
fix(render-cache): observe every read seam and bound the collector report
fix(render-cache): observe authorization reads on the raw gate paths
feat(render-cache): add the database generation ledger and its migration
fix(render-cache): read generations from the primary and order advances
feat(render-cache): advance generations from every supported write path
fix(render-cache): gate write instrumentation and keep advances in the caller transaction
feat(render-cache): advance generations from the remaining write paths
feat(render-cache): add the atomic file L1 store
feat(render-cache): close the raw persist and payments write gaps
fix(render-cache): serialise file store eviction and harden publication durability
test(render-cache): assert the payments and raw entity write paths advance
fix(render-cache): reconcile the file store tally when the directory sync fails
fix(render-cache): advance payments generations after the hydration transaction commits
fix(render-cache): remove the evicted file before dropping it from the tally
fix(render-cache): reject before evicting and keep the tally honest on cleanup
feat(render-cache): serve proven Complete representations through one middleware
fix(render-cache): key by observed privacy and stop clearing global middleware
fix(render-cache): honest transaction events, real stale-on-error, and L1 coverage
fix(render-cache): observe every identity read and key by the narrowed class
fix(render-cache): require the dimension each narrowing reason names
fix(render-cache): decline when the render did not use the key's values
fix(render-cache): compare every observed value against the key
fix(render-cache): observe flag identity by scope and admit anonymous keys
fix(render-cache): record a bare read for a scoped axis with no field
feat(render-cache): keep public-seed Live documents Complete until the seed deadline
fix(render-cache): decline reason-less private classes and record Live mount facts at mount
fix(render-cache): scope the unreasoned-private-class invariant to narrowed classes only
fix(render-cache): strip a classification copy in the test seam, never the real outcome
feat(render-cache): add L1 sweep, store inspection, and hidden console commands
test(render-cache): prove every write window is caught before a stale serve
fix(render-cache): bind L1 retention to the policy dead edge, bound sweep, clear L0 on epoch advance, and harden the console commands
test(render-cache): prove no private material crosses visitors, locales, or tenants
docs(render-cache): document the RenderCache Tier 0 foundation for implementers and application authors
test(render-cache): prove races against production, not the test's own hooks
docs(render-cache): use bare ? in the manual's route examples now that RenderCacheError converts to FrameworkError
fix(render-cache): make the Dead edge class-aware, stop printing a false epoch on ledger failure, and fix stale docs
docs(manual): translate the RenderCache chapter into the six locales and stamp the lock
fix(render-cache): close the arm/fire flag-hook interleaving window
test(render-cache): give every privacy leak test its own positive control
```

### What this closing checkpoint added

`RenderCacheError` now converts into `FrameworkError` as an `Internal`
category through the same shape the other Live subsystem conversions use, so
a route builder that declares a policy propagates with a bare `?`; the
message is the engine's own closed `render_cache_*` token and never key
material. The middleware's honest-boundary section now names Eloquent global
scopes as an uninstrumented seam, with declaring `Tenant` on affected routes
as the remedy. Two further decisions were captured rather than built, in
`docs/specs/suprnova-live/iterations/next/`: a bounded `reason` on the
declined lookup outcome, and global-scope tenant observation. The existing
capture on authorization reads now also names the wider class of public,
uninstrumented accessors onto the authenticated identity and the single
tripwire that is its only present coverage.

### The closing fix round

The final whole-branch review of the range `c2aac7a7` to `c136178c` returned
FIX-THEN-SHIP with fifteen findings and no blocker; the closing fix round
carried out the controller's rulings on each. The render transaction is now
opened at `REPEATABLE READ` on PostgreSQL and MySQL through a shared-body
`DB::transaction_with_isolation`, so the handler's reads and the window-close
generation read share one snapshot on every backend, not only on SQLite; the
query-builder facade (`DB::table(..).get()`, `first()`, `count()`) records
the table it read and the raw `DB::select` family marks a render unstorable;
the permission version is a persisted generation on a reserved identity that
every principal-keyed render observes, so a bump survives a restart; and the
build id is parsed once at install and refused when unparsable, a misplaced
frame is evicted rather than served, an unobservable identity at window
close declines the candidate, the stored header's enum tags are
`snake_case`, and the async route-order test sets its own `APP_ENV`. The
race and consistent-view evidence, previously SQLite-only (finding F15), now
includes
`live_postgres_a_write_committed_during_a_cached_render_is_never_published_as_current`
and its MySQL twin, run and asserted on by name in `scripts/check-postgres.sh`
and `scripts/check-mysql.sh`; the Postgres one fails with the isolation level
removed. The seams the `testing` default feature compiles into ordinary
builds were captured in `iterations/next/test-seams-in-ordinary-builds.md`
rather than changed. The English manual's matching prose (the raw-read
decline, the `Auth::user()` consequence, the PostgreSQL serialization note,
the autocommit qualification, and the bump's new asynchronous signature) was
not edited in this round, because an English-only edit fails the repository
gate's translation-lock step until the six locales are retranslated; it is
recorded in the closing fix report for a translation pass before release.

### The dogfood route and its proof

`app::live::routes` declares a `PublicShared` policy on `/live/public` with a
five-minute fresh interval, one minute of stale service, and five minutes of
stale-on-error; the public seed's own 24-hour promotion deadline bounds the
served `max-age` underneath it. `app/tests/live_render_cache.rs` proves the
route actually caches rather than merely responding: a counting middleware
registered after `RenderCache::install` sits closer to the handler than
`RenderCacheMiddleware`, so a served entry never reaches it, and the second
anonymous GET raises the render count by zero while `store_inspection`
reports one stored entry and `inspect_route_for_test` finds it under the
route's own lookup key, at the undemoted `PublicShared` class. The cached
document's seed still promotes, and a conditional request on the served
validator answers 304 from the same entry. The three-engine Playwright
dogfood suite exercises the same route through the real server.

### The asynchronous route hook, and what it wired up

`RenderCache::install` is `async`, because it probes for the generation
ledger's tables before assembling a runtime, while `Application::try_routes`
takes a synchronous `FnOnce() -> Result<Router, FrameworkError>`. Neither
boot hook can host the install either: `bootstrap` and `http_bootstrap` both
run before a router exists, and a `booted` callback is synchronous and never
sees one. The route closure is the only place with both a container and a
router, so fix round 1 made an asynchronous one.
`Server::try_from_config_with_routes_async` is the asynchronous twin of
`try_from_config_with_routes`, sharing its prologue (services, then the
immutable Live runtime) and its epilogue (mount registration, then
assembly), and `Application::try_routes_async` is the application-level
hook that reaches it. `routes`, `try_routes` and `try_routes_async` write
one slot, so the last of the three called is the one the server builds.

`app/cmd/main.rs` now calls
`.try_routes_async(|| async { live::routes_with_render_cache(routes::register()).await })`,
so `suprnova serve` installs the middleware. `app::live::routes` stays the
synchronous inner half that registers the reserved Live routes, the document
routes, and the cache policy. The CLI scaffold follows: the generated
`cmd/main.rs` uses the same asynchronous form, the generated `src/live/mod.rs`
gains `routes_with_render_cache`, the generated Migrator lists
`suprnova::render_cache::migration::Migration` (without it a generated
application fails the boot probe on its first `suprnova serve`), and the
generated `.env.example` documents `RENDER_CACHE_ENABLED` and
`RENDER_CACHE_L1_DIR`. The acceptance test that builds a freshly generated
application and runs the integrated checker against it still passes.

`RENDER_CACHE_ENABLED=false` is now a real off switch at install time.
`RenderCacheConfig` always carried the `enabled` flag and the middleware
always re-read it per request, but `install` itself ignored it: it probed
and registered regardless, so an application that turned the cache off was
still required to carry the migration and still paid the write side's ledger
SQL. A disabled configuration now returns the router untouched, probes
nothing, assembles nothing, registers nothing, and leaves the
process-installed gate shut. The middleware's own per-request check is
redundant for a process that installs through this path and is left in
place, because a runtime installed by other means (this crate's own test
harnesses build one directly) still relies on it.

### Commands that ran for this checkpoint

```
CARGO_INCREMENTAL=0 cargo check -p suprnova --lib
CARGO_INCREMENTAL=0 cargo check -p suprnova --lib --tests
CARGO_INCREMENTAL=0 cargo check -p suprnova --no-default-features
CARGO_INCREMENTAL=0 cargo check -p app --all-targets
CARGO_INCREMENTAL=0 cargo test -p suprnova --lib render_cache_error_bridge
CARGO_INCREMENTAL=0 cargo test -p suprnova --features database-sqlite --test render_cache_middleware
CARGO_INCREMENTAL=0 cargo test -p suprnova --features database-sqlite --test render_cache_live
CARGO_INCREMENTAL=0 cargo test -p suprnova --features database-sqlite --test render_cache_races
CARGO_INCREMENTAL=0 cargo test -p suprnova --features database-sqlite --test render_cache_privacy
CARGO_INCREMENTAL=0 cargo test -p suprnova --features database-sqlite --test render_cache_operations
CARGO_INCREMENTAL=0 cargo test -p suprnova --features database-sqlite --test render_cache_file_store
CARGO_INCREMENTAL=0 cargo test -p app --test live_render_cache
CARGO_INCREMENTAL=0 cargo test -p app --test live_dogfood --test live_async_dogfood --test live_upload_reacquire
CARGO_INCREMENTAL=0 cargo test -p suprnova-cli --test live_generated_app
npx playwright test e2e/app-dogfood.spec.ts --project=chromium
node scripts/check-specs.mjs
node scripts/check-implementation-docs.mjs
tests/documentation_contract.sh
python3 scripts/check-manual-structure.py
scripts/check-prose-dashes.sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 crates/suprnova-live/scripts/gate.sh
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 scripts/gate.sh
```

Fix round 1 added the asynchronous route hook, the disabled-install no-op,
the application and scaffold wiring, and this documentation, in four
commits:

```
feat(server): add an async route-construction hook for the application
fix(render-cache): make RENDER_CACHE_ENABLED=false a real off switch at install
feat(render-cache): install the cache from serve and from a generated app
docs(render-cache): document the async route hook and the disabled install
```

and ran, in addition to the commands above:

```
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 cargo test -p suprnova --lib
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 cargo fmt --all --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 cargo clippy -p suprnova -p app -p suprnova-cli --all-targets -- -D clippy::disallowed_methods
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 cargo test -p suprnova-cli --test live_generated_app -- --ignored
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
python3 scripts/install-gate.py --source "$PWD" --repo "$PWD" --commit "$(git rev-parse HEAD)"
```

The gate install is the documented step for tooling that changed in a
reviewed commit on this branch: Task 11 added the `render_cache_ledger` live
test blocks to `scripts/check-postgres.sh` and `scripts/check-mysql.sh`, and
the install record still named the commit from before them, so the runner
refused to start on tooling drift. Re-recording from this branch's own tip
is what the record is for.

Every gate run this checkpoint produced, in order, with the tip it covered:

| Tip | Gate | Outcome |
|---|---|---|
| `1c63eeae` | Live | exit 0 |
| `1c63eeae` | repository | exit 2, refused to start on installed-tooling drift; no step ran |
| `11948eed` | Live | exit 0 |
| `11948eed` | repository | exit 1, run `20260905T032853.263434Z-1404788-83b35f4e`; `workspace-tests` failed on two targets this plan had just added, a lib unit test that booted the process-global container and the `env.tpl` half of the scaffold's environment templates |
| `c136178c` (the fix round 2 tip) | Live | exit 0, rerun because that round changed files under `crates/suprnova-live/` |
| `c136178c` (the fix round 2 tip) | repository | exit 0, run `20260905T044827.099098Z-2171055-a58cca6f`, `GATE GREEN: default` |
| the closing fix round tip | Live, then repository | both launched after this table was written, since a ledger row naming their outcome would change the tree they cover; their exit codes, logs, and run identifier are in the closing fix report (`final-fix-report.md` in the plan's SDD workspace) |

Every run was launched detached with its exit code captured to a file. The
recorded install commit, the run logs, and the exit codes live in the task
report alongside the counts, because a ledger edit naming them would change
the tree the gates had just covered.

### A full-tier failure inherited from the branch point

`scripts/check-feature-matrix.sh`, the `feature-matrix` step in the
repository gate's `full` tier only, fails at `c2aac7a7` with four
`Request::for_test_with_headers` errors in `live/async_transport.rs` under
`--no-default-features --features
database-sqlite,database-postgres,broadcasting-fanout --tests`; those predate
this branch, neither gate run for this checkpoint is the full tier, and they
are not this plan's to fix.

## 2026-09-03 -- Framework-integration qualification gate

Plan Task 11 ran the focused static and test gates, the Live crate's own gate
inside the integrated workspace, the architecture and drift audit, and two
independent adversarial reviews of the complete framework integration, one on
architecture and security and one on production quality. The Live crate gate
exposed the first defect itself: the 1-, 10-, and 100-component compile
fixtures under `tests/fixtures/compile/` still carried a hand-written facade
shim from before the integrated macro expansion, so the MSRV phase failed on
the facade's missing `askama` re-export. The fixtures now depend on the
maintained `suprnova-live-macro-fixture` facade, carry the templates the
expansion compiles, and pass the MSRV check.

The reviews returned two blockers, ten majors, and a set of minors between
them. Every blocker and major was either fixed with a regression test or
accepted with the reason recorded here:

- CSRF. Enabling `OriginPolicy::SameOriginOnly` application-wide, as the
  scaffold and the dogfood did, let every same-origin state change pass on
  the browser's origin proof alone. A Live operation now verifies that proof
  on its own inside the CSRF middleware, whatever policy the application
  configured, and ordinary routes keep the configured policy; the scaffold,
  the dogfood, and the manual use the default `CsrfMiddleware::new()`.
  `framework/tests/live_trusted_context.rs` proves the Live proof under the
  default policy, that a Live read records the proof with a not-required
  CSRF check and nothing without it, and that an ordinary same-origin POST
  without a token is still refused.
- Asynchronous routes. The events route and the WebSocket handshake derived
  scope facts without requiring the complete eight-check set, so a bare
  `try_live()` opened anonymous transports. `HostCheckFacts::require_complete`
  is a new engine entry point, `request_host_scope_facts` requires it, and
  `live_async_routes.rs` proves a chain that recorded no facts is refused on
  issuance and on the event stream.
- Public seeds. Anonymous visitors could render a public seed but never act
  on it, because the action route's strict policy needed a principal record
  and the guard's `AuthMiddleware::new()` answered first. The framework
  records each mount's kind at registration, the action boundary closes only
  the identity absences that kind permits before the engine validates the
  request, and `AuthMiddleware::optional()` records a principal when one
  exists and lets an anonymous request continue.
  `framework/tests/live_public_seed_actions.rs` proves the anonymous
  promotion, the identity-bound refusal, and the owner's acceptance through
  the production middleware stack; the dogfood application, its tests, and
  the browser suite now exercise the anonymous increment. That browser case
  exposed a runtime defect the reference-host evidence had never reached: the
  island's morph preflight demanded the island's own instance id, which a
  seed never has, so a seed's first rendered committed response could not
  promote even though the runtime's own authority already preferred the
  response's instance, and the morph preflight then refused the seed root
  itself because it validated every current root as an instance. The
  successor preflight now takes the instance from the response, the morph
  preflight accepts a seed root that carries no instance and revision zero,
  `tests/morph-preflight.test.ts` pins both rules, and the dogfood browser
  suite proves the rendered promotion through the real framework on all three
  engines; the runtime's own static host promotes no seed with a rendered
  response, which is why its evidence never reached this path. Because the core
  bundle changed, the test host's reviewed module-boot and classic-runtime
  integrity pins were recomputed from the same build and reviewed here. The same test
  exposed a fixture defect: the dogfood helper took the first `Set-Cookie`
  header, the XSRF token cookie, so identity-bound requests ran on fresh
  sessions; it now selects the session cookie by name.
- Transports. WebSocket transports were bounded only per rotatable scope and
  the two per-scope counts disagreed; a process-wide bound
  (`MAX_TRANSPORTS_TOTAL`) applies at both creation sites and both count every
  transport in scope. The total bound is a constant reviewed here, not
  exercised by a test that opens thousands of transports.
- `live:make` searched for `.build()` from `registry()` to the end of the
  file and could splice a registration into an unrelated builder; the search
  is bounded to the function body, and `live_scaffold.rs` proves a delegating
  `registry()` leaves the delegate untouched and asks for manual registration.
  Rollback failures are now named in the error instead of being reported as
  untouched, and `live:assets` reports a failed restore with the retired
  path and compares names and sizes before reading a publication.
- `suprnova new` followed a dangling symlink at the project path;
  `symlink_metadata` refuses it, proven by `new_project.rs`.
  `secure_fs::write_private` refuses a symlinked target and pins the mode of
  an existing file, proven by a unit test.
- The dogfood finalizer prefixed the idempotency key past the 128-byte
  finalize-identity budget and retained every upload forever; it now uses
  the key itself and keeps a bounded window. The activity feed renders its
  refresh count so the browser suite asserts the delivered refresh in the
  markup, not only in the request log.
- The browser host defaulted missing registered-event fields; it now fails
  closed on `maximumHops`, `maximumFanout`, and `payloadContract`, which keep
  the runtime's iteration 004 descriptor names. Renaming them to snake case
  is a versioned contract change captured in
  `iterations/next/registered-event-descriptor-casing.md`. The host's bounded
  JSON reader now counts bytes as it reads instead of after.
- A component that declares several streams gets no island-owned
  `live:stream` directive instead of silently getting its first stream;
  `live_multi_stream_root.rs` proves both cases. The classic boot script
  guards a clobbered `SuprnovaLiveAsync` global, the tooling helper bounds
  the base64-encoded asset line and keeps the operation's own failure ahead of
  an end-marker failure, the dogfood's rate limiter is shared and fails
  closed, and its document error page no longer leaks detail in debug builds.
- The Live crate's own gate then exposed a conformance regression from Task
  10: the engine emitted `live:stream` automatically on every single-stream
  island root, which put the directive on the iteration 004 reference host's
  fresh-render island beside its own poll directive and changed the pinned
  polling behavior across three engines. The emission is now an explicit
  opt-in on the engine's mount and execution services
  (`with_island_stream_directive`), off by default so reference-host evidence
  is unchanged, and the framework enables it; the framework's stream tests
  and the reference host matrix prove both settings.
- Accepted with rationale: the identity-bound tenant requirement stays
  `Optional` because `Ok(None)` from `LiveTenantResolver` is a positive
  statement that the request has no tenant, and the trait now documents that
  a resolver which cannot determine the tenant returns an error. The dogfood
  server test keeps its connect-until-accepted readiness probe because
  `Server::run` binds its own listener; the transfer-grant lifetime and the
  byte-budget acquisition order are recorded as review observations for the
  upload domain rather than changed here. The manual no longer claims
  polling loses nothing; it states that event payloads published while a
  transport is unavailable are not replayed.

The drift audit found no engine-to-framework dependency, no application code
naming the internal engine crate, no reference to the retired standalone
macros package, no RenderCache implementation beyond pre-existing error and
doc mentions, and no Magnetar change on the branch: the Magnetar differences
against local `main` come from `main` having advanced past the branch's last
merge, which the branch must merge before hand-off. `rtk tilth diff main..HEAD`
reported 1211 files, 222 modified and 14974 added symbols, and GitNexus
`detect_changes` (compare scope, base `main`) reported 23292 changed symbols
across 1203 files and 208 affected processes at its "critical" level, both
dominated by the imported Live subtree exactly as the cutover entry records.

Verification completed from the integration worktree after the fixes:

```bash
rtk cargo fmt --all -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --no-fail-fast --test <every framework/tests/live_*.rs suite>
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib live::
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib csrf
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p app
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --test live_scaffold --test new_project --test live_cli --test live_assets --test live_generated_app
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --test live_generated_app -- --ignored
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --lib secure_fs
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live --all-targets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live --lib host::checks
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live --test runtime_artifacts
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-macros --test live_ui
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova -p suprnova-live -p app -p suprnova-cli -p suprnova-macros --all-targets --all-features
CARGO_INCREMENTAL=0 rtk cargo +1.94.0 check --manifest-path crates/suprnova-live/tests/fixtures/compile/Cargo.toml --workspace --all-targets
cd crates/suprnova-live/browser && npm run format:check && npm run lint && npm run typecheck && npm run test:unit
cd crates/suprnova-live/browser && npm run build && npm run build:check
cd crates/suprnova-live/browser && npx playwright test e2e/app-dogfood.spec.ts e2e/framework-bootstrap.spec.ts e2e/seed-and-lazy.spec.ts --project=chromium --project=firefox --project=webkit
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1
cd crates/suprnova-live/browser && npx playwright test e2e/iteration-004-integration.spec.ts e2e/app-dogfood.spec.ts --project=chromium
cd crates/suprnova-live && tests/documentation_contract.sh && node scripts/check-implementation-docs.mjs && node scripts/check-specs.mjs
scripts/check-manual-translations.sh && python3 scripts/check-manual-structure.py
git diff --check
rtk tilth diff main..HEAD --blast --budget 12000
```

Those commands passed the 24 framework Live suites (129 cases) and the Live
and CSRF unit cases, 111 application cases, 30 CLI cases plus the ignored
generated-application acceptance case and the private-writer unit case, the
engine's 705 all-target cases plus the new complete-check case, the macro UI
suite, the compile fixtures under the workspace MSRV, 860 browser unit cases,
the reproducible build with byte-identical artifacts, 48 Playwright cases
across three engines (12 dogfood, 27 framework bootstrap, 9 seed and lazy),
the Rust reference-host suite and the iteration 004 integration matrix on
Chromium after the stream opt-in, and the documentation, specification, and
manual gates. Clippy reported only
the four pre-existing findings recorded in the Task 10 entry. The Live
crate's own gate and the repository gate are recorded below.

The Live crate's own gate (`crates/suprnova-live/scripts/gate.sh`) then
passed end to end inside the integrated workspace: the documentation,
specification, and license contracts, formatting and Clippy review, the
fixture, checker, protocol, and security boundaries, the iteration 004 Rust
boundaries, the workspace MSRV check including the compile fixtures, the
nightly fuzz build, the browser dependency and conformance gates, tracked
artifact parity, the Rust reference host, all-target and documentation tests,
the correctness-delay scanner, the browser unit boundaries and broad suite
(860 cases), the iteration 004 browser matrix (249 cases), the real BFCache
lifecycle, the broad browser matrix (631 cases), and the final worktree diff
check. The repository gate refuses an uncommitted tree; it is the push's
precondition and runs on this commit rather than being recorded here.


## 2026-09-02 -- Generated-application proof and the durable dogfood surface

A fresh `suprnova new` application now passes `live:check` out of the box and
the dogfood application under `app/` exercises every Live surface end to end,
closing plan Task 10. Wiring the two surfaces uncovered six integration
defects, each fixed at its owner with a regression test:

- Applications had no way to attach their own middleware to the reserved
  routes. `Router::try_live_with` takes a `LiveRouteGuard` whose middleware
  chain is applied to the action, upload, and three asynchronous control
  routes and to the WebSocket upgrade (which also pins a same-origin
  policy); asset routes stay unguarded. `Router::try_live()` is the empty
  guard.
- The CSRF middleware demanded a token from a runtime that sends none. A Live
  operation whose origin check passes now records `Origin` as passed and
  `Csrf` as not required under the stateless policy; the token path applies
  only when no origin proof is present.
- Identity-bound mounts required a tenant even for single-tenant
  applications; the requirement is now optional for that scope while session
  and principal stay required.
- Upload-capable islands concealed protocol 2 actions because the upload
  selector match compared protocol versions; the match now validates the
  request's own selection against the selector scope.
- Component templates cannot declare the island-owned `live:stream`
  directive and a bare island root is rejected by the checker, so the engine
  now emits `live:stream="<name>"` on the island root it renders from the
  component's first declared stream, at execution, mount, and public mount.
- The browser runtime's asynchronous feature stayed inert without host glue.
  The asynchronous artifacts now ship a default browser host
  (`browserAsyncOptions()`, `BrowserAsyncAuthority`, `browserSseMembership`;
  classic `window.SuprnovaLiveAsync`) that issues and renews through the
  reserved subscription route, drives SSE membership control with the issued
  bearer credential, and opens the native transports. The framework serves a
  third boot script, `suprnova-live.boot.async.esm.js`, selected whenever a
  document's roles include the asynchronous artifact, and the classic boot
  configures the host when the classic asynchronous artifact is present.
- The runtime's bearer SSE reader omitted credentials, so the guarded events
  route answered 401 before the stream opened; it now sends same-origin
  credentials while the bearer stays the transport authority.
- WebKit delivered only the first piece of a two-record batch to the page and
  held the rest until more bytes arrived, which a system-call trace of its
  network process confirmed. The framework now follows every productive SSE
  batch with a 200 ms delayed comment trailer, proven by a framework test and
  the three-engine dogfood suite; the reference host's two-second comment
  cadence had masked the behavior in the runtime's own evidence.

The scaffold writes `src/live/mod.rs` with an empty registry builder and a
`routes()` function that guards the reserved routes with authentication,
single-tenant resolution, and rate limiting; `bootstrap.rs` binds the registry
and the same-origin CSRF policy; `cmd/main.rs` installs the Live routes.
`live:make` inserts into an empty builder and declares the module after the
use block. `suprnova-cli/tests/live_generated_app.rs` holds a fast template
proof plus an ignored acceptance test that scaffolds, generates a component,
points the application at the local framework, and runs `live:check` and
`live:inspect --json`; the repository gate's scaffold step runs that suite.

The dogfood application registers a counter, an avatar uploader, and an
activity feed; mounts them identity-bound on `/live` and the counter as a
public seed on `/live/public`; guards the reserved routes; declares the
reacquisition route `/account/uploads/{handle}/reacquire`; and supplies a
single-tenant resolver, an upload finalizer, and Gate abilities for the
stream and every upload control. `app/examples/live_dogfood_host.rs` is the
real server the Playwright suite drives on port 4178. `manual/live.md` is the
application-facing chapter, translated into the six locales, with `cli.md`,
`cli-generators.md`, and `documentation.md` updated in every locale.

Verification completed from the integration worktree:

```bash
rtk cargo fmt --all -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_assets --test live_tooling_protocol
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib live::
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_async_routes
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test <every framework/tests/live_*.rs suite>
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p app
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live --all-targets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --test live_generated_app --test live_cli --test live_scaffold --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --test live_generated_app -- --ignored
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova -p suprnova-live -p app -p suprnova-cli --all-targets --all-features
cd crates/suprnova-live/browser && npm run format:check && npm run lint && npm run typecheck && npm run test:unit
cd crates/suprnova-live/browser && npm run build && npm run build:check
cd crates/suprnova-live/browser && npx playwright test e2e/app-dogfood.spec.ts e2e/framework-bootstrap.spec.ts --project=chromium --project=firefox --project=webkit
cd crates/suprnova-live && node scripts/check-correctness-delays.mjs
cd crates/suprnova-live && tests/documentation_contract.sh && node scripts/check-implementation-docs.mjs && node scripts/check-specs.mjs
scripts/check-manual-translations.sh && python3 scripts/check-manual-structure.py
git diff --check
```

Those commands passed the 22 framework Live suites (123 cases) and the 11
Live unit cases, 110 application cases including the dogfood, reacquisition,
and SSE, WebSocket, and polling suites, 705 engine cases, 27 CLI cases plus
the ignored generated-application acceptance case, 858 browser unit cases,
the reproducible build with byte-identical artifacts, 39 Playwright cases
across three engines (12 dogfood, 27 framework bootstrap), the delay scanner,
and the documentation, specification, and manual gates. Clippy reported only
the four pre-existing findings in the Magnetar integration module and its
test helpers and the engine's promoted-action constructor, none in files
this task touched.

## 2026-09-02 -- Live CLI workflows and the application tooling protocol

The Suprnova CLI gained `live:make`, `live:check`, `live:inspect`, and
`live:assets`, closing plan Task 9. `live:make` scaffolds a component in
`src/live/`, its view in `templates/live/`, and its registration in a
`registry()` builder in `src/live/mod.rs`, declares `pub mod live;` in
`src/lib.rs`, validates every target and refuses traversal and symlinks
before writing, writes atomically, never overwrites, rolls back every file a
failed run had written, and reports a dry run.
The other three commands are thin clients of a new hidden framework console
command, `__suprnova:live-tool`, registered at link time by
`framework/src/live/tooling.rs`; the CLI starts it through the explicit
console-binary Cargo wrapper and consumes the bounded, versioned JSON-lines
protocol in `framework/src/live/tooling_protocol.rs`. The helper owns
registry access, checked-template validation through the engine
`TemplateChecker`, safe inspection (presence booleans and counts only), and
asset export with lengths and digests; the CLI keeps no framework or engine
dependency and re-verifies every digest, version, sequence, identity, cap,
and marker on the transport, failing closed with no writes on anything
unsupported, stale, truncated, oversized, or unexpected on stdout, and never
echoing stdout content. `live:assets` stages `<out>/<identity>/` and renames it
into place, treats an identical publication as up to date, and refuses drift
unless `--replace` is given. The engine registry gained `ComponentRegistry::names`
so the framework can enumerate registered components.

Verification completed from the integration worktree:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_tooling_protocol
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --test live_cli --test live_scaffold --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli --lib live_
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-cli
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test console --test console_typed --test console_db_seed --test command_macro --test live_boot --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p app --test console_binary_e2e --test console_greet
rtk cargo fmt --all -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova-cli -p suprnova -p suprnova-live --all-targets --all-features
rtk tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs
rtk node scripts/check-specs.mjs
```

Those commands passed eight framework tooling-protocol cases (a registered
hidden helper, proved and failing components, bounded redacted inspection,
byte-exact asset export, unsupported protocols and operations, template root
and symlink refusals, and missing views instead of a vacuous pass), 26 CLI
cases across the three new suites plus six unit cases (help, project and
template-root preconditions, hostile stream matrix, caps, a fake application
console replaying scripted streams for check, inspect, and assets, scaffold
conflicts, dry run, idempotence, invalid names, symlink refusal, rollback of
a failed run, exact idempotent publication, drift refusal and replacement,
and digest mismatches), the existing console and Live suites, zero new Clippy findings,
and the documentation contracts. The generated application's bootstrap does
not yet bind the registry; plan Task 10 wires that so a fresh scaffold passes
`live:check` out of the box. The repository gate run for this checkpoint also
surfaced a pre-existing race in the framework quarantine store: a chunk
write completed after `write_all` while Tokio's buffered file finished the
operating-system write on the blocking pool, so a whole-file verification
could read a short file and reject the checksum. The store now flushes before
a write operation completes (`framework/src/live/ports/upload_provider.rs`),
and both upload suites passed 25 consecutive runs after the change.

## 2026-09-02 -- Framework artifact delivery and document bootstrap

Suprnova now serves the exact reviewed browser artifacts and emits typed
bootstrap markup from documents, closing plan Task 8. The ten deterministic
build outputs are tracked under `browser/dist/` and embedded into the engine
by the new `suprnova_live::artifacts` module, which validates the manifest
against the embedded bytes on first use and fails closed on any drift in
digest, length, file name, role, capability, or version. The Live gate gained a
"tracked artifact parity" phase that rejects a rebuilt `dist/` differing from
the tracked bytes, so the embedded bytes and the reproducible build cannot
diverge silently.

`Router::try_live()` registers `/__live/v1/assets/<asset_identity>/<file>` for
`GET` and `HEAD` with immutable caching, strong digest validators, conditional
requests, `nosniff`, closed misses, and two framework-owned external boot
scripts, so a document loads no inline executable code and a strict
`script-src 'self'` policy holds. `LiveDocument::bootstrap` maps mounted
components to the upload and asynchronous roles, adds the Stimulus bridge on
request, emits the inert configuration element plus ordered preload and script
tags with integrity values for the ESM or classic strategy, and rejects a
second bootstrap or a mount after bootstrap. `Router::try_live_document`
declares a document route without startup mounts. Two engine additions
support the host: `TrustedHtml::framework_generated` for framework-assembled
markup and the public `TrustedLiveRequestContext::host_scope_facts` accessor
from Task 7. The reference host's artifact validation was not replaced; the
engine module is the shared home a later cleanup can point it at.

Verification completed from the integration worktree:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova-live --test runtime_artifacts --test trusted_markup
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib live::assets
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_public_api --test live_facade_contract --test live_dependency_topology --test live_document_routes --test live_routes --test live_boot --test live_hostile_adapter --test live_view_contract
rtk cargo fmt -p suprnova -p suprnova-live -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova -p suprnova-live --all-targets --all-features
(cd crates/suprnova-live/browser && rtk npm run format:check && rtk npm run lint && rtk npm run typecheck)
(cd crates/suprnova-live/browser && rtk npm run test:unit -- tests/build-contract.test.ts tests/optional-artifacts.test.ts)
(cd crates/suprnova-live/browser && rtk npx playwright test e2e/framework-bootstrap.spec.ts --project=chromium --project=firefox --project=webkit)
(cd crates/suprnova-live && rtk git diff --exit-code --stat -- browser/dist)
rtk tests/gate_contract.sh
rtk tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs
rtk node scripts/check-specs.mjs
```

Those commands passed six engine artifact and trusted-markup cases, seven
framework asset and bootstrap cases plus four unit cases, 49 cases across the
eight existing Live suites, zero new Clippy findings, 18 browser artifact unit
cases, and the nine-case real-server Playwright scenario on Chromium,
Firefox, and WebKit (an example binary, `live_bootstrap_host`, is the real
Suprnova server the Playwright configuration starts on port 4177). The
scenario covers ESM and classic role selection, a core-only document, the
optional Stimulus role, duplicate boot tags, an incompatible optional
feature, an integrity failure that leaves SSR content intact, a strict
self-only Content Security Policy, and byte-exact immutable artifacts with
conditional requests. The full Live crate gate and the Suprnova repository
gate were not rerun for this checkpoint; the repository gate runs before the
next push.

## 2026-09-02 -- Framework asynchronous transport routes

Suprnova now registers the reserved versioned `/__live/v1/async/subscriptions`,
`/__live/v1/async/memberships`, `/__live/v1/async/events`, and
`/__live/v1/async/socket` paths next to the action and upload endpoints, using
the existing router, middleware chain, response, and WebSocket upgrade
machinery. Components declare `streams(...)` in the `#[live]` attribute and the
macro emits the engine's subscription metadata. The framework installs the
engine's subscription registry, authorization, continuity, and credential ports
only for asynchronous requests; the engine signs every descriptor, verifies
every membership, and drives bounded document delivery. Stream authorization is
the Gate ability `live:{component}.stream.{stream}`, application code publishes
through `suprnova::live::LiveStreams`, and the route, credential, limit, and
failure contracts are recorded in `docs/implementation/async-updates.md`.

Two engine accessors became public for the host: `SubscriptionError::new` and
`TrustedLiveRequestContext::host_scope_facts`. The production browser artifact
and the async reference host now use the versioned SSE and WebSocket paths. The
framework's WebSocket upgrade path records its pre-chain `Origin` proof through
a new `record_passed_before_chain` attestation entry, because that check runs
before the middleware chain and therefore cannot claim a position in the
enforced execution order. Engine document sessions sit behind per-transport
asynchronous locks so engine callbacks into the host ports never re-enter the
runtime's table mutex.

Verification completed from the integration worktree:

```bash
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_async_backpressure --test live_async_routes --test live_async_security
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --test live_async_backpressure --test live_async_routes --test live_async_security --test live_boot --test live_dependency_topology --test live_document_routes --test live_external_authoring --test live_facade_contract --test live_hostile_adapter --test live_macro_expansion --test live_public_api --test live_routes --test live_trusted_context --test live_upload_policy --test live_upload_providers --test live_upload_routes --test live_upload_security --test live_view_contract
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo test -p suprnova --lib live::async_transport
rtk cargo fmt -p suprnova -p suprnova-live -p suprnova-live-test-support -- --check
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=8 rtk cargo clippy -p suprnova -p suprnova-live -p suprnova-live-test-support --all-targets --all-features
(cd crates/suprnova-live/browser && rtk npm run format:check && rtk npm run lint && rtk npm run typecheck)
(cd crates/suprnova-live/browser && rtk npm run test:unit -- tests/async-connections.test.ts tests/async-feature.test.ts)
(cd crates/suprnova-live/browser && rtk npm run build && rtk npm run build:check)
(cd crates/suprnova-live/browser && rtk npx playwright test e2e/async-lifecycle.spec.ts --project=chromium)
rtk tests/documentation_contract.sh
rtk node scripts/check-implementation-docs.mjs
rtk node scripts/check-specs.mjs
rtk git diff --check
```

The three new framework suites passed 14 cases on two consecutive runs, the
complete framework Live sweep passed 102 cases across 18 binaries, the four
transport parser unit cases passed, and 69 browser async unit cases, the
deterministic artifact check, and the five Chromium async lifecycle cases
passed against the rebuilt artifact and the reference host. Clippy reported
zero errors and no new warnings; the previously reviewed
`execution/service.rs` argument-count warning and the pre-existing test-module
notes outside the Live tree remain. The fairness assertion in
`live_async_backpressure` is a liveness bound (the sibling is served within the
backlog admitted before it joined) because kernel socket buffering makes a
tighter interleaving bound nondeterministic through a real socket; the
coalescing assertion is checked only after envelopes were read, which is the
state barrier proving the document drained. The full integrated gate was not
rerun for this checkpoint.

Before the branch was first pushed to `origin` on 2026-09-02, the Suprnova
repository gate (`scripts/gate.sh`, default tier: formatting, published
document references, dash policy, workspace Clippy, JSON rustdoc,
`cargo test --workspace --no-fail-fast`, Magnetar all-feature tests, Postgres
regressions, and scaffold compile tests) passed on commit `7c2a1123`. Reaching
that took four follow-up commits: the gate's reference checker no longer reads
ignored-name fragments as grep options; the crate's per-directory assistant
guidance file and the two `docs/superpowers` plans left the published tree
(they remain local, ignored files) and `conventions.md` states the authority
rule inline; the spec checker spells its em-dash test as an escape; and every
cookie-queue test that drives the session middleware now holds the crypt hook
guard, which removed a one-in-six parallel failure that predated this branch.
The Live crate's own gate under `crates/suprnova-live/scripts/gate.sh` was not
run in this session.

## 2026-09-02 -- Standalone synchronization and budget removal

The integrated crate merged the final standalone `main`, commit `59395ec`,
through a subtree merge on top of the `6d19d02` import. The merge brought the
WebSocket closure classification fix, the reference-host policy-close
handshake, the same-run bound for the macro expansion check, the removal of
every benchmark and artifact budget from `scripts/gate.sh`, and the deletion
of the artifact budget script together with its reviewed size history. The
provenance-graph hardening this crate had layered on that script left with
it. `npm run build` now prints the raw and Brotli bytes of every artifact and
nothing caps them; the budget scripts remain on-demand tools. Captured
future-iteration notes stayed out of the import as the contract requires.

Dedicated S1 and B1 qualification is release-checklist work outside Iteration
005, and the historical-baseline question is closed because the size history
it concerned no longer exists. The "Qualification still outstanding" paragraph
in the cutover checkpoint below is superseded. The documentation contract had
required the singular `## Child parameter envelope` heading, the removed
standalone disclaimer in the component-authoring document, and the earlier
Stimulus exclusion wording; the contract now names the plural heading, the
real `suprnova::live` facade statement, and the reworded Stimulus sentence.

Verification completed from the integration worktree:

```bash
(cd crates/suprnova-live && bash tests/gate_contract.sh)
(cd crates/suprnova-live && bash tests/documentation_contract.sh)
(cd crates/suprnova-live && node scripts/check-implementation-docs.mjs)
(cd crates/suprnova-live && node scripts/check-specs.mjs)
(cd crates/suprnova-live && node tests/expansion_budget_rules.mjs)
rtk cargo test -p suprnova-live --test upload_budget_contract --test async_budget_contract
rtk cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1
(cd crates/suprnova-live/browser && npm run format:check && npm run typecheck)
(cd crates/suprnova-live/browser && rtk proxy npm run lint)
(cd crates/suprnova-live/browser && npm run test:unit -- tests/budget-contract.test.ts tests/build-contract.test.ts tests/package-contract.test.ts tests/protocol-overhead.test.ts tests/async-websocket-closure.test.ts)
(cd crates/suprnova-live/browser && npm run build)
(cd crates/suprnova-live/browser && npx playwright test e2e/bootstrap.spec.ts --project=chromium)
git diff --check
```

Those commands passed the gate contract, four Rust budget-contract cases, all
28 reference-host cases, twenty focused browser unit cases, the deterministic
build, and the twelve Chromium bootstrap cases. `rtk npm run lint` from this
nested package resolves the system ESLint instead of the package's pinned one
and fails before linting; the unfiltered `rtk proxy npm run lint` passed. The
full integrated gate did not run for this checkpoint.

## 2026-09-01 -- Framework upload boundaries and application reacquisition

Suprnova now registers the versioned `/__live/v1/upload` control/data endpoint
and exposes an explicit router helper for authenticated application-owned
reacquisition paths outside `/__live/`. Generated Live component metadata owns
checked per-field upload policy, including count, declared and aggregate bytes,
accepted media, and replacement behavior. The public `suprnova::live` facade
exposes application configuration and typed policy/host contracts without
leaking the internal engine crate.

Host-owned adapters keep revisioned lifecycle persistence, bounded metadata,
quarantine byte I/O, reverse-proxy transfer, constrained direct-provider
instructions and reports, scanner and application validation, immutable
evidence, finalization, and cleanup separate. The engine remains authoritative
for handle identity, transfer grants, state transitions, ready proposals, and
finalization semantics. Every request revalidates current mount and principal,
session, tenant, component, field, and document scope; a per-handle operation
lock serializes chunk, completion, cancellation, action, finalization, and
cleanup races. Chunk bodies reserve the shared in-flight budget before
buffering, carry an explicit authoritative offset, reject impossible permit
requests, and preserve exact idempotent outcomes without writing bytes before
revision acceptance.

Action dispatch retains only signed ready-handle proposals, commits the Live
outcome before durable finalization, and reconciles retryable finalizer failure
without invoking the action again. Cleanup runs automatically and owns bounded
retry/lease behavior. The browser and Rust host now agree on the versioned
route, `queued` create state, chunk-response shape, and required offset header.
The Rust reference host's ordinary-action fixture was also corrected to emit a
typed base64url correlation identity and the normative v2 `invoke_action`
operation; that correction turned seven shared-host regressions into a green
26-case suite.

Verification completed from the integration worktree:

```bash
rtk cargo test -p suprnova --test live_upload_routes --test live_upload_security --test live_upload_providers
rtk cargo test -p suprnova-live --test upload_file_provider --test upload_service --test upload_direct_provider --test upload_protocol --test upload_state --test upload_validation --test upload_budget_contract --test upload_identity --test upload_finalization --test upload_cleanup --test upload_security --test upload_framework_budget_integrity
rtk cargo test -p suprnova --test live_upload_policy
rtk cargo test -p suprnova-macros --test live_ui
rtk cargo test -p suprnova-live-test-support --test reference_host -- --test-threads=1
(cd crates/suprnova-live/browser && rtk npm run test:unit -- tests/upload-*.test.ts)
(cd crates/suprnova-live/browser && rtk npm run build)
(cd crates/suprnova-live/browser && rtk npm run build:check)
(cd crates/suprnova-live/browser && rtk npm run budget)
(cd crates/suprnova-live/browser && rtk npm run budget:upload)
(cd crates/suprnova-live/browser && rtk npx playwright test e2e/uploads.spec.ts --project=chromium)
rtk cargo clippy -p suprnova -p suprnova-live -p suprnova-macros --all-targets --all-features
```

Those commands passed 22 framework route/security/provider cases, 98 engine
upload cases, two policy cases, the macro UI suite, all 26 reference-host cases,
134 browser upload unit cases, deterministic artifact checks, the existing
artifact and upload budget gates, and the Chromium upload lifecycle. Clippy
reported zero errors and retained the two previously reviewed
`execution/service.rs` argument-count warnings; no blanket warning denial or
new suppression was introduced.

## 2026-09-01 -- Exact-child delivery through the real endpoint

Accepted protocol-v2 parent execution now derives changed-child transitions
from rendered composition, binds each v2 envelope to the accepted successor
lineage, and prepares one deterministic delivery for each changed surviving
child. Unchanged, removed, replaced/remounted, duplicate/invalid-lineage, and
v1-parent outcomes emit none. Envelope signing, response encoding and bounds,
and complete response sealing all occur before host commit and ledger
acceptance, so precommit failure exposes zero response bytes and cannot accept
the parent.

The browser validates the complete parent response, morphs and commits the
successor, then pairs its one top-level signed parent snapshot with each child
delivery. The resulting ordinary scheduler intent sends the child's own current
snapshot and exact `child_parameters` carrier
`{"envelope":...,"parent_snapshot":...}` without raw parameters or a second
queue. Redirect, malformed response, morph failure, stale/mismatched boundary,
unchanged hash, and removal schedule nothing.

The existing Suprnova Live action route now parses exact/bounded carriers and
independently verifies child snapshot, parent snapshot, and purpose-separated
v2 envelope before kernel dispatch. The kernel consults authoritative parent
ledger currentness: logical missing/stale/mismatched authority is concealed,
while provider failure retains an unavailable failure. Modern
`params_changed` consumes only `EligibleChildParametersV2`, hydrates and invokes
the generated lifecycle once, renders/signs a successor child snapshot,
advances the child's ledger, and records the applied parent revision in owner
lineage. The macro generates both modern and explicitly historical v1 hooks
from one declaration; raw v1 never enters production admission.

Focused real-route coverage proves success, exact sealed response projection,
raw-envelope/v1-shaped and malformed rejection, forged signature, cross-child,
cross-session, cross-tenant, and superseded-parent rejection before component
work or child-ledger acceptance. Rejected child delivery leaves the already
accepted parent revision unchanged.

## 2026-09-01 -- Accepted-revision, signed lineage, and exact-child foundation

The host-neutral engine now exposes a provider-neutral
`LiveInstanceLedger::current_accepted_revision` authorization read. The memory
provider performs it under the same mutex as claim and commit: Ready returns the
current revision, Pending returns its accepted base rather than its unaccepted
successor, and missing, pruned/expired, or terminal Consumed authority returns
`None`. Clock or provider synchronization failure remains `LedgerError`.
Diagnostic inspection and browser snapshots are not correctness fallbacks.

Snapshot schema v1 remains stable and recognizes the optional canonical signed
`x_suprnova_live_composition_v1` extension. It carries optional owner lineage
and bounded immediate-child entries binding parent instance/revision, stable
key, child component contract, exact child instance, and depth. Exact-shape,
identity, duplicate, mixed-authority, 256-child, depth-64, and 64-KiB bounds are
enforced before trusted use. Public seeds reject it; unknown well-formed
namespaced extensions retain the existing v1 compatibility rule.

Child-parameter schema v2 has a separate signing purpose and adds exact child
instance binding without changing v1 decoding. Server authorization returns an
`EligibleChildParametersV2` only when verified v2 data matches the signed parent
snapshot lineage and the ledger still reports the exact issuing parent
revision. Superseded revisions, foreign scope/parent/key/component/child,
missing authority, and provider errors fail closed. This foundation checkpoint
deliberately deferred framework HTTP child delivery, parent response emission,
browser scheduling, and `params_changed` execution to the slice recorded above.

Strict TDD evidence includes compile-time REDs for the new ledger read,
composition extension, and v2 envelope APIs; a behavioral RED showing a replay
was still accepted after a later parent revision; and focused GREEN suites for
ledger transitions, signed composition tamper/bounds/compatibility, exact-child
bindings, lineage eligibility, supersession, missing authority, and causal
provider failure.

## 2026-08-31 -- Atomic workspace cutover

The committed standalone history, engine, browser runtime, specifications,
checker, fixtures, tests, benchmarks, and implementation guides are now owned
only by `crates/suprnova-live/` in the Suprnova workspace. The integration branch
was reconciled with Suprnova `main` through commit `a2248c64`; concurrent
framework and Magnetar changes were merged without editing or reverting them.
Outside the imported Live tree, the public tracked worktree changes only the
root workspace manifest, root lockfile, and the checked-in cutover plan. The
separate ignored local tooling repository at
`/home/shawn/workspace2/suprnova/scripts` owns the adapter in local-only commit
`ba03b7f` (`build: gate integrated suprnova live`). That repository has no
remote, was not added to the public worktree, and was not pushed.

The following command passed from the integrated Suprnova worktree after the
authority cutover and workspace reconciliation:

```bash
rtk /home/shawn/workspace2/suprnova/scripts/check-suprnova-live.sh
```

That ordinary gate passed the specification and implementation-documentation
checkers, generated license inventory, Rust formatting and Clippy review, macro
UI suite, MSRV check, fuzz build, all-target and documentation tests, reference
host, correctness-delay scanner, TypeScript formatting/lint/typecheck, 854
browser unit tests, the Iteration 004 three-engine matrix, CSP and BFCache
coverage, the broad three-engine matrix, deterministic artifact checks, reduced
local performance workloads, expansion budget, and final diff check. The gate
reported `Suprnova Live iteration gate passed`.

The first post-merge full run encountered one non-reproducible Firefox lifecycle
observation: the reconnect assertion saw zero active connections after the
membership control had advanced. No code, timeout, retry policy, or assertion
was changed. The exact failed case then passed once and passed 20 repeated runs
from `crates/suprnova-live/browser/`:

```bash
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/async-lifecycle.spec.ts --project=firefox \
    --grep "real async transport exposes bounded semantic feedback without stealing focus")
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/async-lifecycle.spec.ts --project=firefox \
    --grep "real async transport exposes bounded semantic feedback without stealing focus" \
    --repeat-each=20)
```

The complete ordinary gate was rerun unchanged and passed, including the failed
case in the broad Firefox matrix. The isolated result is retained here rather
than hidden or converted into a weakened test. If it recurs, diagnosis must add
test-host-only lifecycle provenance before changing production behavior.

Two earlier WebKit upload-timing observations in the cutover session were also
non-reproducible and are retained here. In
`classic async missing leaves the other optional feature operational`, the
upload remained `transferring` with zero of 31 bytes observed instead of
reaching `ready` within five seconds. In
`freeze and resume preserve active upload retry authority while shutdown
cancels it`, the retried upload likewise remained `transferring` with zero of 20
bytes observed. No file or timing contract was changed. Each exact case passed
on its isolated rerun from `crates/suprnova-live/browser/`:

```bash
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/iteration-004-integration.spec.ts \
    --project=webkit \
    --grep "classic async missing leaves the other optional feature operational")
(cd crates/suprnova-live/browser && \
  rtk npx playwright test e2e/iteration-004-lifecycle.spec.ts \
    --project=webkit \
    --grep "freeze and resume preserve active upload retry authority while shutdown cancels it")
```

Both returned `PASS (1) FAIL (0)`, and later complete broad matrices passed the
same WebKit cases. They therefore remain disclosed nondeterministic signals, not
resolved root causes. A repeat must be investigated with test-host-only upload
progress provenance before any production or assertion change.

Change-impact and drift review ran the following commands against the reconciled
`main` comparison basis:

```bash
rtk tilth diff main..HEAD --blast --budget 12000
rtk git diff --check main..HEAD
rtk git status --short --branch
rtk git diff --name-status main..HEAD -- ':!crates/suprnova-live/**'
```

Tilth reported 889 added files, zero modified files, and 11,611 added symbols;
the imported subtree therefore dominates its deliberately large report. The
range diff check passed, and the clean pre-ledger status was
`## iteration-005-live-integration`. The path-scoped diff listed only
`Cargo.toml`, `Cargo.lock`, and the cutover plan outside the Live subtree, so no
Magnetar file or unrelated framework refactor belongs to the cutover.

GitNexus `detect_changes` ran with project
`home-shawn-workspace2-suprnova-live-integration`, base branch `main`, depth 1,
and scope `crates/suprnova-live`. It reported 889 changed files and zero impacted
pre-existing symbols because the history-preserving subtree is entirely new to
the comparison basis. This is broad import evidence, not a low-risk claim about
the already independently reviewed Live implementation.

### Qualification still outstanding

The ordinary gate truthfully reports compatibility qualification as
`unqualified (0/8)`. Iteration 004's `U4/16`, `E100/1K`, and `R100` workloads
still require qualified S1 and B1 evidence. Its historical-baseline repository-
integrity issue also still requires an explicit developer-approved normative
resolution. Local exploratory or reduced measurements do not satisfy those
release gates, and the workspace move does not relabel them as passing.

The repository cutover is complete. The separate Suprnova framework-facade,
host-adapter, and RenderCache implementation plans remain active work inside
Iteration 005.
