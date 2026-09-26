# Feature map: `manual/mail.md`

Source at d03b4f1. Every entry below is extracted from the code; this file only groups them under the chapter that owns their domain. A checked box means the chapter's documentation of that item has been remediated against the source.

0 of 161 checked.

## Rust API: suprnova

### `suprnova::mail::address`

- [ ] struct `suprnova::Address` · framework/src/mail/address.rs:8 (also `suprnova::mail::Address`, `suprnova::mail::address::Address`, `suprnova::prelude::Address`)
  - Public fields: `email`, `name`
  - [ ] fn `suprnova::Address::new` · framework/src/mail/address.rs:17
  - [ ] fn `suprnova::Address::with_name` · framework/src/mail/address.rs:24
- [ ] struct `suprnova::Attachment` · framework/src/mail/address.rs:73 (also `suprnova::mail::Attachment`, `suprnova::mail::address::Attachment`, `suprnova::prelude::Attachment`)
  - Public fields: `filename`, `content`, `content_type`
  - [ ] fn `suprnova::Attachment::new` · framework/src/mail/address.rs:85

### `suprnova::mail::boot`

- [ ] fn `suprnova::mail::boot::bootstrap_from_env` · framework/src/mail/boot.rs:359
- [ ] fn `suprnova::mail::boot::captured_in_memory` · framework/src/mail/boot.rs:32

### `suprnova::mail::events`

- [ ] struct `suprnova::MessageSending` · framework/src/mail/events.rs:21 (also `suprnova::mail::MessageSending`, `suprnova::mail::events::MessageSending`)
  - Public fields: `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `has_html`, `has_text`, `attachment_count`, `tags`
  - Implements: `suprnova::Event`
- [ ] struct `suprnova::MessageSent` · framework/src/mail/events.rs:55 (also `suprnova::mail::MessageSent`, `suprnova::mail::events::MessageSent`)
  - Public fields: `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `has_html`, `has_text`, `attachment_count`, `tags`
  - Implements: `suprnova::Event`

### `suprnova::mail::file`

- [ ] struct `suprnova::mail::file::FileMailTransport` · framework/src/mail/file.rs:20
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::file::FileMailTransport::new` · framework/src/mail/file.rs:30

### `suprnova::mail::log`

- [ ] struct `suprnova::mail::log::LogMailTransport` · framework/src/mail/log.rs:39
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::log::LogMailTransport::new` · framework/src/mail/log.rs:43

### `suprnova::mail::mailable_registry`

- [ ] fn `suprnova::mail::mailable_registry::build` · framework/src/mail/mailable_registry.rs:116
- [ ] fn `suprnova::mail::mailable_registry::register` · framework/src/mail/mailable_registry.rs:100
- [ ] fn `suprnova::mail::mailable_registry::render_outgoing` · framework/src/mail/mailable_registry.rs:140
- [ ] trait `suprnova::mail::mailable_registry::AnyMailable` · framework/src/mail/mailable_registry.rs:39
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::render_subject` · framework/src/mail/mailable_registry.rs:41 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::render_html` · framework/src/mail/mailable_registry.rs:43 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::render_text` · framework/src/mail/mailable_registry.rs:45 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::from` · framework/src/mail/mailable_registry.rs:47 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::attachments` · framework/src/mail/mailable_registry.rs:49 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::tags` · framework/src/mail/mailable_registry.rs:51 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::metadata` · framework/src/mail/mailable_registry.rs:53 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::priority` · framework/src/mail/mailable_registry.rs:55 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::headers` · framework/src/mail/mailable_registry.rs:57 (required)
  - [ ] fn `suprnova::mail::mailable_registry::AnyMailable::return_path` · framework/src/mail/mailable_registry.rs:59 (required)

### `suprnova::mail::mailable`

- [ ] fn `suprnova::mail::register_mailable_factory` · framework/src/mail/mailable.rs:184 (also `suprnova::mail::mailable::register_mailable_factory`)
- [ ] trait `suprnova::Mailable` · framework/src/mail/mailable.rs:33 (also `suprnova::mail::Mailable`, `suprnova::mail::mailable::Mailable`, `suprnova::prelude::Mailable`)
  - Implemented here by: `EmailVerificationMail`, `PasswordChangedMail`, `PasswordResetMail`
  - [ ] fn `suprnova::Mailable::mailable_name` · framework/src/mail/mailable.rs:35 (required)
  - [ ] fn `suprnova::Mailable::subject` · framework/src/mail/mailable.rs:44 (required)
  - [ ] fn `suprnova::Mailable::subject_template_source` · framework/src/mail/mailable.rs:57 (provided)
  - [ ] fn `suprnova::Mailable::html_template_source` · framework/src/mail/mailable.rs:62 (provided)
  - [ ] fn `suprnova::Mailable::text_template_source` · framework/src/mail/mailable.rs:67 (provided)
  - [ ] fn `suprnova::Mailable::from` · framework/src/mail/mailable.rs:72 (provided)
  - [ ] fn `suprnova::Mailable::attachments` · framework/src/mail/mailable.rs:77 (provided)
  - [ ] fn `suprnova::Mailable::tags` · framework/src/mail/mailable.rs:84 (provided)
  - [ ] fn `suprnova::Mailable::metadata` · framework/src/mail/mailable.rs:91 (provided)
  - [ ] fn `suprnova::Mailable::priority` · framework/src/mail/mailable.rs:97 (provided)
  - [ ] fn `suprnova::Mailable::headers` · framework/src/mail/mailable.rs:104 (provided)
  - [ ] fn `suprnova::Mailable::return_path` · framework/src/mail/mailable.rs:110 (provided)
  - [ ] fn `suprnova::Mailable::queue` · framework/src/mail/mailable.rs:124 (provided)
  - [ ] fn `suprnova::Mailable::render_subject` · framework/src/mail/mailable.rs:135 (provided)
  - [ ] fn `suprnova::Mailable::render_html` · framework/src/mail/mailable.rs:145 (provided)
  - [ ] fn `suprnova::Mailable::render_text` · framework/src/mail/mailable.rs:153 (provided)

### `suprnova::mail::mailgun`

- [ ] struct `suprnova::mail::mailgun::MailgunMailTransport` · framework/src/mail/mailgun.rs:20
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::mailgun::MailgunMailTransport::new` · framework/src/mail/mailgun.rs:30
  - [ ] fn `suprnova::mail::mailgun::MailgunMailTransport::with_endpoint` · framework/src/mail/mailgun.rs:40

### `suprnova::mail::memory`

- [ ] struct `suprnova::mail::memory::InMemoryMailTransport` · framework/src/mail/memory.rs:11
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::memory::InMemoryMailTransport::new` · framework/src/mail/memory.rs:17
  - [ ] fn `suprnova::mail::memory::InMemoryMailTransport::captured` · framework/src/mail/memory.rs:22
  - [ ] fn `suprnova::mail::memory::InMemoryMailTransport::clear` · framework/src/mail/memory.rs:27

### `suprnova::mail::postmark`

- [ ] struct `suprnova::mail::postmark::PostmarkMailTransport` · framework/src/mail/postmark.rs:15
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::postmark::PostmarkMailTransport::new` · framework/src/mail/postmark.rs:22
  - [ ] fn `suprnova::mail::postmark::PostmarkMailTransport::with_endpoint` · framework/src/mail/postmark.rs:32

### `suprnova::mail::resend`

- [ ] struct `suprnova::mail::resend::ResendMailTransport` · framework/src/mail/resend.rs:16
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::resend::ResendMailTransport::new` · framework/src/mail/resend.rs:23
  - [ ] fn `suprnova::mail::resend::ResendMailTransport::with_endpoint` · framework/src/mail/resend.rs:33

### `suprnova::mail::send_job`

- [ ] struct `suprnova::SendMailJob` · framework/src/mail/send_job.rs:26 (also `suprnova::mail::SendMailJob`, `suprnova::mail::send_job::SendMailJob`)
  - Public fields: `to`, `cc`, `bcc`, `reply_to`, `from_override`, `mailable_name`, `mailable_payload`, `tags`, `metadata`, `priority`, `headers`, `return_path`, `subject_override`, `attachments`
  - Implements: `suprnova::Job`

### `suprnova::mail::sendgrid`

- [ ] struct `suprnova::mail::sendgrid::SendGridMailTransport` · framework/src/mail/sendgrid.rs:15
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::sendgrid::SendGridMailTransport::new` · framework/src/mail/sendgrid.rs:22
  - [ ] fn `suprnova::mail::sendgrid::SendGridMailTransport::with_endpoint` · framework/src/mail/sendgrid.rs:32

### `suprnova::mail::ses`

- [ ] struct `suprnova::mail::ses::SesMailTransport` · framework/src/mail/ses.rs:52
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::ses::SesMailTransport::new` · framework/src/mail/ses.rs:65
  - [ ] fn `suprnova::mail::ses::SesMailTransport::with_endpoint` · framework/src/mail/ses.rs:86
  - [ ] fn `suprnova::mail::ses::SesMailTransport::tenant_name` · framework/src/mail/ses.rs:116
  - [ ] fn `suprnova::mail::ses::SesMailTransport::configuration_set_name` · framework/src/mail/ses.rs:129
  - [ ] fn `suprnova::mail::ses::SesMailTransport::list_management` · framework/src/mail/ses.rs:141

### `suprnova::mail::smtp`

- [ ] struct `suprnova::mail::smtp::SmtpMailTransport` · framework/src/mail/smtp.rs:11
  - Implements: `suprnova::mail::MailTransport`
  - [ ] fn `suprnova::mail::smtp::SmtpMailTransport::starttls` · framework/src/mail/smtp.rs:18
  - [ ] fn `suprnova::mail::smtp::SmtpMailTransport::tls` · framework/src/mail/smtp.rs:35
  - [ ] fn `suprnova::mail::smtp::SmtpMailTransport::unencrypted` · framework/src/mail/smtp.rs:46

### `suprnova::mail::transport`

- [ ] fn `suprnova::mail::dispatch_with_telemetry` · framework/src/mail/transport.rs:195 (also `suprnova::mail::transport::dispatch_with_telemetry`)
- [ ] struct `suprnova::OutgoingMessage` · framework/src/mail/transport.rs:35 (also `suprnova::mail::OutgoingMessage`, `suprnova::mail::transport::OutgoingMessage`)
  - Public fields: `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `html`, `text`, `attachments`, `tags`, `metadata`, `priority`, `headers`, `return_path`
  - [ ] fn `suprnova::OutgoingMessage::new` · framework/src/mail/transport.rs:78
  - [ ] fn `suprnova::OutgoingMessage::has_to` · framework/src/mail/transport.rs:100
  - [ ] fn `suprnova::OutgoingMessage::has_cc` · framework/src/mail/transport.rs:105
  - [ ] fn `suprnova::OutgoingMessage::has_bcc` · framework/src/mail/transport.rs:110
  - [ ] fn `suprnova::OutgoingMessage::has_reply_to` · framework/src/mail/transport.rs:115
  - [ ] fn `suprnova::OutgoingMessage::has_from` · framework/src/mail/transport.rs:120
  - [ ] fn `suprnova::OutgoingMessage::has_subject` · framework/src/mail/transport.rs:125
  - [ ] fn `suprnova::OutgoingMessage::has_attachment` · framework/src/mail/transport.rs:131
  - [ ] fn `suprnova::OutgoingMessage::has_tag` · framework/src/mail/transport.rs:136
  - [ ] fn `suprnova::OutgoingMessage::has_metadata` · framework/src/mail/transport.rs:141
  - [ ] fn `suprnova::OutgoingMessage::metadata_equals` · framework/src/mail/transport.rs:146
  - [ ] fn `suprnova::OutgoingMessage::has_header` · framework/src/mail/transport.rs:151
- [ ] trait `suprnova::mail::MailTransport` · framework/src/mail/transport.rs:165 (also `suprnova::mail::transport::MailTransport`)
  - Implemented here by: `mail::file::FileMailTransport`, `mail::log::LogMailTransport`, `mail::mailgun::MailgunMailTransport`, `mail::memory::InMemoryMailTransport`, `mail::postmark::PostmarkMailTransport`, `mail::resend::ResendMailTransport`, `mail::sendgrid::SendGridMailTransport`, `mail::ses::SesMailTransport`, `mail::smtp::SmtpMailTransport`
  - [ ] fn `suprnova::mail::MailTransport::send` · framework/src/mail/transport.rs:169 (required)
  - [ ] fn `suprnova::mail::MailTransport::name` · framework/src/mail/transport.rs:173 (provided)
- [ ] const `suprnova::mail::PRIORITY_HIGH` · framework/src/mail/transport.rs:25 (also `suprnova::mail::transport::PRIORITY_HIGH`)
- [ ] const `suprnova::mail::PRIORITY_HIGHEST` · framework/src/mail/transport.rs:23 (also `suprnova::mail::transport::PRIORITY_HIGHEST`)
- [ ] const `suprnova::mail::PRIORITY_LOW` · framework/src/mail/transport.rs:29 (also `suprnova::mail::transport::PRIORITY_LOW`)
- [ ] const `suprnova::mail::PRIORITY_LOWEST` · framework/src/mail/transport.rs:31 (also `suprnova::mail::transport::PRIORITY_LOWEST`)
- [ ] const `suprnova::mail::PRIORITY_NORMAL` · framework/src/mail/transport.rs:27 (also `suprnova::mail::transport::PRIORITY_NORMAL`)

### `suprnova::mail`

- [ ] struct `suprnova::Mail` · framework/src/mail/mod.rs:109 (also `suprnova::mail::Mail`, `suprnova::prelude::Mail`)
  - [ ] fn `suprnova::Mail::set_transport` · framework/src/mail/mod.rs:114
  - [ ] fn `suprnova::Mail::clear_transport` · framework/src/mail/mod.rs:120
  - [ ] fn `suprnova::Mail::to` · framework/src/mail/mod.rs:127
  - [ ] fn `suprnova::Mail::cc` · framework/src/mail/mod.rs:132
  - [ ] fn `suprnova::Mail::bcc` · framework/src/mail/mod.rs:137
  - [ ] fn `suprnova::Mail::raw` · framework/src/mail/mod.rs:152
  - [ ] fn `suprnova::Mail::html` · framework/src/mail/mod.rs:162
  - [ ] fn `suprnova::Mail::always_from` · framework/src/mail/mod.rs:173
  - [ ] fn `suprnova::Mail::always_reply_to` · framework/src/mail/mod.rs:181
  - [ ] fn `suprnova::Mail::always_to` · framework/src/mail/mod.rs:189
  - [ ] fn `suprnova::Mail::always_return_path` · framework/src/mail/mod.rs:196
  - [ ] fn `suprnova::Mail::forget_always` · framework/src/mail/mod.rs:204
  - [ ] fn `suprnova::Mail::fake` · framework/src/mail/mod.rs:226
- [ ] struct `suprnova::MailBuilder` · framework/src/mail/mod.rs:286 (also `suprnova::mail::MailBuilder`)
  - [ ] fn `suprnova::MailBuilder::to` · framework/src/mail/mod.rs:310
  - [ ] fn `suprnova::MailBuilder::cc` · framework/src/mail/mod.rs:315
  - [ ] fn `suprnova::MailBuilder::bcc` · framework/src/mail/mod.rs:320
  - [ ] fn `suprnova::MailBuilder::reply_to` · framework/src/mail/mod.rs:325
  - [ ] fn `suprnova::MailBuilder::from` · framework/src/mail/mod.rs:331
  - [ ] fn `suprnova::MailBuilder::return_path` · framework/src/mail/mod.rs:336
  - [ ] fn `suprnova::MailBuilder::on_queue` · framework/src/mail/mod.rs:346
  - [ ] fn `suprnova::MailBuilder::on_connection` · framework/src/mail/mod.rs:352
  - [ ] fn `suprnova::MailBuilder::tag` · framework/src/mail/mod.rs:358
  - [ ] fn `suprnova::MailBuilder::metadata` · framework/src/mail/mod.rs:363
  - [ ] fn `suprnova::MailBuilder::priority` · framework/src/mail/mod.rs:369
  - [ ] fn `suprnova::MailBuilder::header` · framework/src/mail/mod.rs:374
  - [ ] fn `suprnova::MailBuilder::subject` · framework/src/mail/mod.rs:380
  - [ ] fn `suprnova::MailBuilder::html` · framework/src/mail/mod.rs:385
  - [ ] fn `suprnova::MailBuilder::text` · framework/src/mail/mod.rs:390
  - [ ] fn `suprnova::MailBuilder::attach` · framework/src/mail/mod.rs:396
  - [ ] fn `suprnova::MailBuilder::reply_to_address` · framework/src/mail/mod.rs:405
  - [ ] fn `suprnova::MailBuilder::send` · framework/src/mail/mod.rs:410
  - [ ] fn `suprnova::MailBuilder::queue` · framework/src/mail/mod.rs:499
  - [ ] fn `suprnova::MailBuilder::later` · framework/src/mail/mod.rs:524
- [ ] struct `suprnova::MailFake` · framework/src/mail/mod.rs:642 (also `suprnova::mail::MailFake`, `suprnova::prelude::MailFake`)
  - [ ] fn `suprnova::MailFake::captured` · framework/src/mail/mod.rs:649
  - [ ] fn `suprnova::MailFake::count` · framework/src/mail/mod.rs:654
  - [ ] fn `suprnova::MailFake::queued` · framework/src/mail/mod.rs:661
  - [ ] fn `suprnova::MailFake::queued_count` · framework/src/mail/mod.rs:678
  - [ ] fn `suprnova::MailFake::outgoing_count` · framework/src/mail/mod.rs:684
  - [ ] fn `suprnova::MailFake::sent` · framework/src/mail/mod.rs:689
  - [ ] fn `suprnova::MailFake::sent_to` · framework/src/mail/mod.rs:702
  - [ ] fn `suprnova::MailFake::queued_named` · framework/src/mail/mod.rs:707
  - [ ] fn `suprnova::MailFake::queued_to` · framework/src/mail/mod.rs:715
  - [ ] fn `suprnova::MailFake::assert_sent` · framework/src/mail/mod.rs:724
  - [ ] fn `suprnova::MailFake::assert_sent_to` · framework/src/mail/mod.rs:742
  - [ ] fn `suprnova::MailFake::assert_not_sent` · framework/src/mail/mod.rs:754
  - [ ] fn `suprnova::MailFake::assert_not_sent_to` · framework/src/mail/mod.rs:768
  - [ ] fn `suprnova::MailFake::assert_sent_count` · framework/src/mail/mod.rs:778
  - [ ] fn `suprnova::MailFake::assert_nothing_sent` · framework/src/mail/mod.rs:787
  - [ ] fn `suprnova::MailFake::assert_queued` · framework/src/mail/mod.rs:799
  - [ ] fn `suprnova::MailFake::assert_queued_with` · framework/src/mail/mod.rs:815
  - [ ] fn `suprnova::MailFake::assert_not_queued` · framework/src/mail/mod.rs:831
  - [ ] fn `suprnova::MailFake::assert_queued_to` · framework/src/mail/mod.rs:844
  - [ ] fn `suprnova::MailFake::assert_nothing_queued` · framework/src/mail/mod.rs:856
  - [ ] fn `suprnova::MailFake::assert_queued_count` · framework/src/mail/mod.rs:867
  - [ ] fn `suprnova::MailFake::queued_on` · framework/src/mail/mod.rs:876
  - [ ] fn `suprnova::MailFake::assert_queued_on` · framework/src/mail/mod.rs:887
  - [ ] fn `suprnova::MailFake::queued_on_connection` · framework/src/mail/mod.rs:907
  - [ ] fn `suprnova::MailFake::assert_queued_on_connection` · framework/src/mail/mod.rs:921
  - [ ] fn `suprnova::MailFake::assert_not_outgoing` · framework/src/mail/mod.rs:938
  - [ ] fn `suprnova::MailFake::assert_nothing_outgoing` · framework/src/mail/mod.rs:947
  - [ ] fn `suprnova::MailFake::assert_outgoing_count` · framework/src/mail/mod.rs:953
- [ ] struct `suprnova::QueuedSnapshot` · framework/src/mail/mod.rs:981 (also `suprnova::mail::QueuedSnapshot`)
  - Public fields: `mailable_name`, `payload`, `to`, `cc`, `bcc`, `delay`, `queue`, `connection`
  - [ ] fn `suprnova::QueuedSnapshot::decode` · framework/src/mail/mod.rs:1008
  - [ ] fn `suprnova::QueuedSnapshot::has_to` · framework/src/mail/mod.rs:1014
