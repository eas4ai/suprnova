# Batched island refresh -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-08-25
Target domain: `11-interaction-scheduling-and-feedback.md`

## What it is

The Live browser runtime may coalesce compatible registered fresh-render intents
from multiple islands into one bounded ordinary HTTP request when the server
advertises a matching batch capability. The request and response remain a
collection of independently authorized island operations and outcomes; batching
does not create document-wide component state, shared snapshot authority, or an
all-or-nothing cross-island commit.

The batch protocol shall be an explicit extension of the ordinary Live HTTP
transport. It shall not use the SSE/WebSocket document transport, which remains
server-to-browser typed event and continuity augmentation and never carries
trusted HTML, snapshots, actions, or a generic bidirectional RPC protocol.

## Acceptance criteria

- Only registered authoritative fresh-render intents are eligible by default.
  User actions, submits, model flushes, uploads, arbitrary methods, effects, and
  domain mutations are not silently converted into a batch operation.
- The runtime coalesces only intents from the same document, compatible endpoint
  and protocol version, current principal/session/tenant authorization scope,
  and bounded scheduling window. A microtask or same-tick opportunity cannot add
  unbounded latency to a visible refresh.
- Batch capability, maximum items, maximum aggregate request/response bytes,
  compatibility window, and fallback behavior are negotiated explicitly. A
  peer without support receives ordinary independent fresh-render requests.
- Every item carries its own island identity, signed snapshot or required fresh
  authority, correlation, base revision, and registered operation. The server
  verifies and authorizes every item independently before rendering it.
- Every result is a typed per-island outcome. One item's denial, expiration,
  stale state, render failure, or recovery requirement does not manufacture
  success for another item and does not require discarding otherwise valid
  independent results.
- Batching promises transport coalescing, not transactional domain atomicity,
  exactly-once execution, shared ledger revision, shared snapshot commit, or
  simultaneous DOM application.
- Each result returns to its owning island scheduler. That scheduler performs
  the existing response validation, morph preflight, commit-after-morph,
  feedback, child delivery, event/effect, and recovery ordering independently.
- Island removal, navigation, cancellation, or supersession before dispatch can
  remove an unsent item. After transmission, cancellation prevents inappropriate
  browser application but does not claim server work was rolled back.
- The document work arbiter, if promoted, decides when the aggregate request is
  admitted and charges it by bounded item/byte cost rather than pretending it is
  one free unit.
- Server implementation may share request parsing, registry lookup, database
  reads, or rendering setup where safe, but no optimization may weaken per-item
  authorization, dependency collection, revision authority, or diagnostics.
- Response parsing is exact-key and bounded before allocation; malformed,
  duplicated, unknown, cross-island, and oversized items fail with typed
  per-batch or per-item dispositions that cannot misroute another island's HTML
  or snapshot.
- Fixtures and adversarial tests cover mixed success, response reordering,
  duplicate item identities, stale revisions, cancellation, partial network
  failure, rolling-version fallback, and independent commit-after-morph.
- Benchmarks must demonstrate a material reduction in request overhead for
  simultaneous multi-island refresh without regressing single-island latency or
  violating document queue and retained-memory budgets.

## Touches

- Primary owner: `11-interaction-scheduling-and-feedback.md`.
- HTTP wire shape, correlation, rolling compatibility, typed outcomes, and
  failure semantics: `06-wire-protocol-and-transport.md`.
- Fresh-render authorization, snapshots, and revision authority: specs 04, 05,
  and 07.
- Push invalidation remains only the trigger; `14-events-and-asynchronous-updates.md`
  must continue to forbid HTML, snapshots, and Live actions on SSE/WebSocket.
- Runtime capability negotiation and lifecycle: `09-runtime-bootstrap-and-directives.md`.
- Fixtures, generated contracts, hostile tests, and performance evidence:
  `19-developer-tooling-and-testing.md` and `00-overview.md`.
- Related staged dependency: `document-work-arbiter.md` may own aggregate
  admission, but batch correctness must not depend on that optimization.
- Current conflict avoided: Task 4 multiplexing routes independent server-pushed
  envelopes by subscription ID; it is not the carrier for browser-originated
  island refresh requests.

## Why not now

This requires a new bounded ordinary-HTTP batch protocol and cross-island
response semantics, neither of which is part of Iteration 004's independently
versioned asynchronous transport contract.
