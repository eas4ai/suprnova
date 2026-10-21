# Morphing and interaction continuity

## Morph preflight

Live owns the complete preflight before any DOM mutation. It bounded-parses the
server HTML, requires exactly one compatible island root, verifies root
authority and replacement metadata, scans identity/morph-control/teleport
limits, and builds a validated plan. Failure leaves the current DOM and current
snapshot metadata untouched.

The private adapter is pinned to Idiomorph 0.7.4. It receives only an opaque
validated plan and is not exported through the public runtime API. No server
fragment, directive, or application option selects a morph implementation or
passes executable callbacks into it.

After preflight, continuity is captured, optional Stimulus state is bracketed,
the DOM is morphed, the resulting root is validated, and continuity is restored.
Only then may accepted successor snapshot/revision metadata become current. A
throw after server acceptance enters fresh-render recovery rather than replaying
the action.

## Identity and controls

Stable `live:key` values, compatible element ids, island roots, and explicit
morph controls form a closed identity plan. Duplicate, oversized, missing,
drifted, or ambiguous identities reject preflight. Nested islands retain their
own scheduler/state ownership even when a parent morph surrounds them; signed
child-parameter work enters the child scheduler after the parent succeeds.

Replace, preserve, children-only, and teleport controls are validated against
both current and replacement trees. Preserved nodes retain the current DOM
without granting their descendants snapshot authority. Active teleports require
the same keyed target contract and bounded external identity. Controlled moves
are distinguished from removals so lifecycle disposal happens exactly once.

Signal scopes and controller roots survive only when their keyed identity and
contract remain compatible. Otherwise they retire and rebuild from the new
server HTML. No hidden global component registry or client virtual DOM owns
identity.

## Focus, forms, and IME

Before morphing, Live records a bounded continuity description for active
focus, selection, form control state, scroll, signal scopes, and controller
roots. Restoration prefers stable keyed identity, validates element/control
compatibility, and falls back to deterministic focus policy when the old target
no longer exists.

Locally newer text, checked state, selected options, selection ranges, and
dirty fields are protected from an older response. Server-authoritative values
still apply when no newer edit exists. Composition events defer model scheduling
and restoration so an IME sequence is never split or replaced mid-composition.
Passwords, files, secrets, and browser-protected values are never copied into
diagnostics or serialized continuity records.

Focus and scroll restoration honors explicit focus intent, autofocus rules,
reduced motion, validation targets, and native navigation boundaries. All
continuity records are short-lived and are cleared on completion, failure,
suspension, or disposal.
