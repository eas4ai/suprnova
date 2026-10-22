# Page budget

A page budget checks the document and the assets it fetches. The test helper
uses the application's in-process HTTP path. It needs no browser, port or
network. The console command reads a HAR capture or fetches a live URL.
Both runners use `PageBudget` and the same evaluator.

## Test a page

Pass the application's router and middleware to `TestClient`. Use an absolute
URL when the document contains absolute asset URLs. Relative paths use
`http://localhost`.

```rust
use suprnova::page_budget::PageBudget;
use suprnova::testing::TestClient;
use suprnova::{HttpResponse, MiddlewareRegistry, Request, Router};

#[tokio::test]
async fn public_page_stays_within_budget() -> Result<(), suprnova::FrameworkError> {
    let router = Router::new().get("/", |_req: Request| async {
        Ok(HttpResponse::html("<h1>Hello</h1>"))
    });
    let client = TestClient::new(router, MiddlewareRegistry::new());
    client.audit_page("/", &PageBudget::default()).await?.ensure_passes()
}
```

The helper fetches stylesheets, scripts, images, source sets, icons, manifests
and preloads. It follows CSS URLs, including `@font-face` fonts and imports.
It fetches manifest icons only when the document references the manifest.
It deduplicates URLs and records external hosts without requesting them.
Every asset request carries no cookie. The client's login and cookie jar do
not affect the audit. A refused or redirected asset fails the audit.

## Set limits

The defaults use decimal bytes. Total transferred response bodies are limited
to 1,000,000 bytes. An asset is limited to 250,000 bytes. Fonts together are
limited to 100,000 bytes. A raster above 100,000 decoded bytes needs a WebP or
AVIF alternative and a fallback. A retained PNG above that threshold must be
within 10 percent of oxipng's fast preset. Manifest icons may remain PNG;
the PNG compression rule still applies.

```rust
use suprnova::page_budget::PageBudget;

let budget = PageBudget {
    total_transfer_bytes: 800_000,
    asset_bytes: 200_000,
    font_bytes: 80_000,
    ..PageBudget::default()
};
```

These limits expose the captured 1.4 MB page, its large PNG and its icon font.
They leave room for a starter page. Set limits for the application's content
and measure changes in review. PNG recompression is lossless and preserves
the media pipeline's colour and metadata policy. Lossy media output keeps
the framework's default quality of 85.

Fingerprint detection uses a configurable regular expression against the file
name without its extension. The default accepts Vite names such as
`main-BjmpJyAU.js`. Fingerprinted assets require
`public, max-age=31536000, immutable`. Other static files default to
`public, max-age=3600`, or the handler's configured cache policy. Static file
responses issue no session or CSRF cookie. Page responses keep their normal
session behavior.

## Audit a capture or URL

Run the application's console binary. A finding returns a non-zero exit code.
Each row names the asset, rule, measured value and allowed value. The output
also includes the request count, transfer bytes and third-party hosts.

```bash
cargo run --bin console -- budget:audit capture.har
cargo run --bin console -- budget:audit https://example.com/
```

HTTP requests have a ten-second timeout. The whole collection has a thirty-second
timeout, a limit of 256 resources and a 16 MiB response limit. These collection
limits are configurable. Redirects are recorded before following same-origin
locations. The initial HTTP-to-HTTPS upgrade may be followed on the same host;
it still fails the first-redirect rule. Other external redirects are listed.

HAR response `bodySize` measures transferred bytes; `content.size` measures
decoded bytes. When `bodySize` is unknown, the audit uses `content.size`.
A capture without PNG bytes cannot prove compression and reports that missing
evidence. Captured request cookies fail the request-cookie rule; they show
that cookies were sent, not that the server required them. The live and test
runners prove cookie independence by requesting assets without cookies.
A bodyless HAR cannot prove document fallback relationships for a large raster.
Export response bodies when those rules matter.

### Why Suprnova diverges

Laravel's HTTP tests inspect individual responses. Suprnova follows the
page's assets so a test catches transfer size, image format, font size and
cache policy together. The same rules run against a deployment capture.
The audit does not execute JavaScript. Assets created only by client-side
code appear in a HAR audit when the capture records them.

## Next

Read [HTTP Tests](http-tests.md), [Testing](testing.md),
[Starter Kits](starter-kits.md) and [Images](images.md).
