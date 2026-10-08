# Inertia root template

Status: Agreed 2026-10-07 (RDOC-005 Draft)
Prefix: RDOC

Drafted 2026-10-05 from issue #143 (an application-owned root document for
Inertia's first visit) and the developer's acceptance of the
recommendations made from the code that day ("I am going to accept your
recommendations"). The plan was posted on the issue. The Observed section
describes framework `2bd4bd53d` (v3.2.1), checked against the code by an
independent reader the same day and again at `c34ee7b15`, where no cited
file had changed. RDOC-001 to RDOC-004 and RDOC-006 are built in the
Inertia protocol commitment (the parity rows RF-01, RS-02, RS-05 and BL-04,
ruled on 2026-10-07); RDOC-005 takes the request's nonce from SEC-003, so it
is built in the commitment that builds security headers (SEC) and the data
cache. Laravel adapter references cite
`reference/inertia-laravel-2.0.25/src/`; Inertia client references cite
`reference/inertia-3.7.1/packages/core/src/`.

## Observed at 2bd4bd53d

Status: Observed

- `build_html_response` writes the whole first-visit document itself
  (`framework/src/inertia/response.rs:2285-2368`): the doctype, `<html
  lang>` from the locale with the `localization` feature and `"en"`
  without it (`response.rs:2457-2467`), charset, viewport, the
  `csrf-token` meta tag, the title from `InertiaConfig::default_title` or a
  per-response override (`response.rs:326,2291`), dropped when the SSR head
  carries one (`response.rs:2325`), the SSR head and body as strings
  (`response.rs:2316,2349-2350`), the Vite tags, and the page data in a
  `<script type="application/json" data-page="app">` element with `<div
  id="app">`. The `csrf-token` meta tag comes before the title
  (`response.rs:2337-2345`). It writes the page JSON straight into one
  buffer through a slash-escaping writer (`response.rs:2266-2283,2357`);
  the comment at `response.rs:2327-2330` records that serializing through
  intermediate strings had copied the page four times on every first
  visit. MEM-003's test measures this path
  (`framework/tests/memory/http.rs:48-91`).
- There is no root view, no view data and no hook for head or body markup;
  the only call site passes the page, config, title and SSR output
  (`response.rs:1226`).
- No nonce reaches the Vite tags or the development preamble
  (`response.rs:2499,2513-2514,2532,2537,2542,2549-2550`). The Inertia
  client stamps the `nonce` passed to `createInertiaApp` on the progress
  and error-dialog styles it injects (`progress-component.ts:288-294`,
  `dialog.ts:54-58`); the scaffold entries pass none
  (`suprnova-cli/src/templates/files/frontend/*/src/main.*.tpl`).
- Vite's runtime takes its nonce from `<meta property="csp-nonce">`: the
  development client stamps it on every `<style>` it injects
  (`vite/dist/client/client.mjs:589,1164-1172`), and the production
  preload helper on the code-split stylesheet and modulepreload links it
  adds (`vite/dist/node/chunks/node.js:28255-28256`), in Vite 8.0.13 under
  `app/frontend/node_modules/`. The framework writes no such element.
- Live documents let the application own an Askama `#[view]` template
  (`suprnova-macros/src/view.rs:17-45`), and `LiveDocument::bootstrap`
  returns the framework's markup for the template to place with
  `|trusted_html` (`framework/src/live/document.rs:830-881`). `TrustedHtml`
  owns a `String` capped at 2 MiB
  (`crates/suprnova-live/src/view/trusted_html.rs:11,180-184,192,213,282-287`,
  re-exported at `framework/src/view/mod.rs:19`), so that shape cannot
  carry a page of any size without a copy.
  `LiveBootstrapOptions::with_nonce` stamps a caller's nonce on script
  elements (`framework/src/live/assets.rs:140-145,577-593`) but not on the
  modulepreload, preload and stylesheet links (`assets.rs:512,519,538`).
- Error pages render through the same Inertia document
  (`framework/src/inertia/error_page_middleware.rs:180`). The middleware
  holds only the captured path, query and headers, as an
  `InertiaRequestExt`, because the handler consumed the request
  (`error_page_middleware.rs:134,249-276`). In debug mode a server error
  becomes the debug page instead, built outside the middleware chain
  (`framework/src/server.rs:743-769`; PAR-012, PAR-015).
- The scaffold's `index.html.tpl` files, written to `frontend/index.html`
  (`suprnova-cli/src/templates/mod.rs:821`), are a shell the server never
  serves (the Vite input is the entry module), and the scaffold never sets
  `default_title`
  (`suprnova-cli/src/templates/files/backend/bootstrap.rs.tpl:179-183`), so
  every page is titled `Suprnova` (`framework/src/inertia/config.rs:519`).
  The scaffold's Dockerfile copies `cmd/` and `src/` and no `templates/`
  (`suprnova-cli/src/templates/files/docker/Dockerfile.tpl:55-56`).

## Requirements

[RDOC-001] An application MUST be able to own its Inertia root document as
an Askama template checked at compile time, receiving the framework's
parts as values it places: `title` (the `<title>` element from the
response's title or `default_title`, empty when the SSR head carries
one), `head` (the `csrf-token` meta tag, under a nonce policy the
`csp-nonce` meta element of RDOC-005, the SSR head and the Vite tags),
`body` (the page data element and mount element, or the SSR body), and
`lang`, `csrf_token`, `nonce` (absent without a nonce policy) and `ssr`
(whether the SSR server rendered this response, for a template that places
fallback head content when it did not) for templates that place them
individually. The mount element's id and the page data element's
`data-page` attribute MUST come from `InertiaConfig::mount_id`, default
`app`, so a client that mounts on another id finds its element. A template
that writes its own `<title>` omits the `title` part. A root template that fails to render
MUST make the response an error and MUST NOT panic or send part of a
document.
Falsifier: a root template cannot place a favicon, meta tags, a `<noscript>` or attributes on `<html>` and `<body>`; a part placed bare is missing or escaped as text; the `title` part is non-empty when the SSR head carries a `<title>`; `ssr` is true for a response the SSR server did not render; with `mount_id` set to `root` the page data element or the mount element still says `app`; a root template that names a field the framework does not supply compiles; or a root template that fails to render panics or sends part of a document.
Mechanism: `inertia-root-template`.
Rationale: Laravel's `app.blade.php` with `@inertiaHead`, `@inertia` and `@vite`; the application-owned template is the shape Live's documents already have.
Status: Agreed 2026-10-07

[RDOC-002] Without an application template, the first-visit document MUST
be byte for byte the one the framework writes today wherever the public
root (PFX-002) is `/` and no nonce policy (SEC-003) applies.
Falsifier: with no template, at public root `/` and with no nonce policy, the body or headers of a first visit differ by any byte from `c34ee7b15`'s output for the same page, title, locale, CSRF token and SSR output, in development with the React preamble, in production with a manifest, on the legacy fallback or under SSR.
Mechanism: `inertia-root-template`.
Rationale: The parts cannot rebuild today's bytes, which put the `csrf-token` meta tag before the title (`response.rs:2337-2345`), so the no-template path keeps its own writer, the one MEM-003 measures.
Status: Agreed 2026-10-07

[RDOC-003] The parts MUST be framework values that write themselves into
the template's output while it renders (placed through Askama's safe
output, not `TrustedHtml`), so the page JSON is never copied into an
intermediate string and the root template adds no size cap.
Falsifier: with a 1 MiB prop, a first visit through a root template allocates at least 1 MiB more (dhat total bytes) than the same visit through the default document; or a first visit with a 3 MiB prop through a root template does not return 200 with the whole page.
Mechanism: `inertia-root-template`.
Rationale: Keeps the single-buffer property `build_html_response` records (`response.rs:2327-2330`); `TrustedHtml` owns a string capped at 2 MiB, and the SSR response keeps its own cap, `InertiaConfig::ssr_max_response_bytes`.
Status: Agreed 2026-10-07

[RDOC-004] `InertiaConfig` MUST accept a function of the request (its
path, query and headers, as `InertiaRequestExt`) that chooses the root
template for each request, and a response MUST accept view data that
reaches the root template and never the page props.
Falsifier: with a chooser that picks template B for paths under `/admin` and A otherwise, a first visit to `/admin/x` is not rendered through B or one to `/` not through A; or a value set as view data appears anywhere in the page object of the HTML or the JSON response, or does not reach the template.
Mechanism: `inertia-root-template`.
Rationale: Laravel's `rootView(Request)` and `withViewData` (`Middleware.php:80`, `Response.php:137,219`); taking `InertiaRequestExt` lets the error page, which holds only the captured request, call the chooser (RDOC-006).
Status: Agreed 2026-10-07

[RDOC-005] The request's CSP nonce (SEC-003) MUST be on every executable
script and every script or style link the framework emits in the
document: the Vite tags, the development preamble, and Live's scripts and
its modulepreload, preload and stylesheet links. Data elements that never
execute (the page data element, Live's configuration element) need none.
Under a nonce policy the document's head (the `head` part in a root
template) MUST carry `<meta property="csp-nonce" nonce="...">` with the
request's nonce, the element Vite's development client and production
preload helper read, and each scaffold entry MUST pass the nonce from that
element to `createInertiaApp`, which stamps it on the styles the Inertia
client injects.
Falsifier: under a nonce policy, a `<script>` that is not `type="application/json"`, or a `<link>` with rel `stylesheet`, `modulepreload` or `preload`, that the framework writes into an Inertia first visit, an Inertia error page or a Live bootstrap whose caller names no nonce lacks `nonce` equal to the request's (in development with the React preamble, production, the legacy fallback, ESM, classic, and with the suprnova-ui base); the document lacks `<meta property="csp-nonce">` carrying the request's nonce; or a scaffold entry does not pass that nonce to `createInertiaApp`.
Mechanism: `inertia-root-template`.
Rationale: Vite's runtime reads its nonce from that meta element and the Inertia client from `createInertiaApp`'s options; without them a scaffold under a nonce policy renders unstyled in development and loses code-split CSS in production.
Status: Draft

[RDOC-006] Error pages rendered through Inertia MUST use the root template
the chooser picks for the captured request, without view data, since the
failing handler's data is gone; the debug page, which is built outside the
middleware chain, is exempt. The scaffold MUST ship a root template in
place of the unserved `frontend/index.html`, set a default title, and copy
`templates/` in its Dockerfile. The manual MUST document the template, its
parts, the per-request choice and view data (the nonce with RDOC-005).
Falsifier: with a chooser that maps `/admin/*` to template B, a browser navigation to an unrouted `/admin/x` (404) or a denied `/admin/y` (403) is not rendered through B, or B receives view data; `suprnova new` writes `frontend/index.html`, titles its first visit `Suprnova`, or writes a Dockerfile that does not copy `templates/`; or `manual/frontend-inertia-responses.md` does not name the template, each part, the chooser and view data.
Mechanism: `inertia-root-template`.
Rationale: Askama reads a template at compile time, and the scaffold's image build copies only `cmd/` and `src/` (`Dockerfile.tpl:55-56`), so without the copy a scaffolded root template breaks it.
Status: Agreed 2026-10-07
