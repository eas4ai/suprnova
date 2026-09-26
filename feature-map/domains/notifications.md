# Feature map: `manual/notifications.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 73 checked.

## Rust API: suprnova

### `suprnova::notifications::anonymous`

- [ ] struct `suprnova::AnonymousNotifiable` · framework/src/notifications/anonymous.rs:30 (also `suprnova::notifications::AnonymousNotifiable`, `suprnova::notifications::anonymous::AnonymousNotifiable`)
  - Implements: `suprnova::Notifiable`
  - [ ] fn `suprnova::AnonymousNotifiable::new` · framework/src/notifications/anonymous.rs:37
  - [ ] fn `suprnova::AnonymousNotifiable::route` · framework/src/notifications/anonymous.rs:47
  - [ ] fn `suprnova::AnonymousNotifiable::routes` · framework/src/notifications/anonymous.rs:66
  - [ ] fn `suprnova::AnonymousNotifiable::raw_routes` · framework/src/notifications/anonymous.rs:80

### `suprnova::notifications::channels::broadcast`

- [ ] struct `suprnova::BroadcastChannel` · framework/src/notifications/channels/broadcast.rs:36 (also `suprnova::notifications::channels::broadcast::BroadcastChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::BroadcastChannel::new` · framework/src/notifications/channels/broadcast.rs:40

### `suprnova::notifications::channels::database`

- [ ] struct `suprnova::DatabaseChannel` · framework/src/notifications/channels/database.rs:26 (also `suprnova::notifications::channels::database::DatabaseChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::DatabaseChannel::new` · framework/src/notifications/channels/database.rs:34

### `suprnova::notifications::channels::mail`

- [ ] fn `suprnova::register_mail_renderer` · framework/src/notifications/channels/mail.rs:119 (also `suprnova::notifications::channels::mail::register_mail_renderer`, `suprnova::prelude::register_mail_renderer`)
- [ ] struct `suprnova::MailChannel` · framework/src/notifications/channels/mail.rs:154 (also `suprnova::notifications::channels::mail::MailChannel`)
  - Implements: `suprnova::Channel`
  - [ ] fn `suprnova::MailChannel::new` · framework/src/notifications/channels/mail.rs:158
- [ ] struct `suprnova::MailRendering` · framework/src/notifications/channels/mail.rs:64 (also `suprnova::notifications::channels::mail::MailRendering`, `suprnova::prelude::MailRendering`)
  - Public fields: `subject`, `html`, `text`, `from`, `cc`, `bcc`, `reply_to`, `attachments`
- [ ] trait `suprnova::NotificationMailable` · framework/src/notifications/channels/mail.rs:96 (also `suprnova::notifications::channels::mail::NotificationMailable`, `suprnova::prelude::NotificationMailable`)
  - [ ] fn `suprnova::NotificationMailable::to_mail` · framework/src/notifications/channels/mail.rs:99 (required)

### `suprnova::notifications::database_read`

- [ ] fn `suprnova::notifications::all_for` · framework/src/notifications/database_read.rs:103 (also `suprnova::notifications::database_read::all_for`)
- [ ] fn `suprnova::notifications::delete_for` · framework/src/notifications/database_read.rs:227 (also `suprnova::notifications::database_read::delete_for`)
- [ ] fn `suprnova::notifications::mark_all_as_read` · framework/src/notifications/database_read.rs:195 (also `suprnova::notifications::database_read::mark_all_as_read`)
- [ ] fn `suprnova::notifications::mark_as_read` · framework/src/notifications/database_read.rs:151 (also `suprnova::notifications::database_read::mark_as_read`)
- [ ] fn `suprnova::notifications::mark_as_unread` · framework/src/notifications/database_read.rs:173 (also `suprnova::notifications::database_read::mark_as_unread`)
- [ ] fn `suprnova::notifications::read_for` · framework/src/notifications/database_read.rs:135 (also `suprnova::notifications::database_read::read_for`)
- [ ] fn `suprnova::notifications::unread_for` · framework/src/notifications/database_read.rs:119 (also `suprnova::notifications::database_read::unread_for`)
- [ ] struct `suprnova::StoredNotification` · framework/src/notifications/database_read.rs:27 (also `suprnova::notifications::StoredNotification`, `suprnova::notifications::database_read::StoredNotification`)
  - Public fields: `id`, `type_name`, `notifiable_type`, `notifiable_id`, `data`, `read_at`, `created_at`, `updated_at`

### `suprnova::notifications::events`

- [ ] struct `suprnova::NotificationFailed` · framework/src/notifications/events.rs:73 (also `suprnova::notifications::NotificationFailed`, `suprnova::notifications::events::NotificationFailed`)
  - Public fields: `notification`, `channel`, `route`, `data`, `error`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::NotificationSending` · framework/src/notifications/events.rs:32 (also `suprnova::notifications::NotificationSending`, `suprnova::notifications::events::NotificationSending`)
  - Public fields: `notification`, `channel`, `route`, `data`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::NotificationSent` · framework/src/notifications/events.rs:51 (also `suprnova::notifications::NotificationSent`, `suprnova::notifications::events::NotificationSent`)
  - Public fields: `notification`, `channel`, `route`, `data`
  - Implements: `suprnova::Event`

### `suprnova::notifications::notify_job`

- [ ] struct `suprnova::SendNotificationJob` · framework/src/notifications/notify_job.rs:21 (also `suprnova::notifications::SendNotificationJob`, `suprnova::notifications::notify_job::SendNotificationJob`)
  - Public fields: `notifiable_route_per_channel`, `notification_name`, `notification_payload`, `channels`
  - Implements: `suprnova::Job`

### `suprnova::notifications`

- [ ] fn `suprnova::notifications::register_notification_factory` · framework/src/notifications/mod.rs:430
- [ ] fn `suprnova::notifications::set_dispatcher` · framework/src/notifications/mod.rs:405
- [ ] struct `suprnova::NotificationDispatcher` · framework/src/notifications/mod.rs:218 (also `suprnova::notifications::NotificationDispatcher`)
  - [ ] fn `suprnova::NotificationDispatcher::new` · framework/src/notifications/mod.rs:224
  - [ ] fn `suprnova::NotificationDispatcher::register_channel` · framework/src/notifications/mod.rs:234
  - [ ] fn `suprnova::NotificationDispatcher::notify` · framework/src/notifications/mod.rs:266
  - [ ] fn `suprnova::NotificationDispatcher::channel` · framework/src/notifications/mod.rs:380
- [ ] struct `suprnova::Notify` · framework/src/notifications/mod.rs:464 (also `suprnova::notifications::Notify`, `suprnova::prelude::Notify`)
  - [ ] fn `suprnova::Notify::queue` · framework/src/notifications/mod.rs:505
  - [ ] fn `suprnova::Notify::send` · framework/src/notifications/mod.rs:583
  - [ ] fn `suprnova::Notify::fake` · framework/src/notifications/mod.rs:613
  - [ ] fn `suprnova::Notify::route` · framework/src/notifications/mod.rs:638
  - [ ] fn `suprnova::Notify::routes` · framework/src/notifications/mod.rs:648
- [ ] trait `suprnova::Channel` · framework/src/notifications/mod.rs:194 (also `suprnova::notifications::Channel`)
  - Implemented here by: `BroadcastChannel`, `DatabaseChannel`, `MailChannel`, `WebPushChannel`
  - [ ] fn `suprnova::Channel::name` · framework/src/notifications/mod.rs:198 (required)
  - [ ] fn `suprnova::Channel::deliver` · framework/src/notifications/mod.rs:202 (required)
- [ ] trait `suprnova::DynNotification` · framework/src/notifications/mod.rs:159 (also `suprnova::notifications::DynNotification`)
  - [ ] fn `suprnova::DynNotification::name` · framework/src/notifications/mod.rs:161 (required)
  - [ ] fn `suprnova::DynNotification::data` · framework/src/notifications/mod.rs:163 (required)
  - [ ] fn `suprnova::DynNotification::should_send` · framework/src/notifications/mod.rs:167 (required)
  - [ ] fn `suprnova::DynNotification::after_sending` · framework/src/notifications/mod.rs:170 (required)
- [ ] trait `suprnova::Notifiable` · framework/src/notifications/mod.rs:53 (also `suprnova::notifications::Notifiable`, `suprnova::prelude::Notifiable`)
  - Implemented here by: `AnonymousNotifiable`
  - [ ] fn `suprnova::Notifiable::route_for` · framework/src/notifications/mod.rs:55 (required)
- [ ] trait `suprnova::Notification` · framework/src/notifications/mod.rs:64 (also `suprnova::notifications::Notification`, `suprnova::prelude::Notification`)
  - [ ] fn `suprnova::Notification::notification_name` · framework/src/notifications/mod.rs:67 (required)
  - [ ] fn `suprnova::Notification::channels` · framework/src/notifications/mod.rs:72 (required)
  - [ ] fn `suprnova::Notification::data` · framework/src/notifications/mod.rs:75 (required)
  - [ ] fn `suprnova::Notification::should_send` · framework/src/notifications/mod.rs:84 (provided)
  - [ ] fn `suprnova::Notification::after_sending` · framework/src/notifications/mod.rs:94 (provided)
  - [ ] fn `suprnova::Notification::queue` · framework/src/notifications/mod.rs:111 (provided)
  - [ ] fn `suprnova::Notification::timeout` · framework/src/notifications/mod.rs:119 (provided)
  - [ ] fn `suprnova::Notification::fail_on_timeout` · framework/src/notifications/mod.rs:130 (provided)
  - [ ] fn `suprnova::Notification::max_tries` · framework/src/notifications/mod.rs:138 (provided)
  - [ ] fn `suprnova::Notification::backoff` · framework/src/notifications/mod.rs:147 (provided)
- [ ] type `suprnova::NotificationFactory` · framework/src/notifications/mod.rs:393 (also `suprnova::notifications::NotificationFactory`)

## Rust API: suprnova-macros

### `suprnova_macros`

- [ ] proc macro `suprnova_macros::NotificationMailable` · suprnova-macros/src/lib.rs:805 (re-exported as `suprnova::NotificationMailable`)
  - Form: derive `#[derive(NotificationMailable)]`
  - Helper attributes: `#[mail]`
  - [ ] argument `#[mail(subject)]` · suprnova-macros/src/notification_mail.rs:74
  - [ ] argument `#[mail(from)]` · suprnova-macros/src/notification_mail.rs:79
  - [ ] argument `#[mail(from_name)]` · suprnova-macros/src/notification_mail.rs:80
  - [ ] argument `#[mail(reply_to)]` · suprnova-macros/src/notification_mail.rs:83
  - [ ] argument `#[mail(cc)]` · suprnova-macros/src/notification_mail.rs:81
  - [ ] argument `#[mail(bcc)]` · suprnova-macros/src/notification_mail.rs:82
  - [ ] argument `#[mail(html)]` · suprnova-macros/src/notification_mail.rs:75
  - [ ] argument `#[mail(text)]` · suprnova-macros/src/notification_mail.rs:77
  - [ ] argument `#[mail(html_template)]` · suprnova-macros/src/notification_mail.rs:76
  - [ ] argument `#[mail(text_template)]` · suprnova-macros/src/notification_mail.rs:78
