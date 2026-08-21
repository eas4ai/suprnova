# Suprnova Live -- 25 Data Display and Layout Components

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns official structural/layout primitives and components for
cards, lists, tables, data grids, descriptions, statistics, badges, avatars,
media, and related server-rendered data presentation. It depends on component
foundations, canonical views, URL/state binding, morph identity, navigation,
and feedback components. It does not create a client-side data model or charting
engine.

## Capabilities

### Layout and structural primitives

Container, stack, cluster, grid, split, sidebar, section, separator, scroll-area,
aspect, and responsive visibility primitives shall compose semantic application
layouts using theme tokens and Tailwind 4 without obscuring document order.

Acceptance criteria:
- Visual reflow preserves logical DOM and reading/focus order.
- Responsive behavior, min/max sizing, overflow, container queries where used,
  and print behavior are documented.
- Components accept semantic elements/landmarks rather than emitting anonymous
  wrapper depth by default.
- Visibility utilities do not hide essential content from one modality alone.
- Scroll areas retain keyboard access and visible focus without trapping.
- Layout markup remains stable enough for keyed Live regions and transitions.

UX flow:
1. Application developer composes structural primitives -> content adapts across
   supported viewports while retaining semantic order.
2. Content grows, localizes, or zooms -> layout reflows without clipping required
   actions or information.

### Cards, descriptions, and statistics

Card, description-list, key/value, statistic, and summary components shall group
related server-rendered information and actions without turning every visual box
into an incorrect landmark or interactive surface.

Acceptance criteria:
- Heading hierarchy, article/section/aside semantics, and clickable-card behavior
  are explicit.
- A card containing multiple actions does not use one invalid nested link over
  the entire surface.
- Description terms/values retain semantic association.
- Statistics include labels, units, timeframes, trends, and non-color direction
  cues where applicable.
- Loading/empty/error states use feedback components with stable region identity.
- Updates can announce material changes without reannouncing the full card.

UX flow:
1. Application user reviews grouped data -> labels, values, hierarchy, and
   actions are clear in SSR and after Live updates.
2. Data changes -> targeted morph updates the stable region and exposes material
   change appropriately.

### Lists, feeds, and collections

List, divided-list, media-list, activity/feed, timeline, and repeated collection
components shall render semantic keyed items with empty/loading/pagination or
streaming integration. Reordering shall retain logical identity.

Acceptance criteria:
- Ordered, unordered, descriptive, feed, and timeline semantics match content.
- Repeated items require stable domain keys when state/focus can persist.
- Insert, remove, reorder, prepend/append, and stream operations have morph
  fixtures.
- Infinite/streamed loading offers accessible position, status, and an
  alternative explicit load/navigation control.
- Collection-level and item-level actions have distinct targets and feedback.
- Empty/no-results/error states retain one stable collection boundary.

UX flow:
1. Application user reads or updates a collection -> items maintain correct
   order, identity, and action ownership.
2. New/removed/reordered items arrive -> morph changes only affected keyed
   content and preserves focus/local state where valid.

### Tables and responsive tabular data

Table components shall use native table semantics for relational data and
provide responsive alternatives that preserve headers and cell relationships.
Sorting, filtering, selection, pagination, and actions shall integrate with real
URL or Live state explicitly.

Acceptance criteria:
- Caption, column/row headers, scope/associations, density, alignment, and
  numeric formatting are supported.
- Responsive presentation does not destroy header/value relationships or hide
  required data/actions silently.
- Sort/filter/page controls expose current state, direction, labels, and real
  shareable URLs when intended.
- Row selection uses stable keys, typed model state, keyboard access, select-all
  scope, and clear selected-count semantics.
- Row/cell actions are real links or buttons and do not make the entire complex
  row an invalid nested control.
- Loading, empty, error, stale, and partial-update states preserve table
  structure and focus.

UX flow:
1. Application user sorts/filters/pages/selects -> chosen route or Live mode
   updates current state and targeted table feedback.
2. Server returns rows -> keyed morph preserves compatible selection/focus and
   announces result-count/material changes.

### Interactive data grid

When an application needs spreadsheet-like keyboard navigation or inline
editing beyond a semantic table, the library may provide a separate data-grid
pattern with explicit complexity, virtualization, selection, editing,
validation, and server-authority contracts.

Acceptance criteria:
- Grid is not the default table component and documents its accessibility and
  performance tradeoffs.
- Row/column/cell identity, keyboard map, active cell, range selection, edit
  mode, and focus model are explicit.
- Virtualization preserves logical counts, positions, labels, and stable keys.
- Inline edits use model/action validation and never become durable solely in
  client grid state.
- Concurrent/stale edits expose conflict recovery rather than silently
  overwriting domain state.
- A simpler table/list alternative is documented where grid behavior is not
  necessary.

UX flow:
1. Application user navigates/edits an eligible grid -> focus and selection move
   predictably and edits enter truthful dirty/validation states.
2. Server accepts/rejects/conflicts -> cells reconcile to authoritative data or
   offer correction/refresh without losing unrelated grid context.

### Badges, avatars, icons, and compact metadata

Badge/status pill, avatar, icon, keyboard-key, and compact metadata components
shall supplement textual meaning without using color, imagery, or iconography
as the only label.

Acceptance criteria:
- Status variants include accessible text or programmatic labels.
- Avatar images have appropriate alternative text; decorative/fallback initials
  follow privacy and localization rules.
- Icons declare decorative versus informative use and do not duplicate labels.
- Compact metadata remains readable at zoom/high contrast and wraps safely.
- Dynamic status updates announce only when material.
- Untrusted image/media URLs use safe loading and privacy policy.

UX flow:
1. Application user encounters compact data -> text and semantics convey the
   meaning available visually.
2. Image/icon fails or style is unavailable -> fallback content preserves
   identity and status.

### Media, code, and visualization containers

Media, figure, code block, prose, and chart-container components shall provide
responsive presentation, captions, controls, loading/error states, and safe
third-party integration boundaries. The library shall not claim to implement a
general charting or media-processing engine.

Acceptance criteria:
- Figures/media have captions/transcripts/alternatives according to content.
- Native media controls and keyboard access remain available where applicable.
- Code blocks preserve language, copy behavior, wrapping/scrolling, and safe
  escaped content.
- Chart containers require textual summary/data access and label third-party
  controller ownership.
- Live ignores/preserves complex widgets through explicit morph boundaries and
  updates data through registered APIs.
- Lazy media respects performance, privacy, canonical-content, and reduced-data
  concerns.

UX flow:
1. Application user loads rich data/media -> meaningful server-rendered
   summary/fallback appears before optional controller enhancement.
2. Widget updates or fails -> owned container presents current data/error without
   allowing third-party state to become server authority.

## Acceptance criteria

- Layout preserves semantic order, reflow, localization, zoom, and stable Live
  boundaries.
- Data-display components expose labels, relationships, identity, and state
  without color/image dependence.
- Lists and tables support keyed updates, URL/Live controls, and complete
  loading/empty/error behavior.
- Complex grids are explicit opt-in patterns with server-authoritative edits.
- Rich third-party visualization remains behind safe controller and morph
  boundaries.

## Decisions and revisions

- 2026-08-21 -- Semantic table/list/layout primitives are the default; complex
  client-like data grids require a separate explicit pattern and justification.
- 2026-08-21 -- The library supplies visualization containers and integration
  contracts, not a new charting runtime.
