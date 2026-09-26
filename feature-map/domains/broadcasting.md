# Feature map: `manual/broadcasting.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 51 checked.

## Rust API: suprnova

### `suprnova::broadcasting::broadcastable` (private module; items are public through re-exports)

- [ ] struct `suprnova::BroadcastListener` · framework/src/broadcasting/broadcastable.rs:93 (also `suprnova::broadcasting::BroadcastListener`)
  - Implements: `suprnova::Listener`
  - [ ] fn `suprnova::BroadcastListener::new` · framework/src/broadcasting/broadcastable.rs:100
- [ ] trait `suprnova::Broadcastable` · framework/src/broadcasting/broadcastable.rs:48 (also `suprnova::broadcasting::Broadcastable`)
  - [ ] fn `suprnova::Broadcastable::broadcast_on` · framework/src/broadcasting/broadcastable.rs:51 (required)
  - [ ] fn `suprnova::Broadcastable::broadcast_event_name` · framework/src/broadcasting/broadcastable.rs:55 (provided)
  - [ ] fn `suprnova::Broadcastable::broadcast_with` · framework/src/broadcasting/broadcastable.rs:64 (provided)
  - [ ] fn `suprnova::Broadcastable::broadcast_when` · framework/src/broadcasting/broadcastable.rs:72 (provided)
  - [ ] fn `suprnova::Broadcastable::broadcast_to_others` · framework/src/broadcasting/broadcastable.rs:86 (provided)

### `suprnova::broadcasting::channel` (private module; items are public through re-exports)

- [ ] struct `suprnova::broadcasting::ChannelParams` · framework/src/broadcasting/channel.rs:26
  - [ ] fn `suprnova::broadcasting::ChannelParams::get` · framework/src/broadcasting/channel.rs:33
  - [ ] fn `suprnova::broadcasting::ChannelParams::is_empty` · framework/src/broadcasting/channel.rs:41
  - [ ] fn `suprnova::broadcasting::ChannelParams::len` · framework/src/broadcasting/channel.rs:46
  - [ ] fn `suprnova::broadcasting::ChannelParams::iter` · framework/src/broadcasting/channel.rs:51
- [ ] struct `suprnova::broadcasting::ChannelRegistry` · framework/src/broadcasting/channel.rs:307
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::new` · framework/src/broadcasting/channel.rs:313
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::register` · framework/src/broadcasting/channel.rs:330
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::resolve` · framework/src/broadcasting/channel.rs:350
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::len` · framework/src/broadcasting/channel.rs:376
  - [ ] fn `suprnova::broadcasting::ChannelRegistry::is_empty` · framework/src/broadcasting/channel.rs:381
- [ ] trait `suprnova::broadcasting::Channel` · framework/src/broadcasting/channel.rs:127
  - [ ] fn `suprnova::broadcasting::Channel::name` · framework/src/broadcasting/channel.rs:137 (required)
  - [ ] fn `suprnova::broadcasting::Channel::authorize` · framework/src/broadcasting/channel.rs:147 (provided)
  - [ ] fn `suprnova::broadcasting::Channel::authorize_publish` · framework/src/broadcasting/channel.rs:168 (provided)
  - [ ] fn `suprnova::broadcasting::Channel::presence_info` · framework/src/broadcasting/channel.rs:207 (provided)
- [ ] trait `suprnova::broadcasting::PresenceChannel` · framework/src/broadcasting/channel.rs:265
  - [ ] fn `suprnova::broadcasting::PresenceChannel::member_info` · framework/src/broadcasting/channel.rs:272 (required)
- [ ] trait `suprnova::broadcasting::PrivateChannel` · framework/src/broadcasting/channel.rs:216
- [ ] type `suprnova::broadcasting::BoxedChannel` · framework/src/broadcasting/channel.rs:280

### `suprnova::broadcasting::fanout::sea_streamer` (private module; items are public through re-exports)

- [ ] struct `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` · framework/src/broadcasting/fanout/sea_streamer.rs:255 (feature: `broadcasting-fanout`, off by default)
  - Implements: `suprnova::BroadcastHub`
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new` · framework/src/broadcasting/fanout/sea_streamer.rs:294
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new_with_presence_ttl` · framework/src/broadcasting/fanout/sea_streamer.rs:316
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new_loopback` · framework/src/broadcasting/fanout/sea_streamer.rs:332
  - [ ] fn `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub::new_loopback_with_presence_ttl` · framework/src/broadcasting/fanout/sea_streamer.rs:343

### `suprnova::broadcasting::handler` (private module; items are public through re-exports)

- [ ] struct `suprnova::BroadcastingWsHandler` · framework/src/broadcasting/handler.rs:106 (also `suprnova::broadcasting::BroadcastingWsHandler`)
  - Implements: `suprnova::WebSocketHandler`
  - [ ] fn `suprnova::BroadcastingWsHandler::new` · framework/src/broadcasting/handler.rs:122
  - [ ] fn `suprnova::BroadcastingWsHandler::with_max_subscriptions` · framework/src/broadcasting/handler.rs:142
- [ ] const `suprnova::broadcasting::DEFAULT_MAX_SUBSCRIPTIONS_PER_CONNECTION` · framework/src/broadcasting/handler.rs:99

### `suprnova::broadcasting::hub` (private module; items are public through re-exports)

- [ ] struct `suprnova::BroadcastEnvelope` · framework/src/broadcasting/hub.rs:60 (also `suprnova::broadcasting::BroadcastEnvelope`)
  - Public fields: `channel`, `event`, `data`, `except`
  - [ ] fn `suprnova::BroadcastEnvelope::new` · framework/src/broadcasting/hub.rs:79
  - [ ] fn `suprnova::BroadcastEnvelope::with_except` · framework/src/broadcasting/hub.rs:91
- [ ] struct `suprnova::InMemoryBroadcastHub` · framework/src/broadcasting/hub.rs:166 (also `suprnova::broadcasting::InMemoryBroadcastHub`)
  - Implements: `suprnova::BroadcastHub`
  - [ ] fn `suprnova::InMemoryBroadcastHub::new` · framework/src/broadcasting/hub.rs:174
- [ ] trait `suprnova::BroadcastHub` · framework/src/broadcasting/hub.rs:102 (also `suprnova::broadcasting::BroadcastHub`)
  - Implemented here by: `InMemoryBroadcastHub`, `broadcasting::RecordingBroadcastHub`, `broadcasting::fanout::SeaStreamerBroadcastHub`
  - [ ] fn `suprnova::BroadcastHub::subscribe` · framework/src/broadcasting/hub.rs:107 (required)
  - [ ] fn `suprnova::BroadcastHub::publish` · framework/src/broadcasting/hub.rs:120 (required)
  - [ ] fn `suprnova::BroadcastHub::subscriber_count` · framework/src/broadcasting/hub.rs:124 (provided)
  - [ ] fn `suprnova::BroadcastHub::track_member` · framework/src/broadcasting/hub.rs:140 (provided)
  - [ ] fn `suprnova::BroadcastHub::untrack_member` · framework/src/broadcasting/hub.rs:153 (provided)
  - [ ] fn `suprnova::BroadcastHub::list_members` · framework/src/broadcasting/hub.rs:160 (provided)

### `suprnova::broadcasting::protocol` (private module; items are public through re-exports)

- [ ] enum `suprnova::broadcasting::ClientFrame` · framework/src/broadcasting/protocol.rs:26
  - Variants: `Subscribe`, `Unsubscribe`, `Publish`
- [ ] enum `suprnova::broadcasting::ServerFrame` · framework/src/broadcasting/protocol.rs:60
  - Variants: `Connected`, `Subscribed`, `Unsubscribed`, `Event`, `Lagged`, `Error`
