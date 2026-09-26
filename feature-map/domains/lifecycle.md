# Feature map: `manual/lifecycle.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 16 checked.

## Rust API: suprnova

### Re-exported from other crates

- [ ] struct `suprnova::RequestBodyStream` re-exports `hyper::body::incoming::Incoming`
- [ ] module `suprnova::hyper` re-exports `hyper`
- [ ] module `suprnova::tokio` re-exports `tokio`

### `suprnova::server`

- [ ] fn `suprnova::handle_request` · framework/src/server.rs:687 (also `suprnova::server::handle_request`)
- [ ] fn `suprnova::handle_request_with_peer` · framework/src/server.rs:701 (also `suprnova::server::handle_request_with_peer`)
- [ ] struct `suprnova::Server` · framework/src/server.rs:73 (also `suprnova::server::Server`)
  - [ ] fn `suprnova::Server::new` · framework/src/server.rs:104
  - [ ] fn `suprnova::Server::from_config` · framework/src/server.rs:145
  - [ ] fn `suprnova::Server::try_from_config_with_routes` · framework/src/server.rs:158
  - [ ] fn `suprnova::Server::try_from_config_with_routes_async` · framework/src/server.rs:188
  - [ ] fn `suprnova::Server::middleware` · framework/src/server.rs:301
  - [ ] fn `suprnova::Server::host` · framework/src/server.rs:308
  - [ ] fn `suprnova::Server::port` · framework/src/server.rs:314
  - [ ] fn `suprnova::Server::max_connections` · framework/src/server.rs:330
  - [ ] fn `suprnova::Server::header_read_timeout` · framework/src/server.rs:340
  - [ ] fn `suprnova::Server::run` · framework/src/server.rs:375
