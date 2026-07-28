//! Log mail transport — emits a `tracing::info!` per send and discards.
//! The line carries the envelope only: message bodies never reach the log
//! (SEC-03).

use crate::error::FrameworkError;
use crate::mail::transport::{MailTransport, OutgoingMessage};
use async_trait::async_trait;

/// Stand-in for anything that could carry a credential.
const REDACTED: &str = "[redacted]";

/// Dev-time transport that emits a `tracing::info!` line per dispatch
/// and discards the message. Useful for confirming *that* mail would be
/// sent, to whom, and with which subject, without contacting an upstream
/// provider.
///
/// The log line carries the envelope — from / to / subject — plus the body
/// sizes. It does **not** carry the bodies.
///
/// # Why the bodies are gone (SEC-03)
///
/// Laravel's `log` mailer writes the whole message, and this transport used
/// to as well: the rendered text body went straight into the log line
/// because in dev the console is the only place a verification or
/// password-reset link surfaces. That convenience is an account-takeover
/// primitive. A password-reset body contains a single-use bearer link, and
/// once it is in a log file that link is a working credential for anyone
/// who can read the file — the app's own operators, whatever ships logs
/// off-box, the retention bucket, an aggregator with a wide access policy.
/// Nothing about the link's own expiry helps: log shipping is faster than a
/// human clicking through their inbox.
///
/// So bodies are summarised by byte length and dropped. The subject still
/// goes out, passed through [`redact_url_credentials`] so a mailable that
/// interpolates a signed link into its subject cannot smuggle one through
/// the one free-text field that remains.
///
/// To read a body in development, use a transport that keeps the message
/// instead of printing it:
///
/// - `MAIL_DRIVER=memory` plus [`captured_in_memory`](crate::mail::boot::captured_in_memory)
///   or [`Mail::fake`](crate::mail::Mail::fake) in tests.
/// - `MAIL_DRIVER=smtp` pointed at a local catcher (mailpit / maildev /
///   mailhog on `127.0.0.1:1025`), which renders the real mail — links,
///   HTML, and all — in a UI that isn't your log retention.
#[derive(Default)]
pub struct LogMailTransport;

impl LogMailTransport {
    /// Construct a fresh log transport.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MailTransport for LogMailTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        let to: Vec<String> = msg.to.iter().map(|a| a.email.clone()).collect();
        tracing::info!(
            from = %msg.from.email,
            to = ?to,
            subject = %redact_url_credentials(&msg.subject),
            html_bytes = msg.html.as_ref().map_or(0, |h| h.len()),
            text_bytes = msg.text.as_ref().map_or(0, |t| t.len()),
            "mail (log driver): would send (bodies redacted)"
        );
        Ok(())
    }
    fn name(&self) -> &'static str {
        "log"
    }
}

/// Replace every URL query and fragment *value* in `input` with
/// [`REDACTED`], keeping the parameter names so the shape of the link is
/// still readable (`?token=[redacted]&expires=[redacted]`).
///
/// Why a scanner and not a URL parser: this runs over free text that may
/// hold zero, one, or several links in arbitrary punctuation, and a parse
/// failure must never mean "log it verbatim". Treating everything after the
/// first `?` or `#` of a `://` token as a credential is fail-safe by
/// construction — the worst case is that an innocuous value is hidden,
/// which costs nothing, while the reverse costs an account.
fn redact_url_credentials(input: &str) -> String {
    // `split_inclusive` keeps the terminating whitespace attached, so
    // rejoining the pieces reproduces the original spacing exactly.
    input
        .split_inclusive(char::is_whitespace)
        .map(redact_token)
        .collect()
}

/// Redact one whitespace-delimited token (plus its trailing whitespace).
fn redact_token(token: &str) -> String {
    let split_at = token.find(char::is_whitespace).unwrap_or(token.len());
    let (word, trailing_ws) = token.split_at(split_at);
    if !word.contains("://") {
        return token.to_string();
    }
    // Both delimiters are single-byte ASCII, so slicing at `cut` and `cut+1`
    // is always on a char boundary even in a multi-byte subject.
    match word.find(['?', '#']) {
        None => token.to_string(),
        Some(cut) => {
            let (head, rest) = word.split_at(cut);
            let delimiter = &rest[..1];
            let query = redact_query(&rest[1..]);
            format!("{head}{delimiter}{query}{trailing_ws}")
        }
    }
}

/// `a=1&b=2` → `a=[redacted]&b=[redacted]`.
///
/// A segment with no `=` is replaced whole: an opaque `?9f3ab12` is as much
/// a bearer credential as a named one, and the name carries no information
/// worth keeping when there isn't one.
fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _)) => format!("{name}={REDACTED}"),
            None if pair.is_empty() => String::new(),
            None => REDACTED.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    //! Pure string tests — no env, no global transport, safe to run in the
    //! parallel lib binary. The end-to-end "no reset link reaches the log"
    //! assertion lives in `framework/tests/mail_log.rs`, which needs a
    //! tracing subscriber.

    use super::*;

    #[test]
    fn a_reset_link_keeps_its_parameter_names_and_loses_its_values() {
        let out = redact_url_credentials(
            "Reset at https://app.example.com/password/reset?token=abc123&signature=deadbeef now",
        );
        assert!(
            !out.contains("abc123"),
            "token value must not survive: {out}"
        );
        assert!(
            !out.contains("deadbeef"),
            "signature value must not survive: {out}"
        );
        assert!(
            out.contains("token=[redacted]&signature=[redacted]"),
            "parameter names stay readable: {out}"
        );
        assert!(
            out.contains("https://app.example.com/password/reset?"),
            "the route itself is still identifiable: {out}"
        );
    }

    #[test]
    fn a_fragment_is_redacted_too() {
        let out = redact_url_credentials("https://app.example.com/verify#tok=s3cret");
        assert!(!out.contains("s3cret"), "{out}");
        assert!(out.contains("#tok=[redacted]"), "{out}");
    }

    #[test]
    fn an_opaque_unnamed_query_is_replaced_whole() {
        // Failure mode: a link whose credential has no `name=` to hide
        // behind. Redacting only named values would emit this verbatim.
        let out = redact_url_credentials("https://app.example.com/i/?9f3ab12deadbeef");
        assert!(!out.contains("9f3ab12"), "{out}");
        assert!(out.contains("[redacted]"), "{out}");
    }

    #[test]
    fn multiple_links_are_all_redacted_and_spacing_survives() {
        let out =
            redact_url_credentials("one https://a.test/x?t=AAA\ttwo https://b.test/y?t=BBB\nend");
        assert!(!out.contains("AAA") && !out.contains("BBB"), "{out}");
        assert_eq!(
            out,
            "one https://a.test/x?t=[redacted]\ttwo https://b.test/y?t=[redacted]\nend"
        );
    }

    #[test]
    fn text_without_links_is_passed_through_untouched() {
        for s in [
            "",
            "Verify Email Address",
            "Order #1234 shipped",
            "café ☕ — question? yes",
        ] {
            assert_eq!(redact_url_credentials(s), s, "unchanged: {s:?}");
        }
    }

    #[test]
    fn a_query_less_url_is_left_intact() {
        let s = "See https://app.example.com/docs/mail for details";
        assert_eq!(redact_url_credentials(s), s);
    }
}
