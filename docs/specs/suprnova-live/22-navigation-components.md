# Suprnova Live -- 22 Navigation Components

Status: Normative design specification
Last revised: 2026-08-21

## Scope

This domain owns official components for links, application/site navigation,
breadcrumbs, tabs, pagination, steppers, and command/navigation discovery. It
depends on component foundations, canonical routes, URL-bound state, document
navigation, and local reactivity, and it interoperates with the later overlay
component domain. It presents route and section movement without introducing
client-router semantics.

## Capabilities

### Links and navigation actions

The library shall distinguish links that navigate, buttons that perform actions,
downloads, external destinations, and disabled/unavailable choices through
native semantic elements and consistent styling.

Acceptance criteria:
- Internal navigation uses anchors with real route URLs and supports
  open-in-new-tab, copy-link, browser status, and assistive-technology semantics.
- Action controls use buttons and never masquerade as links solely for style.
- Current-page, visited where appropriate, external, download, disabled, and
  loading states are distinguishable.
- Optional prefetch and transition attributes preserve safe normal navigation.
- Unsafe URL schemes and untrusted target attributes are rejected.
- Icon and truncated links retain accessible destination names.

UX flow:
1. Application user activates a navigation link -> browser performs normal
   document navigation to the visible destination.
2. Runtime enhancement is unavailable -> the same anchor remains fully usable.

### Primary, secondary, and responsive navigation

Navigation bar, sidebar, rail, footer, and local subnavigation patterns shall
communicate hierarchy and current location across responsive layouts. Collapsed
presentation may use local signals but shall not change route semantics.

Acceptance criteria:
- Navigation landmarks and labels distinguish multiple navigation regions.
- Current route/section state comes from server/route authority.
- Responsive collapse uses accessible disclosure controls with visible focus and
  escape/outside behavior where applicable.
- Keyboard order follows visual/logical hierarchy and does not trap focus.
- Permission-hidden and temporarily disabled destinations remain distinct.
- Morphs preserve open local navigation state only under stable keyed scope.

UX flow:
1. Application user opens a compact navigation menu -> local disclosure responds
   immediately and exposes available route links.
2. Application user selects a destination -> normal navigation occurs and new document
   renders its authoritative current state.

### Breadcrumbs

Breadcrumb components shall express the hierarchical path to the current
document using real links and one non-linked current page item.

Acceptance criteria:
- Breadcrumb navigation has an accessible label and ordered semantic structure.
- Intermediate items use canonical URLs and preserve normal link behavior.
- Current item exposes `aria-current` and is not a redundant self-navigation by
  default.
- Responsive truncation preserves hierarchy meaning and accessible names.
- Dynamic route labels are server rendered and safely escaped.
- Structured metadata, if offered, corresponds to visible breadcrumbs.

UX flow:
1. Application user reviews breadcrumbs -> current location and ancestors are
   clear.
2. Application user chooses an ancestor -> browser navigates to its canonical route.

### Tabs and section navigation

Tabs shall distinguish local in-document panels from route-backed section
navigation. Local tabs use accessible tab semantics and local signals; route
tabs use real anchors and document navigation.

Acceptance criteria:
- Component API requires explicit local-panel or route-navigation mode.
- Local tabs implement tablist/tab/tabpanel roles, arrow-key behavior, focus,
  selection, and panel associations.
- Route tabs are anchors with current-page/section semantics and shareable URLs.
- Lazy panel content retains meaningful initial/empty/loading behavior and Live
  ownership.
- Tab changes do not make server calls unless server-backed state/content is
  explicitly requested.
- Reorder/removal preserves selected identity through stable keys.

UX flow:
1. Application user changes a local tab -> panel state changes instantly without
   network activity.
2. Application user changes a route tab -> browser navigates to the corresponding canonical
   document.

### Pagination

Pagination components shall expose real addressable pages or cursors, current
position, boundary state, and loading feedback while supporting efficient Live
island updates when chosen by the application.

Acceptance criteria:
- Route pagination uses canonical URLs and supports reload, sharing, history,
  crawlability policy, and no-JavaScript navigation.
- Live pagination may reflect its current same-route query with
  `history.replaceState`; it creates no per-page history entries or `popstate`
  action behavior.
- Pagination requiring Back/Forward page traversal, route/path changes,
  crawlability, or no-JavaScript operation uses canonical route links and normal
  document navigation.
- Previous/next, first/last where offered, page labels, current state, and
  unavailable boundaries are accessible.
- Cursor pagination does not invent numeric totals it cannot prove.
- Page-size choice is bounded and represented safely in URL/cache variance.
- Focus/scroll after page change follows declared content-region policy.

UX flow:
1. Application user selects another page -> route mode navigates normally;
   targeted Live mode morphs the island and replaces only the current query URL.
2. Result is empty or cursor expires -> component presents the owning data
   surface's recovery rather than an invalid page control.

### Steps and progress navigation

Stepper/progress-navigation components shall represent multi-step workflows
without claiming that visual progress is durable domain completion. Step
availability and completion come from server-authoritative workflow state.

Acceptance criteria:
- Current, completed, available, disabled, optional, and error steps are
  semantically exposed.
- Direct step navigation is allowed only when workflow policy permits and uses
  a real route or registered action.
- Visual numbering/order remains consistent with accessible labels.
- Validation prevents forward progress without required state while preserving
  correction paths.
- History/back behavior does not replay committed actions.
- Responsive presentation retains step context.

UX flow:
1. Application user completes a step -> server outcome advances authoritative
   workflow state.
2. Application user selects a permitted prior/next step -> normal route/action behavior
   loads it; forbidden steps remain unavailable with explanation where useful.

### Command and navigation discovery

Command palette or quick-navigation components may search permitted destinations
and registered actions using local or server-backed data. They shall not become
an arbitrary command dispatcher.

Acceptance criteria:
- Entries are typed as navigation, registered action, or informational result.
- Keyboard invocation, search, groups, active option, selection, and dismissal
  follow accessible combobox/dialog patterns.
- Server-backed search uses scheduling, stale suppression, authorization, and
  loading/empty/error states.
- Action entries name consequence and require confirmation where destructive.
- Navigation entries use visible canonical destinations.
- Recent/favorite persistence has explicit privacy and storage policy.

UX flow:
1. Application user opens and searches commands -> local or authorized server
   results appear with truthful state.
2. Application user selects an entry -> component navigates normally or invokes only the
   registered declared action.

## Acceptance criteria

- Navigation components preserve native route, URL, history, and link semantics.
- Responsive and local disclosure behavior uses accessible browser-local state.
- Tabs and pagination require an explicit local, Live, or route-backed mode.
- Workflow and command surfaces cannot bypass server authority.
- Every component remains usable through keyboard and assistive technology.

## Decisions and revisions

- 2026-08-21 -- Official navigation components reinforce canonical routes;
  rejected client-router abstractions for stylistic convenience.
- 2026-08-21 -- Local and route-backed tabs are separate explicit modes because
  they have different history, accessibility, and authority semantics.
- 2026-08-21 -- Targeted Live pagination may reflect state with
  `history.replaceState` but creates no page history; navigation-significant
  pagination remains route-backed.
