# Component library - foundations

Status: Draft
Prefix: UI
Scope: every commitment

The cross-cutting rules every component family inherits: tokens, the
base layer, headless styling, asset delivery, light DOM, the qualified
engines, RenderCache classification, explicit registration with reserved
namespaces, and the component unit `live:add` installs. Live spec 20
(`crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md`)
holds the agreed capability text this file refines into requirements
with falsifiers; its "Decisions and revisions" section carries the four
dated entries of 2026-09-13 that record the developer's rulings below.
Nothing here is Agreed yet: the developer confirms the requirement texts
and falsifiers as one set.

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

## The rulings this file rests on (developer, 2026-09-13)

- Styling (10:22): token-driven rules under the `suprnova-ui` cascade
  layer; Tailwind CSS 4 supported through a `@theme` preset, never
  required in component markup.
- Distribution (10:23): behavioral components' Rust in the in-tree crate;
  views, macros, per-component CSS and JavaScript vendored by `live:add`
  from a JSON manifest.
- Custom elements (10:36): the enhancement tier is light-DOM,
  form-associated custom elements, each defined by its component's own
  vendored JavaScript on a small reviewed helper.
- Headless (10:36-10:37): a component as built is structure, behavior,
  and state; every visual value comes from a token; no inline styles,
  tokens only; the skin ships on and is removable with nothing breaking.
- Registration (09:57): explicit, through the existing registry;
  reserved names, template root, cascade layer, tags, and asset role.

## Tokens, base layer, and headless styling

[UI-001] The library MUST ship one versioned token stylesheet that defines
semantic custom properties, prefixed `--sn-`, for color, typography,
spacing, radius, shadow, motion, density, and interaction state, with
light and dark values.
Falsifier: a token role named in Live spec 20 has no `--sn-` custom
property in the shipped token stylesheet, or the stylesheet defines a
role for light only.
Mechanism: `.cairn/mechanisms/ui-tokens`.

[UI-002] The library MUST ship a base layer, inside the `suprnova-ui`
cascade layer, that styles bare semantic HTML (`button`, `input`,
`select`, `textarea`, `fieldset`, `table`, `details`, `dialog`, `a`,
headings, lists) and every shipped component from the tokens, so a
scaffolded document renders coherently with zero components.
Falsifier: one of the listed elements has no rule in the base layer, or a
base-layer rule sits outside the `suprnova-ui` cascade layer.
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

[UI-004] A component stylesheet MUST take every visual value (color, font
family, radius, shadow, duration, easing) from a `--sn-` token. A
component stylesheet MUST NOT contain a literal visual value.
Falsifier: a hex or named color, a font-family name, a length on
`border-radius`, a `box-shadow` value, or a duration appears in a
component stylesheet outside a `var(--sn-...)` reference.
Mechanism: `.cairn/mechanisms/ui-tokens`.
Reading: structural rules (display, position, grid, overflow, scroll
snap) may be literal; they are behavior, not appearance.

[UI-005] A shipped view MUST NOT carry a `style` attribute.
Falsifier: `style=` appears in a shipped view.
Mechanism: `.cairn/mechanisms/ui-tokens` (a grep over the vendored views)
and `.cairn/mechanisms/ui-live-check`.

[UI-006] Every shipped component MUST keep its behavior, semantics, and
state with the base layer removed.
Falsifier: with the `suprnova-ui` layer absent, a component loses a
behavior, an accessible name, or a state attribute.
Mechanism: one browserless harness run with the base layer stripped.
Reading: this is what "headless by default" proves; the skin ships on.

[UI-007] The library MUST ship a Tailwind CSS 4 `@theme` preset that maps
Tailwind's theme namespaces to the `--sn-` tokens, documented and tested
against a pinned Tailwind range.
Falsifier: a `--sn-` color, spacing, radius, or font token has no
counterpart in the preset, or the preset fails against the pinned range.
Mechanism: a node check over the preset against the token stylesheet
(`.cairn/mechanisms/ui-tokens`, second input).

## Delivery, DOM, and engines

[UI-008] The framework MUST serve the shared library bases - the token
stylesheet, the base layer, and the element helper - as runtime feature
artifacts under the runtime's identity, caching, and integrity contract.
Each shared base MUST carry its own row in the reviewed artifact-size
baseline.
Falsifier: a shared base is served outside the `__live/assets` namespace,
or lands without a baseline row.
Mechanism: `.cairn/mechanisms/live-gate` (tracked artifact parity).
Reading: per-component CSS and JavaScript are vendored into the
application (UI-017) and served by the application; the artifact
contract binds only what Suprnova ships.

[UI-009] Every shipped component view MUST pass `suprnova live:check`
without `--allow-unproved`.
Falsifier: a shipped view needs the flag to pass.
Mechanism: `.cairn/mechanisms/ui-live-check`.

[UI-010] The library MUST render every shipped component's content in
light DOM. A custom-element enhancement MUST NOT attach a shadow root.
Falsifier: `attachShadow` appears in library source, or a fetched document
lacks content that a component displays.
Mechanism: `.cairn/mechanisms/ui-light-dom`; the fetchability half is
asserted by the dogfood document tests.

[UI-011] A custom-element enhancement that stands in for a form control
MUST be form-associated through `ElementInternals` so `live:model` and
validation see a real control.
Falsifier: a library custom element that carries a value is not
form-associated and the surrounding form submits without it.
Mechanism: the browserless component harness
(`crates/suprnova-live/docs/implementation/component-harness.md`) plus one
Playwright case per enhancement.

[UI-012] The library MUST target the qualified engines only. A component
that uses a platform feature the three engines disagree on MUST degrade
on the engine that lacks it, as the Playwright matrix decides.
Falsifier: a component relies on a feature one qualified engine lacks and
no Playwright case exercises that component on that engine.
Mechanism: `.cairn/mechanisms/live-gate`, with one Playwright case per
shipped component that uses a platform feature beyond plain HTML and CSS.

[UI-013] Every behavioral component MUST document its RenderCache
classification (shell bytes, stitch slot, or varies). A behavioral
component MUST mount as one island per widget, never one island per row
or cell.
Falsifier: a shipped table or list component mounts an island per item.
Mechanism: review against the dogfood stitched-dashboard test
(`app/tests/`); a grep for per-item mounts in library views.

## Registration and namespaces

[UI-014] An application MUST register each library behavioral component
it uses explicitly through `LiveRegistry::builder().register::<T>()`. The
library MUST NOT register any component on its own.
Falsifier: a library component is reachable through a Live route in an
application whose registry builder never named it.
Mechanism: `.cairn/mechanisms/ui-live-check` (the checker reports the
bound registry) and a source grep for `inventory::submit!` under the
library crate.

[UI-015] Every library component name MUST carry the reserved prefix
`suprnova.`. The registry MUST reject that prefix on a component from any
other crate.
Falsifier: a component outside the library registers under `suprnova.`
and the registry builds.
Mechanism: a registry unit test in the library crate.

[UI-016] Every library view MUST live under the reserved template root
`suprnova-ui/`. A library view MUST NOT shadow a path under the
application's own template roots.
Falsifier: a library template resolves at a path an application template
can also occupy.
Mechanism: `.cairn/mechanisms/ui-live-check` with both template roots
declared; a duplicate view fails registration.

[UI-017] A library component MUST be one directory under the reserved
template root holding its view, its stylesheet when it has one, and its
JavaScript when it has one, described by one JSON manifest that names
those files. The `live:add` command MUST install a component from its
manifest without overwriting a file the application has edited. The
`live:add` command MUST accept a third-party manifest in the same format.
Falsifier: a shipped component's files are scattered across roots, a
manifest omits a file the component needs, or `live:add` overwrites an
edited file without the developer asking for it.
Mechanism: a CLI test in `suprnova-cli/tests/` that installs a component
twice, edits it between runs, and asserts the edit survives.

[UI-018] Every custom-element enhancement tag MUST carry the `sn-`
prefix. A library custom element MUST be defined only by its own
component's vendored JavaScript, so a document that never added the
component never defines the tag.
Falsifier: a library-free document defines an `sn-` element, or a library
tag lacks the prefix.
Mechanism: `.cairn/mechanisms/ui-light-dom` and one Playwright case that
boots a library-free document and asserts no `sn-` definition.

[UI-019] The framework MUST load the shared library bases only when a
document opts in through `LiveBootstrapOptions`, in the same way the
Stimulus role loads only through `with_stimulus`.
Falsifier: a document that never opted in serves a library base in its
bootstrap markup.
Mechanism: a framework test beside `framework/tests/live/assets.rs`.
