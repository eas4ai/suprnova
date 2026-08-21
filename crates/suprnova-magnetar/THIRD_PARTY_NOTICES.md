# Third-party notices

This crate is standalone: it has no runtime dependency on Suprnova, Torii, or
the broker listed below, with one narrow exception behind an optional
feature. `src/oauth/protocol.rs`'s RFC 6749 §5 token-response types are
adapted from `io-oauth` (below); `src/plugins/oauth_apple.rs` (behind the
`oauth-apple` feature) is a real, declared dependency on `suprnova-apple-rs`,
not an adapted pattern (see "Direct dependencies" below); the source-derived
password records are test evidence only. Detailed construction provenance is
private/local and is not included in this public repository; the fixture
paths and checksums are recorded in `tests/fixtures/manifest.json`.

## Direct dependencies

| Project | Repository | Revision used | License | Where it is used |
| --- | --- | --- | --- | --- |
| suprnova-apple-rs | https://github.com/eas4ai/suprnova-apple-rs | tag `v0.3.1` | MIT (`LICENSE`) | Optional Cargo dependency (`dep:apple`) behind the `oauth-apple` feature. `src/plugins/oauth_apple.rs`'s `LiveApplePublicKeySource` calls its `apple::jwks`/`apple::user` modules to verify Apple's signed ID token via JWKS. `AppleOAuthProvider` does not use its `apple::signing::AppleKeyPair` signing API: v0.3.1's `AppleKeyPair::from_pem_bytes`/`from_file`/`from_base64` parse only a raw 32-byte scalar (`SigningKey::from_slice`), not a standard PKCS8 DER structure, so they cannot load a real Apple `.p8` key; the ES256 client secret is instead signed directly against a `p256::SecretKey` decoded via `p256::pkcs8::DecodePrivateKey`. No source from this crate is copied or vendored -- it is linked as an ordinary Cargo dependency, gated out entirely unless `oauth-apple` is enabled. |

## MIT sources

| Project | Repository | Revision used | Exact license file | Planned treatment |
| --- | --- | --- | --- | --- |
| Torii | https://github.com/eas4ai/suprnova-torii-rs | `968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d` | `LICENSE` | MIT service/repository behavior may be adapted only behind Magnetar traits; no Torii types or tables enter this crate. |
| Suprnova | https://github.com/eas4ai/suprnova | `27f7ddf4bb6c523c4ffa42fa12e4a568a7990f88` | `LICENSE` | Suprnova-owned behavior is adapted as behavior, not represented as copied framework code. |
| Arctic OAuth | https://github.com/danielkov/arctic-oauth | local reference snapshot at Torii checkout revision `968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d` | `reference/arctic-oauth-master/LICENSE` | MIT request/provider patterns may be adapted with this notice retained. Endpoint constants and request-shape quirks (PKCE posture, `client_key`/comma-delimited-scope handling, HTTP-200 error bodies) were adapted from its `providers/apple.rs`, `providers/google.rs`, `providers/facebook.rs`, `providers/twitter.rs`, and `providers/tiktok.rs` for `src/plugins/oauth_apple.rs`, `oauth_google.rs`, `oauth_facebook.rs`, `oauth_x.rs`, and `oauth_tiktok.rs` (iteration 003, task 3); no arctic code is copied, only endpoint values and shape facts. |

## MIT OR Apache-2.0 sources

| Project | Repository | Revision used | Exact license files | Planned treatment |
| --- | --- | --- | --- | --- |
| better-auth-rs | https://github.com/better-auth-rs/better-auth-rs | local reference snapshot at Torii checkout revision `968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d` | `reference/better-auth-rs-1.0.0-alpha.2/LICENSE-MIT`; `reference/better-auth-rs-1.0.0-alpha.2/LICENSE-APACHE` | SeaORM/schema patterns may be adapted; bundled routes, tables, and assumptions are excluded. |
| io-oauth | https://github.com/pimalaya/io-oauth | local reference snapshot at Torii checkout revision `968b0be66b1d49f60a2bcb1ab28b5f1b93fa3a5d` | `reference/io-oauth/LICENSE-MIT`; `reference/io-oauth/LICENSE-APACHE` | Evaluated as a dependency and rejected: its `io-http` dependency is mandatory (not feature-gated), and its `base64`/`sha2` majors duplicate this crate's own. Adapted instead: `src/oauth/protocol.rs`'s RFC 6749 §5 token-response types derive from `rfc6749/issue_access_token.rs`, kept strictly I/O-free, both notices retained. |

## Barred source

`oauth2-broker` 0.1.3 is GPL-3.0 (`https://github.com/hack-ink/oauth2-broker`,
license file `reference/oauth2-broker-0.1.3/LICENSE`). The reference may be
read for behavioral and design patterns, but this crate must not copy, adapt,
translate, or closely paraphrase its code. No dependency, vendored source,
generated output, or vendor/reference path may enter the tree.
`tests/source_firewall.rs` continuously rejects broker path names.
