# Inertia root template

Status: Draft
Prefix: RDOC

Drafted 2026-10-05 from issue #143 (an application-owned root document for
Inertia's first visit) and the developer's acceptance of the
recommendations made from the code that day ("I am going to accept your
recommendations"). The plan was posted on the issue. The Observed section
describes framework `2bd4bd53d` (v3.2.1), checked against the code by an
independent reader the same day. Inertia references cite
`reference/inertia-laravel-2.0.25/src/`.

## Observed at 2bd4bd53d

Status: Observed

- `build_html_response` writes the whole first-visit document itself
  (`framework/src/inertia/response.rs:2285-2368`): the doctype, `<html
  lang>` from the locale with the `localization` feature and `"en"`
  without it (`response.rs:2457-2467`), charset, viewport, the
  `csrf-token` meta tag, the title from `InertiaConfig::default_title` or a
  per-response override (`response.rs:326,2291`), dropped when the SSR head
  carries one (`response.rs:2325`), the SSR head and body as strings
  (`response.rs:2316,2341`), the Vite tags, and the page data in a
  `<script type="application/json" data-page>` element with `<div
  id="app">`. It writes the page JSON straight into one buffer through a
  slash-escaping writer (`response.rs:2356`); the comment there records
  that serializing through intermediate strings had copied the page four
  times on every first visit.
- There is no root view, no view data and no hook for head or body markup;
  the only call site passes the page, config, title and SSR output
  (`response.rs:1226`).
- No nonce reaches the Vite tags or the development preamble
  (`response.rs:2499,2513-2514,2532,2537,2542,2549-2550`), and nothing hands
  a nonce to the Inertia client, which needs one for the styles it injects.
- Live documents let the application own an Askama `#[view]` template
  (`suprnova-macros/src/view.rs:19-45`), and `LiveDocument::bootstrap`
  returns the framework's markup for the template to place with
  `|trusted_html` (`framework/src/live/document.rs:830-881`). `TrustedHtml`
  owns a `String` capped at 2 MiB (`framework/src/view/trusted_html.rs:11,180-184,282-287`),
  so that shape cannot carry a page of any size without a copy.
  `LiveBootstrapOptions::with_nonce` stamps a caller's nonce on script
  elements (`framework/src/live/assets.rs:140-145,577-593`) but not on the
  modulepreload, preload and stylesheet links (`assets.rs:512,519,538`).
- Error pages render through the same Inertia document
  (`framework/src/inertia/error_page_middleware.rs:180`); in debug mode a
  server error becomes the debug page instead, built outside the middleware
  chain (`framework/src/server.rs:735-765`; PAR-015).
- The scaffold's `index.html.tpl` files are a shell the server never serves
  (the Vite input is the entry module), and the scaffold never sets
  `default_title`, so every page is titled `Suprnova`
  (`framework/src/inertia/config.rs:519`).

## Requirements

[RDOC-001] An application MUST be able to own its Inertia root document as
an Askama template checked at compile time, receiving the framework's
parts as values it places: `title` (the default `<title>` element, empty
when the SSR head carries one), `head` (the `csrf-token` meta tag, the SSR
head and the Vite tags), `body` (the page data element and mount element,
or the SSR body), and `lang`, `csrf_token` and `nonce` for templates that
place them individually. A template that writes its own `<title>` omits
the `title` part.
Falsifier: a root template cannot place a favicon, meta tags or a `<noscript>` in the first-visit document; a part is missing or escaped as text; or the default document carries two titles.
Mechanism: `inertia-root-template`.
Rationale: Laravel's `app.blade.php` with `@inertiaHead`, `@inertia` and `@vite`; the application-owned template is the shape Live's documents already have.
Status: Draft

[RDOC-002] Without an application template, the first-visit document MUST
be the one the framework writes today wherever no prefix (PFX-002) and no
nonce policy (SEC-003) applies.
Falsifier: an application that sets no template, at `/` and with no nonce policy, gets a different document.
Mechanism: `inertia-root-template`.
Status: Draft

[RDOC-003] The parts MUST be framework values that write themselves into
the template's output while it renders (placed through Askama's safe
output, not `TrustedHtml`), so the page JSON is never copied into an
intermediate string and no size cap applies.
Falsifier: rendering a first visit through a root template allocates a copy of the page JSON that the default document does not, or a page over 2 MiB fails.
Mechanism: `inertia-root-template`.
Rationale: Keeps the single-buffer property `build_html_response` records; `TrustedHtml` owns a string and is capped.
Status: Draft

[RDOC-004] `InertiaConfig` MUST accept a function that chooses the root
template for each request, and a response MUST accept view data that
reaches the root template and never the page props.
Falsifier: two requests cannot get different root templates; view data appears in the page props, or does not reach the template.
Mechanism: `inertia-root-template`.
Rationale: Laravel's `rootView(Request)` and `withViewData` (`Middleware.php:80`, `Response.php:137,153`).
Status: Draft

[RDOC-005] The request's CSP nonce (SEC-003) MUST be on every executable
script and every script or style link the framework emits in the
document: the Vite tags, the development preamble, and Live's scripts and
its modulepreload, preload and stylesheet links. Data elements that never
execute (the page data element, Live's configuration element) need none.
The nonce MUST reach the Inertia client for the styles it injects, and the
scaffold MUST pass it on.
Falsifier: under a nonce CSP, a framework-emitted executable script or link lacks the nonce, or the Inertia client's injected styles are blocked.
Mechanism: `inertia-root-template`.
Status: Draft

[RDOC-006] Error pages rendered through Inertia MUST use the root template
the chooser picks for the captured request, without view data, since the
failing handler's data is gone; the debug page, which is built outside the
middleware chain, is exempt. The scaffold MUST ship a root template in
place of `index.html.tpl` and set a default title. The manual MUST
document the template, its parts, the per-request choice and view data.
Falsifier: an Inertia error page ignores the application's template; a new scaffold carries `index.html.tpl` or titles its pages `Suprnova`; or the manual omits the template.
Mechanism: `inertia-root-template`, `manual-check`.
Status: Draft
