# Document navigation

Live is not a SPA router. Links and forms retain native semantics and real
routes; every document change is a real HTTP navigation. Live actions may
return terminal redirects, and URL-bound state may reflect the current query
with `history.replaceState`, but Live never owns a client route table or
`popstate` rendering protocol.

## Native navigation

Trusted, unmodified primary-button clicks and native form submissions are
observed only to coordinate dirty guards, prefetch cancellation, transition
intent, and cleanup. Downloads, non-self targets, modified clicks,
cross-origin destinations, non-HTTP(S) URLs, same-document fragments, POST
forms, and non-HTML responses retain their native browser behavior.

`live:url.reflect` may replace only the query on the current same-origin path.
Anything requiring back/forward history semantics uses a real HTTP navigation.
A response redirect is terminal: the runtime skips morph application and calls
the injected/native navigation port once.

Dirty navigation guards attach `beforeunload` only while needed. An in-page
attempt may use the owning application's confirmation policy; canceling leaves
focus, history, and the current island state unchanged.

## Prefetch and View Transitions

Eligible same-origin GET/HEAD links may request eager, visible, hover, or focus
prefetch. The coordinator is bounded to two concurrent requests and 256 scanned
targets, deduplicates by link identity, cancels work when a target leaves, and
uses browser prefetch mechanisms only. Prefetch never executes a Live action,
promotes a seed, or creates an application response.

When supported, allowed by policy, and not disabled by reduced motion, a named
View Transition may decorate a native document departure. Names use a closed
bounded grammar. Failure or lack of browser support falls back to the same
native navigation; transitions never change response eligibility or keep an
island alive.

## Page lifecycle and bfcache

`pagehide` with persistence suspends the document runtime; final pagehide and
explicit stop dispose it. A persisted `pageshow` validates asset identity,
runtime contract, protocol compatibility, and current island metadata before
resuming. Incompatible restoration performs a native reload rather than using
stale snapshots or duplicate listeners.

Freeze/resume follows the same resource ledger when the browser exposes it.
Suspension disconnects observers, cancels prefetch/transitions, and prevents
new effects or calls. Resume reattaches exactly one listener/observer set and
rescans current DOM. The leak and bfcache suites prove repeated cycles do not
accumulate resources.
