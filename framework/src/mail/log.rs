//! Log mail transport — emits a `tracing::info!` per send and discards.
//! The line carries the whole message, bodies included, the same way
//! Laravel's `log` mailer does.

use crate::error::FrameworkError;
use crate::mail::transport::{MailTransport, OutgoingMessage};
use async_trait::async_trait;

/// Dev-time transport that logs the message and discards it.
///
/// Mirrors Laravel's `log` mailer: the line carries the envelope — from /
/// to / subject — **and the rendered bodies**. That is the point of the
/// driver. In development the console is where you read the verification
/// or password-reset link the app just "sent", and a transport that hides
/// it is a transport nobody can use.
///
/// # Why logging bearer links here is safe (SEC-03)
///
/// A password-reset body contains a single-use bearer link, and a log file
/// holding one is a working credential for anyone who can read it. The
/// protection is not to cripple this driver — it is to keep the driver out
/// of the environments where that matters:
/// [`bootstrap_from_env`](crate::mail::boot::bootstrap_from_env) refuses to
/// boot a production app on the `log` driver at all (likewise `memory`,
/// unknown, and unset). So the bodies written here only ever exist on a
/// developer's machine.
///
/// If you deliberately override that refusal in a deployed environment,
/// you are choosing to put reset links in your logs — size your log
/// retention and access policy accordingly, or point `MAIL_DRIVER=smtp`
/// at a local catcher (mailpit / maildev / mailhog on `127.0.0.1:1025`),
/// which renders the real mail in a UI instead.
///
/// For assertions in tests, prefer [`Mail::fake`](crate::mail::Mail::fake)
/// or `MAIL_DRIVER=memory` with
/// [`captured_in_memory`](crate::mail::boot::captured_in_memory) — those
/// hand you the message as a value rather than making you scrape a log.
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
        // `info!`, where Laravel logs at debug: this driver exists so a
        // developer can read the link off their console, and a line that
        // needs `RUST_LOG=debug` to appear would not be found by the person
        // who most needs it.
        tracing::info!(
            from = %msg.from.email,
            to = ?to,
            subject = %msg.subject,
            text = msg.text.as_deref().unwrap_or(""),
            html = msg.html.as_deref().unwrap_or(""),
            "mail (log driver): would send"
        );
        Ok(())
    }
    fn name(&self) -> &'static str {
        "log"
    }
}
