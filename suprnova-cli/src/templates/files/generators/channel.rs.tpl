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

    /// Whether the request may join the channel. Nobody may until you write
    /// the rule, as Laravel's channel stub denies by default: check that the
    /// user `req.auth_user_id()` names may see what the channel carries.
    async fn authorize(&self, _req: &Request, _params: &ChannelParams, _data: &Value) -> bool {
        false
    }
}

impl PrivateChannel for {name} {}
