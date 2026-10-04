//! Channel and user authorization endpoints for Pusher-protocol
//! clients (pusher-js, Laravel Echo).

use super::config::PusherConfig;
use super::names::{WireKind, strip_prefix, wire_name};
use super::{encryption, signing};
use crate::broadcasting::ChannelRegistry;
use crate::http::{HttpResponse, Request, Response};
use crate::{App, Auth, FrameworkError, ValidationErrors};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// The state the authorization endpoints sign with: the driver's
/// configuration and the channel registry the hub names channels by.
///
/// Get it from [`PusherBroadcastHub::auth`](crate::PusherBroadcastHub::auth)
/// and bind it with `App::singleton(hub.auth())`, so the endpoints and
/// the hub can never disagree about a channel's wire name.
#[derive(Clone)]
pub struct PusherAuth {
    config: Arc<PusherConfig>,
    registry: Arc<ChannelRegistry>,
}

impl PusherAuth {
    /// Built only by the hub, from the hub's own config and registry.
    pub(crate) fn new(config: Arc<PusherConfig>, registry: Arc<ChannelRegistry>) -> Self {
        Self { config, registry }
    }
}

/// The body cap for an authorization request. Pusher clients send two
/// short fields; the cap matches the one the CSRF middleware puts on
/// form bodies.
const AUTH_BODY_LIMIT: usize = 64 * 1024;

/// The longest `socket_id` accepted, in bytes.
const MAX_SOCKET_ID_BYTES: usize = 50;

/// The longest `channel_name` accepted, in bytes: Pusher's 164-character
/// limit on channel names.
const MAX_CHANNEL_NAME_BYTES: usize = 164;

/// `true` when `socket_id` matches `^[0-9]+\.[0-9]+$` and is at most 50
/// bytes.
///
/// The signed string is `{socket_id}:{channel_name}[:{channel_data}]`.
/// Refusing `:` here (and in the channel name) is what stops a caller
/// from shifting the field boundaries to get a different string signed.
fn is_valid_socket_id(socket_id: &str) -> bool {
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    socket_id.len() <= MAX_SOCKET_ID_BYTES
        && socket_id
            .split_once('.')
            .is_some_and(|(left, right)| digits(left) && digits(right))
}

/// `true` when `channel_name` matches `^[A-Za-z0-9_\-=@,.;]+$` and is
/// at most 164 bytes.
fn is_valid_channel_name(channel_name: &str) -> bool {
    !channel_name.is_empty()
        && channel_name.len() <= MAX_CHANNEL_NAME_BYTES
        && channel_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-=@,.;".contains(&b))
}

/// The fields of an authorization request body.
struct AuthBody {
    socket_id: Option<String>,
    channel_name: Option<String>,
    /// Every other field, as a JSON object: the `data` handed to
    /// [`Channel::authorize`](crate::broadcasting::Channel::authorize).
    data: Value,
}

impl AuthBody {
    /// Read a buffered form or JSON body. Any other body, or a JSON body
    /// that is not an object, has no fields, so validation refuses it.
    fn read(req: &Request) -> Self {
        if req.is_json() {
            let mut object = match req
                .cached_body()
                .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            {
                Some(Value::Object(object)) => object,
                _ => Map::new(),
            };
            let mut take = |name: &str| match object.remove(name) {
                Some(Value::String(value)) => Some(value),
                _ => None,
            };
            let socket_id = take("socket_id");
            let channel_name = take("channel_name");
            return Self {
                socket_id,
                channel_name,
                data: Value::Object(object),
            };
        }

        // `cached_form_field` answers only for a form body and resolves
        // a repeated field to its last occurrence; the remaining fields
        // follow the same rule, so `authorize` sees what was validated.
        let mut data = Map::new();
        if is_form(req)
            && let Some(bytes) = req.cached_body()
        {
            for (name, value) in url::form_urlencoded::parse(bytes) {
                if name != "socket_id" && name != "channel_name" {
                    data.insert(name.into_owned(), Value::String(value.into_owned()));
                }
            }
        }
        Self {
            socket_id: req.cached_form_field("socket_id"),
            channel_name: req.cached_form_field("channel_name"),
            data: Value::Object(data),
        }
    }
}

/// Whether the request carries a form-urlencoded body.
fn is_form(req: &Request) -> bool {
    req.header("content-type")
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"))
}

/// Resolve the bound [`PusherAuth`], or a 500 that says how to bind it.
fn bound_auth() -> Result<PusherAuth, HttpResponse> {
    App::get::<PusherAuth>().ok_or_else(|| {
        FrameworkError::internal(
            "no PusherAuth is bound in the container: bind the hub's with \
             App::singleton(hub.auth()) (PusherBroadcastHub::auth()) in bootstrap",
        )
        .into()
    })
}

/// 403 with an empty JSON object, the answer Pusher clients expect for
/// a refused subscription.
fn forbidden() -> HttpResponse {
    HttpResponse::json(json!({})).status(403)
}

/// A 422 in the framework's validation error shape.
fn invalid(fields: &[(&str, &str)]) -> HttpResponse {
    let mut errors = ValidationErrors::new();
    for (field, message) in fields {
        errors.add(*field, *message);
    }
    FrameworkError::validation_errors(errors).into()
}

const SOCKET_ID_MESSAGE: &str =
    "The socket_id must be digits, a dot, and digits, at most 50 bytes long.";
const CHANNEL_NAME_MESSAGE: &str = "The channel_name may contain only letters, digits and \
                                    _ - = @ , . ; and be at most 164 bytes long.";

/// Serialize authorization data, which cannot fail for a `Value` built
/// here; the error is mapped instead of unwrapped.
fn to_json_string(value: &Value) -> Result<String, FrameworkError> {
    serde_json::to_string(value)
        .map_err(|e| FrameworkError::from_external_with("serializing Pusher auth data failed", e))
}

/// The Pusher channel authorization endpoint. Mount it at
/// `/broadcasting/auth` inside the session and CSRF middleware group,
/// with [`PusherAuth`] bound in the container.
///
/// It reads `socket_id` and `channel_name` from a form or JSON body and
/// answers:
///
/// - 422 when either field is malformed;
/// - 403 (`{}`) for a public (unprefixed) name, an unknown channel, a
///   prefix that does not match the channel's wire name, a refused
///   [`Channel::authorize`](crate::broadcasting::Channel::authorize), or
///   a presence channel without a logged-in user;
/// - 200 with `auth`, plus `shared_secret` for an encrypted channel or
///   `channel_data` for a presence channel;
/// - 500 when an encrypted channel is requested and no master key is
///   configured, or when presence `member_info` fails.
///
/// The other body fields reach `authorize` as its `data`.
pub async fn pusher_channel_auth(req: Request) -> Response {
    let auth = bound_auth()?;
    let req = req.buffer_body(AUTH_BODY_LIMIT).await?;
    let body = AuthBody::read(&req);

    let socket_id = body.socket_id.as_deref().filter(|s| is_valid_socket_id(s));
    let channel_name = body
        .channel_name
        .as_deref()
        .filter(|c| is_valid_channel_name(c));
    let (Some(socket_id), Some(channel_name)) = (socket_id, channel_name) else {
        let mut fields = Vec::new();
        if socket_id.is_none() {
            fields.push(("socket_id", SOCKET_ID_MESSAGE));
        }
        if channel_name.is_none() {
            fields.push(("channel_name", CHANNEL_NAME_MESSAGE));
        }
        return Err(invalid(&fields));
    };

    let Some((requested, name)) = strip_prefix(channel_name) else {
        return Err(forbidden());
    };
    let Some((channel, params)) = auth.registry.resolve(name) else {
        return Err(forbidden());
    };
    // `wire_name` is the one place that decides wire names, and the hub
    // publishes under it. It refuses every name that would not read back
    // through `strip_prefix` as itself, so each wire name belongs to
    // exactly one channel and one kind. Requiring the computed name to
    // equal the requested one therefore refuses any other name for this
    // channel (`private-` for a public channel, plain `private-` for an
    // encrypted one), and a name the hub refuses to publish under is
    // never signed either.
    if !wire_name(&auth.registry, name).is_ok_and(|wire| wire == channel_name) {
        return Err(forbidden());
    }
    if !channel.authorize(&req, &params, &body.data).await {
        return Err(forbidden());
    }

    let key = auth.config.key.as_str();
    let secret = auth.config.secret();
    match requested {
        WireKind::Private => {
            let signature = signing::channel_auth(key, secret, socket_id, channel_name, None)?;
            Ok(HttpResponse::json(json!({ "auth": signature })))
        }
        WireKind::Encrypted => {
            let master_key = auth.config.encryption_master_key().ok_or_else(|| {
                FrameworkError::internal(format!(
                    "Pusher authorization for encrypted channel '{channel_name}' failed: no \
                     encryption master key is configured"
                ))
            })?;
            let signature = signing::channel_auth(key, secret, socket_id, channel_name, None)?;
            let shared_secret =
                STANDARD.encode(encryption::shared_secret(channel_name, master_key));
            Ok(HttpResponse::json(json!({
                "auth": signature,
                "shared_secret": shared_secret,
            })))
        }
        WireKind::Presence => {
            // The route's user: behind `AuthMiddleware::for_guard(name)`,
            // that guard's user as `<guard>:<id>`, never the default
            // guard's user in the same session.
            let Some(user_id) = Auth::route_principal().await? else {
                return Err(forbidden());
            };
            let Some(presence) = channel.presence_info() else {
                return Err(forbidden());
            };
            let user_info = presence.member_info(&req, &params).await.map_err(|e| {
                FrameworkError::from_external_with(
                    format!("presence member_info for '{channel_name}' failed"),
                    e,
                )
            })?;
            // Serialized once: the string signed is the string returned.
            let channel_data = to_json_string(&json!({
                "user_id": user_id,
                "user_info": user_info,
            }))?;
            let signature =
                signing::channel_auth(key, secret, socket_id, channel_name, Some(&channel_data))?;
            Ok(HttpResponse::json(json!({
                "auth": signature,
                "channel_data": channel_data,
            })))
        }
    }
}

/// The Pusher user authentication endpoint (`signin()` in pusher-js).
/// Mount it at `/broadcasting/user-auth` inside the session and CSRF
/// middleware group, with [`PusherAuth`] bound in the container.
///
/// It reads `socket_id` like [`pusher_channel_auth`] and answers 422
/// when it is malformed, 403 (`{}`) without a logged-in user, and
/// otherwise 200 with `auth` and `user_data` (`{"id": <user id>}`).
///
/// The user is the route's: the user of the guard the last
/// `AuthMiddleware` checked, as its bare id for the default guard and as
/// `<guard>:<id>` for any other. A presence member's `user_id` from
/// [`pusher_channel_auth`] follows the same rule.
pub async fn pusher_user_auth(req: Request) -> Response {
    let auth = bound_auth()?;
    let req = req.buffer_body(AUTH_BODY_LIMIT).await?;
    let body = AuthBody::read(&req);
    let Some(socket_id) = body.socket_id.as_deref().filter(|s| is_valid_socket_id(s)) else {
        return Err(invalid(&[("socket_id", SOCKET_ID_MESSAGE)]));
    };
    let Some(user_id) = Auth::route_principal().await? else {
        return Err(forbidden());
    };
    let user_data = to_json_string(&json!({ "id": user_id }))?;
    let signature = signing::user_auth(
        auth.config.key.as_str(),
        auth.config.secret(),
        socket_id,
        &user_data,
    )?;
    Ok(HttpResponse::json(json!({
        "auth": signature,
        "user_data": user_data,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pusher_socket_id_validation_matches_the_pattern() {
        for valid in ["1234.1234", "0.0", "1.2", &format!("{}.1", "9".repeat(48))] {
            assert!(is_valid_socket_id(valid), "{valid} is valid");
        }
        for invalid in [
            "",
            "1234",
            "1234.",
            ".1234",
            "12.34.56",
            "1234.1234:evil",
            "1234.1234\n",
            "a.1",
            "1.a",
            "-1.1",
            " 1.1",
            "１.1",
            // 51 bytes, otherwise valid.
            &format!("{}.1", "9".repeat(49)),
        ] {
            assert!(!is_valid_socket_id(invalid), "{invalid:?} is invalid");
        }
    }

    #[test]
    fn pusher_channel_name_validation_matches_the_pattern() {
        for valid in [
            "private-orders.42",
            "presence-chat",
            "AZaz09_-=@,.;",
            &"a".repeat(164),
        ] {
            assert!(is_valid_channel_name(valid), "{valid} is valid");
        }
        for invalid in [
            "",
            "private-orders:42",
            "private orders",
            "private-#1",
            "private-orders/42",
            "private-ordérs",
            "private-orders\n",
            &"a".repeat(165),
        ] {
            assert!(!is_valid_channel_name(invalid), "{invalid:?} is invalid");
        }
    }
}
