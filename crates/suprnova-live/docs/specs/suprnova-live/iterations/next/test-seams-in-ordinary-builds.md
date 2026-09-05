# Test seams compiled into ordinary builds -- staged for next iteration

Status: Staged (not in current contract)
Captured: 2026-09-05
Target domain: `conventions.md` (framework-wide build and verification
convention), with `15-render-representations-and-storage.md` as the first affected
domain

## What it is

The framework's `testing` cargo feature is a default feature, and every
test seam the framework offers is gated on `any(test, feature = "testing")`.
An ordinary `cargo build` of an application therefore compiles those seams
into the production binary. The RenderCache Tier 0 foundation added several
under that existing convention: `render_cache::middleware::race_points`
(the `AFTER_REREAD` and `EPOCH_CAPTURED` race points, one relaxed atomic
load per covered request when disarmed),
`render_cache::collector::strip_classification_reasons_for_test` (which can
only make the unreasoned-private-class invariant decline, never weaken a
guard), `render_cache::testing::policy_table`, the `_for_test` operators on
`RenderCache` (`key_for_route_for_test`, `inspect_route_for_test`,
`inspect_l1_for_test`, `lease_count_for_test`), `middleware::key_input_for_test`,
`RenderCacheConfig::with_clock_for_test` and `with_coordinator_for_test`, and
the `render_cache::console` `_for_test` report builders. None of them is
reachable from application code by accident (each is `#[doc(hidden)]` and
named `_for_test`), and the race suite compiles to nothing when the feature
is off, so this is a build-shape concern, not a correctness one.

A future revision MAY define a release profile, or a documented build
recipe, that turns `testing` off for production binaries, so that seams of
this kind exist only in test builds. Any such change is framework-wide:
`testing` also gates encryption-key installation, the
force-the-next-encrypt-call-to-fail switch, request query overrides, and
the session test scopes, so removing it from the default set needs the
whole feature matrix rerun and every application template's build
documented, not a RenderCache-local change.

## Acceptance criteria

- A documented production build shape exists in which the `testing`
  feature is off, and the framework, the CLI scaffold's generated
  application, and the dogfood application all build and boot in it.
- Every seam named above is absent from a binary built that way, checked
  by a test or a build assertion rather than by reading.
- The race suite and every `_for_test` consumer still compile and pass
  under the default (testing-on) build, so day-to-day verification is
  unchanged.
- The feature matrix step of the repository gate covers the testing-off
  build of the framework crate.
