# Glossary

Status: Observed

This file owns the vocabulary this spec set uses. Suprnova Live has a
glossary of its own at
`crates/suprnova-live/docs/specs/suprnova-live/glossary.md`, agreed term by
term during Live's iterations; for any Live term, that file is the
authority and the entry here is a pointer with the line cited. Terms the
library work introduces are marked Draft until the developer confirms
them. The code's name for a thing wins over a synonym.

**Live island.** A region of a server-rendered document that one Live
component owns: mounted with a signed snapshot, updated by morphing after an
accepted action. Live glossary, "Live island". Avoid: widget (see below),
fragment, partial.

**Canonical document.** The complete server-rendered HTML a plain GET
returns, legible without JavaScript. Live glossary, "Canonical document".
The developer's fetchability bar (2026-09-13): an LLM or crawler fetching
the page gets all meaningful content.

**Morph.** The runtime's replacement of an island's markup with the
server's new render while preserving identity under `live:key`, focus, and
compatible local state. Live glossary, "Morph".

**Live directive.** An attribute in the closed `live:` grammar (`live:click`,
`live:submit`, `live:model`, `live:key`, `live:loading`, ...) that the
checker proves against the component. Live glossary, "Live directive";
`manual/live.md`, Views.

**Local signal.** Browser-local state declared with `live:signal` and
projected through `live:toggle`, `live:show`, `live:class`, `live:attr`,
`live:expanded` and siblings; no server round trip. Live glossary, "Local
signal".

**Component state.** Server-owned typed state of a Live component:
`#[public]` fields render and ride the signed snapshot; `#[model]` fields
also accept browser proposals. Live glossary, "Component state",
"Model-bindable field", "Transient model field".

**Browser effect.** A registered client-side behavior the reviewed runtime
performs on the server's instruction. Adding one is engine work, not
library work. Live glossary, "Browser effect".

**Runtime feature artifact.** One reviewed, integrity-checked browser
artifact served under the reserved `__live/assets` namespace; roles are
core, stimulus, uploads, and async, each in ESM and classic form
(`crates/suprnova-live/src/artifacts.rs:87-103`). Live glossary, "Runtime
feature artifact".

**Reviewed artifact-size baseline.** The recorded byte size each artifact
may not exceed without review; the Live gate's tracked-artifact parity
phase rebuilds and compares. Live glossary, "Reviewed artifact-size
baseline". Avoid: byte budget (the informal name used in the 2026-09-12
scoping notes).

**Theme token.** A versioned semantic design value (color, typography,
spacing, radius, motion, density, interaction state) that drives official
component presentation. Live glossary line 791. Avoid: raw palette value,
Tailwind utility class, component-specific hard-code.

**RenderCache, Stitch slot, Cache variance.** The document cache, the
per-principal segment stitched into a shared cached shell, and the declared
axes a cached document varies on. Live glossary, same names. A library
component documents whether its render is principal-bound (a stitch slot)
or shell bytes.

**Progressive enhancement.** The canonical document is complete before the
runtime loads; the runtime adds behavior. Live glossary, "Progressive
enhancement".

**Official component library.** The versioned set of components Suprnova
ships for Live, composable as application markup rather than an opaque
client-widget runtime; specified at capability level in Live specs 20-25.
Live spec 20, "Official library and application ownership". Avoid: kit,
design system, Flux (the developer's ruling, 2026-09-12: no reference to
that product).

**Component family.** One of the five groups Live's specs partition the
library into: form and input (21), navigation (22), overlay and disclosure
(23), feedback and status (24), data display and layout (25). Avoid:
category (the 2026-09-12 inventory's six-way grouping, which the developer
gave as examples; the spec families are the structure).

**Presentational component.** Status: Draft. A library component with no
server state: an Askama macro over markup and tokens. Avoid: Tier 1 (the
2026-09-12 scoping name; "tier" already means a RenderCache deployment tier
and a Live ledger provider tier in the Live glossary, so the scoping name
collides and is retired).

**Behavioral component.** Status: Draft. A library component that is a real
`LiveComponent` with state, actions, validation, uploads, or streams.
Avoid: Tier 2 (same collision).

**Custom-element enhancement.** Status: Draft. A light-DOM custom element
that upgrades server-rendered markup with behavior the platform does not
give natively (the combobox listbox machinery, one-time-code auto-advance,
date-strip snap selection); form-associated through `ElementInternals` so
`live:model` sees a real control. The developer's direction (2026-09-13):
light DOM only, an Elena-class helper as the base. Avoid: widget (ambiguous
with island), web component (implies shadow DOM).

**Qualified engines.** Status: Draft. The three browser engines the Live
gate pins and runs the Playwright matrix on: chromium, firefox, webkit
(`crates/suprnova-live/browser/playwright.config.ts:58-62`, Playwright
1.62.1). The developer's ruling (2026-09-13): the support matrix is these
three; a feature they disagree on waits or degrades.

**Checker.** `suprnova live:check`: proves every directive in every
registered view against its component and fails on an unknown action, an
unknown model field, a raw `safe` filter, or an accessibility violation,
reporting file, line, and column; `--allow-unproved` accepts dynamic
structures it makes no claim about (`manual/live.md`, Views and
Diagnostics; `suprnova-cli/src/commands/live_check.rs:145-147`). Avoid:
linter.
