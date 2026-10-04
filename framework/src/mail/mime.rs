//! lettre MIME construction shared by the SMTP and file transports.
//!
//! These helpers were private to `smtp.rs` until the file transport needed
//! the same envelope, the same header superset, and the same body shape. A
//! preview transport that built its own would drift from what actually goes
//! on the wire, which defeats the point of previewing.
//!
//! Deliberately **not** built on `ses.rs`'s `build_mime`: that one filters
//! `X-SES-*` control headers, omits `priority` / `tags` / `metadata` /
//! `return_path` entirely, and always wraps in `multipart/mixed`.

use crate::error::FrameworkError;
use crate::mail::transport::OutgoingMessage;
use crate::mail::wire;
use lettre::message::{
    Attachment as LettreAttachment, MessageBuilder, MultiPart, SinglePart, header::ContentType,
};

/// Envelope plus the full header superset: custom headers, `X-Priority` and
/// `Importance`, `X-Tag`, `X-Metadata-*`, and `Return-Path`.
///
/// Every address and header goes through [`wire`], so a header name with CR
/// or LF, or a metadata key that would make one, is an error here instead of
/// a second header in the output. `transport` names the caller in that error.
pub(crate) fn base_builder(
    transport: &str,
    msg: &OutgoingMessage,
) -> Result<MessageBuilder, FrameworkError> {
    wire::check_message(transport, msg)?;
    let mut builder = lettre::Message::builder()
        .from(wire::mailbox(transport, &msg.from)?)
        .subject(&msg.subject);

    for a in &msg.to {
        builder = builder.to(wire::mailbox(transport, a)?);
    }
    for a in &msg.cc {
        builder = builder.cc(wire::mailbox(transport, a)?);
    }
    for a in &msg.bcc {
        builder = builder.bcc(wire::mailbox(transport, a)?);
    }
    for a in &msg.reply_to {
        builder = builder.reply_to(wire::mailbox(transport, a)?);
    }

    // Tags / metadata / priority / return-path / custom headers ride on
    // RFC 5322 headers so a backend MTA can route on them.
    for (name, value) in &msg.headers {
        builder = builder.raw_header(wire::mime_header(transport, name, value)?);
    }
    if let Some(p) = msg.priority {
        builder = builder.raw_header(wire::mime_header(transport, "X-Priority", &p.to_string())?);
        // Importance: 1-2 = High, 3 = Normal, 4-5 = Low.
        let imp = match p {
            1..=2 => "High",
            4..=5 => "Low",
            _ => "Normal",
        };
        builder = builder.raw_header(wire::mime_header(transport, "Importance", imp)?);
    }
    for t in &msg.tags {
        builder = builder.raw_header(wire::mime_header(transport, "X-Tag", t)?);
    }
    for (k, v) in &msg.metadata {
        // The metadata key becomes part of a header name here, so it is
        // held to the header-name rule.
        builder = builder.raw_header(wire::mime_header(
            transport,
            format!("X-Metadata-{k}").as_str(),
            v.as_str(),
        )?);
    }
    if let Some(rp) = &msg.return_path {
        builder = builder.raw_header(wire::mime_header(
            transport,
            "Return-Path",
            &wire::email(transport, rp)?,
        )?);
    }

    Ok(builder)
}

/// Build the message body.
///
/// `substitute_empty_body` guards the case where a message carries neither
/// a text nor an HTML body: `MultiPart::alternative().build()` then returns
/// a **zero-part** `multipart/alternative`, which is not valid MIME and
/// which no mail client will open.
///
/// Only the file transport passes `true`. SMTP passes `false` and keeps the
/// exact output it has always produced - fixing SMTP is a behaviour change
/// to a production transport and belongs in its own commit with its own
/// changelog entry.
pub(crate) fn build_body(
    msg: &OutgoingMessage,
    substitute_empty_body: bool,
) -> Result<MultiPart, FrameworkError> {
    let mut alternative = MultiPart::alternative().build();
    if let Some(text) = &msg.text {
        alternative = alternative.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(text.clone()),
        );
    }
    if let Some(html) = &msg.html {
        alternative = alternative.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_HTML)
                .body(html.clone()),
        );
    }
    if substitute_empty_body && msg.text.is_none() && msg.html.is_none() {
        alternative = alternative.singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(String::new()),
        );
    }

    if msg.attachments.is_empty() {
        return Ok(alternative);
    }

    let mut mixed = MultiPart::mixed().multipart(alternative);
    for att in &msg.attachments {
        let ct: ContentType = att.content_type.parse().map_err(|e| {
            FrameworkError::internal(format!(
                "mail attachment content-type {}: {e}",
                att.content_type
            ))
        })?;
        let part = LettreAttachment::new(att.filename.clone()).body(att.content.clone(), ct);
        mixed = mixed.singlepart(part);
    }
    Ok(mixed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::address::Address;

    fn bodiless() -> OutgoingMessage {
        OutgoingMessage {
            from: Address {
                email: "from@example.test".into(),
                name: None,
            },
            to: vec![Address {
                email: "to@example.test".into(),
                name: None,
            }],
            cc: vec![],
            bcc: vec![],
            reply_to: vec![],
            subject: "s".into(),
            html: None,
            text: None,
            attachments: vec![],
            tags: vec![],
            metadata: Default::default(),
            priority: None,
            headers: vec![],
            return_path: None,
        }
    }

    #[test]
    fn the_guard_changes_the_file_body_and_only_the_file_body() {
        let msg = bodiless();
        let smtp = build_body(&msg, false).expect("builds");
        let file = build_body(&msg, true).expect("builds");
        assert_ne!(
            smtp.formatted(),
            file.formatted(),
            "the guard must actually substitute a part"
        );
    }

    #[test]
    fn smtp_body_for_a_bodiless_message_is_unchanged_by_the_hoist() {
        // Pins SMTP's historical output: a zero-part multipart/alternative
        // with no `Content-Type: text/plain` part inside it.
        let msg = bodiless();
        let raw = String::from_utf8_lossy(&build_body(&msg, false).expect("builds").formatted())
            .into_owned();
        assert!(
            raw.contains("multipart/alternative"),
            "expected an alternative container:\n{raw}"
        );
        assert!(
            !raw.contains("text/plain"),
            "SMTP must still produce no body part here:\n{raw}"
        );
    }
}
