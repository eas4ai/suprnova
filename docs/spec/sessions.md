# Sessions

Status: Draft
Prefix: SESS

Drafted 2026-09-15 from the live-native review's open finding on the
session middleware, routed here by the developer's `ok` on escalation
`form-008`. The session subsystem is otherwise undocumented in this record;
its observed behavior lives in `manual/session.md` and
`framework/src/session/`. Only the requirement below is contract.

## Concurrent requests on one session

[SESS-001] The framework MUST offer per-session request serialization
("session blocking") that an application enables for a route group or
globally: while it is enabled, the session middleware acquires a lock for
the request's session id through the cache lock driver before it loads the
session, holds it through the handler and the session write, and releases
it afterwards, with a bounded wait to acquire and a bounded hold, so a
request's session write never overwrites a mutation that a request handled
concurrently persisted after this one loaded the session.
Falsifier: with blocking enabled, two requests carrying one session are
handled concurrently, the first persists a flash after the second loaded
the session, and the second's write removes the flash; or a request waits
past the bounded acquire without a decisive outcome.
Evidence: the defect without blocking, `.cairn/backlog/concurrent-requests-on-one-session-write-back-last-writer-wins-so-a-flash-set-by-a-redirect-can-be-overwritten-by-a-request-that-started-earlier.md`, observed in the dogfood browser suite on 2026-09-15; `framework/src/session/middleware.rs` load and write paths; `framework/src/cache/mod.rs`, `Cache::lock`.
Mechanism: `.cairn/mechanisms/session-blocking` (declared with the
commitment `session-request-serialization`).
Refines: none; parity with Laravel's `block()` middleware method.
Status: Agreed 2026-09-15
