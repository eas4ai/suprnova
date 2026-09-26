# Feature map: `manual/mocking.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 71 checked.

## Rust API: suprnova

### `suprnova::broadcasting::testing` (private module; items are public through re-exports)

- [ ] struct `suprnova::broadcasting::RecordingBroadcastHub` · framework/src/broadcasting/testing.rs:28
  - Implements: `suprnova::BroadcastHub`
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::new` · framework/src/broadcasting/testing.rs:35
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::broadcasts` · framework/src/broadcasting/testing.rs:44
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::count` · framework/src/broadcasting/testing.rs:49
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::assert_broadcast` · framework/src/broadcasting/testing.rs:54
  - [ ] fn `suprnova::broadcasting::RecordingBroadcastHub::assert_nothing_broadcast` · framework/src/broadcasting/testing.rs:71

### `suprnova::bus::testing`

- [ ] fn `suprnova::bus::testing::assert_dispatched` · framework/src/bus/testing.rs:101
- [ ] fn `suprnova::bus::testing::assert_dispatched_times` · framework/src/bus/testing.rs:127
- [ ] fn `suprnova::bus::testing::assert_not_dispatched` · framework/src/bus/testing.rs:113
- [ ] fn `suprnova::bus::testing::assert_nothing_dispatched` · framework/src/bus/testing.rs:140
- [ ] fn `suprnova::bus::testing::install_fake` · framework/src/bus/testing.rs:59
- [ ] struct `suprnova::bus::testing::BusFakeGuard` · framework/src/bus/testing.rs:67

### `suprnova::container::testing`

- [ ] struct `suprnova::testing::TestContainer` · framework/src/container/testing.rs:88 (also `suprnova::container::testing::TestContainer`)
  - [ ] fn `suprnova::testing::TestContainer::fake` · framework/src/container/testing.rs:104
  - [ ] fn `suprnova::testing::TestContainer::scope` · framework/src/container/testing.rs:157
  - [ ] fn `suprnova::testing::TestContainer::spawn` · framework/src/container/testing.rs:200
  - [ ] fn `suprnova::testing::TestContainer::singleton` · framework/src/container/testing.rs:228
  - [ ] fn `suprnova::testing::TestContainer::factory` · framework/src/container/testing.rs:258
  - [ ] fn `suprnova::testing::TestContainer::bind` · framework/src/container/testing.rs:292
  - [ ] fn `suprnova::testing::TestContainer::bind_factory` · framework/src/container/testing.rs:322
- [ ] struct `suprnova::testing::TestContainerGuard` · framework/src/container/testing.rs:345 (also `suprnova::container::testing::TestContainerGuard`)

### `suprnova::events::testing`

- [ ] fn `suprnova::events::assert_dispatched` · framework/src/events/testing.rs:266 (also `suprnova::events::testing::assert_dispatched`)
- [ ] fn `suprnova::events::assert_dispatched_once` · framework/src/events/testing.rs:308 (also `suprnova::events::testing::assert_dispatched_once`)
- [ ] fn `suprnova::events::assert_dispatched_times` · framework/src/events/testing.rs:314 (also `suprnova::events::testing::assert_dispatched_times`)
- [ ] fn `suprnova::events::assert_listening` · framework/src/events/testing.rs:392 (also `suprnova::events::testing::assert_listening`)
- [ ] fn `suprnova::events::assert_not_dispatched` · framework/src/events/testing.rs:276 (also `suprnova::events::testing::assert_not_dispatched`)
- [ ] fn `suprnova::events::assert_nothing_dispatched` · framework/src/events/testing.rs:328 (also `suprnova::events::testing::assert_nothing_dispatched`)
- [ ] fn `suprnova::events::dispatched` · framework/src/events/testing.rs:354 (also `suprnova::events::testing::dispatched`)
- [ ] fn `suprnova::events::dispatched_count` · framework/src/events/testing.rs:288 (also `suprnova::events::testing::dispatched_count`)
- [ ] fn `suprnova::events::dispatched_events` · framework/src/events/testing.rs:377 (also `suprnova::events::testing::dispatched_events`)
- [ ] fn `suprnova::events::has_dispatched` · framework/src/events/testing.rs:338 (also `suprnova::events::testing::has_dispatched`)
- [ ] fn `suprnova::events::testing::install_fake` · framework/src/events/testing.rs:198
- [ ] fn `suprnova::events::testing::install_fake_except` · framework/src/events/testing.rs:224
- [ ] fn `suprnova::events::testing::install_fake_only` · framework/src/events/testing.rs:210
- [ ] fn `suprnova::events::testing::muted` · framework/src/events/testing.rs:244
- [ ] struct `suprnova::EventFakeGuard` · framework/src/events/testing.rs:254 (also `suprnova::events::EventFakeGuard`, `suprnova::events::testing::EventFakeGuard`)

### `suprnova::filesystem::testing` (feature: `filesystem`)

- [ ] struct `suprnova::filesystem::testing::StorageFakeGuard` · framework/src/filesystem/testing.rs:34
- [ ] trait `suprnova::filesystem::testing::DiskAssertExt` · framework/src/filesystem/testing.rs:65
  - Implemented here by: `opendal::Operator`
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_exists` · framework/src/filesystem/testing.rs:68 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_contents` · framework/src/filesystem/testing.rs:72 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_missing` · framework/src/filesystem/testing.rs:79 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_count` · framework/src/filesystem/testing.rs:84 (required)
  - [ ] fn `suprnova::filesystem::testing::DiskAssertExt::assert_directory_empty` · framework/src/filesystem/testing.rs:93 (required)

### `suprnova::http_client::fake` (private module; items are public through re-exports)

- [ ] fn `suprnova::assert_not_sent` · framework/src/http_client/fake.rs:153 (also `suprnova::http_client::assert_not_sent`)
- [ ] fn `suprnova::assert_sent` · framework/src/http_client/fake.rs:137 (also `suprnova::http_client::assert_sent`)
- [ ] fn `suprnova::fake_response` · framework/src/http_client/fake.rs:92 (also `suprnova::http_client::fake_response`)
- [ ] struct `suprnova::RecordedRequest` · framework/src/http_client/fake.rs:56 (also `suprnova::http_client::RecordedRequest`)
  - Public fields: `method`, `url`, `headers`, `body`

### `suprnova::notifications::testing`

- [ ] fn `suprnova::notifications::assert_count` · framework/src/notifications/testing.rs:145 (also `suprnova::notifications::testing::assert_count`)
- [ ] fn `suprnova::notifications::assert_nothing_sent` · framework/src/notifications/testing.rs:132 (also `suprnova::notifications::testing::assert_nothing_sent`)
- [ ] fn `suprnova::notifications::assert_nothing_sent_to` · framework/src/notifications/testing.rs:158 (also `suprnova::notifications::testing::assert_nothing_sent_to`)
- [ ] fn `suprnova::notifications::assert_sent` · framework/src/notifications/testing.rs:87 (also `suprnova::notifications::testing::assert_sent`)
- [ ] fn `suprnova::notifications::assert_sent_named` · framework/src/notifications/testing.rs:112 (also `suprnova::notifications::testing::assert_sent_named`)
- [ ] fn `suprnova::notifications::assert_sent_times` · framework/src/notifications/testing.rs:117 (also `suprnova::notifications::testing::assert_sent_times`)
- [ ] fn `suprnova::notifications::assert_sent_to` · framework/src/notifications/testing.rs:98 (also `suprnova::notifications::testing::assert_sent_to`)
- [ ] fn `suprnova::notifications::assert_sent_to_on` · framework/src/notifications/testing.rs:103 (also `suprnova::notifications::testing::assert_sent_to_on`)
- [ ] fn `suprnova::notifications::testing::install_fake` · framework/src/notifications/testing.rs:59
- [ ] fn `suprnova::notifications::recorded_notifications` · framework/src/notifications/testing.rs:79 (also `suprnova::notifications::testing::recorded`)
- [ ] struct `suprnova::notifications::FakeRecord` · framework/src/notifications/testing.rs:21 (also `suprnova::notifications::testing::FakeRecord`)
  - Public fields: `notification`, `channel`, `route`, `data`
- [ ] struct `suprnova::NotifyFakeGuard` · framework/src/notifications/testing.rs:66 (also `suprnova::notifications::NotifyFakeGuard`, `suprnova::notifications::testing::NotifyFakeGuard`)

### `suprnova::queue::testing`

- [ ] fn `suprnova::queue::testing::assert_pushed` · framework/src/queue/testing.rs:156
- [ ] fn `suprnova::queue::testing::assert_pushed_later` · framework/src/queue/testing.rs:194
- [ ] fn `suprnova::queue::testing::assert_pushed_on_connection` · framework/src/queue/testing.rs:324
- [ ] fn `suprnova::queue::testing::assert_pushed_on_queue` · framework/src/queue/testing.rs:301
- [ ] fn `suprnova::queue::testing::delayed_jobs` · framework/src/queue/testing.rs:369
- [ ] fn `suprnova::queue::testing::install_fake` · framework/src/queue/testing.rs:134
- [ ] fn `suprnova::queue::testing::pending_jobs` · framework/src/queue/testing.rs:352
- [ ] fn `suprnova::queue::testing::pushed` · framework/src/queue/testing.rs:219
- [ ] fn `suprnova::queue::testing::pushed_with_available_at` · framework/src/queue/testing.rs:174
- [ ] fn `suprnova::queue::testing::pushed_with_id` · framework/src/queue/testing.rs:241
- [ ] fn `suprnova::queue::testing::pushed_with_overrides` · framework/src/queue/testing.rs:271
- [ ] struct `suprnova::queue::testing::QueueFakeGuard` · framework/src/queue/testing.rs:142
