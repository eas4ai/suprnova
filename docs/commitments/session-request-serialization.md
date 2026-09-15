# Session request serialization

Slug: session-request-serialization
Requirements: SESS-001
Rests on: none

## Goal

Suprnova offers session blocking: an application enables per-session
request serialization for a route group or globally, and the session
middleware holds a cache lock for the session from load to write, with a
bounded wait and hold, so concurrent requests on one session never lose a
mutation to a last-writer-wins write. Drafted 2026-09-15 from the
live-native review's open finding, on the developer's `ok` to escalation
`form-008`.

## Deliverables

- A `block` option on the session configuration and a route-level
  middleware parameter, both off by default, with a bounded acquire wait
  and lock hold, backed by `Cache::lock`.
- The session middleware acquires the lock before reading the session and
  releases it after the write, on every exit path.
- A framework integration test under `framework/tests/session/` that
  handles two requests on one session concurrently with blocking on and
  proves the flash survives, and one with blocking off that documents the
  race; the dogfood flash case enables blocking on the feedback routes.
- Manual: `manual/session.md` gains the section; the six mirrors are
  re-stamped under the translation lock; the 2.0.2 changelog gains the
  entry.

## Records

- Requirements: `docs/spec/sessions.md` (SESS-001).
- Mechanism: `.cairn/mechanisms/session-blocking`, declared with this
  commitment, running the framework session tests.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/session-request-serialization.md`.

## Done when

The mechanism passes on a committed tree, the review is recorded, and
`cairn wake` says Done.
