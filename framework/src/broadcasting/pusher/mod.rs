//! Pusher-protocol broadcast driver.
//!
//! The driver publishes through the Pusher REST API while keeping the
//! in-process hub working, so a `ws!` endpoint in the same process keeps
//! receiving events. Channel authorization follows the Pusher signing
//! contract and the registry decides the wire name per channel.

mod auth;
mod config;
mod encryption;
mod hub;
pub(crate) mod names;
mod signing;

pub use auth::{PusherAuth, pusher_channel_auth, pusher_user_auth};
pub use config::{PusherConfig, PusherScheme};
pub use hub::PusherBroadcastHub;
