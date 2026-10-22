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
and preloads. It follows video and audio sources, video posters and CSS URLs
in style attributes and stylesheets, including `@font-face` fonts, imports
and string candidates in `image-set()`.
It fetches manifest icons only when the document references the manifest.
It deduplicates URLs and records external hosts without requesting them.
All `srcset` candidates count; the helper does not choose a viewport.
Data URL commas do not hide later candidates.
Every asset request carries no cookie. The client's login and cookie jar do
not affect the audit. A refused or redirected asset fails the audit.

React, Vue, Svelte and Live kits ship `tests/page_budget.rs` over their five
public entry pages. The API kit has no public pages and ships no page test.

## Set limits

The defaults use decimal bytes. Total transferred response bodies are limited
to 1,000,000 bytes. An asset is limited to 250,000 bytes. Fonts together are
limited to 100,000 bytes. A raster above 100,000 decoded bytes needs a WebP or
AVIF alternative and a fallback. A retained PNG above that threshold must be
within 10 percent of oxipng's fast preset. Manifest icons follow the same
raster rule. A PNG fallback needs another entry in the same manifest with
`type` `image/webp` or `image/avif` and the same `sizes`. The PNG compression
rule still applies to that fallback.

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
name without its extension. The default matches Vite's eight-character hash when it contains a digit,
uppercase letter, underscore or hyphen, in names such as
`main-BjmpJyAU.js`. Fingerprinted assets require
`public, max-age=31536000, immutable`. Other static files default to
`public, max-age=3600`, or the handler's configured cache policy. Static file
responses skip session persistence and cookie issuance only when the handler
changes no session state, issues no CSRF token and queues no security work.
Automatic flash aging alone does not count as a change. Stored flash survives
the asset request for the following page. Token issuance finalizes normally
and sends the response as `private, no-store`. Remember credential rotation,
regeneration, identity changes and queued sign-in rebinds also finalize normally
and deliver any required cookies. Responses carrying those cookies become private. Page responses keep their
normal session behavior.

Select another naming convention on both the audit and the static handler.
The expression matches the filename without its extension. An invalid handler
pattern returns an error on its first request.

```rust
use suprnova::{StaticFiles, page_budget::PageBudget};

let pattern = r"\.v[0-9]+$";
let files = StaticFiles::public().fingerprint(pattern).handler();
let budget = PageBudget { fingerprint_pattern: pattern.into(), ..PageBudget::default() };
```

## Audit a capture or URL

Run the application's console binary. Capture a fresh visit with response bodies.
A finding returns a non-zero exit code.
Each row names the asset, rule, measured value and allowed value. The output
also includes the request count, transfer bytes and third-party hosts.

```bash
cargo run --bin console -- budget:audit capture.har
cargo run --bin console -- budget:audit https://example.com/
```

HTTP requests have a ten-second timeout. The whole collection has a thirty-second
timeout, a limit of 256 resources and a 16 MiB response limit.
`budget:audit` reads these collection limits through `PageBudget::from_env()`.
Discovery deduplicates and caps references as it
parses HTML, CSS and manifests. The pending queue is bounded too. Repeating
one reference does not allocate another queue entry. Every `Cache-Control`
header line contributes to the policy; a second `no-cache` line fails the
immutable rule. Redirects are recorded before following same-origin
locations. The initial HTTP-to-HTTPS upgrade may be followed on the same host;
it still fails the first-redirect rule. Other external redirects are listed.

`PageBudget::default()` keeps the shipped values. `PageBudget::from_env()`
reads one variable per field and rejects invalid values with the variable's
name. Boolean values use literal `true` or `false`. Host lists use commas;
spaces around each host are ignored. Unset lists only report hosts. An empty
list rejects every external host. An invalid fingerprint expression fails
before collection.

```bash
PAGE_BUDGET_TOTAL_BYTES=800000 PAGE_BUDGET_FONT_BYTES=80000 cargo run --bin console -- budget:audit capture.har
```

| Variable | Default | Type | Purpose |
|---|---|---|---|
| `PAGE_BUDGET_TOTAL_BYTES` | `1000000` | u64 | Total transferred response bytes. |
| `PAGE_BUDGET_ASSET_BYTES` | `250000` | u64 | Transferred bytes per asset. |
| `PAGE_BUDGET_RASTER_THRESHOLD_BYTES` | `100000` | u64 | Decoded bytes above which a raster needs a modern format and fallback. |
| `PAGE_BUDGET_PNG_MARGIN_PERCENT` | `10` | u32 | Margin above the fast PNG recompression size. |
| `PAGE_BUDGET_FONT_BYTES` | `100000` | u64 | Combined transferred font bytes. |
| `PAGE_BUDGET_REQUIRE_IMMUTABLE` | `true` | bool | Check fingerprinted cache policy. |
| `PAGE_BUDGET_FORBID_ASSET_SET_COOKIE` | `true` | bool | Reject asset response cookies. |
| `PAGE_BUDGET_FORBID_ASSET_REQUEST_COOKIE` | `true` | bool | Reject captured asset request cookies. |
| `PAGE_BUDGET_REQUIRE_COOKIE_INDEPENDENT` | `true` | bool | Require successful anonymous asset access. |
| `PAGE_BUDGET_FORBID_FIRST_REDIRECT` | `true` | bool | Reject an initial document redirect. |
| `PAGE_BUDGET_FINGERPRINT_PATTERN` | Strict Vite hash expression | regex | Match the filename stem. |
| `PAGE_BUDGET_ALLOWED_THIRD_PARTY_HOSTS` | unset | comma-separated hosts | Restrict external hosts when set. Empty forbids all external hosts. |
| `PAGE_BUDGET_MAX_RESOURCES` | `256` | usize | Bound discovery and queued resources. |
| `PAGE_BUDGET_MAX_RESPONSE_BYTES` | `16777216` | usize | Bound one response body. |
| `PAGE_BUDGET_REQUEST_TIMEOUT_SECS` | `10` | u64 | Bound one HTTP request in seconds. |
| `PAGE_BUDGET_COLLECTION_TIMEOUT_SECS` | `30` | u64 | Bound the complete collection in seconds. |

HAR response `bodySize` measures transferred bytes; `content.size` measures
decoded bytes. When `bodySize` is unknown, the audit uses `content.size`.
A capture without PNG bytes cannot prove compression and reports that missing
evidence. Captured request cookies fail the request-cookie rule; they show
that cookies were sent, not that the server required them. The live and test
runners prove cookie independence by requesting assets without cookies.
Cookie independence also fails when a capture carries cookies, even when the
separate request-cookie privacy rule is disabled.
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
