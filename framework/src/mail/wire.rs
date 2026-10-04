//! The one place a transport turns an [`Address`] or a header into wire text.
//!
//! Each transport used to do this its own way. Postmark and Mailgun joined
//! `Address`'s `Display` output with commas, and `Display` does not quote the
//! display name, so a name such as `attacker@example.com, Victim` became a
//! second recipient and an ordinary `Doe, Jane` broke the list. SMTP and the
//! file transport accepted CR and LF in header names, which lettre writes
//! verbatim. Every transport now builds addresses and headers through these
//! functions, so a value one transport refuses, every transport refuses.
//!
//! Bad input is an error for the caller of `send`. Nothing here rewrites a
//! value into something else: a silent rewrite of an address can change who
//! receives the message.

use crate::error::FrameworkError;
use crate::mail::address::Address;
use crate::mail::transport::OutgoingMessage;
use lettre::message::Mailbox;
use lettre::message::header::{HeaderName, HeaderValue};
use std::fmt::Write as _;

/// Validate `address` and turn it into a lettre [`Mailbox`].
///
/// The email must parse as exactly one RFC 5322 `addr-spec`, so a comma, a
/// space or angle brackets in it are refused rather than read as a list. The
/// display name may hold any printable text, commas and quotes included,
/// because the serializer quotes it; only control characters are refused,
/// since CR and LF cannot be represented inside a quoted string at all.
pub(crate) fn mailbox(transport: &str, address: &Address) -> Result<Mailbox, FrameworkError> {
    if address.email.chars().any(char::is_control) {
        return Err(FrameworkError::param(format!(
            "{transport}: email address {:?} contains a control character",
            address.email
        )));
    }
    if let Some(name) = &address.name
        && name.chars().any(|c| c.is_control() && c != '\t')
    {
        // The name itself stays out of the message: it is caller data, and
        // the email already says which recipient failed.
        return Err(FrameworkError::param(format!(
            "{transport}: the display name for {:?} contains a control character \
             (CR, LF, NUL or another); a display name must be one line of text",
            address.email
        )));
    }
    let email: lettre::Address = address.email.parse().map_err(|e| {
        FrameworkError::param(format!(
            "{transport}: {:?} is not one email address: {e}",
            address.email
        ))
    })?;
    Ok(Mailbox::new(address.name.clone(), email))
}

/// One mailbox as RFC 5322 text: `Name <email>`, with the name quoted and
/// escaped whenever it holds anything outside an atom (a comma, a quote, an
/// `@`, angle brackets), or the bare email when there is no name.
pub(crate) fn mailbox_text(transport: &str, address: &Address) -> Result<String, FrameworkError> {
    let mailbox = mailbox(transport, address)?;
    let mut text = String::new();
    // lettre's `Display` fails only for CR or LF in the name, which
    // `mailbox` has already refused. Writing into a `String` keeps a
    // formatter error an error instead of the panic `to_string` would raise.
    write!(text, "{mailbox}").map_err(|_| {
        FrameworkError::param(format!(
            "{transport}: the display name for {:?} cannot be written as one mailbox",
            address.email
        ))
    })?;
    Ok(text)
}

/// Each address as its own RFC 5322 mailbox, for providers that take a JSON
/// array of recipient strings (SES, Resend).
pub(crate) fn mailbox_texts(
    transport: &str,
    addresses: &[Address],
) -> Result<Vec<String>, FrameworkError> {
    addresses
        .iter()
        .map(|address| mailbox_text(transport, address))
        .collect()
}

/// A comma-separated RFC 5322 mailbox list, for providers that take every
/// recipient in one string (Postmark, Mailgun). The separator is safe only
/// because every name in the list is quoted by [`mailbox_text`].
pub(crate) fn mailbox_list(
    transport: &str,
    addresses: &[Address],
) -> Result<String, FrameworkError> {
    Ok(mailbox_texts(transport, addresses)?.join(", "))
}

/// The bare email of a bounce address, for `Return-Path` and the provider
/// fields that mean the same thing. A return path is an `addr-spec` with no
/// display name, so any name on `address` is validated and then left out.
pub(crate) fn path(transport: &str, address: &Address) -> Result<String, FrameworkError> {
    Ok(mailbox(transport, address)?.email.to_string())
}

/// RFC 5322 `field-name = 1*ftext`, where `ftext` is printable US-ASCII
/// except `:` (bytes 33-57 and 59-126). That one rule refuses CR, LF, NUL,
/// every other control byte, space and `:` - the bytes that end a header
/// early or start a new one.
pub(crate) fn check_header_name(transport: &str, name: &str) -> Result<(), FrameworkError> {
    let valid = !name.is_empty() && name.bytes().all(|b| matches!(b, 33..=57 | 59..=126));
    if valid {
        return Ok(());
    }
    Err(FrameworkError::param(format!(
        "{transport}: invalid header name {name:?}: a header name is one or more \
         printable ASCII characters other than ':', with no CR, LF, NUL or space"
    )))
}

/// A header value may hold any text except CR, LF and NUL. Encoders fold a
/// long value themselves; a raw line break in a value is how it becomes a
/// second header once a provider writes the message out.
pub(crate) fn check_header_value(
    transport: &str,
    name: &str,
    value: &str,
) -> Result<(), FrameworkError> {
    if value.bytes().any(|b| matches!(b, b'\r' | b'\n' | 0)) {
        return Err(FrameworkError::param(format!(
            "{transport}: the value of header {name:?} contains CR, LF or NUL"
        )));
    }
    Ok(())
}

/// Both header checks: [`check_header_name`] and [`check_header_value`].
pub(crate) fn check_header(transport: &str, name: &str, value: &str) -> Result<(), FrameworkError> {
    check_header_name(transport, name)?;
    check_header_value(transport, name, value)
}

/// A validated lettre header for the transports that write MIME themselves
/// (SMTP, file, SES raw). lettre's own name check runs after the shared one
/// for its extra limit, a name of at most 76 bytes.
pub(crate) fn mime_header(
    transport: &str,
    name: &str,
    value: &str,
) -> Result<HeaderValue, FrameworkError> {
    check_header(transport, name, value)?;
    let header_name = HeaderName::new_from_ascii(name.to_string()).map_err(|e| {
        FrameworkError::param(format!("{transport}: invalid header name {name:?}: {e}"))
    })?;
    Ok(HeaderValue::new(header_name, value.to_string()))
}

/// Validate every address and every caller-set header on `msg`.
///
/// Each transport runs this before it serializes anything, so a message is
/// accepted or refused the same way whichever transport is bound, including
/// for fields a given provider does not send (SendGrid keeps one reply-to,
/// Postmark has no return path). Metadata and tags are not headers on every
/// transport, so the transports that turn them into headers check them
/// where they do.
pub(crate) fn check_message(transport: &str, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
    mailbox(transport, &msg.from)?;
    for address in msg
        .to
        .iter()
        .chain(&msg.cc)
        .chain(&msg.bcc)
        .chain(&msg.reply_to)
        .chain(&msg.return_path)
    {
        mailbox(transport, address)?;
    }
    for (name, value) in &msg.headers {
        check_header(transport, name, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, email: &str) -> Address {
        Address::new(email).with_name(name)
    }

    #[test]
    fn a_display_name_with_list_syntax_is_quoted_into_one_mailbox() {
        let text = mailbox_text(
            "test",
            &named(
                "attacker@evil.example, \"Victim\" <x@evil.example>",
                "victim@example.com",
            ),
        )
        .unwrap();
        assert_eq!(
            text,
            r#""attacker@evil.example, \"Victim\" <x@evil.example>" <victim@example.com>"#
        );
    }

    #[test]
    fn a_plain_name_and_a_bare_address_keep_their_old_text() {
        assert_eq!(
            mailbox_text("test", &named("Suprnova", "noreply@suprnova.dev")).unwrap(),
            "Suprnova <noreply@suprnova.dev>"
        );
        assert_eq!(
            mailbox_text("test", &Address::new("alice@example.org")).unwrap(),
            "alice@example.org"
        );
    }

    #[test]
    fn the_framework_default_sender_is_a_valid_address() {
        // `Mail::raw` and `MailChannel` fall back to this address.
        assert!(mailbox("test", &Address::new("noreply@localhost")).is_ok());
    }

    #[test]
    fn control_characters_in_a_name_or_an_email_are_refused() {
        for name in ["Victim\r\nBcc: a@evil.example", "Victim\nX", "Vic\0tim"] {
            let err = mailbox("test", &named(name, "victim@example.com")).unwrap_err();
            assert!(format!("{err}").contains("display name"), "{err}");
        }
        assert!(mailbox("test", &Address::new("victim@example.com\r\n")).is_err());
    }

    #[test]
    fn an_email_that_is_a_list_is_refused() {
        for email in [
            "victim@example.com, attacker@evil.example",
            "Victim <victim@example.com>",
            "not-an-address",
        ] {
            assert!(mailbox("test", &Address::new(email)).is_err(), "{email}");
        }
    }

    #[test]
    fn a_return_path_is_the_bare_email() {
        assert_eq!(
            path("test", &named("Bounces", "bounces@example.com")).unwrap(),
            "bounces@example.com"
        );
    }

    #[test]
    fn header_names_follow_the_field_name_grammar() {
        for name in ["X-Custom", "List-Unsubscribe", "X-A_b.c~1"] {
            assert!(check_header_name("test", name).is_ok(), "{name}");
        }
        for name in [
            "",
            "X-Bad\r\nReply-To",
            "X-Bad\nHeader",
            "X-Bad\0Header",
            "X-Bad\tHeader",
            "X-Foo: bar",
            "X-Foo Bar",
            "X-Caf\u{e9}",
        ] {
            assert!(check_header_name("test", name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn header_values_refuse_line_breaks_and_nul_only() {
        assert!(check_header_value("test", "X-A", "a: b, \"c\" <d>\t").is_ok());
        for value in ["a\r\nBcc: x", "a\nb", "a\rb", "a\0b"] {
            assert!(
                check_header_value("test", "X-A", value).is_err(),
                "{value:?}"
            );
        }
    }
}
