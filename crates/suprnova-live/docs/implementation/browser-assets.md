# Live browser assets

## Artifact contract

`rtk npm --prefix browser run build` produces ten deterministic files:

- `suprnova-live.esm.js`, the module runtime;
- `suprnova-live.classic.js`, the classic bootstrap runtime;
- `suprnova-live.stimulus.esm.js` and
  `suprnova-live.stimulus.classic.js`, the optional Stimulus adapter;
- `suprnova-live.uploads.esm.js` and
  `suprnova-live.uploads.classic.js`, the optional upload feature;
- `suprnova-live.async.esm.js` and
  `suprnova-live.async.classic.js`, the optional asynchronous feature;
- `index.d.ts`, the public TypeScript contract; and
- `suprnova-live.assets.json`, the immutable asset manifest.

The manifest names the engine/runtime/protocol/snapshot versions and records
each JavaScript file's byte length, SHA-256 digest, SRI value, content type,
script kind, preload relationship, and cache policy. Its timestamp is fixed by
the build contract. `build:check` performs two clean temporary builds and
requires identical names and bytes; `generate:check` independently rejects
drift in Rust-derived browser contracts.

The core ESM and classic artifacts expose one runtime. Applications choose one
delivery form per document; loading both does not create two runtimes, but is a
deployment mistake and wastes bytes. Optional pairs register before core boot
through the same exact lifecycle driver and do not start another runtime.
Stimulus uses a singleton beside the fixed upload/async slots. Source maps and
floating dependency resolution are excluded from the production artifacts.

## Serving and CSP

The host should fingerprint or address assets by manifest identity and serve
the recorded `public, max-age=31536000, immutable` cache policy. ESM uses
`modulepreload`; classic script preload uses `preload` with the script
destination. Emit the manifest SRI value and the exact recorded JavaScript
content type. The inert JSON configuration is document data, not executable
JavaScript.

The runtime is compatible with a strict policy built around `script-src
'self'`: it uses no `eval`, `new Function`, inline event handler, injected
executable script, or arbitrary module URL. A deployment may use a nonce or
hash for its own bootstrap policy. Prefetch speculation rules are created only
when supported and permitted; link-prefetch fallback remains available, and a
CSP refusal is a bounded enhancement failure rather than an action failure.

Cross-origin Live endpoints are rejected unless the embedding application
passes the exact origin at boot. Asset authorization, compression, and CDN
purge remain application-host responsibilities.
Iteration 005's Suprnova host adapter owns asset authorization, compression, CDN
purge, and framework route registration; the framework delivery path below is
exercised by real framework asset-route tests and by a Playwright scenario
against a real Suprnova server.

## Suprnova delivery

The ten build outputs are tracked under `browser/dist/` and embedded into the
engine crate by `suprnova_live::artifacts`. On first use the engine validates
the manifest against the embedded bytes: exact schema, runtime, protocol, and
snapshot versions; every role recorded once with its contracted file name,
capability, execution kind, preload relationship, media type, and cache
policy; and a byte length, SHA-256 digest, and integrity value that match the
bytes. Any drift fails closed before a byte is served. The reproducible build
check plus the tracked bytes mean a rebuilt `dist/` must be byte-identical to
what the engine embeds.

`Router::try_live()` serves the artifacts at
`/__live/v1/assets/<asset_identity>/<file>` for `GET` and `HEAD`. The asset
identity is `suprnova-live-<runtime version>-<first sixteen hex characters of
the manifest digest>`, so the URLs are immutable and carry the recorded
`public, max-age=31536000, immutable` policy; the manifest itself is served
with `must-revalidate`. Every response carries an exact `Content-Type`,
`Content-Length`, a strong `ETag` equal to the quoted SHA-256 digest,
`X-Content-Type-Options: nosniff`, and honours `If-None-Match` with 304. A
wrong identity, an unknown or differently cased name, a query string, or a
path segment that is not a recorded file is a closed 404 with no body; other
methods are 405 with `Allow: GET, HEAD`. Two framework-owned boot scripts,
`suprnova-live.boot.esm.js` (`import { boot } ...; boot();`) and
`suprnova-live.boot.classic.js` (`window.SuprnovaLive.boot();`), are served
the same way with their own integrity values, so a document loads only
external scripts.

A document calls `LiveDocument::bootstrap(LiveBootstrapOptions)` after its
last mount and inserts the returned markup in `<head>` through
`|trusted_html`. The markup is the inert `suprnova-live-config` JSON element
(canonical key order, `endpoint` `/__live/v1/action`, the asset identity, the
protocol range, and bounded limits; the response budget is the configured
`LiveConfig` limit bounded to the runtime's accepted 1 KiB to 4 MiB range)
followed by one delivery form: for ESM a
`modulepreload` link for the core, one `type="module"` script per optional
role, and the boot module; for classic a `preload` link, deferred optional
scripts, the deferred core, and the deferred boot script. Every artifact tag
carries its manifest integrity value and `crossorigin="anonymous"`; an
optional nonce is stamped on script elements. The upload role is emitted when
a mounted component declares an upload policy, the asynchronous role when a
component declares streams, and the Stimulus bridge only with
`with_stimulus()`. Roles are a set, so repeated islands never duplicate a
tag; a second `bootstrap()` call or a mount after bootstrap fails closed.

## Dependency notices

Idiomorph 0.7.4 is the only production browser dependency and is bundled
privately behind the morph adapter. The JavaScript banner and asset manifest
retain its name, exact version, 0BSD license, and bundled status. It is not an
application-facing API.

The Stimulus adapter contains only Suprnova's structural bridge and continuity
logic. It never imports or bundles `@hotwired/stimulus`; applications provide
their own compatible `Application`.

`THIRD_PARTY_LICENSES.md` is generated from all locked Cargo and npm packages.
The npm graph labels production runtime, production build, test-only, and
development-tooling reachability. Terser and esbuild are production build
tools; Playwright, axe-core, Vitest, fast-check, and the Stimulus conformance
dependency do not become runtime dependencies. The unattended gate rejects a
stale inventory.
