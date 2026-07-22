//! Hosted-checkout-session trait for payment providers.

use crate::payments::{
    CheckoutSessionState, PaymentError, PaymentResult, SessionPayload, StartSessionRequest,
};
use async_trait::async_trait;

/// Hosted checkout flow. Implemented by every payment provider that
/// supports a redirect, popup, embed, or out-of-band confirmation flow
/// (which is to say: all of them).
///
/// Returns a [`SessionPayload`] tagged by `flow` so the frontend SDK can
/// dispatch to the right widget — Stripe Elements, Paddle inline,
/// Mobile Money prompt, generic redirect, etc.
#[async_trait]
pub trait Checkout: Send + Sync {
    /// Start a hosted checkout session for the request.
    ///
    /// The returned [`SessionPayload`] carries the provider-specific tokens
    /// the frontend needs to complete the flow.
    async fn start_session(&self, req: StartSessionRequest) -> PaymentResult<SessionPayload>;

    /// Report the provider-side state of a previously-started session.
    ///
    /// This is the server-side verification primitive for redirect flows:
    /// a return page (or a reconciliation sweep) passes the
    /// `provider_session_id` it recorded at [`Self::start_session`] time
    /// and gets back an authoritative [`CheckoutSessionState`] — never
    /// trust the query parameters the customer's browser carried home.
    ///
    /// The default implementation returns [`PaymentError::NotSupported`];
    /// providers whose sessions can be interrogated (e.g. Stripe Checkout
    /// Sessions) override it.
    async fn session_status(
        &self,
        provider_session_id: &str,
    ) -> PaymentResult<CheckoutSessionState> {
        Err(PaymentError::NotSupported(format!(
            "session_status ({provider_session_id})"
        )))
    }
}
