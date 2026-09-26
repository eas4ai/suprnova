# Feature map exclusions

Source at d03b4f1. What the extractors saw and deliberately did not list, with the reason.

## Environment-like names that are not runtime configuration

Read at build time through `env!` / `option_env!`, set by Cargo; listed so nothing is dropped silently:

- `CARGO_MANIFEST_DIR` · suprnova-macros/src/inertia.rs:229
- `CARGO_PKG_NAME` · suprnova-macros/src/live/live_impl.rs:488
- `CARGO_PKG_VERSION` · framework/src/render_cache/config.rs:636
- `OUT_DIR` · suprnova-cli/src/commands/live_add.rs:44

Uppercase string literals that are not environment variables (reviewed):

- `VEC_DISTANCE_COSINE` · framework/src/vector/mariadb.rs:152
- `VEC_DISTANCE_EUCLIDEAN` · framework/src/vector/mariadb.rs:153

## Commands

Excluded as demo-app commands (`app/src/commands`): `bench:enqueue-abort`, `bench:enqueue-records`, `bench:enqueue-sleep`, `bench:password-hash`, `bench:verify-records`, `bench:verify-ticks`, `greet`

## Macro parser keywords that are internal

- `__eager / __pivot`: hidden eager-load and pivot carrier fields the model macro injects
- `__suprnova_live`: internal twin of #[live] used by generated code
- `primitive type names (bool, i64, u64, f32, ...)`: type classification inside Data and Live codecs
- `self / super`: path rewriting in #[suprnova::view] field access
- `suprnova`: crate-path detection in #[handler] parameter classification
- `id`: default key name inside relation code generation
- `static`: `'static` detection in #[service]
- `suprnova_test`: the attribute recognising itself
- `form_request / multipart / data / factory / mail / console / live / field`: the attribute names themselves
- `bool (policy.rs)`: return-type classification in #[policy]

## Rust items

- Private, `pub(crate)` and `#[doc(hidden)]` items: rustdoc removes them before extraction.
- Implementations of external traits (`Debug`, `Clone`, `Serialize`, ...).
- Demo application code (`app/`), test-support crates and fixtures.
- Re-exports of sibling Suprnova crates (222): each is noted as "re-exported as" on the item it names instead of being listed twice.

