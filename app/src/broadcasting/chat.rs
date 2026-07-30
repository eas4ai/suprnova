//! Private chat channel — gates subscription on the authenticated
//! session, not on anything the client sends.
//!
//! This channel used to accept any subscriber whose `data` carried a
//! token starting with `"chat_"`. That is not a gate: the value comes
//! from the client's own subscribe frame, so `{"token":"chat_x"}` from
//! anyone at all passed it. It was written as a placeholder for "until
//! the auth stack covers WebSocket upgrades" — but the upgrade path
//! already runs the full middleware chain, session middleware included,
//! so the authenticated identity existed the whole time. It simply had
//! no way to reach a handler that runs after the chain unwinds, until
//! `Request::auth_user_id` carried it across that boundary.

use async_trait::async_trait;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;
use suprnova::serde_json::Value;

/// Private channel for chat rooms.
///
/// Subscribers must arrive with an authenticated session on the upgrade
/// request — the same session cookie every other route requires. There
/// is deliberately no token to put in the subscribe frame: a credential
/// the client chooses for itself is not a credential.
pub struct ChatChannel;

#[async_trait]
impl Channel for ChatChannel {
    fn name(&self) -> &'static str {
        "chat.lobby"
    }

    /// Accept subscribers the session middleware authenticated during the
    /// WebSocket upgrade.
    ///
    /// `data` is ignored entirely. Consulting the client's own frame for
    /// authorization is exactly what made the previous gate decorative.
    async fn authorize(&self, req: &Request, _params: &ChannelParams, _data: &Value) -> bool {
        req.auth_user_id().is_some()
    }

    /// Restrict publishing to this channel's own events, and only for
    /// subscribers carrying an authenticated identity.
    ///
    /// The identity check is repeated rather than inherited from
    /// [`Self::authorize`]: that call gated the subscription, while a
    /// publish frame arrives later on a long-lived socket. "They were
    /// authenticated when they connected" is a weaker claim than "this
    /// frame comes from an authenticated session".
    async fn authorize_publish(
        &self,
        req: &Request,
        _params: &ChannelParams,
        event: &str,
        _data: &Value,
    ) -> bool {
        req.auth_user_id().is_some() && matches!(event, "MessagePosted" | "Typing")
    }
}

impl PrivateChannel for ChatChannel {}

#[cfg(test)]
mod tests {
    //! The old gate accepted `{"token":"chat_<anything>"}` from anyone.
    //! These pin that identity now comes from the session and that the
    //! subscribe frame cannot influence the decision.

    use super::*;
    use suprnova::serde_json::json;

    fn upgrade_request() -> Request {
        Request::for_test("GET", "/ws/broadcast")
    }

    #[tokio::test]
    async fn an_unauthenticated_subscriber_is_refused() {
        let refused = !ChatChannel
            .authorize(
                &upgrade_request(),
                &ChannelParams::default(),
                &json!({ "token": "chat_looks_legitimate" }),
            )
            .await;

        assert!(
            refused,
            "a client-supplied token must not authorize anything; the old \
             gate accepted any string beginning with `chat_`"
        );
    }

    #[tokio::test]
    async fn an_authenticated_subscriber_is_accepted_without_any_token() {
        let req = upgrade_request().with_auth_user_id("42");

        assert!(
            ChatChannel
                .authorize(&req, &ChannelParams::default(), &json!({}))
                .await,
            "an authenticated session is the credential — there is deliberately \
             nothing to put in the subscribe frame"
        );
    }

    /// The frame must not be able to influence the outcome in either
    /// direction, so a hostile payload cannot revoke a real session and a
    /// forged one cannot mint access.
    #[tokio::test]
    async fn the_subscribe_frame_cannot_change_the_decision() {
        let authed = upgrade_request().with_auth_user_id("42");
        let anon = upgrade_request();

        for payload in [
            json!({}),
            json!({ "token": "chat_x" }),
            json!({ "token": null }),
            json!({ "user_id": "1" }),
        ] {
            assert!(
                ChatChannel
                    .authorize(&authed, &ChannelParams::default(), &payload)
                    .await,
                "an authenticated subscriber must stay authorized regardless of \
                 the frame; payload {payload}"
            );
            assert!(
                !ChatChannel
                    .authorize(&anon, &ChannelParams::default(), &payload)
                    .await,
                "an anonymous subscriber must stay refused regardless of the \
                 frame; payload {payload}"
            );
        }
    }

    /// Publishing re-checks identity rather than trusting that `authorize`
    /// ran earlier — a publish frame arrives later on a long-lived socket.
    #[tokio::test]
    async fn publishing_requires_identity_and_a_known_event() {
        let authed = upgrade_request().with_auth_user_id("42");
        let anon = upgrade_request();
        let params = ChannelParams::default();

        assert!(
            ChatChannel
                .authorize_publish(&authed, &params, "MessagePosted", &json!({}))
                .await
        );
        assert!(
            !ChatChannel
                .authorize_publish(&anon, &params, "MessagePosted", &json!({}))
                .await,
            "an unauthenticated socket must not publish"
        );
        assert!(
            !ChatChannel
                .authorize_publish(&authed, &params, "AdminBroadcast", &json!({}))
                .await,
            "the event allowlist still applies to authenticated subscribers"
        );
    }
}
