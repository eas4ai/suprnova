# Static Publication

Static publication serves explicitly public Inertia pages from a named storage
disk. Each page has separate HTML and JSON objects. The origin keeps the route
available when storage or a CDN is unavailable.

## Declare a public page

Use the same checked props contract as [RenderCache](render-cache.md). Every
field must be public. Register shared props by name and list every shared key
as public. Authentication, errors, flash and credential props cannot be public.
An opaque shared provider cannot prove its fields and refuses publication.

```rust
use suprnova::{InertiaProps, InertiaResponse, Request, Response, Router};

#[derive(InertiaProps)]
#[inertia(cacheable)]
struct AboutProps {
    #[inertia(public)]
    title: String,
}

async fn about(request: Request) -> Response {
    InertiaResponse::new("About")
        .cacheable(AboutProps { title: "About us".into() })?
        .resolve(&request).await.map_err(Into::into)
}

fn routes() -> Result<Router, suprnova::FrameworkError> {
    let router: Router = Router::new().get("/about", about).into();
    router.public_page::<AboutProps>("/about", "About")
}
```

A group can opt in through `router.public_page_group("/articles")`. Each
handler still needs an `Inertia::public_page::<Props>(pattern, component)`
declaration. An unmarked route never enters the publication inventory.

```rust
suprnova::Inertia::share("site", "Example")?;
suprnova::Inertia::public_shared(&["site"])?;
```

Public rendering uses no visitor session. It skips session and CSRF middleware,
excludes session props and uses the dependency collector to refuse a handler
that reads session, identity or secret context. A refusal names the route and
cause. It fails the whole publication and keeps the previous inventory live. The failure
and route appear in status. It does not stop the server listener.

## Configure a disk and boot publication

The disk can be local, S3, GCS, Azure or any registered OpenDAL operator. Give
publication its own disk root. Pruning owns that root and can remove objects
that no retained publication references.

```rust
use suprnova::{FrameworkError, Router, Storage};
use suprnova::static_publish::{PublishConfig, Publisher, StaticPublication};

fn publish(router: &Router) -> Result<(), FrameworkError> {
    Storage::register_memory("pages");
    let publisher = Publisher::new("pages", "build-20261022", PublishConfig::default())?;
    let publication = StaticPublication::install(router, publisher)?;
    publication.assets(Storage::disk("public-assets")?)?;
    publication.add_path("/articles/welcome")?;
    Ok(())
}
```

Register the source disk `public-assets` against the application's public
directory before this function. It supplies built assets and source images.
Use the same Inertia asset version for the page config and the publisher.
Asset versions reserve `lease`, `manifest.json`, `lease.json`, every name that
begins with `_` and every name that contains `/`. Both publisher constructors
refuse these names because they overlap the publication control namespace.
Concrete parameterized paths can be listed with `add_path`. A missing public
path is also rendered and published when its first origin request arrives.

`Server::run` starts publication after `TcpListener::bind`. Health checks can
run while assets upload. Shutdown cancels rendering and staging. Once delivery
starts, it runs as owned work; graceful shutdown waits for the switch and
manifest commit or rollback. The configured timeout bounds this work, including
cleanup. A quarter of the timeout, up to ten seconds, is reserved for recovery.
Uploads have a configured concurrency limit. An upload failure leaves the live
manifest in place and logs an error.

```bash
cargo run --bin console -- static:publish
cargo run --bin console -- static:status
```

The publish command runs the same pipeline as boot. Status reports the live
revision, last outcome and pending work. No page body or credential appears in
the status output.

## Objects, versions and switching

Pages use `_pub/<asset-version>/<revision>/<path>.html` and
`_pub/<asset-version>/<revision>/<path>.json`. The root page uses `index`.
The extra revision keeps regeneration from overwriting the currently live
page. Fingerprinted assets retain their names and receive
`public, max-age=31536000, immutable`. Other assets, such as an SVG favicon,
receive bounded cache directives. Pages reference their versioned keys, so every referenced object exists before delivery begins.

Page objects never receive `immutable`. Their `Cache-Control` contains
`s-maxage`, `stale-while-revalidate` and `stale-if-error`. Browser `max-age`
is clamped to `revalidate`. Object entries retain content type and hash CSP.
Cookies are excluded. `Vary` may name only `X-Inertia` and `X-Inertia-Version`;
other fields and `*` refuse publication. Request cache directives compare
case-insensitively before storage lookup.

```rust
use std::time::Duration;
use suprnova::static_publish::PublishConfig;

let policy = PublishConfig {
    revalidate: Duration::from_secs(60),
    browser_max_age: Duration::from_secs(10),
    s_maxage: Duration::from_secs(60),
    stale_while_revalidate: Duration::from_secs(30),
    stale_if_error: Duration::from_secs(300),
    concurrency: 4,
    ..PublishConfig::default()
};
```

`_pub/manifest.json` records `live` and a `versions` map. Each version records
its asset version, publication time, retirement time and objects. Each object
records its storage path, public URL, SHA-256 digest, asset version, dependency
tags and response headers. Previous versions remain until their grace expires.

All versioned uploads finish before the manifest PUT. Unchanged digests skip
uploads. A CDN driver selects `_pub/<asset-version>/<revision>/` as the page
origin path. Without a driver, stable `<path>/index.html` and
`<path>/index.json` aliases are written one at a time after the manifest commit.
The manifest is the origin switch; stable bucket aliases are a post-commit mirror that converges.

## Publishers in separate processes

Publication and pruning acquire an epoch record under `_pub/lease/` before
reading the manifest or planning work. Keys use a zero-padded 20-digit unsigned
integer, such as `_pub/lease/00000000000000000001.json`. A record contains
`owner`, `epoch` and `expires_at_ms`. The highest epoch is the current lease;
foreign keys are ignored and no lease key is served by the origin.

Installing static publication reserves the `/_pub/lease/` URL prefix for
GET and HEAD requests. Such a request below it receives 404 before application
routing, so a GET or HEAD application route declared there is never reached.
Other methods are not intercepted.

If no record exists, acquisition atomically creates epoch 1. An unexpired
highest record refuses another publisher. An expired record permits atomic
creation of the next epoch. A competing creation of that same key refuses as
held. Acquisition never deletes or replaces a record. Takeover needs only
`write_with_if_not_exists` or `write_with_if_none_match` and works the same on
every disk. Every publisher sharing the disk must use this epoch protocol.
Fs lease creation uses the registered-disk confined exclusive-create primitive
at the operator's root, including for application operators with `atomic_write_dir`.
Application operator layers still handle publication data, lease reads and release;
lease creation uses the confined operator directly. A root that cannot support
this primitive refuses setup and names `Storage::register_fs` in the diagnostic.

Verification lists the chain and checks the highest epoch, owner and acquired
expiry. Release overwrites only the holder's own record with expiry zero, so
the next acquisition advances the epoch instead of reusing its key. Bounded
Drop cleanup does the same; failed release leaves expiry as the fallback.
Prune verifies ownership before deleting records below the highest, which it
always retains. All other keys in the lease namespace stay outside orphan
cleanup.

The manifest retains its separate conditional write: `write_with_if_match`
uses the observed manifest ETag when available; otherwise a byte comparison
detects a changed manifest. Atomic epoch creation does not require conditional
replacement. A process that ignores the lease must not write the publication
disk.

Hosts must use NTP and keep their clock difference below
`STATIC_PUBLISH_CLOCK_SKEW` seconds, which defaults to 10; non-integer settings refuse publication setup. A holder stops using
its lease at `expires_at - skew`; a taker waits until `expires_at + skew`.
Rollback verifies ownership first. If ownership changes or the holder's usable
window expires, it leaves the CDN to the new owner and records the failure.

```bash
STATIC_PUBLISH_CLOCK_SKEW=10 cargo run --bin console -- static:publish
```

Mirror writes, prune deletions and prune manifest commits verify ownership
before each operation; a lost lease stops maintenance and reports the current
owner.

Run the same commands with the application's `console` binary, from the server deployment or a console process:

```bash
cargo run --bin console -- static:publish
cargo run --bin console -- static:status
```

The publisher releases its lease before bounded status persistence, so a slow
status write cannot consume the lease release budget.

A refused lease appears in status. Boot continues serving. An interrupted
manifest PUT is reconciled by reading the durable manifest: the attempted live
version means success; the predecessor means failure and a fenced rollback.
A failed read or a third version records `outcome: "unknown"`, `previous_version`
and `attempted_version` in status and leaves the CDN unchanged. The next
publication reads the manifest fresh and checks `CdnDriver::selected_version`
before rendering. A different or unknown selection calls `switch_version` for
the manifest's live version, independently of cached status. CloudFront reads
its configured origin path; custom drivers may return unknown. The first
publication has no live version to restore.

Mutable assets and the fingerprinted dependencies in their relative asset graph
form one set at `_pub/<asset-version>/_assets/<set-digest>/<public path>`.
The set digest is SHA-256 over the JSON array of sorted public path and content
digest pairs. HTML and JSON reference set members at these paths, so relative
CSS URLs and module imports resolve inside the set. Page-only changes reuse the
set without uploading or copying assets. A changed set writes every member once,
using server-side copy for unchanged members when the disk supports it, or upload
otherwise. Fingerprinted assets also retain their canonical names.

Each manifest version records `asset_set_digest` and a `page_asset_sets` map of
page URLs to set roots. Omitted pages keep their set references in a partial
publication. Prune removes a set directory only when no retained version
references it; the live version and retired versions inside grace retain theirs.

## CDN drivers

`CdnDriver` is the single publish-side interface. It switches a version,
advertises purge capabilities and purges tags, URLs or a prefix. The in-memory
driver records operations for tests. Cloudflare and CloudFront drivers ship with
the framework. An application can add another provider by implementing this trait.

```rust
use std::sync::Arc;
use suprnova::static_publish::{Publisher, PublishConfig};
use suprnova::static_publish::cdn::MemoryCdnDriver;

let driver = Arc::new(MemoryCdnDriver::default());
let publisher = Publisher::new("pages", "build-42", PublishConfig::default())?
    .with_cdn(driver);
```

### Enable a CDN driver

Both drivers are on by default: `cdn-cloudflare`, which adds no dependency,
and `cdn-cloudfront`, which shares the AWS signing stack with the S3 disk and
adds `quick-xml`. Cloudflare with R2 is the recommended setup; R2 charges no
egress. An application built with `default-features = false` lists the
feature for the driver it constructs, or
`suprnova::static_publish::cdn::CloudflareDriver` or `CloudFrontDriver` does
not exist and the build names the missing item.

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v4.6.2", default-features = false, features = ["filesystem", "cdn-cloudflare"] }
```

### Cloudflare and R2

Use an R2 bucket as the publication disk and a Worker on the site's hostname.
Keep that hostname's DNS record pointed at the application and enable Cloudflare's
proxy. A Worker route intercepts requests before they reach the application.
An R2 custom domain alone cannot send unpublished paths to the application.

Register the bucket through `S3Config`. R2 uses its account endpoint and the
`auto` region. Supply static access and secret keys from an R2 API token with
read and write access to the bucket. The separate zone API token needs Cache
Purge permission. Read these credentials from the application's configuration
and pass them to the driver; the driver reads no environment variables.

```rust
use std::{sync::Arc, time::Duration};
use suprnova::{FrameworkError, S3Config, Storage};
use suprnova::static_publish::{PublishConfig, Publisher};
use suprnova::static_publish::cdn::CloudflareDriver;

fn cloudflare(
    account_id: &str,
    access_key: &str,
    secret_key: &str,
    zone_id: &str,
    purge_token: &str,
) -> Result<Publisher, FrameworkError> {
    let s3 = S3Config {
        bucket: "public-pages".into(),
        endpoint: Some(format!("https://{account_id}.r2.cloudflarestorage.com")),
        region: Some("auto".into()),
        access_key_id: Some(access_key.into()),
        secret_access_key: Some(secret_key.into()),
        root: Some("pages".into()),
    };
    Storage::register_s3("pages", s3.clone())?;
    let driver = CloudflareDriver::new(zone_id, "www.example.com", purge_token, &s3)?
        .with_manifest_ttl(Duration::from_secs(5))?;
    Ok(Publisher::new("pages", "build-42", PublishConfig::default())?
        .with_cdn(Arc::new(driver)))
}
```

Write the shipped Worker to the deployment directory with a small binary:

```rust
use suprnova::static_publish::cdn::CLOUDFLARE_WORKER;

fn main() -> std::io::Result<()> {
    std::fs::write("cloudflare_worker.js", CLOUDFLARE_WORKER)
}
```

Create `wrangler.toml` beside that file, then run `wrangler deploy`:

```toml
name = "suprnova-publication"
main = "cloudflare_worker.js"
compatibility_date = "2026-10-23"
routes = [{ pattern = "www.example.com/*", zone_name = "example.com" }]

[[r2_buckets]]
binding = "BUCKET"
bucket_name = "public-pages"

[vars]
ROOT_PREFIX = "pages"
MANIFEST_TTL_SECONDS = "5"
```

`ROOT_PREFIX` must match `S3Config.root`; use an empty string when the disk has
no root. The constructor refuses the same ambiguous root segments as CloudFront.
The Worker reads `<root>/_pub/manifest.json` and selects its `live` revision.
It serves GET and HEAD from R2, returns ETag-based 304 responses and restores
native HTTP metadata and validated public headers. It preserves CSP metadata
and applies the same restrictive default CSP as the CloudFront function when
HTML has none. Other methods, missing objects and missing or unreadable manifests
go to the application with `fetch(request)`. The manifest and status objects
are never served from R2.

The Worker maps page paths to versioned HTML, `/index.json` paths to versioned
JSON, and asset paths directly to their bucket keys. When the manifest includes
a page inventory, its recorded keys take precedence. This supports the current
publisher's flat page stems as well as directory-style `index.html` keys.
Fingerprint names and `/_pub/<version>/_assets/...` paths retain the disk root.
The Worker uses Cloudflare's Cache API to cache R2 responses under their unchanged
public URLs. Requests with query strings read R2 directly because the publisher's
purge URLs have no query dimension. This avoids Worker-defined custom cache keys,
which [single-file purge cannot clear](https://developers.cloudflare.com/workers/reference/how-the-cache-works/#purge-assets-stored-with-the-cache-api).
Each cached entry records its object key internally, so a manifest change cannot
reuse a previous revision's page. That internal header is removed before delivery.
The Worker attaches dependency Cache-Tag headers to cached entries for tag purges.
It never caches application fallback responses. Cache writes run through
`waitUntil` when the runtime provides a request context. This follows the
[R2 Cache API example](https://developers.cloudflare.com/r2/examples/cache-api/).
The Cache API honors the shared cache lifetime but does not implement
`stale-while-revalidate` or `stale-if-error`; see its
[runtime contract](https://developers.cloudflare.com/workers/runtime-apis/cache/).

The manifest cache defaults to five seconds per Worker isolate. Set
`with_manifest_ttl` to the same duration as `MANIFEST_TTL_SECONDS`.
`selected_version` returns unknown because the manifest selects the version.
Switching waits one TTL without changing Cloudflare configuration. The current
publisher calls the switch before committing its manifest, so the driver also
waits one TTL at purge time, after commit. This keeps the purge behind cached
manifest selections. Include both waits in the publication timeout; recovery
realignment can add another switch wait before rendering. Rollback
has no provider configuration to restore; the durable manifest remains authoritative.

The driver advertises URL and prefix purges, so the publisher purges each
republished page by URL: full HTTPS URLs in batches of 100, the per-request
limit on Free, Pro and Business (Enterprise allows 500). Single-file rate
limits are 800 URLs per second on Free, 1500 on Pro and Business, and 3000 on
Enterprise. Cloudflare also purges by tag on every plan, but a tag purge clears
only responses that carried a `Cache-Tag` header, which the shipped Worker does
not set; an application whose Worker adds the header can call
`purge(Purge::Tags(...))` itself. Prefix and tag purge requests are limited to
5 per minute on Free, 5 per second on Pro, 10 per second on Business and 50
per second on Enterprise, with at most 100 operations per request; tags are
batched to that.
Prefix values contain a host and path without a scheme, query or fragment.
Provider refusals include the HTTP status and first error code and message,
with the token redacted. See the [purge API](https://developers.cloudflare.com/api/resources/cache/methods/purge/),
[single-file guide](https://developers.cloudflare.com/cache/how-to/purge-cache/purge-by-single-file/)
and [current plan limits](https://developers.cloudflare.com/cache/how-to/purge-cache/#availability-and-limits).

### CloudFront

CloudFront uses AWS signed requests with the same credential provider chain
as the S3 disk. It reads the complete distribution configuration and its ETag,
changes the selected origin's path, sends `If-Match`, waits for deployment and
creates invalidations. Static keys can be passed from `S3Config`; otherwise
the AWS profile or instance role supplies credentials.

The default `cdn-cloudfront` feature supplies the driver independently of SQS.
Register the publication disk and construct the driver from its configuration:

```rust
use std::{sync::Arc, time::Duration};
use suprnova::{FrameworkError, S3Config, Storage};
use suprnova::static_publish::{PublishConfig, Publisher};
use suprnova::static_publish::cdn::CloudFrontDriver;

fn cloudfront(s3: S3Config) -> Result<Publisher, FrameworkError> {
    Storage::register_s3("pages", s3.clone())?;
    let driver = CloudFrontDriver::new("E123EXAMPLE", "public-pages", &s3)?
        .with_response_headers_function(
            "arn:aws:cloudfront::123456789012:function/suprnova-public-headers",
        )?;
    let policy = PublishConfig {
        timeout: Duration::from_secs(900),
        ..PublishConfig::default()
    };
    Ok(Publisher::new("pages", "build-42", policy)?.with_cdn(Arc::new(driver)))
}
```

`public-pages` is the distribution's origin ID. An S3 root of `pages` selects
`/pages/_pub/<version>`; the constructor refuses ambiguous root segments. The driver preserves other
origins and configuration fields. A failed switch or manifest write restores
the previous origin version, including the original distribution configuration
when the first publication fails. Rollback failures appear in the publication
error. A custom driver implements `rollback_version` to restore its initial
configuration as well.

Configure the distribution to map `/about` to `/about.html` and
`/about/index.json` to `/about.json`. Send fingerprinted asset paths and `/_pub/*` asset paths to the
asset origin without the page version prefix, retaining the S3 root prefix. An origin-path switch changes
the prefix; it does not create these viewer-request rewrite rules.

S3 returns custom headers as `x-amz-meta-*`. Those names do not enforce CSP.
Publish the supplied viewer-response function once. Write its source with:

```rust
use suprnova::static_publish::cdn::RESPONSE_HEADERS_FUNCTION;

fn main() -> std::io::Result<()> {
    std::fs::write("public-headers.js", RESPONSE_HEADERS_FUNCTION)
}
```

Create and publish the function with the AWS CLI:

```sh
aws cloudfront create-function --name suprnova-public-headers \
  --function-config 'Comment=PublicHeaders,Runtime=cloudfront-js-2.0' \
  --function-code fileb://public-headers.js
aws cloudfront publish-function --name suprnova-public-headers \
  --if-match ETAG_FROM_CREATE
```

Associate its ARN with the `viewer-response` event on each cache behavior
that uses the public page origin. Pass that ARN to
`with_response_headers_function`. The driver refuses a switch if any matching
behavior lacks the function. The function replays every validated recorded header, including Content-Disposition,
Reporting-Endpoints and HSTS, then removes the metadata headers. Headers the
viewer-response function cannot carry refuse publication before switching.
The origin applies HSTS only for trusted HTTPS delivery. It blocks scripts in an HTML object
that has lost its CSP metadata. This follows the
[CloudFront response event format](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/functions-event-structure.html).

Storage writes delivery policy headers as metadata: Cache-Control, CSP, Vary,
HSTS, reporting, content type protection, referrer and permissions policies,
cross-origin policies and validated application headers. Content-Type is native
storage metadata. Content-Length, ETag and Content-Encoding belong to the served
representation; the CloudFront function preserves them after compression.

A local static server or another CDN must likewise serve the manifest's
`headers` as HTTP headers. A plain S3 website exposes custom headers only as
metadata; place a header bridge in front of it before serving public documents.
The Suprnova origin fallback already serves the headers directly. Header restrictions
follow the [AWS edge function contract](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/edge-function-restrictions-all.html).

Fingerprint classification uses the same default file-stem pattern as StaticFiles:
a hyphen followed by eight base64url characters with a digit, uppercase letter,
underscore or hyphen. Names such as `brand-background.svg` remain mutable.
Select another build convention explicitly:

```rust
use suprnova::static_publish::{Publisher, PublishConfig};
let publisher = Publisher::new("pages", "build-42", PublishConfig::default())?
    .fingerprint(r"\.release$")?;
```

## CSP and private requests

Nonce documents cannot be published. Select the public hash mode described in
[Security Headers](security-headers.md). It hashes the trusted document's
executable inline scripts. Public pages contain no nonce attributes. Private
pages retain their fresh per-request nonce. A later injected script does not
gain a hash source.

```rust
let headers = suprnova::SecurityHeaders::balanced().hash_public_pages();
suprnova::middleware::register_global_middleware(headers);
```

All three [starter kits](starter-kits.md) install the same static HTTP client.
The document lists public page JSON URLs. Public navigation fetches these
objects while preserving the canonical page URL. Partial reloads, polling,
deferred and once props, and mutations use the live URL with
`__suprnova_origin=1` and `Cache-Control: no-store`. A version mismatch reloads
the document. Flash and validation errors arrive through this private path.

The client reads `XSRF-TOKEN`. If the cookie is absent before a mutation, it
makes a same-origin private GET first. The existing CSRF middleware issues the
cookie and accepts its `X-XSRF-TOKEN` header. Public objects contain no CSRF
token. See [Inertia Responses](frontend-inertia-responses.md).

## Regenerate after writes and by time

Generation writes keep RenderCache's transactional advancement. A successful
framework commit queues affected dependency tags. Rollback discards the work.
The worker coalesces queued tags and renders dependent HTML and JSON through
the original publication pipeline. It purges through the installed driver
after the switch. Without a purge driver, the edge lifetimes bound staleness.
The configured revalidation interval also refreshes pages by time.

`Publisher::publish` and `publish_with` replace the complete inventory, so
removed routes and assets can retire. `publish_partial` and
`publish_partial_with` retain omitted pages and assets during a targeted
regeneration. Boot, the console command and timed revalidation publish the full
inventory; dependency regeneration and first-visit fallback publish a subset.

```rust
use std::sync::Arc;
use suprnova::{FrameworkError, Schedule, static_publish::StaticPublication};

fn scheduled(publication: &Arc<StaticPublication>, schedule: &mut Schedule) -> Result<(), FrameworkError> {
    publication.schedule(schedule)
}
```

Bind the application's `Arc<dyn BroadcastHub>` before installing publication to
carry committed dependency tags from standalone workers to serving nodes. The
publication subscriber uses the same configured hub transport as other framework
broadcasting and queues each received tag for the serving worker. For example:

```rust
use std::sync::Arc;
use suprnova::{App, BroadcastHub, InMemoryBroadcastHub};
let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
App::bind::<dyn BroadcastHub>(hub);
```

Use a cross-process hub driver in a worker deployment. Without a configured hub,
notifications remain process-local and the revalidate interval bounds freshness.
A disconnected or lagging hub also falls back to timed revalidation and logs the
failure. Only dependency digests travel through the hub, never rendered props.

## Prepare images

Publication resolves local image URLs against the document path. Posters, image
preloads and other collected raster references use the same preparation path.
Without the media feature, raster references refuse publication with a route
diagnostic. It emits
fingerprinted WebP objects at quality 85 and keeps a metadata-stripped original
fallback. Picture sources preserve responsive descriptors, media and sizes.
Quality must stay between 85 and 100. AVIF requires explicit enablement and
the ImageMagick driver with an AVIF codec.

```rust
use suprnova::static_publish::images::ImageOptions;

let mut images = ImageOptions::default();
images.keep_png.insert("icons/app.png".into());
publication.images(images)?;
```

`data-static-keep-png`, configured `keep_png` paths, linked PNG icons and web
manifest PNG icons use oxipng Maximum with Zopfli. Metadata is stripped from
every prepared raster. Absolute URLs on the application's public origin use the local source disk.
Raster data URLs are decoded and published as metadata-stripped objects, with
modern variants and a rewritten fallback just like local images. An invalid
raster data URL refuses publication with the route and cause; the origin keeps
serving the page. Third-party images are neither transcoded nor published.
Linked stylesheets are build outputs published byte for byte; their images belong
to the build, including small images Vite inlines below `assetsInlineLimit`.
Raster data URLs and same-origin raster `url(...)` references in document style
attributes or `<style>` elements refuse publication with the route and element;
move the image into an `<img>`, a `<picture>` or a stylesheet file.
Non-raster data URLs stay unchanged. SVG and ICO files
keep their existing representation. See [Images](images.md).

The collector reads the complete Vite inventory before image preparation.
A raster referenced only by a Vite entry or CSS keeps its key and its original
format. The media pipeline strips its metadata and recompresses retained PNGs
with the Maximum preset. Without the media feature, these dependencies refuse
publication with the route and the reason.

## Prune retired objects

Every successful attempt prunes, including an unchanged inventory. The same pass is available on
demand. Reports list removed and kept keys. Live objects, stable aliases and
versions inside the grace period stay protected. Grace starts when a version
stops being live. Prune commits removal of expired version records before any
object deletion. A failed deletion leaves orphans rather than dangling manifest
references. Set reuse consults only versions still in the manifest. The next
prune sweeps unreferenced asset-set directories, including failed staging sets,
within the publication deadline. Other objects with no reliable age stay
protected. A prune failure does not remove the live version or undo its switch.

```bash
cargo run --bin console -- static:prune
cargo run --bin console -- static:status
```

### Why Suprnova diverges

Laravel does not publish Inertia routes to object storage itself. Packages
provide that behavior. Suprnova uses its typed props declarations, transactional
dependency collector and named disk drivers. Separate HTML and JSON keys avoid
depending on an object store varying responses on `X-Inertia`.

The origin trusts a forwarded HTTPS scheme only from the proxies named in
`APP_TRUSTED_PROXIES`; `APP_TRUSTED_PROXIES` is empty by default, so a deployment
behind a TLS-terminating proxy must list it or HSTS is never sent.

## Next

- [RenderCache](render-cache.md)
- [Security Headers](security-headers.md)
- [Starter Kits](starter-kits.md)
