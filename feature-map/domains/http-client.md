# Feature map: `manual/http-client.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 39 checked.

## Rust API: suprnova

### `suprnova::http_client`

- [ ] struct `suprnova::ClientResponse` · framework/src/http_client/mod.rs:923 (also `suprnova::http_client::ClientResponse`)
  - [ ] fn `suprnova::ClientResponse::status` · framework/src/http_client/mod.rs:964
  - [ ] fn `suprnova::ClientResponse::header` · framework/src/http_client/mod.rs:972
  - [ ] fn `suprnova::ClientResponse::json` · framework/src/http_client/mod.rs:988
  - [ ] fn `suprnova::ClientResponse::text` · framework/src/http_client/mod.rs:999
  - [ ] fn `suprnova::ClientResponse::bytes` · framework/src/http_client/mod.rs:1010
  - [ ] fn `suprnova::ClientResponse::into_inner` · framework/src/http_client/mod.rs:1028
- [ ] struct `suprnova::FailOnRealCallsGuard` · framework/src/http_client/mod.rs:349 (also `suprnova::http_client::FailOnRealCallsGuard`)
  - [ ] fn `suprnova::FailOnRealCallsGuard::install` · framework/src/http_client/mod.rs:355
- [ ] struct `suprnova::Http` · framework/src/http_client/mod.rs:112 (also `suprnova::http_client::Http`, `suprnova::prelude::Http`)
  - [ ] fn `suprnova::Http::get` · framework/src/http_client/mod.rs:116
  - [ ] fn `suprnova::Http::post` · framework/src/http_client/mod.rs:121
  - [ ] fn `suprnova::Http::put` · framework/src/http_client/mod.rs:126
  - [ ] fn `suprnova::Http::patch` · framework/src/http_client/mod.rs:131
  - [ ] fn `suprnova::Http::delete` · framework/src/http_client/mod.rs:136
  - [ ] fn `suprnova::Http::fake` · framework/src/http_client/mod.rs:177
  - [ ] fn `suprnova::Http::fake_response_text` · framework/src/http_client/mod.rs:217
  - [ ] fn `suprnova::Http::fail_on_real_calls` · framework/src/http_client/mod.rs:236
  - [ ] fn `suprnova::Http::allow_real_calls` · framework/src/http_client/mod.rs:244
  - [ ] fn `suprnova::Http::is_guarded` · framework/src/http_client/mod.rs:249
  - [ ] fn `suprnova::Http::set_max_response_bytes` · framework/src/http_client/mod.rs:258
  - [ ] fn `suprnova::Http::max_response_bytes` · framework/src/http_client/mod.rs:265
  - [ ] fn `suprnova::Http::spawn_with_fake_inheritance` · framework/src/http_client/mod.rs:309
- [ ] struct `suprnova::RequestBuilder` · framework/src/http_client/mod.rs:473 (also `suprnova::http_client::RequestBuilder`)
  - [ ] fn `suprnova::RequestBuilder::no_redirects` · framework/src/http_client/mod.rs:526
  - [ ] fn `suprnova::RequestBuilder::header` · framework/src/http_client/mod.rs:533
  - [ ] fn `suprnova::RequestBuilder::json` · framework/src/http_client/mod.rs:540
  - [ ] fn `suprnova::RequestBuilder::form` · framework/src/http_client/mod.rs:552
  - [ ] fn `suprnova::RequestBuilder::body` · framework/src/http_client/mod.rs:562
  - [ ] fn `suprnova::RequestBuilder::timeout` · framework/src/http_client/mod.rs:569
  - [ ] fn `suprnova::RequestBuilder::max_response_bytes` · framework/src/http_client/mod.rs:577
  - [ ] fn `suprnova::RequestBuilder::bearer_token` · framework/src/http_client/mod.rs:583
  - [ ] fn `suprnova::RequestBuilder::basic_auth` · framework/src/http_client/mod.rs:589
  - [ ] fn `suprnova::RequestBuilder::retry` · framework/src/http_client/mod.rs:618
  - [ ] fn `suprnova::RequestBuilder::retry_non_idempotent` · framework/src/http_client/mod.rs:639
  - [ ] fn `suprnova::RequestBuilder::retry_when` · framework/src/http_client/mod.rs:673
  - [ ] fn `suprnova::RequestBuilder::send` · framework/src/http_client/mod.rs:686
- [ ] struct `suprnova::RetryContext` · framework/src/http_client/mod.rs:455 (also `suprnova::http_client::RetryContext`)
  - Public fields: `attempt`, `method`, `url`, `outcome`
- [ ] enum `suprnova::RetryOutcome` · framework/src/http_client/mod.rs:442 (also `suprnova::http_client::RetryOutcome`)
  - Variants: `TransportError`, `Status`
