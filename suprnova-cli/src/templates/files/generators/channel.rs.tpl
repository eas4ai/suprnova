//! The `{channel}` broadcast channel.

use suprnova::async_trait;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;
use suprnova::serde_json::Value;

/// The private `{channel}` channel. Register it in the `ChannelRegistry`
/// the bootstrap binds: `registry.register({name});`.
pub struct {name};

#[async_trait]
impl Channel for {name} {
    /// The name a client subscribes to. A `{param}` segment matches one
    /// segment of the name and reaches `authorize` in `params`.
    fn name(&self) -> &'static str {
        "{channel}"
    }

    /// Whether the request may join the channel. This admits a subscriber
    /// the session authenticated on the WebSocket upgrade; check that the
    /// user may see what the channel carries.
    async fn authorize(&self, req: &Request, _params: &ChannelParams, _data: &Value) -> bool {
        req.auth_user_id().is_some()
    }
}

impl PrivateChannel for {name} {}
