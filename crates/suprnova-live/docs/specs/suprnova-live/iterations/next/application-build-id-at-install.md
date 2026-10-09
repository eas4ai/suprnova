# The application supplies its own build id at install -- promoted to iteration 006

Status: Promoted (iteration 006, 2026-09-08)
Captured: 2026-09-08
Promoted: 2026-09-08
Target domain: `15-render-representations-and-storage.md`

## What it is

`RenderCache::install` SHALL take the application's build id, or derive it in
the application crate, so that the default build id changes with the
application rather than with the framework crate. Two installs carrying
different application build ids SHALL NOT share a stored entry. The rustdoc and
the deployment chapter SHALL name where the value comes from, so an operator
can tell what a deployment actually changed.

The default `APP_BUILD_ID` is the framework crate's `CARGO_PKG_VERSION`, read
by an `env!` expansion that sits inside the framework itself
(`framework/src/render_cache/config.rs`). That value equals the application's
version only under workspace versioning, and it never changes from one deploy
to the next, so a deployment that changes rendering without changing the
framework version keeps the same build id and the same keys. The deployment
chapter states this and recommends an explicit per-deploy value; ruling R28
recorded the framework-crate default as the present behavior and the
application-supplied value as the correction.

## Acceptance criteria

- `RenderCache::install` SHALL take the application's build id, or derive it in
  the application crate, so the default no longer equals the framework crate's
  version.
- A test SHALL prove that two installs with different application build ids
  never share an entry.
- The rustdoc and the deployment chapter SHALL name the source of the value.
