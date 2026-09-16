# Review - session-request-serialization

commitment: session-request-serialization
commit: 237d5970
examined:
  - SESS-001 against the built tree: the session middleware's lock wrapper around load, handler and write; the config option, its env reader and the route and group builders' `block_session`; the receipts recorded on it.
  - The five session blocking tests and the three unit tests on the route-block registry, plus the same test group run against a scratch mutant whose `block_for` always answers `None` (`~/workspace2/scratchpads/session-blocking/mutation-no-lock.log`, 2026-09-15): exactly the three feature tests failed and the race and cookieless tests passed.
  - The dogfood application with blocking on for every route: its dogfood tests, and the dogfood browser suite on chromium.
  - The manual chapter, its six mirrors, the changelog entry in seven locales and the translation lock.
  - Re-examined at `237d5970` after closure retired the backlog record SESS-001 cites: the requirement's Evidence line now names the race test instead of the record, the obligation and falsifier are unchanged, the mechanism was reviewed against the new block (LOOP-059) and its receipt at `20260916T022542608Z-1136416` passes.
findings:
  - resolved: The commitment said the dogfood flash case enables blocking on the feedback routes, but the requests that raced the notice's flash were the dashboard islands' connecting requests on other routes, and a lock only serializes the requests that take it; the dogfood application enables blocking for every route in its bootstrap instead, and the commitment records the correction.
  - resolved: With blocking on, the flash case was first rewritten to drop its wait for the dashboard's islands and failed on chromium; every request that loads the session ages the flash, so an island request landing after the notice consumed it, which is flash semantics shared with Laravel and not the write race SESS-001 names. The case keeps its wait with that reason written beside it, and the commitment says so.
  - resolved: A route's block has to be known before the session loads, and the router hands route-level middleware to the chain only after the global session middleware ran; the block is recorded at registration in a process-global registry keyed by method and pattern, like route names, and the middleware reads it through the pattern the server stamps on the request, with a HEAD request reading the GET block the way the server's middleware lookup does.
  - resolved: With blocking on for every dogfood route, ten in-process dogfood tests answered 500 "session blocking needs a reachable cache store", because the test harness boots the application without the cache store `Server::run` binds from `CACHE_DRIVER`; the harness now binds the in-memory cache the way a server boot does, and the fail-closed answer stands, since blocking that ran without its lock would be the race with a false promise.

  - resolved: The combined check over every declared requirement after the kernel change failed the Live gate on one webkit case: the input OTP's cells stayed blank after typing. Both with and without blocking, a morph re-renders the cells empty and resets the input, and the runtime restores the input's value in a later phase without an input event, so the element's mirror ran on an empty value; the case's assertion had been winning a race against that morph, and the lock's few extra milliseconds flipped it on webkit. The element now mirrors again on the frame after a mutation, once the value is restored, proved four times over on webkit with blocking on (`fa3e572e`); the tool's FORM-006 checks still pass.
  - resolved: The same run failed the spec lint on LIVE-020, which stated two obligations in one sentence (PKG-007); no earlier receipt covered LIVE-014, so the finding predates this commitment. The sentence is now two, with the meaning and the Agreed date unchanged (`ee60c838`).

## Build review, 2026-09-15

### Attacked: contradictions

- "Holds it through the handler and the session write, and releases it
  afterwards" against every exit path: the wrapper takes the lock before
  `handle_session` and releases after it returns, whichever of `Ok` or
  `Err` it returns; a handler panic skips the release and the hold bound
  is the TTL that ends it, which is why the hold is bounded at all.
- "Bounded wait to acquire" against a decisive outcome: the acquire loop
  answers `503` with `Retry-After: 1` at the deadline and never loads the
  session; a cache the framework cannot reach answers `500`, because
  blocking that silently ran unlocked would be the race with a false
  promise.
- "For a route group or globally" against the builder surface: the route
  builder, the multi-method builder, the macro route, `any!` route and
  `group!` definitions, and `Router::group` all take `block_session`; a
  nested group or a route overrides its parent's block, and a route block
  takes precedence over the config's.
- The lazy-persistence contract: a request without a valid session cookie
  never touched the store before, and now never touches the cache either,
  since it names no row two requests could race over.

### Falsifiers, each demonstrated

- `without_blocking_the_last_writer_wins_and_the_flash_is_lost` pins the
  race deterministically through a recording store: the second load
  happens before the first write and the stored row ends without the
  flash.
- `with_blocking_the_second_request_loads_after_the_first_wrote` asserts
  the store saw read, write, read, write and that the second handler read
  the flash; the mutant without the lock fails it.
- `the_wait_to_acquire_is_bounded_and_ends_in_a_503` holds the lock from
  the test and proves the request answers `503` after the wait bound and
  before two seconds, with no store read; the mutant fails it.
- `a_route_level_block_applies_without_the_global_option` registers a
  block through the router and proves the request on that pattern waits
  while a request matched elsewhere runs; the mutant fails it.
- `a_cookieless_request_is_not_serialized` proves the store-free request
  stays store-free with blocking on.

### Limits recorded

- The in-memory cache driver serializes requests within one process; a
  deployment with several nodes needs Redis for the lock to hold across
  them, as the manual says.
- The route-block registry is process-global like the route-name
  registry, so two routers registering different blocks for one pattern
  in one process share the last one written.
- Suprnova answers `503` where Laravel throws `LockTimeoutException`; the
  manual's divergence callout records why.

## Mechanism demonstrations

Receipts on `5c561a51`, from the combined checks over every declared requirement after the kernel change (2026-09-16 UTC, logs under `~/workspace2/scratchpads/session-blocking/`):

- SESS-001, `session-blocking`: `.cairn/evidence/runs/20260916T022351458Z-1118126`, five tests run, five passed, exit 0.
- The Live gate group (LIVE-010, LIVE-011, LIVE-012, UI-008, UI-012, OVL-005, FDB-004, NAV-003): `.cairn/evidence/runs/20260916T020857330Z-966158`, pass, after the input OTP fix; the first run at `20260916T013747288Z-509871` failed on the webkit live-native case named in the findings.
- LIVE-014, `spec-lint`: pass after the LIVE-020 rewording; the first run failed on PKG-007.
- LIVE-020, `live-session-reverification`: pass, after the mechanism was reviewed against the reworded requirement (LOOP-059, `reviewed:` entry on the declaration).
- Every other declared requirement: pass, receipts under `.cairn/evidence/runs/` from processes 509871, 966158 and 1118126, committed at `e5607e60` and `5c561a51`.

