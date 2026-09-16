# Live key vocabulary

Slug: live-key-vocabulary
Requirements: LIVE-024
Rests on: OVL-006

## Goal

A keyed morph control written the way the checker requires works at
runtime: `live:key` is the one stable-key attribute a template writes, the
runtime reads it for morph identity, controls and preservation scopes,
and the library's components stop writing the key twice. Promoted from the
backlog on the developer's `ok` to escalation `ovl-006`, 2026-09-15.

## Deliverables

- The browser runtime resolves the stable key from `live:key` in
  `morph/keys.ts`, `morph/controls.ts` and `morph/preserve.ts`, keeps
  `data-suprnova-live-key` as the engine's spelling on the roots it
  renders, and fails morph validation when both are present and disagree.
- A vitest fixture proving a `live:key`-only control keeps its identity and
  its preserved state across a compatible morph, and that a disagreeing
  pair is refused; a checker regression that a `live:key` template still
  passes.
- Every library component and its vendored copy under
  `app/templates/suprnova-ui/`, and the dogfood galleries, write `live:key`
  alone; a dogfood browser case proves a keyed disclosure from a template
  stays open across an action's morph.
- The form gallery's `live:submit="save"` gains `.prevent` and one browser
  case drives it, the one-attribute fix the escalation named to ride along.
- Live spec 12 names `live:key` as the key attribute and records the
  revision; spec 09 lists it with the directive set; `manual/live.md`
  states it with the six mirrors re-stamped under the translation lock; the
  2.0.2 changelog records the fix in seven locales.

## Records

- Requirement: `docs/spec/live.md` (LIVE-024).
- Mechanism: `.cairn/mechanisms/live-key-vocabulary`, declared with this
  commitment, running the vitest fixture and the checker regression.
- Evidence: `.cairn/evidence/`, committed after each `cairn check`.
- Review: `.cairn/reviews/live-key-vocabulary.md`.

## Done when

The mechanism passes on a committed tree, the Live gate passes with the
doubled attributes gone, the review is recorded, and `cairn wake` says
Done.
