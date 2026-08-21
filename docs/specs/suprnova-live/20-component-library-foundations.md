# Suprnova Live -- 20 Component Library Foundations

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns the official component library's package and authoring model,
Tailwind CSS 4 integration, theme-token contract, semantic DOM and Live
directive conventions, variants and states, accessibility baseline, composition,
catalog presentation, compatibility, and verification. It depends on all core
Live browser and server contracts and feeds the component-family specs. The Live
runtime itself remains CSS-framework agnostic, and a visual Theme Studio is
excluded.

## Capabilities

### Official library and application ownership

Suprnova shall ship an official, versioned component library that application
developers can compose and customize as application markup rather than an
opaque client-widget runtime. Components shall demonstrate the intended Live
interaction contracts without being required for Live adoption.

Acceptance criteria:
- Library packages, templates, Rust helpers/controllers, assets, and peer
  compatibility are versioned with Suprnova Live.
- Applications can use individual components without loading the entire catalog.
- Component markup and behavior are inspectable, testable, and overridable
  through supported composition points.
- Runtime protocol and directives work with custom application CSS and markup.
- Library use does not introduce Inertia, Turbo, React, Vue, Svelte, Alpine, or
  another state authority.
- Reference catalogs guide coverage and semantics; third-party implementation
  code is not copied without compatible licensing and deliberate adaptation.

UX flow:
1. Application developer selects an official component -> documented markup and
   supported helpers integrate into a Live or SSR view.
2. Developer chooses custom presentation -> core Live runtime remains fully
   usable without the official styles.

### Tailwind CSS 4 styling contract

Official components shall use Tailwind CSS 4-compatible styling and build
semantics with stable library-owned composition boundaries. Tailwind version
compatibility shall be explicit rather than depending on incidental generated
class output.

Acceptance criteria:
- Supported Tailwind CSS range and required build/content configuration are
  documented and tested.
- Components expose stable semantic/variant APIs while internal utility
  composition may evolve under compatibility policy.
- Dynamic classes remain statically discoverable or explicitly safelisted
  without accepting arbitrary untrusted class injection.
- Applications can extend variants and layout without editing generated vendor
  artifacts that will be overwritten.
- Production CSS includes only required library/application utilities under the
  supported build path.
- No runtime JavaScript is required merely to generate styling.

UX flow:
1. Application developer installs/configures the official library -> Tailwind 4
   discovers component styles and produces expected CSS.
2. Unsupported Tailwind/config combination is detected -> tooling reports the
   compatibility issue before production.

### Theme-token contract

Component presentation shall derive from a documented set of semantic theme
tokens for color, typography, spacing, radius, shadow, motion, density, and
state. Tokens shall be representable in versioned CSS and JSON-compatible data
without coupling runtime behavior to a theme editor.

Acceptance criteria:
- Tokens use semantic roles rather than component-specific raw colors alone.
- Default light/dark and high-contrast behavior has sufficient contrast and
  visible focus indicators.
- Token schema has versioning, validation, fallbacks, and migration rules.
- Application overrides cascade predictably and can scope to a document or
  subtree where safe.
- Runtime/local signals may select an already-authorized theme but do not make
  theme values server-authoritative domain state.
- Visual Theme Studio functionality is not required by or included in this
  specification.

UX flow:
1. Application developer supplies valid theme tokens -> every official
   component reflects the semantic theme consistently.
2. Token is missing or invalid -> validation identifies it and safe defaults
   preserve usability.

### Component anatomy, variants, and states

Every component shall document semantic anatomy, required/optional parts,
variants, sizes, states, supported slots/content, Live/local behavior, and
composition constraints. State shall come from native HTML, server component
state, or local signals according to authority.

Acceptance criteria:
- Component APIs distinguish visual variant from behavioral/application state.
- Disabled, readonly, invalid, selected, expanded, pressed, busy, loading,
  empty, success, error, offline, and destructive states are defined where
  applicable.
- Required keys and morph preservation boundaries are documented.
- Slots/content accept semantic HTML and do not require arbitrary string HTML.
- Unsupported part/variant/state combinations fail checking or have explicit
  fallback behavior.
- Component-specific JavaScript is implemented through Live local primitives or
  supported Stimulus controllers.

UX flow:
1. Application developer composes documented parts and variants -> component
   presents and behaves consistently.
2. Contract is incomplete or contradictory -> view checking points to the
   missing/invalid anatomy.

### Accessibility and interaction baseline

Official components shall meet a defined accessibility baseline across semantic
HTML, keyboard use, focus, assistive technology, motion, contrast, zoom,
localization, and touch. ARIA shall supplement rather than replace native
semantics.

Acceptance criteria:
- Each interactive component documents roles, names, properties, keyboard map,
  focus entry/exit, and announced feedback.
- Components remain usable at supported zoom/reflow and pointer target sizes.
- Reduced motion, forced/high-contrast colors, RTL, localization expansion, and
  disabled JavaScript initial content are covered where applicable.
- Focus is not lost across Live morphs, overlay changes, validation, or
  navigation.
- Automated accessibility checks and manual keyboard/assistive-technology
  review are release requirements.
- Component examples avoid inaccessible placeholder-only labels or color-only
  state.

UX flow:
1. Application user operates an official component with keyboard or assistive
   technology -> every action and state is available and perceivable.
2. Motion or visual affordance is unavailable -> semantic state and alternative
   feedback remain complete.

### Catalog, examples, and compatibility

The library shall provide a searchable catalog showing canonical markup,
variants, states, Live behavior, SSR behavior, accessibility notes, and tests.
Compatibility shall cover Suprnova Live, Tailwind, browser, token schema, and
component markup contracts.

Acceptance criteria:
- Every component-family spec maps to catalog entries and executable examples.
- Examples include default, disabled, loading, empty, error, validation,
  localization, RTL, reduced-motion, and responsive states where relevant.
- Catalog pages are themselves canonical server-rendered documents.
- Compatibility matrix and migration notes accompany breaking anatomy or token
  changes.
- Copy/use workflows do not silently overwrite application customizations.
- Visual regression, interaction, accessibility, and Live morph tests cover
  catalog examples.

UX flow:
1. Application developer explores a component -> catalog demonstrates complete
   behavior and provides inspectable usage.
2. Upgrade changes a contract -> tooling and migration notes identify affected
   components and supported remediation.

## Acceptance criteria

- Official components are optional, inspectable, individually consumable, and
  compatible with Live's ownership model.
- Tailwind CSS 4 and theme-token contracts are versioned and verifiable.
- Anatomy, variants, states, morph identity, and JavaScript behavior are
  documented per component.
- Accessibility is a release contract across all families.
- Catalog examples are executable canonical Live/SSR demonstrations.

## Decisions and revisions

- 2026-08-21 -- Official components use Tailwind CSS 4 and semantic theme tokens;
  the Live runtime remains CSS agnostic.
- 2026-08-21 -- Theme Studio is intentionally excluded from Suprnova Live's
  current system spec while token compatibility preserves that future option.
