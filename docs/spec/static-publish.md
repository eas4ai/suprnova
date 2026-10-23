# Static publication of public Inertia pages

Status: Agreed 2026-10-21
Prefix: PUB

Drafted 2026-10-21 from the developer's rulings in conversation: "I think
that the static inertiajs should be done with a hook at build to S3 or
whatever CDN is supported"; "so build the asset -> upload to CDN (wait) ->
Proceed to serve"; "The coolify health check should not be held up by the
image serving... those things should be done once the image is up... Like
an application boot not the image boot"; "yes all of those" to the failed
publication leaving the previous one live and to regeneration reusing the
same sequence; "quality should not be below 85%. It produces blurry images
below 85%"; "oh well pngcrush then" and the oxipng maximum preset,
"confirmed" at 23:27. The model is fydellys/inertia-static
(`reference/inertia-static-main`), read for its rules, not its API. This
file delivers backlog item `public-inertia-delivery` (from CACHE-001) and
the public half of `inertia-render-cache`; the baseline measurement is
the Pingdom capture of suprnova.app on 2026-10-18 (`reference/har.json`).

## Observed at bfcbd61fe

Status: Observed

- RenderCache admits configured GET and HEAD routes through
  `RenderCachePolicyTable` and `RenderCacheMiddleware`
  (`framework/src/render_cache/registry.rs:77,109`,
  `framework/src/render_cache/middleware.rs:849`). Public responses carry
  `public`, `max-age` and `s-maxage`; the writer emits no
  stale-while-revalidate and no surrogate directive
  (`crates/suprnova-live/src/render_cache/http.rs:71`).
- Every Inertia resolve stages session values and records a session read
  even when no session exists (`framework/src/inertia/response.rs:199,1372`;
  `framework/src/session/middleware.rs:515`). Inertia adds
  `Vary: X-Inertia`, which the cache's key dimensions do not cover, so
  publication is declined (`framework/src/inertia/headers_middleware.rs:118`;
  `crates/suprnova-live/src/render_cache/variance.rs:44`).
- Dependency generations advance in the writing transaction (CACHE-009)
  and identify dependencies, not published objects
  (`crates/suprnova-live/src/render_cache/generation.rs:58`).
- Storage disks are OpenDAL operators: local, S3, GCS and Azure
  (`framework/src/filesystem/mod.rs:179,300,349`). The CSRF middleware
  issues an `XSRF-TOKEN` cookie carrying the session's token, which also
  travels in the document (`framework/src/csrf/middleware.rs:169`).
- The starter kits share `auth` on every page and install the session and
  CSRF middleware (`suprnova-cli/src/templates/files/backend/bootstrap.rs.tpl:130,236,251`).
- The media pipeline's default encode quality is 70, Laravel's default
  (`framework/src/media/driver.rs:24`).

## Requirements

[PUB-001] An application MUST mark a GET route as a public page for
publication explicitly, on the route or on a group, and the framework MUST
render a public page without a session: no session read, no `auth`,
flash, errors or per-request credential among its props, shared props
limited to those the application lists as public, and no nonce CSP. A
marked route whose render reads the session, resolves a shared prop that
is not listed public, or runs under a nonce CSP MUST be refused for
publication at boot with a diagnostic naming the route and the cause.
Nothing is published unless marked.
Falsifier: an unmarked route is published; a public render that read the session or a non-public shared prop is published; a nonce-CSP route is published; or the refusal names no route or no cause.
Mechanism: `static-publish`.
Rationale: SEC-005 forbids serving a nonce page from a stored rendering; inertia-static lists the shared props a static render may evaluate (`src/Http/Middleware/RenderStatically.php:63`).
Status: Agreed 2026-10-21

[PUB-002] For every public page the framework MUST publish two objects,
the HTML document and the Inertia page JSON, each under a path that
carries the asset version, together with the fingerprinted assets the
document references, to the storage disk the application names, which
MAY be any configured disk (local, S3, GCS or Azure). An asset whose path
carries its content fingerprint MUST carry
`Cache-Control: public, max-age=31536000, immutable`. A page object MUST
NOT be marked immutable: it is re-published in place when its data
changes, so it MUST carry the edge directives its policy names
(`s-maxage`, `stale-while-revalidate`, `stale-if-error`), a browser
lifetime no longer than its revalidate interval, and no cookie. The
framework MUST write a manifest (path, content digest, asset version,
dependency tags) and MUST switch the live manifest in one write, only
after every object of the publication has finished uploading.
Falsifier: a page has no JSON object, or the JSON shares the HTML's path; a fingerprinted asset lacks the immutable directive; a page object carries `immutable` or a browser lifetime past its revalidate interval; the manifest switches before an upload completes; or a disk other than S3 is refused.
Mechanism: `static-publish`.
Rationale: object storage cannot vary on `X-Inertia`, so inertia-static gives page JSON its own URL (`src/Http/Controllers/PageJsonController.php:25`); the developer, 2026-10-21: "upload to CDN (wait) -> Proceed to serve".
Status: Agreed 2026-10-21

[PUB-003] Publication MUST run as an application boot task after the
listener opens, never before it, so a health check is answered while it
runs; the previous publication MUST stay live until the switch, and its
objects MUST be kept for a configured grace period after it; a
publication MUST upload only objects whose digest differs from the live
manifest, in parallel within a configured bound; and the same publication
MUST be runnable as a console command. A failed or timed-out publication
MUST leave the live manifest unchanged, MUST be reported in the log and
by a status command, and MUST NOT fail the boot.
Falsifier: the listener opens only after publication; the manifest switches during a failed publication; an unchanged object is uploaded again; the previous assets are deleted at the switch; or a failed publication stops the server from serving.
Mechanism: `static-publish`.
Rationale: the developer, 2026-10-21: "those things should be done once the image is up... Like an application boot not the image boot"; Coolify rolled back suprnova.app 0.13.16 on a 70 s boot.
Status: Agreed 2026-10-21

[PUB-004] A committed write that advances a dependency generation of a
published page MUST re-publish that page, HTML and JSON, through the same
render, upload and switch sequence, coalesced to one publication per tag
per request or job and never before the commit; a configured
`revalidate` interval MUST re-publish pages by time; and a `CdnPurge`
trait MUST let a driver purge the CDN in front of the disk by tag, URL or
prefix, with drivers for the CDNs the framework supports, and no purge
required when the objects' edge directives bound staleness.
Falsifier: a committed change leaves a dependent page unpublished past its revalidate interval; a rolled-back write publishes; ten writes to one tag in one request publish ten times; or a purge driver cannot be installed behind the trait.
Mechanism: `static-publish`.
Rationale: CACHE-009 advances generations in the write transaction; inertia-static purges tags, URLs and prefixes for both variants (`src/StaticCache.php:35`, `src/Purger.php:21`).
Status: Agreed 2026-10-21

[PUB-005] On a published page the Inertia client MUST navigate to another
public page by fetching its published JSON object; MUST send partial
reloads, polling, deferred and once props to the origin on a URL that is
never published; MUST reload the document on an asset version mismatch;
and MUST take its CSRF token from a same-origin cookie the framework
issues (`XSRF-TOKEN`, returned as `X-XSRF-TOKEN`), which the CSRF
middleware MUST accept beside the session token. Flash and validation
errors MUST arrive through the private origin response, never from a
published object. The three starter kits MUST ship this through one
shared client core with React, Vue and Svelte adapters.
Falsifier: a navigation between two public pages fetches the page from the origin; a partial reload is answered from a published object; a form on a published page cannot pass CSRF; a published object carries a flash or an error; or one kit lacks behaviour another kit has.
Mechanism: `static-publish`.
Rationale: inertia-static acquires a CSRF cookie before a mutation and routes dynamic requests to a live URL (`js/dist/shared-U2YX7IMO.js:161`; `resources/boost/skills/inertia-static-development/references/client.md:83`).
Status: Agreed 2026-10-21

[PUB-006] A request for a public path the live manifest does not cover
MUST be rendered by the origin and published from that render, so the
next request is served from storage; the origin MUST keep answering every
public route itself, so a CDN with an origin fallback and a development
machine without a publication both work.
Falsifier: an unpublished public path answers 404 at the origin, or its first render is not published.
Mechanism: `static-publish`.
Rationale: inertia-static renders pages that do not exist yet on their first visit (README, "Pages that do not exist yet").
Status: Agreed 2026-10-21

[PUB-007] At publication the framework MUST transcode the raster images a
document references to WebP, and to AVIF when the application enables it,
at a quality not below 85, keeping the original as the fallback; a PNG
the document must keep as PNG (a manifest icon, or an image the
application marks) MUST be recompressed losslessly with oxipng at its
maximum effort with Zopfli; every image MUST be stripped of metadata as
IMG-002 strips uploads; and the published document MUST reference the
transcoded objects.
Falsifier: a published raster is served only as PNG or JPEG when WebP was possible; a transcode runs below quality 85; a kept PNG is published without recompression; or metadata survives publication.
Mechanism: `static-publish`.
Rationale: the developer, 2026-10-21: "quality should not be below 85%. It produces blurry images below 85%"; "oh well pngcrush then", with oxipng `-o max --zopfli` as the equivalent, "confirmed".
Status: Agreed 2026-10-21

[PUB-008] The framework MUST remove from the disk every object no
publication within the grace period references, assets and page objects
alike: a publication prunes after its switch, a console command prunes on
demand, and both MUST report what they removed and what they kept. An
object the live manifest references MUST never be removed, nor an object
of a publication younger than the grace period, and a failed prune MUST
leave the live publication serving.
Falsifier: an unreferenced object older than the grace period survives a publication and the command; an object the live manifest references is removed; an object of a publication inside the grace period is removed; or a failed prune breaks the live publication.
Mechanism: `static-publish`.
Rationale: the developer, 2026-10-21: "we should have some kind of cleanup mechanism in our static"; every build adds new fingerprinted names and leaves the old ones behind.
Status: Agreed 2026-10-21
