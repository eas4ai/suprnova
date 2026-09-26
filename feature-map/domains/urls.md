# Feature map: `manual/urls.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 19 checked.

## Rust API: suprnova

### `suprnova::routing::signed` (private module; items are public through re-exports)

- [ ] fn `suprnova::sign_route` · framework/src/routing/signed.rs:416 (also `suprnova::routing::sign_route`)
- [ ] fn `suprnova::sign_url` · framework/src/routing/signed.rs:224 (also `suprnova::routing::sign_url`)
- [ ] fn `suprnova::verify_signature` · framework/src/routing/signed.rs:296 (also `suprnova::routing::verify_signature`)
- [ ] enum `suprnova::SignatureVerdict` · framework/src/routing/signed.rs:84 (also `suprnova::routing::SignatureVerdict`)
  - Variants: `Valid`, `Expired`, `Invalid`
  - [ ] fn `suprnova::SignatureVerdict::is_valid` · framework/src/routing/signed.rs:99
  - [ ] fn `suprnova::SignatureVerdict::is_expired` · framework/src/routing/signed.rs:105
- [ ] const `suprnova::routing::EXPIRES_KEY` · framework/src/routing/signed.rs:80
- [ ] const `suprnova::routing::SIGNATURE_KEY` · framework/src/routing/signed.rs:76

### `suprnova::routing::url`

- [ ] fn `suprnova::url::current` · framework/src/routing/url.rs:91 (also `suprnova::routing::url::current`)
- [ ] fn `suprnova::url::full` · framework/src/routing/url.rs:101 (also `suprnova::routing::url::full`)
- [ ] fn `suprnova::url::has_valid_signature` · framework/src/routing/url.rs:168 (also `suprnova::routing::url::has_valid_signature`)
- [ ] fn `suprnova::url::previous` · framework/src/routing/url.rs:111 (also `suprnova::routing::url::previous`)
- [ ] fn `suprnova::url::secure` · framework/src/routing/url.rs:75 (also `suprnova::routing::url::secure`)
- [ ] fn `suprnova::url::signature_has_not_expired` · framework/src/routing/url.rs:207 (deprecated; also `suprnova::routing::url::signature_has_not_expired`)
- [ ] fn `suprnova::url::signature_verdict` · framework/src/routing/url.rs:214 (also `suprnova::routing::url::signature_verdict`)
- [ ] fn `suprnova::url::signed_route` · framework/src/routing/url.rs:123 (also `suprnova::routing::url::signed_route`)
- [ ] fn `suprnova::url::signed_url` · framework/src/routing/url.rs:149 (also `suprnova::routing::url::signed_url`)
- [ ] fn `suprnova::url::temporary_signed_route` · framework/src/routing/url.rs:137 (also `suprnova::routing::url::temporary_signed_route`)
- [ ] fn `suprnova::url::to` · framework/src/routing/url.rs:65 (also `suprnova::routing::url::to`)
