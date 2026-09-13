# Component library - foundations

Status: Draft
Prefix: UI
Scope: every commitment

The cross-cutting rules every component family inherits: tokens, the
base layer, state styling, asset delivery, light DOM, the qualified
engines, RenderCache classification, and explicit registration with
reserved namespaces. Live spec 20
(`crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md`)
holds the agreed capability text this file refines into requirements
with falsifiers. Nothing here is Agreed yet; the rulings the developer
still owes are listed at the end, and a requirement they block waits.

The family specs (`component-library-forms.md`, `-navigation.md`,
`-overlays.md`, `-feedback.md`, `-data-display.md`) inherit every
Agreed block here and order their own components by complexity:
presentational components (markup and tokens, no server state), then
behavioral components (real `LiveComponent`s), then custom-element
enhancements.

## The thesis every requirement serves

Navigation is real routing and the real request model: complete
server-rendered documents over plain HTTP. Live's dynamism is island-scoped
inside the page. The library makes that feel great with CSS first and small
element upgrades, never with a client application that owns the page
(developer, 2026-09-13: "no SPA for Live"; Live spec 20: no Inertia,
Turbo, React, Vue, Svelte, Alpine, or other state authority).

## Tokens, base layer, and state styling

[UI-001] The library MUST ship one versioned token stylesheet that defines
semantic custom properties for color, typography, spacing, radius, shadow,
motion, density, and interaction state, with light and dark values.
Falsifier: a token role named in Live spec 20 (color, typography, spacing,
radius, shadow, motion, density, state) has no custom property in the
shipped token stylesheet, or the stylesheet defines a role for light only.
Mechanism: `.cairn/mechanisms/ui-tokens`.

[UI-002] The library MUST ship a base layer that styles bare semantic HTML
(`button`, `input`, `select`, `textarea`, `fieldset`, `table`, `details`,
`dialog`, `a`, headings, lists) from the tokens, so a scaffolded document
renders coherently with zero components.
Falsifier: one of the listed elements has no rule in the base layer.
Mechanism: `.cairn/mechanisms/ui-tokens`.
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
Mechanism: `.cairn/mechanisms/ui-tokens`.

## Delivery, DOM, and engines

[UI-004] The framework MUST serve every library asset as a runtime feature
artifact under the same identity, caching, and integrity contract as the
runtime. Every library asset MUST carry its own row in the reviewed
artifact-size baseline.
Falsifier: a library stylesheet or script is served outside the
`__live/assets` namespace, or lands without a baseline row.
Mechanism: `.cairn/mechanisms/live-gate` (tracked artifact parity).

[UI-005] Every shipped component view MUST pass `suprnova live:check`
without `--allow-unproved`.
Falsifier: a shipped view needs the flag to pass.
Mechanism: `.cairn/mechanisms/ui-live-check`.

[UI-006] The library MUST render every shipped component's content in
light DOM. A custom-element enhancement MUST NOT attach a shadow root.
Falsifier: `attachShadow` appears in library source, or a fetched document
lacks content that a component displays.
Mechanism: `.cairn/mechanisms/ui-light-dom`; the fetchability half is
asserted by the dogfood document tests.
Reading: the developer's rulings, 2026-09-13 ("Light DOM is what I want";
"I want an llm to be able to fetch the page").

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

## Registration and namespaces

Confirmed by the developer on 2026-09-13 09:57: the library rides Live's
existing explicit registry (`LiveRegistry::builder().register::<T>()`,
immutable after `build()`, duplicate names and views rejected with a typed
error); no default set registers itself, and each namespace a default
library could share with an application's or a third party's components
is reserved.

[UI-010] An application MUST register each library behavioral component
it uses explicitly through `LiveRegistry::builder().register::<T>()`. The
library MUST NOT register any component on its own.
Falsifier: a library component is reachable through a Live route in an
application whose registry builder never named it.
Mechanism: `.cairn/mechanisms/ui-live-check` (the checker reports the
bound registry) and a source grep for `inventory::submit!` under the
library crate.

[UI-011] Every library component name MUST carry the reserved prefix
`suprnova.`. The registry MUST reject that prefix on a component from any
other crate.
Falsifier: a component outside the library registers under `suprnova.`
and the registry builds.
Mechanism: a registry unit test in the library crate.

[UI-012] Every library view MUST live under the reserved template root
`suprnova-ui/`. A library view MUST NOT shadow a path under the
application's own template roots.
Falsifier: a library template resolves at a path an application template
can also occupy.
Mechanism: `.cairn/mechanisms/ui-live-check` with both template roots
declared; a duplicate view fails registration.

[UI-013] The library stylesheet MUST declare every rule inside the cascade
layer `suprnova-ui` and every token with the `--sn-` prefix.
Falsifier: a library rule outside the layer, or a token without the
prefix, is found in the shipped stylesheet.
Mechanism: `.cairn/mechanisms/ui-tokens`.
Reading: an application's or a third party's unlayered CSS then wins over
the library's by cascade-layer order, never by specificity.

[UI-014] Every custom-element enhancement tag MUST carry the `sn-`
prefix. The element helper MUST define only the tags whose components the
application registered.
Falsifier: a document that registers no library component defines a
library custom element, or a library tag lacks the prefix.
Mechanism: `.cairn/mechanisms/ui-light-dom` and one Playwright case that
boots a document without library components and asserts no `sn-`
definition.

[UI-015] The framework MUST load library assets only when a document opts
in through `LiveBootstrapOptions`, in the same way the Stimulus role loads
only through `with_stimulus`.
Falsifier: a document that never opted in serves a library asset in its
bootstrap markup.
Mechanism: a framework test beside `framework/tests/live/assets.rs`.

## Rulings the developer still owes

Each row blocks the requirements it names from Agreed. Both sides are
cited in `docs/recon.md`, Contradicted.

1. Styling system: Live spec 20 (agreed 2026-08-21) says Tailwind CSS 4
   utilities plus theme tokens; the developer reopened this on 2026-09-13
   and linked the Tailwind v4 Play CDN. Blocks the build path behind
   UI-001 and UI-013 (a token stylesheet stands either way; whether
   component styles are utilities or token rules is the ruling).
2. Distribution: vendored macros into the application versus an in-tree
   crate re-exported behind a framework feature. Blocks UI-012's root and
   every family's presentational tier.
3. Custom-element enhancements: spec 20 says component JavaScript is Live
   local primitives or Stimulus controllers; the developer's 2026-09-13
   direction is light-DOM custom elements on an Elena-class helper. Blocks
   UI-006, UI-007, UI-014 until spec 20 carries a dated revision.
4. Family-level disagreements between the 2026-09-12 rulings and specs
   21, 22, 23, 25 (tag input, command palette, stepper, nested submenus,
   custom chart, date picker) are listed in each family file and block
   only that family's requirement.
