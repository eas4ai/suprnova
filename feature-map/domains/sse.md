# Feature map: `manual/sse.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 25 checked.

## Rust API: suprnova

### `suprnova::sse`

- [ ] fn `suprnova::sse::last_event_id` · framework/src/sse/mod.rs:627
- [ ] fn `suprnova::sse::last_event_id_from_value` · framework/src/sse/mod.rs:602
- [ ] struct `suprnova::SseEvent` · framework/src/sse/mod.rs:134 (also `suprnova::sse::SseEvent`)
  - [ ] fn `suprnova::SseEvent::data` · framework/src/sse/mod.rs:157
  - [ ] fn `suprnova::SseEvent::json` · framework/src/sse/mod.rs:171
  - [ ] fn `suprnova::SseEvent::comment` · framework/src/sse/mod.rs:198
  - [ ] fn `suprnova::SseEvent::keep_alive` · framework/src/sse/mod.rs:211
  - [ ] fn `suprnova::SseEvent::error` · framework/src/sse/mod.rs:227
  - [ ] fn `suprnova::SseEvent::with_event` · framework/src/sse/mod.rs:247
  - [ ] fn `suprnova::SseEvent::with_id` · framework/src/sse/mod.rs:261
  - [ ] fn `suprnova::SseEvent::with_retry` · framework/src/sse/mod.rs:276
  - [ ] fn `suprnova::SseEvent::try_with_event` · framework/src/sse/mod.rs:293
  - [ ] fn `suprnova::SseEvent::try_with_id` · framework/src/sse/mod.rs:304
  - [ ] fn `suprnova::SseEvent::event` · framework/src/sse/mod.rs:317
  - [ ] fn `suprnova::SseEvent::id` · framework/src/sse/mod.rs:326
  - [ ] fn `suprnova::SseEvent::retry` · framework/src/sse/mod.rs:335
  - [ ] fn `suprnova::SseEvent::payload` · framework/src/sse/mod.rs:350
  - [ ] fn `suprnova::SseEvent::is_comment` · framework/src/sse/mod.rs:359
  - [ ] fn `suprnova::SseEvent::comment_text` · framework/src/sse/mod.rs:365
  - [ ] fn `suprnova::SseEvent::to_wire` · framework/src/sse/mod.rs:411
- [ ] struct `suprnova::StreamedEvent` · framework/src/sse/mod.rs:494 (also `suprnova::sse::StreamedEvent`)
  - Public fields: `event`, `data`
  - [ ] fn `suprnova::StreamedEvent::message` · framework/src/sse/mod.rs:505
  - [ ] fn `suprnova::StreamedEvent::named` · framework/src/sse/mod.rs:513
- [ ] enum `suprnova::EndSignal` · framework/src/sse/mod.rs:539 (also `suprnova::sse::EndSignal`)
  - Variants: `None`, `Message`, `Event`
  - [ ] fn `suprnova::EndSignal::text` · framework/src/sse/mod.rs:550
