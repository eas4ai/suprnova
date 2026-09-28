# The browser admits what the protocol admits

Slug: live-protocol-bounds
Requirements: LIVE-028, LIVE-029, LIVE-030
Rests on: LIVE-027, LIVE-025

## Goal

A Live form submits whatever the framework's server accepts: the browser's
protocol validators admit the counts the framework's protocol limits set,
`live:check` refuses a form a single request cannot carry, and a request
refused for size fails visibly as a resource limit. Promoted from the
backlog on the developer's `ok` to escalation `live-027`, 2026-09-16.

## Deliverables

- The browser validators admit 128 model proposals, operations, arguments,
  validation entries, events, effects, and extensions per message, one
  named constant each, with vitest cases at 128 and 129.
- The checker counts the distinct model fields under a `live:submit` form
  and fails past 127, with a regression.
- The island transport reports a request refused for a protocol bound as
  `resource_limit` / `resource_exhausted` and finishes the action
  rejected, with a vitest case.
- The dogfood form gallery's save form holds its model controls again,
  proving a submit past eight fields in the browser.
- Live spec 06 records the bounds; `manual/live.md` states the per-request
  limit with the six mirrors re-stamped; the 2.0.2 changelog records the fix
  in seven locales.

## Records

- Requirements: `docs/spec/live.md` (LIVE-028 to LIVE-030).
- Mechanism: `.cairn/mechanisms/live-protocol-bounds`.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-protocol-bounds.md`.

## Done when

The mechanism passes on a committed tree, the Live gate passes with the
form gallery's full save form, the review is recorded, and `cairn wake`
says Done.
