# Page budget audit

Status: Agreed 2026-10-21
Prefix: BUDGET

Drafted 2026-10-21 from the developer's direction on the Pingdom capture
of suprnova.app (`reference/har.json`, 2026-10-18): "this needs to be
converted to webp. I think that the framework should test for this
stuff." The capture showed 1.4 MB over 21 requests, a 590 KB PNG icon
fetched by the page, one-day caching on fingerprinted assets, the session
cookie sent on every asset, and a redirect on the first request. The
audit turns that report into a test an application runs without a
browser. It delivers backlog items `static-asset-caching`,
`cookie-free-static-assets`, `avoid-url-redirects` and
`suprnova-app-page-weight` together with the static publication (PUB).

## Observed at bfcbd61fe

Status: Observed

- No framework helper measures a page's weight, asset headers or image
  formats; the testing surface drives `handle_request` and asserts on one
  response at a time (`manual/http-tests.md`).
- The image subsystem encodes JPEG, WebP, GIF and BMP through the
  pipeline's quality (`framework/src/media/driver.rs:212`,
  `framework/src/media/oxideav.rs:1508`); oxipng is not a dependency.
- The static asset handler sets `public, max-age=86400` and the session
  middleware sets the session cookie on every response of the host, which
  is what the capture shows (`framework/src/http/`,
  `framework/src/session/middleware.rs`).

## Requirements

[BUDGET-001] The framework MUST provide a page budget audit with one rule
set run two ways: a testing helper that renders a page through the
in-process request path, follows the assets the document references and
asserts the budgets; and a console command that applies the same rules to
a live URL or to a HAR file. The rules MUST cover total transfer weight;
per-asset size; raster format against size (a raster over the threshold
MUST be WebP or AVIF with a fallback; a PNG over the threshold MUST be
within a configured margin of its size under oxipng's fast preset);
fingerprinted assets carrying `public, max-age=31536000, immutable`; no
`Set-Cookie` on a static asset response and no cookie needed to fetch it;
no redirect on the first request; fonts within a budget; and the third
party hosts a page reaches, listed. Each budget MUST be configurable with
the defaults the framework ships, and a failed rule MUST name the asset,
the rule, the measured value and the allowed value.
Falsifier: the helper and the command disagree on one page; one rule above is missing; a failure names no asset or no values; or the audit needs a browser to run.
Mechanism: `page-budget`.
Rationale: the developer, 2026-10-21: "I think that the framework should test for this stuff"; the Pingdom capture of suprnova.app, 2026-10-18.
Status: Agreed 2026-10-21

[BUDGET-002] Each starter kit and the dogfood application MUST ship the
page budget test for their public pages, passing on a fresh scaffold and
in the dogfood suite, and the static asset handler MUST serve a
fingerprinted asset with `public, max-age=31536000, immutable` and without
setting a cookie, so a kit passes the audit before any publication.
Falsifier: a fresh kit fails its own budget test; a kit lacks the test; or a fingerprinted asset from the origin carries a one-day lifetime or a `Set-Cookie`.
Mechanism: `page-budget`.
Status: Agreed 2026-10-21
