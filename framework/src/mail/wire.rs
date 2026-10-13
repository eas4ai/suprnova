//! The mail serializer: how an [`Address`], a header or a whole
//! [`OutgoingMessage`] becomes wire text, and what is refused on the way.
//!
//! Every built-in transport serializes through these functions, and the
//! dispatch path ([`Mail`](crate::mail::Mail) sends, queued mail, the
//! notification mail channel) runs [`check_message`] before any transport,
//! including one bound with `Mail::set_transport` or `Mail::fake`, sees the
//! message. A message is therefore accepted or refused the same way
//! whichever transport is bound. The only extra refusals are provider limits
//! a transport cannot carry: Postmark takes one tag, and SES tags take only
//! `[A-Za-z0-9_-]`.
//!
//! A transport you write should build recipient text with [`mailbox_text`]
//! or [`mailbox_list`], never with `Address`'s `Display`: `Display` does not
//! quote the display name, so a name such as `attacker@example.com, Victim`
//! reads as a second recipient in a comma-separated list.
//!
//! Input that would change who receives the message or which headers it
//! carries is an error from `send`, never rewritten. The error is
//! [`FrameworkError::internal`]: a bad address is a fault in the application,
//! so an HTTP response shows a 500 and keeps the detail in the logs. Two
//! harmless forms are normalized instead of refused: a line break in a
//! display name becomes one space, and whitespace around an email address is
//! trimmed.
//!
//! Every function takes a `transport` label that names the caller in the
//! error message, for example `"acme"`.

use crate::error::FrameworkError;
use crate::mail::address::Address;
use crate::mail::transport::OutgoingMessage;
use lettre::message::Mailbox;
use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use std::borrow::Cow;

/// lettre's limit on a header name, applied on every transport so a name
/// one transport refuses is refused by all of them.
const MAX_HEADER_NAME_BYTES: usize = 76;

/// The prefix SMTP, the file transport and Resend put before a metadata key
/// to make it a header name.
const METADATA_HEADER_PREFIX: &str = "X-Metadata-";

/// Headers the message structure owns. Setting one as a custom header would
/// duplicate or contradict a typed field - a raw `Bcc` header even added an
/// SMTP recipient - so they are refused in favour of the builder methods.
const STRUCTURAL_HEADERS: [&str; 11] = [
    "To",
    "Cc",
    "Bcc",
    "From",
    "Sender",
    "Reply-To",
    "Return-Path",
    "Subject",
    "Date",
    "Message-ID",
    "MIME-Version",
];

/// The characters that end a line: CR, LF, VT, FF, NEL and the Unicode line
/// and paragraph separators.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\r' | '\n' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// A character no single-line text field may hold: any control character
/// other than TAB, and the Unicode line and paragraph separators.
fn is_forbidden_in_text(c: char) -> bool {
    (c.is_control() && c != '\t') || matches!(c, '\u{2028}' | '\u{2029}')
}

/// The display name of `address` as it goes on the wire, or `None` when it
/// has none.
///
/// Each run of line breaks becomes one space and the result is trimmed, so a
/// name pasted with a line break still sends; any other control character
/// is refused. The name may hold commas, quotes and angle brackets:
/// [`mailbox_text`] quotes it. Structured providers such as SendGrid take
/// this value as their `name` field.
pub fn display_name(transport: &str, address: &Address) -> Result<Option<String>, FrameworkError> {
    let Some(raw) = &address.name else {
        return Ok(None);
    };
    let mut name = String::with_capacity(raw.len());
    let mut in_break = false;
    for c in raw.chars() {
        if is_line_break(c) {
            if !in_break {
                name.push(' ');
            }
            in_break = true;
            continue;
        }
        in_break = false;
        if c.is_control() && c != '\t' {
            // The name itself stays out of the message: it is caller data,
            // and the email already says which recipient failed.
            return Err(FrameworkError::internal(format!(
                "{transport}: the display name for {:?} contains a control character",
                address.email.trim()
            )));
        }
        name.push(c);
    }
    let name = name.trim();
    Ok((!name.is_empty()).then(|| name.to_string()))
}

/// The email of `address` as it goes on the wire: one RFC 5322 `addr-spec`
/// written as a dot-atom, with surrounding whitespace trimmed.
///
/// A quoted local part (`"a, b"@example.com`) and a domain literal
/// (`user@[127.0.0.1]`) are refused, as is any comma, space, quote, angle or
/// square bracket, parenthesis, colon, semicolon or backslash. Those are the
/// characters a provider could read as list or address syntax. This is
/// also the bare form for `Return-Path` and the provider fields that mean
/// the same thing.
pub fn email(transport: &str, address: &Address) -> Result<String, FrameworkError> {
    Ok(parsed_email(transport, address)?.to_string())
}

fn parsed_email(transport: &str, address: &Address) -> Result<lettre::Address, FrameworkError> {
    let email = address.email.trim();
    let structural = |c: char| c.is_whitespace() || c.is_control() || "\"(),:;<>[\\]".contains(c);
    if email.chars().any(structural) {
        return Err(FrameworkError::internal(format!(
            "{transport}: {email:?} is not one email address: a quoted local part, a \
             domain literal, whitespace and the characters \"(),:;<>[\\] are not accepted"
        )));
    }
    email.parse().map_err(|e| {
        FrameworkError::internal(format!(
            "{transport}: {email:?} is not one email address: {e}"
        ))
    })
}

/// Validate `address` and turn it into a lettre [`Mailbox`] for the
/// transports that let lettre write the headers (SMTP, file, SES raw).
pub(crate) fn mailbox(transport: &str, address: &Address) -> Result<Mailbox, FrameworkError> {
    Ok(Mailbox::new(
        display_name(transport, address)?,
        parsed_email(transport, address)?,
    ))
}

/// One mailbox as RFC 5322 text: `Name <email>`, or the bare email when
/// there is no name.
///
/// The name is written bare only when every character is an ASCII atom
/// character or a space. Anything else is quoted, with `"` and `\` escaped:
/// a comma, a quote, an `@`, angle brackets, any non-ASCII character (a
/// provider that normalizes a fullwidth comma to `,` must not see one
/// outside quotes), and any name holding `=?`, which a provider could decode
/// as an RFC 2047 encoded word before it splits the list. Inside a quoted
/// string an encoded word is never decoded.
pub fn mailbox_text(transport: &str, address: &Address) -> Result<String, FrameworkError> {
    let email = email(transport, address)?;
    let Some(name) = display_name(transport, address)? else {
        return Ok(email);
    };
    let bare = !name.contains("=?")
        && name
            .chars()
            .all(|c| c == ' ' || c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c));
    if bare {
        return Ok(format!("{name} <{email}>"));
    }
    let mut text = String::with_capacity(name.len() + email.len() + 6);
    text.push('"');
    for c in name.chars() {
        if matches!(c, '"' | '\\') {
            text.push('\\');
        }
        text.push(c);
    }
    text.push_str("\" <");
    text.push_str(&email);
    text.push('>');
    Ok(text)
}

/// Each address as its own RFC 5322 mailbox, for providers that take a JSON
/// array of recipient strings (SES, Resend).
pub fn mailbox_texts(
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
/// because [`mailbox_text`] quotes every name that needs it.
pub fn mailbox_list(transport: &str, addresses: &[Address]) -> Result<String, FrameworkError> {
    Ok(mailbox_texts(transport, addresses)?.join(", "))
}

/// Check a header name: RFC 5322 `field-name = 1*ftext`, where `ftext` is
/// printable US-ASCII except `:` (bytes 33-57 and 59-126), at most 76 bytes.
///
/// That one rule refuses CR, LF, NUL, every other control character, space,
/// `:` and anything outside ASCII - the bytes that end a header early or
/// start a new one.
pub fn check_header_name(transport: &str, name: &str) -> Result<(), FrameworkError> {
    let valid = !name.is_empty()
        && name.len() <= MAX_HEADER_NAME_BYTES
        && name.bytes().all(|b| matches!(b, 33..=57 | 59..=126));
    if valid {
        return Ok(());
    }
    Err(FrameworkError::internal(format!(
        "{transport}: invalid header name {name:?}: a header name is 1 to 76 printable \
         ASCII characters other than ':', with no control character or space"
    )))
}

/// Check a header value: any text except a control character other than
/// TAB, and the Unicode line and paragraph separators.
///
/// A raw line break in a value is how it becomes a second header once a
/// provider writes the message out. lettre folds a long value at its spaces;
/// a single word longer than a line is written unbroken.
pub fn check_header_value(transport: &str, name: &str, value: &str) -> Result<(), FrameworkError> {
    if value.chars().any(is_forbidden_in_text) {
        return Err(FrameworkError::internal(format!(
            "{transport}: the value of header {name:?} contains a line break or a \
             control character"
        )));
    }
    Ok(())
}

/// Check a header a caller sets: [`check_header_name`],
/// [`check_header_value`], and a refusal for the headers the message
/// structure owns (`To`, `Cc`, `Bcc`, `From`, `Sender`, `Reply-To`,
/// `Return-Path`, `Subject`, `Date`, `Message-ID`, `MIME-Version` and every
/// `Content-*`). Those are set through the builder methods.
pub fn check_header(transport: &str, name: &str, value: &str) -> Result<(), FrameworkError> {
    check_header_name(transport, name)?;
    let structural = STRUCTURAL_HEADERS
        .iter()
        .any(|owned| name.eq_ignore_ascii_case(owned))
        || name
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Content-"));
    if structural {
        return Err(FrameworkError::internal(format!(
            "{transport}: header {name:?} is set by the message itself; use the builder \
             methods (to, cc, bcc, from, reply_to, return_path, subject, attach) instead"
        )));
    }
    check_header_value(transport, name, value)
}

/// A validated lettre header for the transports that write MIME themselves
/// (SMTP, file, SES raw). The caller decides whether `name` is a caller
/// header ([`check_header`]) or one the transport adds itself.
pub(crate) fn mime_header(
    transport: &str,
    name: &str,
    value: &str,
) -> Result<HeaderValue, FrameworkError> {
    check_header_name(transport, name)?;
    check_header_value(transport, name, value)?;
    let header_name = HeaderName::new_from_ascii(name.to_string()).map_err(|e| {
        FrameworkError::internal(format!("{transport}: invalid header name {name:?}: {e}"))
    })?;
    Ok(HeaderValue::new(header_name, value.to_string()))
}

/// A single-line text field that is not a header everywhere (a tag, a
/// metadata value, an attachment name): no control character, TAB included.
fn check_text(transport: &str, what: &str, value: &str) -> Result<(), FrameworkError> {
    if value
        .chars()
        .any(|c| c.is_control() || is_forbidden_in_text(c))
    {
        return Err(FrameworkError::internal(format!(
            "{transport}: {what} {value:?} contains a line break or a control character"
        )));
    }
    Ok(())
}

/// Validate the whole message: every address, the subject, every caller
/// header, every tag, every metadata key and value, and every attachment's
/// name and content type.
///
/// The dispatch path runs this before `MessageSending` fires and before any
/// transport sees the message; each built-in transport runs it again so a
/// direct `send` call is checked too. A metadata key follows the header-name
/// grammar on every transport, because SMTP, the file transport and Resend
/// write it into an `X-Metadata-<key>` header name.
pub fn check_message(transport: &str, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
    for address in addresses(msg) {
        email(transport, address)?;
        display_name(transport, address)?;
    }
    check_header_value(transport, "Subject", &msg.subject)?;
    for (name, value) in &msg.headers {
        check_header(transport, name, value)?;
    }
    for tag in &msg.tags {
        check_text(transport, "tag", tag)?;
    }
    for (key, value) in &msg.metadata {
        let valid_key = !key.is_empty()
            && METADATA_HEADER_PREFIX.len() + key.len() <= MAX_HEADER_NAME_BYTES
            && key.bytes().all(|b| matches!(b, 33..=57 | 59..=126));
        if !valid_key {
            return Err(FrameworkError::internal(format!(
                "{transport}: invalid metadata key {key:?}: a metadata key is 1 to 65 \
                 printable ASCII characters other than ':', with no space, because it \
                 becomes part of an X-Metadata-<key> header name"
            )));
        }
        check_text(transport, "metadata value", value)?;
    }
    for attachment in &msg.attachments {
        check_text(transport, "attachment name", &attachment.filename)?;
        attachment
            .content_type
            .parse::<ContentType>()
            .map_err(|e| {
                FrameworkError::internal(format!(
                    "{transport}: attachment {:?} has content type {:?}, which is not a \
                     MIME type: {e}",
                    attachment.filename, attachment.content_type
                ))
            })?;
    }
    Ok(())
}

/// [`check_message`], then the message with each address in its wire form:
/// line breaks in display names collapsed and emails trimmed.
///
/// The dispatch path hands this to the transport, so a transport that reads
/// `Address` fields directly never sees the raw forms. Borrows `msg` when
/// nothing changes, which is the common case, so attachments are not copied.
pub(crate) fn prepare<'a>(
    transport: &str,
    msg: &'a OutgoingMessage,
) -> Result<Cow<'a, OutgoingMessage>, FrameworkError> {
    check_message(transport, msg)?;
    let mut unchanged = true;
    for address in addresses(msg) {
        if email(transport, address)? != address.email
            || display_name(transport, address)? != address.name
        {
            unchanged = false;
            break;
        }
    }
    if unchanged {
        return Ok(Cow::Borrowed(msg));
    }
    let mut prepared = msg.clone();
    let OutgoingMessage {
        from,
        to,
        cc,
        bcc,
        reply_to,
        return_path,
        ..
    } = &mut prepared;
    for address in std::iter::once(from)
        .chain(to.iter_mut())
        .chain(cc.iter_mut())
        .chain(bcc.iter_mut())
        .chain(reply_to.iter_mut())
        .chain(return_path.iter_mut())
    {
        address.name = display_name(transport, address)?;
        address.email = email(transport, address)?;
    }
    Ok(Cow::Owned(prepared))
}

/// Every address on `msg`: from, to, cc, bcc, reply-to and return path.
fn addresses(msg: &OutgoingMessage) -> impl Iterator<Item = &Address> {
    std::iter::once(&msg.from)
        .chain(&msg.to)
        .chain(&msg.cc)
        .chain(&msg.bcc)
        .chain(&msg.reply_to)
        .chain(&msg.return_path)
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
    fn a_plain_name_stays_bare_and_a_non_ascii_name_is_quoted() {
        assert_eq!(
            mailbox_text("test", &named("Suprnova Orders", "noreply@suprnova.dev")).unwrap(),
            "Suprnova Orders <noreply@suprnova.dev>"
        );
        assert_eq!(
            mailbox_text("test", &named("José García", "jose@example.com")).unwrap(),
            r#""José García" <jose@example.com>"#
        );
        assert_eq!(
            mailbox_text("test", &Address::new("alice@example.org")).unwrap(),
            "alice@example.org"
        );
    }

    #[test]
    fn a_name_holding_an_encoded_word_or_a_backslash_is_quoted() {
        assert_eq!(
            mailbox_text("test", &named("=?utf-8?b?QQ==?=", "a@example.com")).unwrap(),
            r#""=?utf-8?b?QQ==?=" <a@example.com>"#
        );
        assert_eq!(
            mailbox_text("test", &named(r"Back\slash", "a@example.com")).unwrap(),
            r#""Back\\slash" <a@example.com>"#
        );
    }

    #[test]
    fn the_framework_default_sender_is_a_valid_address() {
        // `Mail::raw` and `MailChannel` fall back to this address.
        assert!(mailbox("test", &Address::new("noreply@localhost")).is_ok());
    }

    #[test]
    fn line_breaks_in_a_name_collapse_and_other_controls_are_refused() {
        assert_eq!(
            display_name("test", &named(" A\r\n\r\nB\u{2028}C\n", "a@example.com")).unwrap(),
            Some("A B C".to_string())
        );
        assert_eq!(
            display_name("test", &named("\r\n", "a@example.com")).unwrap(),
            None
        );
        for name in ["Vic\0tim", "Vic\x01tim", "Vic\x7ftim"] {
            let err = display_name("test", &named(name, "victim@example.com")).unwrap_err();
            assert!(format!("{err}").contains("display name"), "{err}");
        }
    }

    #[test]
    fn an_email_is_trimmed_and_anything_but_one_dot_atom_address_is_refused() {
        assert_eq!(
            email("test", &Address::new(" victim@example.com\t")).unwrap(),
            "victim@example.com"
        );
        for bad in [
            "victim@example.com, attacker@evil.example",
            "Victim <victim@example.com>",
            "not-an-address",
            "\"quoted\"@example.com",
            "victim@[127.0.0.1]",
            "vic tim@example.com",
            "victim@exa\r\nmple.com",
        ] {
            assert!(email("test", &Address::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn header_names_follow_the_field_name_grammar() {
        for name in ["X-Custom", "List-Unsubscribe", "X-A_b.c~1"] {
            assert!(check_header_name("test", name).is_ok(), "{name}");
        }
        let long = "X".repeat(77);
        for name in [
            "",
            "X-Bad\r\nReply-To",
            "X-Bad\nHeader",
            "X-Bad\0Header",
            "X-Bad\tHeader",
            "X-Foo: bar",
            "X-Foo Bar",
            "X-Caf\u{e9}",
            long.as_str(),
        ] {
            assert!(check_header_name("test", name).is_err(), "{name:?}");
        }
    }

    #[test]
    fn header_values_refuse_line_breaks_and_controls_but_keep_tab() {
        assert!(check_header_value("test", "X-A", "a: b, \"c\" <d>\t").is_ok());
        for value in [
            "a\r\nBcc: x",
            "a\nb",
            "a\rb",
            "a\0b",
            "a\x01b",
            "a\x7fb",
            "a\u{2028}b",
            "a\u{2029}b",
        ] {
            assert!(
                check_header_value("test", "X-A", value).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn structural_headers_are_refused_as_caller_headers() {
        for name in [
            "Bcc",
            "to",
            "Content-Type",
            "content-disposition",
            "MIME-Version",
        ] {
            assert!(check_header("test", name, "x").is_err(), "{name}");
        }
        for name in ["List-Unsubscribe", "X-Content-Id", "Contents"] {
            assert!(check_header("test", name, "x").is_ok(), "{name}");
        }
    }

    #[test]
    fn prepare_borrows_a_clean_message_and_normalizes_a_raw_one() {
        let mut msg = OutgoingMessage::new(Address::new("from@example.com"));
        msg.to = vec![Address::new("to@example.com").with_name("To")];
        assert!(matches!(prepare("test", &msg).unwrap(), Cow::Borrowed(_)));

        msg.cc = vec![Address::new(" cc@example.com ").with_name("C\r\nC")];
        let prepared = prepare("test", &msg).unwrap();
        assert_eq!(prepared.cc[0].email, "cc@example.com");
        assert_eq!(prepared.cc[0].name.as_deref(), Some("C C"));
        assert_eq!(prepared.to, msg.to);
    }
}
