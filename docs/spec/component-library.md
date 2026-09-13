# Live component library

Status: Draft
Prefix: UI

The official component library for Suprnova Live. Live's specs 20-25 hold
the agreed capability text (foundations, form and input, navigation,
overlay and disclosure, feedback and status, data display and layout); this
file turns the first commitment's slice of that text into requirements with
falsifiers, in the developer's own rulings from 2026-09-12 and 2026-09-13.
Nothing here is Agreed yet. Where a 2026-09-12 ruling and a spec 20-25
sentence disagree, the disagreement is listed under "Rulings the developer
still owes" and the requirement waits.

Reading this file: a requirement names the library (the code that ships
with Suprnova), the checker (`suprnova live:check`), or the Live gate as
its actor.

## The thesis every requirement serves

Navigation is real routing and the real request model: complete
server-rendered documents over plain HTTP. Live's dynamism is island-scoped
inside the page. The library makes that feel great with CSS first and small
element upgrades, never with a client application that owns the page
(developer, 2026-09-13 00:19: "no SPA for Live"; Live spec 20: no Inertia,
Turbo, React, Vue, Svelte, Alpine, or other state authority).

## Foundations

[UI-001] The library MUST ship one versioned token stylesheet that defines
semantic custom properties for color, typography, spacing, radius, shadow,
motion, density, and interaction state, with light and dark values.
Falsifier: a token role named in Live spec 20 (color, typography, spacing,
radius, shadow, motion, density, state) has no custom property in the
shipped token stylesheet, or the stylesheet defines a role for light only.
Mechanism: a token-schema check over the stylesheet (to be built under the
first commitment; declared as `.cairn/mechanisms/ui-tokens`).

[UI-002] The library MUST ship a base layer that styles bare semantic HTML
(`button`, `input`, `select`, `textarea`, `fieldset`, `table`, `details`,
`dialog`, `a`, headings, lists) from the tokens, so a scaffolded document
renders coherently with zero components.
Falsifier: one of the listed elements has no rule in the base layer.
Mechanism: the same stylesheet check (`.cairn/mechanisms/ui-tokens`).
Reading: the developer named Pico CSS (2026-09-12) as the reference to
expand on; its ~10KB footprint is the baseline-size bar.

[UI-003] Every component stylesheet MUST select state styling from the
accessibility or native state attribute the checker already proves
(`aria-invalid`, `aria-busy`, `aria-expanded`, `aria-pressed`,
`aria-current`, `aria-selected`, `:disabled`, `:invalid`). A component
stylesheet MUST NOT select state styling from a visual-only class.
Falsifier: a component's invalid, busy, expanded, pressed, current, or
selected presentation is selected by a class with no corresponding
attribute selector.
Mechanism: the stylesheet check (`.cairn/mechanisms/ui-tokens`).
Reading: this is how a component cannot look correct while being
inaccessible.

[UI-004] The framework MUST serve every library asset as a runtime feature
artifact under the same identity, caching, and integrity contract as the
runtime. Every library asset MUST carry its own row in the reviewed
artifact-size baseline.
Falsifier: a library stylesheet or script is served outside the
`__live/assets` namespace, or lands without a baseline row.
Mechanism: the Live gate's tracked artifact parity phase
(`.cairn/mechanisms/live-gate`).

[UI-005] Every shipped component view MUST pass `suprnova live:check`
without `--allow-unproved`.
Falsifier: a shipped view needs the flag to pass.
Mechanism: `.cairn/mechanisms/ui-live-check`, run against the dogfood
application with every library component mounted.

[UI-006] The library MUST render every shipped component's content in
light DOM. A custom-element enhancement MUST NOT attach a shadow root.
Falsifier: `attachShadow` appears in library source, or a fetched document
lacks content that a component displays.
Mechanism: a source grep in `.cairn/mechanisms/ui-light-dom`; the
fetchability half is asserted by the dogfood document tests.
Reading: the developer's ruling, 2026-09-13 00:17 ("Light DOM is what I
want") and 00:16 ("I want an llm to be able to fetch the page").

[UI-007] A custom-element enhancement that stands in for a form control
MUST be form-associated through `ElementInternals` so `live:model` and
validation see a real control.
Falsifier: a library custom element that carries a value is not
form-associated and the surrounding form submits without it.
Mechanism: the browserless component harness
(`crates/suprnova-live/docs/implementation/component-harness.md`) plus one
Playwright case per enhancement.

[UI-008] The library MUST target the qualified engines only. A component
that uses a platform feature the three engines disagree on MUST degrade
on the engine that lacks it, as the Playwright matrix decides.
Falsifier: a component relies on a feature one qualified engine lacks and
no Playwright case exercises that component on that engine.
Mechanism: `.cairn/mechanisms/live-gate`, with one Playwright case per
shipped component that uses a platform feature beyond plain HTML and CSS.

[UI-009] Every behavioral component MUST document its RenderCache
classification (shell bytes, stitch slot, or varies). A behavioral
component MUST mount as one island per widget, never one island per row
or cell.
Falsifier: a shipped table or list component mounts an island per item.
Mechanism: review against the dogfood stitched-dashboard test
(`app/tests/`); a grep for per-item mounts in library views.

## Form and input family (first commitment)

[UI-010] The library MUST ship presentational components for field (label,
control, hint, error), input, textarea, checkbox and checkbox group, radio
group, switch, select, label, button and link-button, button group,
fieldset, form actions bar, and validation summary, each built on native
controls and the `live:model`, `live:error`, and `live:loading` vocabulary.
Falsifier: one listed component is missing from the shipped set, or one
replaces a native control with a scripted stand-in.
Mechanism: `.cairn/mechanisms/ui-live-check` over a dogfood view that
mounts every listed component.

[UI-011] Password and one-time-code controls MUST bind through transient
model fields. The library MUST NOT dehydrate a password or one-time-code
value into a snapshot.
Falsifier: a rendered snapshot contains a password or one-time-code value.
Mechanism: the framework's Live snapshot tests
(`framework/tests/live/document_routes.rs`) extended with a library case.
Reading: Live spec 21, Decisions and revisions, 2026-08-21.

## Rulings the developer still owes

Each row blocks the requirements it names from Agreed. Both sides are
cited in `docs/recon.md`, Contradicted.

1. Styling system: Live spec 20 (agreed 2026-08-21) says Tailwind CSS 4
   utilities plus theme tokens; the developer reopened this on 2026-09-13
   and linked the Tailwind v4 Play CDN. Blocks UI-001's build path (a
   token stylesheet stands either way; whether component styles are
   utilities or token rules is the ruling).
2. Distribution: vendored macros into the application versus an in-tree
   crate re-exported behind a framework feature; the 2026-09-12 report
   recommended vendored presentational components and an in-tree crate
   for behavioral ones. Blocks UI-010's shape.
3. Naming: the reserved component prefix and the CLI verb. Blocks UI-010.
4. Tag input, command palette, stepper, nested submenus, custom chart:
   specs 21, 22, 23, 25 include them; the 2026-09-12 rulings put them out
   of the built-in set or change their shape. Not in the first commitment;
   each needs a dated revision in its spec or a withdrawal of the ruling
   before its family's commitment.
5. Date picker: the developer's horizontal scroll-snap design versus spec
   21's "prefer native where adequate". Not in the first commitment.
6. Custom-element enhancements: spec 20 says component JavaScript is Live
   local primitives or Stimulus controllers; the developer's 2026-09-13
   direction is light-DOM custom elements on an Elena-class helper. Blocks
   UI-006 and UI-007 until spec 20 carries the revision.
