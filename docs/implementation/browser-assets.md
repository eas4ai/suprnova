# Live browser assets

## Artifact contract

`rtk npm --prefix browser run build` produces four deterministic files:

- `suprnova-live.esm.js`, the module runtime;
- `suprnova-live.classic.js`, the classic bootstrap runtime;
- `index.d.ts`, the public TypeScript contract; and
- `suprnova-live.assets.json`, the immutable asset manifest.

The manifest names the engine/runtime/protocol/snapshot versions and records
each JavaScript file's byte length, SHA-256 digest, SRI value, content type,
script kind, preload relationship, and cache policy. Its timestamp is fixed by
the build contract. `build:check` performs two clean temporary builds and
requires identical names and bytes; `generate:check` independently rejects
drift in Rust-derived browser contracts.

The ESM and classic artifacts expose one runtime. Applications choose one
delivery form per document; loading both does not create two runtimes, but is a
deployment mistake and wastes bytes. Source maps and floating dependency
resolution are excluded from the production artifacts.

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
passes the exact origin at boot. Asset authorization, compression, CDN purge,
and framework route registration belong to the eventual Suprnova host adapter.

## Dependency notices

Idiomorph 0.7.4 is the only production browser dependency and is bundled
privately behind the morph adapter. The JavaScript banner and asset manifest
retain its name, exact version, 0BSD license, and bundled status. It is not an
application-facing API.

`THIRD_PARTY_LICENSES.md` is generated from all locked Cargo and npm packages.
The npm graph labels production runtime, production build, test-only, and
development-tooling reachability. Terser and esbuild are production build
tools; Playwright, axe-core, Vitest, fast-check, and the Stimulus conformance
dependency do not become runtime dependencies. The unattended gate rejects a
stale inventory.
