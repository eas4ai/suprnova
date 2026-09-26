# Feature map: `manual/websockets.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 15 checked.

## Rust API: suprnova

### `suprnova::ws::heartbeat`

- [ ] fn `suprnova::ws::heartbeat::run` · framework/src/ws/heartbeat.rs:37

### `suprnova::ws::socket` (private module; items are public through re-exports)

- [ ] struct `suprnova::WsSocket` · framework/src/ws/socket.rs:34 (also `suprnova::ws::WsSocket`)
  - [ ] fn `suprnova::WsSocket::from_stream` · framework/src/ws/socket.rs:78
  - [ ] fn `suprnova::WsSocket::from_stream_with_heartbeat` · framework/src/ws/socket.rs:91
  - [ ] fn `suprnova::WsSocket::send_text` · framework/src/ws/socket.rs:203
  - [ ] fn `suprnova::WsSocket::send_binary` · framework/src/ws/socket.rs:211
  - [ ] fn `suprnova::WsSocket::recv_text` · framework/src/ws/socket.rs:241
  - [ ] fn `suprnova::WsSocket::recv` · framework/src/ws/socket.rs:260
  - [ ] fn `suprnova::WsSocket::close` · framework/src/ws/socket.rs:295

### `suprnova::ws`

- [ ] struct `suprnova::WsConfig` · framework/src/ws/mod.rs:88 (also `suprnova::ws::WsConfig`)
  - Public fields: `ping_interval`, `max_message_size`, `max_frame_size`, `max_missed_pings`, `origin_policy`, `accepted_protocols`
  - [ ] fn `suprnova::WsConfig::generous` · framework/src/ws/mod.rs:219
- [ ] enum `suprnova::ws::OriginPolicy` · framework/src/ws/mod.rs:67
  - Variants: `SameOrigin`, `AllowAny`, `AllowList`
- [ ] trait `suprnova::WebSocketHandler` · framework/src/ws/mod.rs:44 (also `suprnova::ws::WebSocketHandler`)
  - Implemented here by: `BroadcastingWsHandler`
  - [ ] fn `suprnova::WebSocketHandler::handle` · framework/src/ws/mod.rs:48 (required)
- [ ] type `suprnova::ws::BoxedWebSocketHandler` · framework/src/ws/mod.rs:268
