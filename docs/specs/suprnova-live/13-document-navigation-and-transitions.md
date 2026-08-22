# Suprnova Live -- 13 Document Navigation and Transitions

Status: Normative design specification
Last revised: 2026-08-22

## Scope

This domain owns navigation behavior for Live applications: ordinary links and
forms, action redirects, optional safe prefetching, browser history, focus and
scroll restoration, document lifecycle, and document-level visual transitions.
It depends on canonical documents, runtime lifecycle, scheduling, and morph
identity. It explicitly does not own an SPA router or partial-document protocol.

## Capabilities

### Normal document navigation

All route changes shall use normal browser navigation to real Suprnova routes
that return complete canonical documents. Live directives may initiate or
decorate navigation but shall not replace it with client-side route authority.
Live may reflect shareable same-route query state into the current history entry
with `history.replaceState`; reflection is not a route transition, creates no
new entry, and installs no `popstate` island behavior.

Acceptance criteria:
- Ordinary anchors, GET/POST forms, action redirects, refresh, open-in-new-tab,
  download, and external links preserve native semantics.
- URL, history, status, headers, cookies, middleware, content negotiation, and
  error pages remain controlled by HTTP and the target route.
- A protocol-v2 `navigated` URL intent is a terminal ordinary navigation result,
  mutually exclusive with committed island render and child-delivery output. It
  follows the same no-morph/no-event/no-effect browser precedence as redirect.
- Navigation works when the Live runtime is unavailable.
- Same-document fragments retain standard browser behavior.
- Reflected Live query state remains reloadable/shareable but cannot create a
  sequence of Live states for Back/Forward traversal; history-significant state
  uses ordinary navigation.
- Live never turns an Inertia page object or HTML fragment into its document
  response protocol.

UX flow:
1. Application user follows a route link or accepted redirect -> browser loads
   the target canonical document.
2. Runtime enhancement is absent -> the same navigation and destination still
   work without animation or prefetch.

### Safe prefetching

Applications may prefetch eligible canonical document requests to reduce
latency, but prefetch shall remain advisory, privacy-aware, cancellable, and
semantically indistinguishable from later normal navigation.

Acceptance criteria:
- Only safe, idempotent, explicitly eligible GET/HEAD targets prefetch.
- Live uses browser-native `<link rel="prefetch">`, Speculation Rules, or an
  equivalent standards-based navigation/cache primitive; it does not inject a
  JavaScript-fetched document body as a partial navigation result.
- Prefetch respects credentials, tenant, locale, cache variance, authorization,
  rate limits, data-saver preferences, and server cache headers.
- Hover, focus, viewport, and explicit programmatic triggers have bounded delay
  and concurrency.
- Prefetched private content cannot be reused by another principal or context.
- A stale, failed, redirected, or incompatible prefetch falls back to ordinary
  navigation.
- Prefetch never commits mutations, consumes flash state incorrectly, or marks
  analytics navigation as completed.
- Routes that read-and-consume session/flash state or otherwise cannot tolerate
  speculative GETs are ineligible unless they provide an explicit safe path.

UX flow:
1. Application user signals likely navigation -> eligible content may prefetch
   without changing URL or document.
2. Application user activates the link -> browser navigates normally and its
   native cache/speculation machinery may reuse a still-valid compatible
   response.

### Document View Transitions

Supporting browsers may visually transition between canonical documents using
the browser's document transition capabilities. Transition support shall remain
an enhancement and shall respect reduced motion and route authority.

Acceptance criteria:
- Transition names are stable, unique where required, and derived from safe
  server markup.
- Navigation commits the target document even if capture or animation fails.
- Reduced-motion can suppress non-essential transitions.
- Transition duration and cancellation cannot block history or leave an old
  document interactive after commit.
- Cross-origin, download, error, and unsupported navigation use native behavior.
- Island and document transition identities cannot collide ambiguously.

UX flow:
1. Application user navigates between compatible documents -> browser may
   animate declared old/new elements.
2. Capability or preference disallows motion -> destination appears through
   ordinary navigation with identical content and state.

### History, scroll, and focus restoration

Back/forward traversal, fragment navigation, scroll restoration, and focus shall
follow browser semantics with only explicit accessibility-preserving
enhancements.

Acceptance criteria:
- Back and forward restore URL-bound server state and do not replay Live actions.
- Repeated Live `replaceState` reflections do not become separate history
  entries and never register a `popstate` action path.
- New document navigation establishes a useful focus target when native behavior
  would strand keyboard or assistive-technology users.
- Scroll preservation/restoration is explicit for navigation type and does not
  override application-user intent after load.
- Fragment targets work with SSR and delayed/lazy regions.
- Browser back-forward cache restoration reconnects islands without duplicate
  listeners or stale in-flight response application.

UX flow:
1. Application user traverses history -> browser restores or requests the
   corresponding canonical document and Live reconnects safely.
2. Target contains a fragment or focus declaration -> the final document places
   viewport and focus predictably.

### Dirty state, uploads, and pending work

Navigation may warn about declared unsaved state or active uploads, but Live
shall distinguish browser-response cancellation from server-side action
rollback and shall not trap application users indefinitely.

Acceptance criteria:
- Applications opt into navigation guarding for specific dirty/upload states.
- Warnings state what may be lost and offer leave/stay choices through supported
  browser behavior.
- Pending HTTP responses are prevented from applying after document departure.
- Leaving does not falsely claim to cancel already committed server effects.
- Guards are bypassable for safe redirects or explicit discard according to
  policy and accessible without a pointer.

UX flow:
1. Application user leaves with declared unsaved work -> the application warns
   or allows navigation according to its explicit policy.
2. Application user leaves -> browser navigates and old island queues retire;
   later server
   truth is reflected by future routes rather than late DOM mutation.

### Document lifecycle cleanup

The runtime shall stop document-scoped queues, signals, observers,
subscriptions, controllers, and temporary enhancement state when a document is
replaced or discarded, while supporting safe restoration from browser caches.

Acceptance criteria:
- Page hide, freeze, resume, pageshow, unload limitations, and bfcache behavior
  are covered by lifecycle tests.
- Cleanup is idempotent and does not depend solely on unreliable unload network
  calls.
- Local signals reset on true document replacement.
- Restored documents validate snapshot/runtime compatibility before accepting
  new actions.
- Broadcast and observer connections are not duplicated after restoration.

UX flow:
1. Browser leaves or freezes a document -> runtime retires or suspends resources
   without blocking navigation.
2. Browser restores it -> runtime reconnects compatible islands or requests
   fresh state before interaction.

## Acceptance criteria

- Every URL transition remains normal browser navigation to a canonical route.
- Prefetching is safe, advisory, variance-aware, and disposable.
- View transitions and focus/scroll enhancements degrade without semantic loss.
- History does not replay Live actions or accept stale late responses.
- Dirty, upload, and lifecycle behavior are truthful about committed server
  work.

## Decisions and revisions

- 2026-08-22 -- Classified protocol-v2 navigated URL intent as terminal ordinary
  document navigation, mutually exclusive with committed morph and child output;
  it shares redirect precedence rather than becoming a post-morph client route.
- 2026-08-21 -- Rejected SPA and Turbo-style navigation for Live; real routes
  and documents remain authoritative.
- 2026-08-21 -- Prefetch and View Transitions are optional enhancements, not an
  alternate navigation architecture.
- 2026-08-21 -- Live URL reflection is limited to same-route query
  `history.replaceState`; history-significant state uses real navigation.
- 2026-08-21 -- Prefetch uses browser-native navigation/cache primitives only;
  rejected JavaScript body reuse as a hidden partial-document protocol.
