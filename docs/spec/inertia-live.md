# Inertia live bindings

Status: Agreed 2026-10-21
Prefix: SIGNAL

Drafted 2026-10-21 from the developer's direction to take the inertia-live
model into the next commitment ("I'd like you to take all 3 in the next
commitment"), modelled on freepeace13/inertia-live-laravel
(`reference/inertia-live-laravel-0.1.0`), read for its correctness rules:
a server-side change publishes a data-free signal on a topic, and the
Inertia pages bound to that topic reload only the affected props through
their own controller, so policies, hidden attributes and per-user fields
keep working unchanged. The binding shares Live's topic layer rather than
adding a second real-time system.

## Observed at bfcbd61fe

Status: Observed

- `LiveStreams::refresh(topic)` publishes a data-free refresh at once,
  with no request buffer and no after-commit flush
  (`framework/src/live/streams.rs:108`).
- Each issued membership owns a subscription log with an epoch and
  sequence, a delivery cursor and bounded reconnect replay; the delivery
  cursor is per membership, not a topic version a page render can read
  (`framework/src/live/async_updates.rs:282,314,347,465,554,931`).
- Transport draining coalesces refreshes per lane; it is delivery
  coalescing, not one signal per topic per request
  (`framework/src/live/async_updates.rs:2641`).
- Subscription authorization resolves a registered ability and the
  current principal, and publication re-checks the Gate and the session
  (`framework/src/live/ports/subscription.rs:67,79`,
  `framework/src/live/async_updates.rs:1647`); a missing principal is
  denied (`framework/src/live/ports/subscription.rs:85`).
- Broadcasting captures `X-Socket-ID` and excludes a sender per envelope,
  and the Redis Streams hub fans out across nodes
  (`framework/src/broadcasting/request_socket.rs:25`,
  `framework/src/broadcasting/broadcastable.rs:130`,
  `framework/src/broadcasting/hub.rs:88`).
- `DB::after_commit` and `Transaction::after_commit` run callbacks only
  after a commit and drop them on rollback, savepoints included
  (`framework/src/database/after_commit.rs:74`,
  `framework/src/database/transaction.rs:1539`; PAR-007).
- Inertia resolves partial reloads and always props
  (`framework/src/inertia/prop.rs:1571`); the kit dashboards poll `stats`
  with `usePoll` (PAR-079), and the stream helpers offer fetch and
  EventSource clients with no topic to prop binding
  (`suprnova-cli/src/templates/files/frontend/stream/core.ts.tpl:535`).

## Requirements

[SIGNAL-001] Live topics MUST be the one real-time subsystem: an Inertia
page binding and a Live island subscribe to the same topic names, under
the same registered ability and principal check, with LIVE-016's
re-evaluation before delivery, through the same subscription log, epoch
and sequence, and over the same transports (SSE, WebSocket, polling
fallback). The framework MUST NOT add a second topic registry, authorizer
or transport for pages.
Falsifier: a page binding needs a topic, ability or transport declared apart from the Live one; or a principal whose authorization was revoked keeps receiving page signals past LIVE-016's window.
Mechanism: `inertia-live`.
Status: Agreed 2026-10-21

[SIGNAL-002] An Inertia response MUST be able to bind a topic to a list of
its props, checked at compile time against the page's `InertiaProps`
struct and carried into the generated TypeScript; the render MUST include
each bound topic's current cursor, a monotonic per-topic counter the
framework owns; and a signal MUST carry only the topic, its cursor and
the affected prop names, never model data.
Falsifier: a binding that names a prop the struct lacks compiles; a render omits a bound topic's cursor; or a signal carries a model field.
Mechanism: `inertia-live`.
Rationale: inertia-live `README.md:3`, `src/LiveBindings.php:10`, `src/Broadcasting/LiveChangeBroadcast.php:45`.
Status: Agreed 2026-10-21

[SIGNAL-003] A change MUST be signalled after the commit of the
transaction that made it and never after a rollback; the changes to one
topic within one request or one job MUST coalesce into one signal with
the union of the affected props; a signal MUST exclude the sender's own
transport when the request's socket id names it; a private topic with no
authorizer MUST fail the boot; and signals MUST be suppressed during a
projector replay, with one final signal per touched topic when the
replay ends.
Falsifier: a rolled-back write signals; ten writes to one topic in one request signal ten times; the sender's own page reloads from its own write; a private topic with no authorizer boots; or a replay signals once per event.
Mechanism: `inertia-live`.
Rationale: PAR-007 governs after-commit; inertia-live `src/ChangeBuffer.php:25,38`, `src/ChangeFlusher.php:36,68`, `src/Concerns/EmitsLiveChanges.php:54`.
Status: Agreed 2026-10-21

[SIGNAL-004] The client MUST drop a signal whose cursor is at or below the
cursor its page rendered with; MUST coalesce the signals that arrive
within a bounded window into one partial reload of the union of their
props; MUST fall back to polling when no push transport is available; and
MUST be shipped as one core with React, Vue and Svelte adapters that
behave the same. Each kit's dashboard MUST use the binding for `stats`,
with polling kept as the fallback.
Falsifier: a signal older than the render reloads the page; a burst of signals reloads once per signal; a kit lacks the binding another kit has; or a dashboard without a push transport never updates.
Mechanism: `inertia-live`.
Rationale: PAR-079's `usePoll` becomes the fallback; inertia-live `README.md:121`.
Status: Agreed 2026-10-21

[SIGNAL-005] Signals MUST reach pages held by other nodes through the
broadcast hub's fanout when one is configured, and the framework MUST
state the limit when none is: signals reach the publishing process's
subscribers only. Publication per topic MUST be bounded by a configured
rate, so a hot topic cannot make every subscriber reload faster than the
bound.
Falsifier: with fanout configured, a page on a second node misses a signal; without fanout the manual claims cross-node delivery; or a topic signalling faster than the bound reloads its subscribers at the unbounded rate.
Mechanism: `inertia-live`.
Status: Agreed 2026-10-21
